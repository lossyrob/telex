# Plan — Issue #18: Robust live-holder message visibility for Postgres

Work ID: `tx-2026-06-17a-18` · Base: `main` (on merged #15 / decision 0010) · Outcome anchor:
**the live holder delivers a concurrently-committed lower id without a restart.**

## Problem (root cause)

The holder's live drain (`drain_new` in `src/commands/attach.rs`) tracks delivery with a single
in-memory **high-water cursor** over `bigserial` ids and fetches with `fetch_after`
(`WHERE id > cursor`). On Postgres an id is *allocated* at insert time but only becomes *visible*
at commit time, and concurrent transactions can commit out of id order. So:

1. T_a inserts → gets id=N (uncommitted, invisible). T_b inserts → id=N+1.
2. T_b commits. Drain runs, sees N+1, advances cursor to N+1.
3. T_a commits. N is now visible — but `id > N+1` is false, so the **live holder skips N forever**.

#10/PR #15 (the durable `deliveries` table) made a holder **restart** recover N via
`undelivered_backlog`, but the **live-holder window** (skip → next restart) is exactly what #18
asks us to close. SQLite serializes writes (commit order == id order) so it is unaffected, but the
fix is applied uniformly through the shared `Backend` trait so there is one code path.

## Candidate approaches (issue is "open to design")

**A. Snapshot-aware / commit-ordered cursor.** Don't advance the cursor past the oldest in-flight
transaction (`pg_snapshot_xmin(pg_current_snapshot())` / row `xmin`). *Rejected:* Postgres-specific
MVCC code (xid wraparound, xid→id mapping), a backend-divergent path, and it does not actually
remove the core flaw — a pure cursor fundamentally cannot both deliver a high id *now* and still
re-detect a late lower id later without re-delivering the high id. It needs per-message dedup
anyway, so it's strictly more complex than C for no extra robustness.

**B. Periodic catch-up scan layered on the cursor.** Keep `fetch_after` as primary; add a periodic
scan of undelivered `id <= cursor`. *Rejected:* this is the "narrow the window with a scan" pattern
the issue explicitly warns against — it keeps the brittle high-water filter as the correctness
mechanism and bolts a second code path on top to paper over it.

**C. Per-message delivery state as the authority (CHOSEN).** Stop using id ordering to answer "have
I delivered this?". The durable `deliveries` table from #10 already records every holder→waiter
handoff per recipient. Make the live drain select **undelivered, non-terminal** messages
(`NOT EXISTS` in `deliveries` AND latest disposition not terminal), ordered by id — i.e. the exact
`undelivered_backlog` predicate, but with **no id bound**. The in-memory high-water cursor is
deleted; intra-holder dedup (don't re-queue a message already buffered but not yet handed off) is a
small in-memory `seen` set of in-flight ids, pruned when delivery is marked.

The issue itself nominates this ("track a delivered set / per-message delivery state … rather than a
single high-water mark") and it lets us *delete* the brittle mechanism instead of patching around it.

## Robustness argument (why a late lower id can no longer be skipped)

Whether a message is queued depends on exactly two durable facts — *does it have a delivery record?*
and *is its latest disposition terminal?* — **never** on its id relative to any cursor. MVCC commit
order is therefore irrelevant: the instant a lower id N becomes visible (its txn commits), it is by
definition an undelivered, non-terminal message, so the **next drain tick** (the existing poll
backstop every `poll_secs`, or immediately on a Postgres `LISTEN/NOTIFY` push) selects and queues
it. There is no value of any cursor that can exclude N, because there is no cursor. The only
remaining latency is bounded by the poll interval (a *latency* window, not a *loss/skip correctness*
window) — and the in-memory `seen` set is what prevents a message that is already buffered from
being re-queued by an overlapping drain. This is the acceptance bar: **live delivery of the
out-of-order lower id with no restart.**

Where brittleness would remain (and does not): there is no consecutive-id assumption left in the
holder's delivery path; the only ordering still used is `ORDER BY id` for *presentation* order of
the queue, which is cosmetic, not a correctness gate.

## Design details

### Backend trait (`src/backend/mod.rs`, `postgres.rs`, `sqlite.rs`)
- **Add** `fetch_undelivered(&self, address) -> Vec<MessageRow>`: messages to `address` with no
  delivery record for that recipient AND latest disposition not terminal, ordered by id. (Same SQL
  as `undelivered_backlog` minus the `id <= upto_id` clause.)
- **Remove** `fetch_after`, `max_id`, and `undelivered_backlog`. All three existed solely to serve
  the high-water-cursor delivery model this issue replaces; leaving them keeps the footgun's
  scaffolding in the trait. (Greenfield, single-user, pre-beta — clean removal over dead code.)

### Holder (`src/commands/attach.rs`)
- `State`: drop `cursor`; add `seen: Mutex<HashSet<i64>>` (ids currently buffered / in the
  pop→mark handoff window).
- Replace `drain_new` with `drain`: `fetch_undelivered(address)`, then for each row not already in
  `seen`, insert into `seen` and push onto the queue; `notify_waiters()` if anything was queued.
- Startup: remove `start_cursor = max_id` and the separate `undelivered_backlog` seeding block; run
  one initial `drain` (after the heartbeat task is live) — it naturally fetches the full backlog.
  The poll task and the optional push task call the same `drain`.
- `handle_conn`: on a **successful** `mark_delivered`, remove the id from `seen` (keeps `seen`
  bounded to in-flight ids; the deliveries table now excludes it from `fetch_undelivered`, so no
  drain can re-queue it). On a write failure the id stays in `seen` (the message is requeued at the
  front, still in flight). On a `mark_delivered` *failure* (logged, delivery still reported success)
  the id stays in `seen` so this holder won't redeliver — matching the existing at-least-once
  contract.

### Behavior delta to call out
Previously the live path queued any `id > cursor` regardless of disposition; now the live drain
excludes a message whose latest disposition is already terminal (e.g. `telex handle` via `inbox`
before any waiter popped it). This makes the live path consistent with the backlog path and with
"don't deliver an already-handled message"; it is a deliberate, minor improvement, not a regression.

## Validation (Postgres is NOT in CI — issue #19)

Green CI is necessary but **not sufficient**; CI has no real Postgres. Three layers:
1. **Backend conformance** (`tests/conformance.rs`, SQLite always + Postgres when `TELEX_PG_URL`
   set): a scenario asserting `fetch_undelivered` returns a **lower undelivered id even after a
   higher id is `mark_delivered`** — the backend-level invariant that closes the gap.
2. **Postgres MVCC out-of-order-commit test** (PG-only, gated by `TELEX_PG_URL`): use raw
   `tokio_postgres` transactions to force two inserts to **commit in reverse id order** (hold T_a@N
   open, commit T_b@N+1, mark N+1 delivered, then commit T_a) and assert `fetch_undelivered` now
   returns N — while the old `id > cursor` query would not. This is the faithful reproduction.
3. **Holder-level test** (`attach.rs` tests, SQLite, deterministic, runs in CI): mark a higher id
   delivered, run `drain`, assert the lower undelivered id is queued (the live holder no longer
   skips it). Reproduces the *consequence* without needing real concurrency.
Document exactly how to run layer 2 locally; flag the CI gap (refs #19) in PR + field report.

## Docs / decisions
- **DECISIONS.md 0011** (append-only, next number): record the rethink — live-holder visibility via
  per-recipient delivery state; supersede the live-drain cursor clause of 0010/0005.
- **DESIGN.md**: update the holder "poll-with-cursor" delivery description to "poll the undelivered
  set (deliveries-table authority)"; reference 0011. Minimal, honest edits to load-bearing spots.
- **Docs.md** block for the PR body (paw-docs-guidance).

## Work items (SQL todos)
- `backend-fetch-undelivered` — trait + postgres + sqlite: add `fetch_undelivered`, remove
  `fetch_after`/`max_id`/`undelivered_backlog`.
- `holder-drain` — rewrite `drain`, `State`, startup seeding, `handle_conn` seen-pruning.
- `tests` — conformance scenario rewrite + checklist, PG MVCC test, holder-level test, fix existing
  attach tests.
- `docs-decisions` — DECISIONS 0011, DESIGN.md, Docs.md.

Sequential (one engineer): backend → holder → tests → docs. Small, tightly coupled diff; no fleet
parallelism warranted.

## Open-PR overlap (for field report)
PRs #16 (issue #4) and #17 (issue #5) are open and also touch the holder/lease/IPC (drain loop,
shutdown). This PR rewrites the drain loop and `State`, so expect reconcile/rebase conflicts in
`attach.rs`. We branch from `main` without them and do not incorporate them.
