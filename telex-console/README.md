# telex-console

A read-only, live-tailing terminal UI for inspecting a [Telex](../README.md) message
fabric — the operator's console. Watch every message stream by in real time, browse the
address directory with occupancy, and read threaded conversations with their disposition
history.

It is a **separate, separately-installable** binary that reuses the core `telex` library
in-process (via the `Backend` trait), so the core `telex` binary stays dependency-light
for agents. It works against either backend — local SQLite or networked Postgres.

> **Read-only by design.** `telex-console` never sends a message, claims a lease,
> heartbeats, or writes a disposition — it only reads, polling on a timer (no blocking
> `telex wait`). It opens the store the same way the `telex` CLI does, which ensures the
> schema exists; against an existing telex database that is a no-op.

## Install

```sh
cargo install --git https://github.com/lossyrob/telex telex-console
```

Or, from a checkout of this repo:

```sh
cargo build -p telex-console      # target/debug/telex-console
cargo install --path telex-console
```

To inspect an **Entra-authenticated Postgres** backend, build with the `entra` feature
(same as the core `telex` binary):

```sh
cargo install --git https://github.com/lossyrob/telex telex-console --features entra
```

## Usage

```sh
telex-console                          # inspect the configured default backend
telex-console --db ~/.telex/telex.db   # inspect a specific SQLite store
telex-console --backend prod           # inspect a configured backend by name
telex-console --address orchestrator   # start with an address filter applied
```

Backend selection mirrors the `telex` CLI (`--backend` / `--db` and the
`TELEX_BACKEND` / `TELEX_DB` environment variables). As a convenience for inspection,
`--db <path>` on its own opens that SQLite file directly, even when a non-SQLite default
backend is configured.

### Options

| Flag | Default | Meaning |
|------|---------|---------|
| `--backend <name>` | configured default | Use a configured backend profile by name. |
| `--db <path>` | — | Inspect a SQLite file directly. |
| `--address <text>` | — | Seed the address filter on startup. |
| `--poll-secs <n>` | `1` | Feed poll interval, in seconds. |
| `--backfill <n\|0\|all>` | `200` | Recent messages to load on startup before tailing. `0` = tail only; `all` = full history. |

## Views and keys

Three views over a shared detail pane on the right. `Tab` cycles **Feed ⇄ Addresses**;
`Enter` opens the **Thread** for the selected message.

- **Feed** — the global, chronological stream of all messages, live-tailing the newest at
  the bottom. A `*` marks messages that require a disposition.
- **Addresses** — a Miller-column drill-down: the address directory (with ● live / ○ idle
  / ? unknown occupancy, and a `⧗N` badge for N undelivered queued messages) → that
  address's recent messages (with latest disposition) → detail.
- **Thread** — the selected message's thread as an indented transcript, with inline
  disposition summaries.

The **detail pane** shows a delivery badge: `✓ delivered` (with the holder `occupant` and
time it reached a waiter) or `⧗ undelivered` (queued, not yet handed to a waiter). Delivery
is the durable record that a message reached a live `wait` — distinct from a *disposition*
(which records that it was acted on). Per DECISIONS 0010 the contract is at-least-once, so a
rare duplicate delivery is normal, not an error.

| Key | Action |
|-----|--------|
| `q` | quit |
| `Tab` | switch view (Feed ⇄ Addresses) |
| `j` / `k` or `↓` / `↑` | move selection |
| `←` / `→` | switch column (Addresses view) |
| `Enter` | open the thread for the selected message |
| `t` | toggle live tail (auto-scroll) |
| `f` | set the address filter (type, then `Enter`; `Esc` cancels) |
| `g` / `G` | jump to top / bottom |
| `Esc` | leave Thread / clear the filter |

The header shows the backend, current view, live/paused state, active filter, and counts.
Below ~50×8 the UI shows a "terminal too small" notice instead of cramped panes.

## How it reads

The feed is a cursor poll over the core `Backend::export(None, None, cursor)` (global,
id-ordered); the cursor advances past the greatest message id seen, and the in-memory feed
is bounded to the most recent ~2000 messages. The address directory and occupancy refresh
on a slower cadence than the feed, and a single failed address lookup degrades to
"unknown" rather than breaking the view. Dispositions and delivery records are loaded lazily
for the selected message; the per-address undelivered count comes from
`Backend::undelivered_backlog`, refreshed on the directory cadence.

## Limitations

- Address filtering only (substring on `from`/`to`); attention/kind filters and free-text
  search are not yet implemented.
- No write actions (send, reply, disposition) — read-only.
- The delivered/undelivered badge is shown in the detail pane for the selected message
  (loaded lazily) and as a per-address count; there is no per-row feed badge yet (it would
  cost a delivery lookup per visible row each poll).
- SQLite is the primary tested backend; Postgres is supported through the same trait.
- The backend is opened like the CLI (which runs `CREATE TABLE IF NOT EXISTS` and, for
  SQLite, sets WAL mode). This is a no-op on an existing telex store and the console issues
  no message/lease/disposition writes, but it is not yet strictly read-only against a
  read-only file or a `SELECT`-only Postgres role — a dedicated read-only open path is a
  planned follow-up.
- The feed retains the most recent ~2000 messages in memory; when paused with more than
  that arriving, selection tracks position rather than a pinned message id.
