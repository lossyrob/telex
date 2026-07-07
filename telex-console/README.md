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
| `--utc` | off | Show timestamps in UTC instead of system local time. |

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
- **Reader** — a full-screen, scrollable view of a single message for reading long bodies
  and metadata in full (see below).

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
| `Enter` | open the thread (Feed/Addresses) or the message reader (Thread) |
| `o` | open the selected message in the full-screen reader (any view) |
| `t` | toggle live tail (auto-scroll) |
| `f` | set the address filter (type, then `Enter`; `Esc` cancels) |
| `g` / `G` | jump to top / bottom |
| `Esc` | leave Reader/Thread / clear the filter |

### Reading long messages (the reader)

The side detail pane clips anything taller than itself. To read a long body or metadata in
full, open the **reader**: press `Enter` on a message in the Thread view, or `o` from any
view. It shows the whole message — headers, full body, pretty metadata, delivery records,
and dispositions — full-screen and **scrollable**, with a scrollbar and a `line X/Y`
indicator in the title.

| Key (reader) | Action |
|--------------|--------|
| `j` / `k` or `↓` / `↑` | scroll one line |
| `Space` / `b` (or `PgDn` / `PgUp`) | scroll one page |
| `g` / `G` | jump to top / bottom |
| `←` / `Esc` | close the reader (back to where you opened it) |
| `q` | quit |


The header shows the backend, current view, live/paused state, active filter, counts, and
the timestamp timezone (`local` by default, or `UTC` with `--utc`). Timestamps are
`HH:MM:SS` time-of-day; the underlying store records full epoch-millisecond UTC, which
`telex export` emits verbatim.
Below ~50×8 the UI shows a "terminal too small" notice instead of cramped panes.

### Live tail vs. paused

The header's `● LIVE` / `⏸ PAUSED` indicator controls **auto-scroll only** — it does *not*
stop polling. When paused, new messages still arrive (the `msgs:` count keeps climbing); the
feed simply stays put on your selection instead of jumping to the newest row. The feed
**auto-pauses** as soon as you scroll up (`k` / `↑`) or jump to the top (`g`), so reading
history isn't interrupted by live arrivals. Press **`t`** (toggle) or **`G`** (jump to
bottom) to resume tailing — the footer shows a `t/G resume-tail` hint while paused.

## How it reads

The feed is a **bounded** cursor poll: each tick drains new messages (`id > cursor`) in pages
via `Backend::feed_page`, advancing the cursor, so even `--backfill all` or a large burst never
materializes an unbounded result; the in-memory feed is capped at the most recent ~2000 messages.
The address directory refreshes on a slower cadence than the feed. Per-address occupancy is read
per address; the per-address **undelivered backlog** counts come from a single **read-only** bulk
query (`Backend::undelivered_counts`, a pure `SELECT`/`GROUP BY` that never materializes delivery
rows). Dispositions and delivery records are loaded lazily for the selected message.

The console opens the backend **read-only** (`profiles::build_readonly`): it never creates schema,
and for SQLite it uses a read-only connection with no journal/synchronous pragmas — so pointing the
inspector at a store never mutates it. Reader text is word-wrapped by terminal **display width**
(`unicode-width`), so wide/CJK/emoji content scrolls correctly; message bytes are stripped of
terminal-control characters before display.

## Limitations

- Address filtering only (substring on `from`/`to`); attention/kind filters and free-text
  search are not yet implemented.
- No write actions (send, reply, disposition), and the store is opened read-only — the
  inspector never mutates the fabric.
- The delivered/undelivered badge is shown in the detail pane for the selected message
  (loaded lazily) and as a per-address count; there is no per-row feed badge yet (it would
  cost a delivery lookup per visible row each poll).
- SQLite is the primary tested backend; Postgres is supported through the same trait. For a
  hard read-only guarantee on Postgres, connect with a `SELECT`-only role.
- The feed retains the most recent ~2000 messages in memory; when paused with more than
  that arriving, selection tracks position rather than a pinned message id.
