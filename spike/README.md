# Telex Spike

A throwaway validation spike for the Telex design (see [`../DESIGN.md`](../DESIGN.md)
and [`../DECISIONS.md`](../DECISIONS.md)). It is **not** the Telex implementation —
it is the minimal Rust + Postgres code written to de-risk the riskiest design
assumptions before committing to the build. It is kept in-tree as a record of what
was proven.

## What it validated

| Question | Result |
|---|---|
| Rust talks to Azure Postgres Flexible Server with **Entra** auth | ✅ token from a normal `az login`, TLS via Windows schannel |
| Telex owns the wait as a **native** primitive (not agent loop scripts) | ✅ `holder` + `waiter` binaries |
| **Two-process split** (resident holder / ephemeral waiter) keeps the address occupied across waiter exits and agent turns | ✅ holder survives waiter cycles; liveness stays `occupied` |
| **TTL-heartbeat** liveness | ✅ `occupied` while alive, drops after the TTL window lapses |
| **Poll-with-cursor** delivery | ✅ blocked waiter wakes on new message, exits with payload |
| **Holder-gone** detection | ✅ waiter exit code 3 |
| **Hang** detection (wedged holder / stale DB heartbeat) | ✅ waiter exit code 4 |
| Two independent agent sessions exchange messages through the shared backend | ✅ live cross-session "increment game", 3 round-trips both ways |
| Sub-agent topology (main owns holder, sub runs waiter, reports on completion) | ✅ |
| **One `Backend` trait, two implementations** (Postgres + SQLite), same holder/waiter/sender code | ✅ same generic binaries over `--backend postgres` and `--backend sqlite` |
| **SQLite multi-process concurrency** on one shared file (the local two-session case) | ✅ 6 concurrent writer processes + 2 holders, 90 writes, 0 failures, 91/91 distinct ids, no corruption (WAL + `busy_timeout`) |

Notably, **`LISTEN/NOTIFY` and advisory locks were not needed** — poll + TTL was
sufficient for the agent-turn-scale workload. See decision 0005.

## Latency findings (measured)

The spike was instrumented end-to-end (epoch-ms timestamps at every hop, single
machine) to hunt down the sources of a ~15 s lag observed during an interactive
two-session run. The `bench` binary measures backend delivery; the two-round
cross-session "increment game" (3 round-trips on poll, 3 on push) measured the full
chain including agent-wake.

Decomposition, by magnitude:

| Hop | Cost | Notes |
|---|---|---|
| **Agent-wake** (waiter exit → runtime wakes the agent) | **~6–26 s** | The dominant term. Above the telex layer; runtime-dependent. First wake of a session was the worst (26 s). |
| Backend delivery (sender insert → holder buffers) | poll ~0.6 s (0.2–1.0), push ~0.14 s (0.1–0.2) | poll ≈ ½ the poll interval; push ≈ cloud RTT floor |
| Per-`send` connect | ~0.4 s warm, up to ~2.8 s cold | a short-lived CLI opening a fresh TLS connection to Azure |
| Entra token fetch | ~2.7 s, **once** | now cached in TEMP; was paid per-call before |
| Holder → waiter (local socket) | ~1 ms | negligible |

`bench` standalone (10–20 msgs, backend only): push median ~65–75 ms, poll median
~500 ms (≈ one poll interval).

**Conclusion.** The perceived lag was **agent-wake latency, not telex transport**.
Push improves the backend leg (≈630→140 ms) and matters for machine-to-machine
dispatch, but cannot reduce agent-wake. Token caching and connection reuse are the
fixable transport-side costs.

## Backend abstraction & SQLite

A `Backend` trait (`src/backend.rs`) abstracts the primitives the holder/waiter/
sender need — ensure-address, claim-lease, heartbeat, max-id, fetch-after-cursor,
insert, notify, occupancy — with two implementations: `PgBackend` (tokio-postgres)
and `SqliteBackend` (rusqlite + `spawn_blocking`). The same generic binaries select
a backend with `--backend postgres|sqlite` (and `--db <path>` for SQLite). The
ephemeral `waiter` needs no backend knowledge at all — it only speaks the local
socket protocol to the holder.

This made the "same semantic core, two backends" promise (decision 0005) concrete
and validated the SQLite-specific risk: **multi-process concurrency on one shared
file.** With `PRAGMA journal_mode=WAL` and `busy_timeout=5000`, a stress of 6
concurrent writer processes (90 inserts) running alongside 2 holders (each
heartbeating and polling the same file) produced **0 write failures and 91/91
distinct, monotonic ids** — no `SQLITE_BUSY` surfacing, no loss, no corruption.
Monotonic AUTOINCREMENT under contention matters because the cursor-based delivery
model depends on it. SQLite delivery (poll-only) and TTL liveness behaved
identically to Postgres through the trait.

## Binaries

- `initdb` — apply the Postgres schema (SQLite schema is created on first connect).
- `holder` — resident answerback drum (backend-generic via `--backend`): holds the
  connection, writes the TTL heartbeat, learns of messages (poll-with-cursor, plus optional `--push`
  LISTEN/NOTIFY), buffers them, and serves waiters over a local TCP socket.
  Long-lived.
- `waiter` — ephemeral delivery client: blocks on the holder, prints one message as
  JSON (with a per-hop timing breakdown), exits. Exit codes: `0` delivered, `2` idle
  timeout, `3` holder gone, `4` holder hung.
- `sender` — insert a message for an address (stamps `sent_at_ms`, fires NOTIFY).
- `liveness` — report whether an address is occupied by TTL heartbeat freshness.
- `bench` — watch the same inserted messages via both poll and push concurrently and
  report per-message and summary delivery lag.

## Helper scripts

- `_env.ps1` — dot-sourced; sets connection env, caching the Entra token in TEMP and
  reporting cache hit / fetch time.
- `attach.ps1` — start a holder for an address (attached background task); `-Push`
  enables LISTEN/NOTIFY.
- `send.ps1` — send a message to an address (prints total send time).
- `recv.ps1` — block on your holder until one message arrives (attached background
  task; exits on delivery with a timing JSON).
- `nowms.ps1` — print epoch ms, for capturing agent-wake time on a waiter
  completion.
- `HANDOFF-SESSION-B.md` — the instruction file handed to a second session for the
  live two-round (poll then push) cross-session lag test.

## Running it

Requires `az login` (as a principal with access to the Flex Server) and Rust.

```powershell
cd spike
. .\_env.ps1
cargo run --bin initdb

# terminal 1 — attach a holder (long-lived)
.\attach.ps1 -Address "workstream:spike/node:a" -Port 47655

# terminal 2 — block waiting for a message
.\recv.ps1 -Address "workstream:spike/node:a" -Port 47655

# terminal 3 — send one
.\send.ps1 -To "workstream:spike/node:a" -Body "hello" -Attention next-checkpoint
```

Connection target defaults to the author's Flex Server and can be overridden with
`TELEX_PG_HOST`, `TELEX_PG_USER`, `TELEX_PG_DB`, and `TELEX_PG_PASSWORD` (an Entra
token or a SQL password — the spike treats them the same).
