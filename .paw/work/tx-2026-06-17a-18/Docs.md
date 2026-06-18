# Robust live-holder message visibility for Postgres (issue #18)

## Overview

The telex holder delivers messages to waiters by polling the backend and buffering new
messages locally. Previously that drain tracked "what have I already delivered?" with a single
in-memory **high-water cursor** over `bigserial` ids (`fetch_after`: `WHERE id > cursor`). On
Postgres this is unsound: an id is *allocated* at insert time but only becomes *visible* at commit
time, and concurrent transactions can commit out of id order. An id allocated before — but committed
after — a higher id becomes visible *behind* the cursor and was **skipped by the live holder** until
the next restart (issue #10/PR #15 made a restart recover it via the durable `deliveries` table, but
the live-holder window remained). SQLite serializes writes, so commit order == id order and it was
never affected.

This work makes the live holder's visibility depend on **per-recipient delivery state** (the
`deliveries` table), never on id ordering. A concurrently-committed lower id is now delivered by the
**live** holder with no restart.

## Architecture and Design

### High-Level Architecture

- New backend method `Backend::fetch_undelivered(address)` returns every message addressed to
  `address` that has **no delivery record** for that recipient and whose **latest disposition is not
  terminal**, ordered by id. There is no id floor.
- The holder runs a single `drain` (used by poll, optional LISTEN/NOTIFY push, and startup) that
  calls `fetch_undelivered` and enqueues whatever it returns, deduplicated by an in-memory
  `seen: HashSet<i64>`.
- The in-memory high-water cursor is removed, and with it the now-obsolete `fetch_after`, `max_id`,
  and `undelivered_backlog` backend methods. Startup backlog recovery and the live drain are now the
  same query.

### Design Decisions

- **Delivery-state authority over a cursor (vs. snapshot-aware cursor / periodic catch-up scan).**
  A cursor cannot both deliver a high id now and re-detect a late lower id later without
  re-delivering the high id, so any robust fix needs per-message delivery state — which #10 already
  provides. A snapshot-aware (`xmin`) cursor adds Postgres-specific MVCC complexity for no extra
  robustness; a catch-up scan layered on the cursor only narrows the window. See DECISIONS 0011.
- **`seen` is monotonic (never pruned).** It is a "queued at most once per holder lifetime" guard;
  the `HashSet::insert` under its lock serializes concurrent drains. Pruning on `mark_delivered`
  would re-open a TOCTOU where a drain whose `fetch_undelivered` snapshot predates a concurrent mark
  re-queues the just-pruned id (a duplicate the cursor model never had). The durable delivery record
  — not `seen` — prevents redelivery across restarts.
- **Robustness argument.** Queueing depends only on a delivery record (primary) and a terminal
  disposition (secondary), never on id order. The moment a lower id commits it is, by definition,
  undelivered and non-terminal, so the next drain tick queues it. No cursor value can exclude it
  because there is no cursor.

### Integration Points

Builds directly on the `deliveries` table and `mark_delivered` from #10/PR #15 (decision 0010),
promoting that table from a restart-recovery aid to the live drain's source of truth.

## User Guide

No user-facing CLI or behavior change for normal use: `telex attach` (the holder) and `telex wait`
work exactly as before. The only observable difference is correctness — on Postgres, messages sent
concurrently are no longer delayed-until-restart.

One intentional, minor behavior change: a message whose **latest disposition is already terminal**
(e.g. someone ran `telex handle` via `telex inbox` before any drain has queued it) is no longer
handed to a waiter by the live drain — consistent with how the restart-backlog path already behaved.
A message already buffered in the holder's queue is still delivered (the handoff does not re-check
disposition).

## API Reference

### `Backend::fetch_undelivered(address: &str) -> Result<Vec<MessageRow>>`

Returns the address's undelivered (no `deliveries` record), non-terminal messages, ordered by id.
Replaces `fetch_after` + `undelivered_backlog` for the holder's drain. Implemented for both SQLite
and Postgres with the same predicate.

Removed: `Backend::fetch_after`, `Backend::max_id`, `Backend::undelivered_backlog`.

## Testing

### How to Test

- SQLite + holder logic run in normal CI: `cargo test`.
- **Postgres is not in CI (issue #19),** so the core behavior is validated by gated tests run
  locally against a real Postgres:
  ```
  docker run -d --name pg -e POSTGRES_PASSWORD=telex -e POSTGRES_USER=telex -e POSTGRES_DB=telex -p 55432:5432 postgres:16
  $env:TELEX_PG_URL="postgres://telex:telex@localhost:55432/telex"; $env:TELEX_PG_REQUIRE="1"
  cargo test --test conformance
  ```
  `TELEX_PG_REQUIRE=1` turns a missing URL into a failure so the Postgres leg cannot silently skip.

### Edge Cases

- `postgres_out_of_order_commit_delivers_lower_id` (PG-only): forces two transactions to commit in
  **reverse id order** on independent connections; asserts `fetch_undelivered` returns the late lower
  id after the higher id is delivered, while a raw `id > <delivered>` cursor query returns nothing.
- `live_drain_delivers_lower_id_behind_a_delivered_higher_id` (SQLite, deterministic): a waiter
  actually **receives** the lower id over the IPC harness with no restart.
- Drain idempotency, terminal-disposition exclusion, and the write-failure requeue path are covered
  by holder-level tests; delivery/disposition interplay is covered in conformance.

## Limitations and Future Work

- **Per-tick cost is O(address history), not O(new).** `fetch_undelivered` has no id floor, so each
  poll/push tick anti-joins `deliveries` and the latest-disposition subquery over the address's
  messages. Acceptable at telex's single-user, pre-beta scale.
- **A safe id floor is deferred.** A naive floor (max delivered/visible id) would re-open the bug,
  since a late-committing lower id sits below it. A correct floor needs the contiguous-delivered
  prefix accounting for the in-flight commit horizon (snapshot `xmin`); an optional
  `dispositions(message_id, recipient, id)` / partial-undelivered index is the eventual mitigation.
- **`seen` grows by one `i64` per distinct message queued during a holder's lifetime** (negligible;
  holders are session-bound and restart regularly). A bounded prune is left for the watermark work.
- **CI gap:** real Postgres is not exercised in CI (issue #19); green CI is necessary but not
  sufficient for this fix.
