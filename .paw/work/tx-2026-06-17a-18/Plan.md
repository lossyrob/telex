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
- `State`: drop `cursor`; add `seen: Mutex<HashSet<i64>>` — a **monotonic** "ids this holder has
  ever queued" guard (never pruned; see Synchronization below).
- Replace `drain_new` with `drain`: `fetch_undelivered(address)`, then under the `seen` lock, for
  each row, enqueue **iff `seen.insert(row.id)` returns true** (newly inserted); `notify_waiters()`
  if anything was queued.
- Startup: remove `start_cursor = max_id` and the separate `undelivered_backlog` seeding block; run
  one initial `drain` (after the heartbeat task is live) — it naturally fetches the full backlog.
  The poll task and the optional push task call the same `drain`.
- `handle_conn`: unchanged delivery/mark/requeue flow — **no `seen` pruning** (see below). The
  durable `deliveries` mark still prevents cross-restart redelivery; `seen` prevents intra-holder
  re-queue.

### Synchronization — why monotonic `seen` is race-free (resolves planning-review blocking #1)
Both planning reviewers flagged a TOCTOU if `seen` were pruned on `mark_delivered`: a `drain` that
snapshotted `fetch_undelivered` (row N, no delivery record yet) *before* a concurrent waiter marked
N delivered and pruned N from `seen` would then insert N and re-queue it → a **new steady-state
duplicate** the cursor model never had. We close this by **never pruning `seen`**:
- Once N is queued, `N ∈ seen` for the holder's lifetime. Any later/stale drain that fetched N
  pre-mark finds `N ∈ seen` and skips it. No duplicate.
- Concurrent poll+push drains both fetch N; the `HashSet::insert` under the `seen` mutex is the
  serialization point — exactly one returns `true` and enqueues. No duplicate.
- No drain ever needs to re-queue an already-seen id: the only re-queue path is the
  write-failure front-requeue of the *same* buffered item (id stays in `seen`). No loss.
This is simpler than pruning (no handoff-path bookkeeping) and holds **no lock across DB I/O**.
Trade-off: `seen` grows by one `i64` per distinct message this holder queues over its lifetime.
At telex's scale (single-user, pre-beta, session-bound holders that restart regularly) this is
negligible (~tens of MB even at 1M messages); recorded as a known bound with a deferred prune
option in DECISIONS 0011.

### Behavior delta to call out
Previously the live path queued any `id > cursor` regardless of disposition; now the live drain
excludes a message whose latest disposition is already terminal (e.g. `telex handle` via `inbox`
before any waiter popped it). This makes the live path consistent with the backlog path and with
"don't deliver an already-handled message"; it is a deliberate, minor improvement, not a regression.

### Performance note (resolves planning-review non-blocking; recorded in 0011)
`fetch_undelivered` has **no id floor**, so each poll/push tick scans an address's messages and
anti-joins `deliveries` (unique `(message_id, recipient)`) plus the latest-disposition correlated
subquery (`dispositions_msg_idx`). Cost is O(address history), not O(new) like the old cursor seek.
Acceptable at telex's scale, and the undelivered result set stays small (anti-join). A naive
low-water floor is **deliberately not** added: advancing a floor to the max delivered/visible id
would re-introduce exactly this bug, because a late-committing lower id sits *below* that floor and
breaks contiguity. A safe floor needs the contiguous-delivered-prefix accounting for the in-flight
commit horizon (snapshot `xmin`) — the same complexity Option A was rejected for — so it (and an
optional `dispositions(message_id, recipient, id)` index / partial undelivered index) is deferred
and recorded in DECISIONS 0011 as the eventual mitigation.

## Validation (Postgres is NOT in CI — issue #19)

Green CI is necessary but **not sufficient**; CI has no real Postgres. Layers:
1. **Backend conformance** (`tests/conformance.rs`, SQLite always + Postgres when `TELEX_PG_URL`
   set): a scenario asserting `fetch_undelivered` returns a **lower undelivered id even after a
   higher id is `mark_delivered`** (the gap-closing invariant), plus terminal-disposition exclusion
   and address scoping. Replaces `cursor_delivery`; folds in `delivery_backlog`.
2. **Postgres MVCC out-of-order-commit test** (PG-only, gated by `TELEX_PG_URL`): a **separate
   `#[tokio::test]` in `postgres_fixture`** (not inside `run_all`, which only hands a `Store`). It
   parses `cfg`/schema like `postgres_conformance`, then opens **two independent `tokio_postgres`
   connections** (search_path = test schema) and forces reverse-id commit order with minimal raw
   inserts (`INSERT INTO {schema}.messages(to_addr, body, sent_at_ms, created_at_ms) ...`;
   `thread_id` NULL is fine — `map_message` falls back to `id`):
   - conn A: `BEGIN; INSERT ... RETURNING id` → `idA` (hold open, uncommitted/invisible).
   - conn B: `BEGIN; INSERT ... RETURNING id; COMMIT` → `idB` (`idB > idA`).
   - fresh `PgBackend.fetch_undelivered` returns only `idB`; `mark_delivered(idB)`.
   - conn A: `COMMIT`. Now `fetch_undelivered` returns `idA` — the late lower id, deliverable live.
   - **Contrast** with an inline raw `SELECT id FROM {schema}.messages WHERE to_addr=$1 AND id>$idB`
     (NOT the removed `fetch_after`): it returns nothing — i.e. the old cursor model would skip
     `idA`. This is the faithful reproduction.
3. **Holder-level tests** (`attach.rs` tests, SQLite, deterministic, run in CI): the headline test
   asserts a waiter **actually receives** the lower undelivered id over the `duplex` harness (the
   literal acceptance bar): mark a higher id delivered, run `drain`, connect a waiter, assert it is
   handed the lower id. Plus: drain **idempotency** (a second `drain` does not re-queue a queued or a
   delivered id — the monotonic-`seen` + deliveries guarantees), terminal-disposition **live
   exclusion**, and the existing write-failure path extended to assert no duplicate after a
   subsequent `drain`.
Document exactly how to run layer 2 locally; flag the CI gap (refs #19) in PR + field report.

## Docs / decisions
- **DECISIONS.md 0011** (append-only, next number after 0010): record the rethink — live-holder
  visibility via per-recipient delivery state; **supersede** (do not rewrite) the live-drain cursor
  clause of 0005/0010, set their Status to note the supersede. Record: the monotonic-`seen`
  trade-off, the O(history) per-tick cost, and why a safe low-water floor is deferred. Note that the
  validated-trait-surface enumerations in 0006/0010 (`max_id`, `fetch_after`, `undelivered_backlog`)
  are superseded by their removal.
- **DESIGN.md**: update the holder "poll-with-cursor" delivery description (the load-bearing spot,
  ~line 507) to "poll the undelivered set (deliveries-table authority)"; reference 0011. Minimal,
  honest edits.
- **Conformance coverage checklist** (`tests/conformance.rs:96-97`): remap to
  `mark_delivered / fetch_undelivered` and the renamed scenario.
- **Docs.md** block for the PR body (paw-docs-guidance).

## Planning-docs review outcome (gpt-5.5 + claude-opus-4.8)
Direction APPROVED by both. Blocking findings resolved above: (1) the `seen` TOCTOU duplicate →
monotonic `seen`, never pruned, atomic check-insert; (2) PG test must not call the removed
`fetch_after` → inline raw cursor SQL for the contrast; (3) PG test feasibility → separate gated
`#[tokio::test]` with two raw connections + minimal inserts. Non-blocking items (per-tick cost
acknowledgement, expanded tests incl. end-to-end waiter delivery + terminal-exclusion + idempotency,
doc consistency) folded in. Trait removal confirmed safe (only `attach.rs` + conformance call them;
`spike/` is a separate crate).

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
