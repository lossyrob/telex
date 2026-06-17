# WorkShaping: Telex Console (TUI message inspector)

## Problem Statement

There is no human-facing way to observe what is happening on the Telex message
fabric. Today you inspect messages through one-shot CLI commands (`telex inbox`,
`telex read`, `telex export`) that are per-address, static, and JSON/text oriented —
fine for agents, poor for a human trying to *watch* a live run (e.g. a
backlog-orchestrator coordinating workers across many addresses).

**Who benefits:** a human operator (Rob) watching agent coordination in real time —
following messages, threads, addresses, occupancy, and dispositions as they happen.

**What it solves:** a read-only, live-tailing terminal UI — the "operator's console"
for Telex — that makes the fabric legible at a glance and navigable by thread and
address.

## Chosen Approach (decided)

- **Language/stack:** Rust + `ratatui` + `crossterm`.
- **Packaging:** a new **in-repo Cargo workspace member crate**, separately
  installable, that depends on the core `telex` library crate via a **path
  dependency** and calls the `Backend` trait in-process. The core `telex` binary's
  dependency graph stays unchanged (no `ratatui`/`crossterm` in core) so the agent
  install stays lean.
- **Name:** `telex-console` (binary `telex-console`). Framed as the operator's
  console so the name still fits if it later gains send/disposition abilities.
  (Rejected: `telex-top`/`-monitor`/`-watch` — lock into read-only; `telex-station`
  — collides with SDK `Station` from issue #12.)
- **Read-only in v1:** no send, reply, or disposition mutation from the UI.
- **Both backends** via the `Backend` trait (SQLite primary test target; Postgres
  works through the same trait).
- **Live tail** via cursor polling (`export(None, None, cursor)` advancing past the
  max message id), ~1s interval. **No blocking `telex wait`** — consistent with the
  runtime lesson that blocking waits don't self-advance.

## Work Breakdown

### Core (v1)

1. **Workspace + crate scaffolding**
   - Promote repo to a workspace with members `["", "telex-console"]` (root stays the
     core `telex` package; `spike` remains excluded).
   - `telex-console/Cargo.toml`: `telex = { path = ".." }`, `ratatui`, `crossterm`,
     `tokio`, `anyhow`. Enable core features `sqlite` (+ `postgres`).

2. **Core-lib refactor (minimal, benefits both binaries)**
   - Extract a library helper `telex::backend::open(...)` (or `BackendBuilder`) from
     the CLI's `Ctx::backend()` so the console reuses profile/backend/db selection
     instead of reimplementing it. Keep `telex` binary behavior unchanged.
   - Optional convenience: a `list_leases(...)` (or reuse `occupancy`/`get_lease`
     per address) to drive the Addresses occupancy dots without N ad-hoc calls.

3. **TUI application shell**
   - Terminal setup/teardown (raw mode, alternate screen, panic-safe restore).
   - Async event loop: input (crossterm) + periodic poll tick; no blocking waits.
   - App state model + view router (Tab cycles views).

4. **Views**
   - **Feed (home):** global chronological stream of all messages, newest at bottom,
     auto-scroll while tailing. Source: `export(None, None, cursor)` poll.
   - **Addresses:** Miller-column drill-down (addresses + occupancy -> messages for
     selected address -> detail). Source: `list_addresses`, `occupancy`/`get_lease`,
     `inbox`/`export(Some(addr), ...)`.
   - **Thread:** threaded transcript for a thread id with inline dispositions.
     Source: `thread_messages`, `dispositions_for`.
   - **Shared detail pane (right):** headers, body, pretty-printed `metadata` JSON,
     disposition history. Enter on a message opens the full Thread view.

5. **Address filter (v1, requested)**
   - Filter the Feed (and Addresses scope) by address — at minimum exact/substring
     match on `to`/`from`. UI affordance (e.g. `f` to set/clear an address filter).

6. **Backend selection**
   - Mirror CLI globals: `--backend`, `--db`, `--address` (env equivalents), so the
     console opens the same store the CLI would.

7. **Live tail control**
   - Global tail toggle (`t`); in Feed it auto-scrolls/append, elsewhere it appends
     matching rows. Poll interval default ~1s (consider `--poll-secs`).

### Supporting / nice-to-have (v1 if cheap)
- Color/symbol legend for attention (interrupt/next-checkpoint/background/fyi),
  actionable flag, disposition state.
- Empty/least states ("waiting for messages…", no addresses, backend error).
- Status/header bar: backend kind, db path/profile, live indicator, counts.

### Deferred (explicit follow-ups)
- Additional filtering/search: attention floor, kind filter, `/` full-text on
  subject/body. (Address filter only is in v1.)
- Install-script/release plumbing (`install.sh`/`install.ps1` `--with-tui`, release
  binary, README).
- Send/reply/disposition (write actions) — the name leaves room; out of scope now.
- Postgres-specific polish/perf testing beyond trait correctness.

## Architecture Sketch

```
telex-console (bin)
  main -> parse args (backend/db/address) -> telex::backend::open() -> Box<dyn Backend>
  app loop:
     ┌── input events (crossterm) ──┐
     │                              ├─> update AppState ─> render (ratatui)
     └── poll tick (~1s) ───────────┘        ^
          └ Backend reads: export(None,None,cursor) [Feed]
            list_addresses + occupancy           [Addresses]
            thread_messages + dispositions_for   [Thread/detail]
            export(Some(addr),...)/inbox         [address scope]
core telex lib (unchanged binary; +backend::open helper)
  Backend trait -> SqliteBackend | PostgresBackend
```

Data flow is pull-only: the console never holds a lease or heartbeats; it just reads.
Cursor is the max message `id` seen; each tick fetches rows with `id > cursor`.

## Codebase Fit / Reuse

- `Backend` trait already exposes everything needed: `export` (global feed via
  `address=None`), `thread_messages`, `list_addresses`, `inbox`, `dispositions_for`,
  `occupancy`, `get_lease`. (Confirmed in `src/backend/sqlite.rs`.)
- `model.rs` types (`MessageRow`, `AddressRow`, `LeaseRow`, `DispositionRow`,
  `InboxItem`, `Attention`, `Disposition`) are public and reusable directly.
- `config.rs`/`profiles.rs` carry backend selection; `Ctx::backend()` in `cli.rs`
  has the construction logic to extract into the lib.
- Repo is already a workspace (it `exclude`s `spike`), so adding a member is small.

## Risks / Gotchas

- **`export` returns messages only** (no disposition rollup, no `actionable`). The
  Feed must compute/show disposition via `dispositions_for` on selection (cheap) or a
  light per-row lookup; don't over-fetch on every tick.
- **`since` in `export` is an id, not a timestamp** — cursor logic must track max id.
- **Occupancy is per-address** (no bulk lease listing today) — N calls per refresh;
  fine for small N, but consider a `list_leases` helper to avoid fan-out.
- **Cross-backend timestamps/ids** — Postgres vs SQLite id semantics should be
  confirmed equivalent for cursor paging (both autoincrement-style).
- **Terminal restore on panic** — must restore the terminal on error/panic or it
  corrupts the user's shell.
- **`backend::open` extraction** must not change `telex` CLI behavior (regression
  risk) — keep `Ctx::backend()` delegating to the new helper.
- **Poll load** — 1s global `export` is cheap on SQLite WAL; keep queries id-bounded.

## Open Questions for Planning

- Exact shape of the lib helper: free fn `backend::open(&Config, profile)` vs a
  builder; and whether to add `list_leases` now or defer.
- Whether the Feed enriches rows with disposition/actionable eagerly or lazily.
- Detail pane vs full-screen Thread transition details and keymap finalization.
- Async model: `tokio` current-thread runtime + `crossterm` EventStream vs a simple
  poll loop with `crossterm::event::poll(timeout)`.
- Test strategy: unit-test cursor/feed-merge logic and a `Backend`-fake; how much UI
  to test (snapshot of render buffers vs logic-only).
- Address filter semantics: exact vs substring; applies to `to`, `from`, or both.

## Session Notes (decisions)

- Approach **#1 (in-repo Rust ratatui TUI)** chosen over a TS/Ink TUI; the planned TS
  SDK (issue #12) is a napi binding over the same Rust core, so Rust is the perf
  floor and a Node toolchain in-repo isn't justified now. A future Ink console on the
  SDK remains possible and complementary.
- **Read-only, live-tail** confirmed as the v1 charter.
- **Separate, separately-installable crate** confirmed (keep core binary lean).
- **Address filter included in v1**; other filters/search deferred.
- **Name `telex-console`** chosen (operator-console framing; future-proof for send).
- Three views agreed via ASCII mockups: Feed (home), Addresses (Miller columns),
  Thread (transcript), with a shared right-hand detail pane.
