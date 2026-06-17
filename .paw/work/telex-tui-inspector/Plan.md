# Plan: Telex Console (read-only live-tail TUI)

## Approach Summary

Add a new in-repo Cargo **workspace member crate** `telex-console` (binary
`telex-console`) that depends on the core `telex` library via a **path dependency**
and drives the existing `Backend` trait in-process. The crate is a read-only,
live-tailing ratatui TUI with three views (Feed / Addresses / Thread) over a shared
detail pane, plus an address filter. The core `telex` binary and its dependency graph
are left unchanged (no `ratatui`/`crossterm` in core), so the agent install stays lean.

Backend construction is **already factored** in `profiles.rs`
(`resolve(selector, db) -> (String, BackendProfile)` and `build(profile, db) ->
Arc<dyn Backend>`, both public). **The console calls `profiles::resolve` +
`profiles::build` directly.** We do **not** refactor `Ctx::backend()` (avoids any CLI
regression risk — review finding #5); at most we add a *purely additive* convenience
wrapper that the CLI is not required to adopt.

Live tail is pull-only cursor polling: the cursor is the max message `id` seen; each
tick calls `export(None, None, cursor)` (global) or `export(Some(addr), None, cursor)`
(scoped) and advances the cursor. The console never holds a lease or heartbeats, and
contains **no blocking `telex wait`**.

**Feed startup / backfill (review finding #1 — must-fix).** `export` has no `LIMIT`,
so an unbounded `export(None, None, 0)` on a large DB is unsafe. On startup the console
reads a **global max message id** and seeds the cursor for a bounded recent window:
- Add one read-only core helper `Backend::max_message_id() -> i64` (global
  `SELECT COALESCE(MAX(id),0) FROM messages`), implemented in both SQLite and Postgres.
  (No global max-id method exists today — `max_id` is per-`to_addr`.)
- `--backfill <N>` (default `200`): start cursor at `max(0, max_message_id - N)` so the
  feed shows recent context then tails. `--backfill 0` = tail-from-now only;
  `--backfill all` loads full history (explicit opt-in). Messages are append-only (no
  deletes), so "last N ids" ≈ "last N messages" is a safe approximation.
The in-memory ring (cap ~2000) bounds retention thereafter.

## Key Decisions

- **Crate layout** (`telex-console/`):
  - `Cargo.toml` — `telex = { path = ".." }` (default features sqlite+postgres),
    `ratatui`, `crossterm` (with `event-stream`), `tokio`, `futures`, `anyhow`,
    `serde_json` (pretty metadata). Optional `entra` feature → `telex/entra`.
  - `src/main.rs` — clap args (`--backend`/`--db`/`--address`/`--poll-secs`, env
    `TELEX_BACKEND`/`TELEX_DB`/`TELEX_ADDRESS`), open backend, run app, panic-safe
    terminal restore.
  - `src/app.rs` — `AppState`, `View` enum (Feed/Addresses/Thread), `Focus`,
    selection indices, tail flag, address-filter state; `update()` handling keys/ticks.
  - `src/event.rs` — unified event source: `crossterm::event::EventStream` +
    `tokio::time::interval(poll)` merged via `tokio::select!`.
  - `src/data.rs` — `Store { backend: Arc<dyn Backend> }` async wrappers:
    `feed_since(cursor, filter)`, `addresses_with_occupancy()`, `address_messages(addr)`,
    `thread(id)`, `dispositions(msg_id)`. Centralizes all `Backend` calls.
  - `src/ui/{mod,feed,addresses,thread,detail,theme}.rs` — render + layout; `theme`
    maps attention/disposition/actionable → symbols+colors.
  - `src/filter.rs` — address filter (case-insensitive substring on `from_addr`/
    `to_addr`) + predicate.
- **Async model:** `#[tokio::main]`; single select-loop drains input + poll ticks →
  `update()` → `render()`. Keep `Backend` calls off the render path (await in the loop,
  not mid-draw).
- **Cursor = max id.** Feed merges newly fetched rows (id-ordered) onto an in-memory
  ring (cap e.g. last N=2000) to bound memory.
- **Two poll cadences (review finding #4):** a fast **feed tick** (~1s, `--poll-secs`)
  for messages, and a slower **directory tick** (~3–5s) for `list_addresses` +
  per-address `occupancy`/`get_lease`. Occupancy results are cached; per-address lookups
  are bounded in concurrency and a single address failure must not break the UI
  (degrade that row to "unknown", keep rendering).
- **Disposition / actionable semantics (review finding #3):** Feed rows render from raw
  `MessageRow` (show a "needs-disposition" marker from `requires_disposition`; do **not**
  fake a global actionable rollup). Latest disposition + full history are loaded **lazily
  for the selected row** via `dispositions_for` into the detail pane. The **Addresses**
  drilldown uses `inbox(addr, all, limit)` (which already returns `latest_disposition` +
  `actionable` per recipient) — **not** `export` — so per-address actionable/disposition
  state is correct.
- **Read-only.** No send/reply/disposition. Keymap: `Tab` cycle view, `j/k`/arrows
  nav, `Enter` open Thread for selected msg, `a` jump to address (Addresses view),
  `t` toggle tail, `f` set address filter (text input), `Esc` clear filter/back,
  `g/G` top/bottom, `q` quit.
- **Detail pane** is the right column in Feed/Addresses; `Enter` opens full Thread.
- **Workspace install story preserved:** root stays the `telex` package *and*
  workspace root; add `members = ["telex-console"]` to the existing `[workspace]`.
  `cargo install --git <repo>` still installs core `telex`; console installs via
  `cargo install --git <repo> telex-console`.

## Work Items

Implementation is **sequential** (the views share `AppState`, the view router, and
`ui`/`data` modules; parallel subagents editing the same files in one worktree would
conflict). Todos below are phases, each with success criteria. IDs use
`lite:telex-tui-inspector:work:<slug>`.

### 1. `scaffold` — workspace + crate + global max-id helper
- Add `members = ["telex-console"]` to root `[workspace]`, **preserving** existing
  `exclude = ["spike"]`, resolver, and root `[profile.release]` (review finding #6).
- Create `telex-console/Cargo.toml` + minimal `src/main.rs` that parses args, opens the
  backend via `telex::profiles::resolve` + `telex::profiles::build` (no `Ctx::backend()`
  refactor — review finding #5), prints backend kind, and exits.
- Add `async fn Backend::max_message_id() -> Result<i64>` to the trait + SQLite and
  Postgres impls (`SELECT COALESCE(MAX(id),0) FROM messages`) — needed for bounded feed
  backfill (review finding #1). (Optional: a purely additive `telex::open_backend`
  wrapper; the CLI is left unmodified.)
- **Success:** `cargo build` (workspace) and `cargo build -p telex` both green;
  `cargo tree -p telex` shows **no** `ratatui`/`crossterm`; `telex-console --help` runs;
  `cargo install --git <repo>` still selects core `telex`, and
  `cargo install --git <repo> telex-console` selects the console (verify resolution; no
  live install required).

### 2. `app-shell` — terminal, event loop, router, chrome
- Panic-safe terminal lifecycle: an RAII **terminal guard** (restores raw mode + leaves
  alternate screen on `Drop`) plus a chained **panic hook** that restores the terminal
  before printing the panic (review finding #7). Select-loop in `event.rs`. `AppState` +
  `View` router. Header bar (backend kind, target, ● LIVE/⏸, active filter, counts) and
  footer keybinding hints. `q` quits.
- Keep `update(state, event) -> state-change` **UI-free and pure** and `render(state)`
  read-only, so both are unit-testable (seam for tick/input ordering — finding #7).
- `Store` in `data.rs` wrapping `Arc<dyn Backend>` (methods may start as stubs).
- **Success:** console launches against a temp sqlite db, shows empty chrome, `q`
  exits cleanly with terminal restored; no panics on resize or on a very small terminal
  (renders a graceful "terminal too small" notice below a minimum size).

### 3. `feed-view` — global live tail
- On entry, seed the cursor for bounded backfill: `cursor = max(0, max_message_id - N)`
  with `N = --backfill` (default 200; `0` = tail-only; `all` = full) — review finding #1.
- `Store::feed_since(cursor, filter)` via `export(None,None,cursor)`; cursor advance;
  bounded in-memory ring (~2000); auto-scroll when tailing; rows render from **raw
  `MessageRow`** with a `requires_disposition` "needs-disposition" marker (no faked
  global actionable rollup — finding #3); row format `time attn from→to kind subj/body`;
  `theme` symbols/colors; `t` toggle tail.
- Selecting a row populates the shared detail pane (lazy `dispositions(id)` +
  pretty `metadata`; handle absent/odd/non-UTF8 metadata and very long bodies via
  wrapping/scroll).
- **Success:** seeding messages via the `Backend` (temp db) makes them appear within
  one poll interval; backfill shows the last N on launch then tails; tail toggle
  stops/starts auto-scroll; detail renders body + metadata + dispositions.

### 4. `addresses-view` — Miller-column drill-down
- Left: `addresses_with_occupancy()` (`list_addresses` + per-address `occupancy`/
  `get_lease`) refreshed on the **slower directory cadence**, cached, with bounded
  concurrency; a per-address lookup failure degrades that row to `?`/"unknown" without
  breaking the UI (review finding #4). Dots: ●live/○idle/?unknown.
- Center: messages for the selected address via **`inbox(addr, all, limit)`** (returns
  `latest_disposition` + `actionable` per recipient — finding #3), not `export`. Right:
  shared detail.
- **Success:** addresses list with correct occupancy dots; selecting an address lists
  its messages with disposition/actionable state; detail pane works; `Enter` opens the
  message's Thread.

### 5. `thread-detail` — transcript view
- `Store::thread(id)` (`thread_messages`) rendered as an indented transcript by
  `parent_id`, with inline disposition lines (`dispositions(id)`); `Enter` from Feed/
  Addresses opens it; `Esc`/`←` returns. Expand body/metadata for the selected node.
- **Success:** a multi-message thread renders parent→child with dispositions; back
  navigation restores prior view + selection.

### 6. `address-filter` — v1 filter
- `filter.rs` state; `f` opens a small input line; predicate filters Feed rows and
  scopes the Addresses list; `Esc` clears; active filter shown in header.
- **Success:** entering a filter narrows Feed/Addresses to matching `from`/`to`;
  clearing restores; live tail respects the active filter.

### 7. `docs` — documentation phase
- `telex-console/README.md` (what it is, install, usage, keymap), crate/module doc
  comments, and a short mention in root `README.md` "Learn more"/tools. Note the
  read-only/live-tail charter and that it reuses the core lib. Follow `paw-docs-guidance`.
- **Success:** README + doc comments present; root README references the console;
  `cargo doc -p telex-console` builds.

## Dependencies

```
scaffold → app-shell → feed-view → address-filter
                    ├→ addresses-view → (address-filter)
                    └→ thread-detail
docs depends on feed-view, addresses-view, thread-detail, address-filter
```

## Verification / Test Plan

- **Build:** `cargo build` (workspace) and `cargo build -p telex` (core unchanged).
- **Lean-core check:** `cargo tree -p telex` excludes `ratatui`/`crossterm`.
- **Install resolution (finding #6):** confirm `cargo install --git <repo>` selects core
  `telex` and `cargo install --git <repo> telex-console` selects the console; root
  `exclude=["spike"]`/resolver/`[profile.release]` preserved.
- **Lint:** `cargo clippy --workspace` clean (or no new warnings).
- **Unit tests (logic, headless):** seed a temp `SqliteBackend` via the trait and test
  `Store` reads; test `max_message_id` + backfill cursor seeding, cursor-advance/
  feed-merge, address-filter predicate, and thread grouping. Pure-logic functions kept
  UI-free for testability.
- **Render tests (finding #7):** ratatui `TestBackend` snapshot/layout tests for the
  empty state, a too-small terminal, and a populated feed; assert no panic and key
  regions present.
- **Postgres parity (finding #2):** source-verify `backend/postgres.rs` implements the
  same contract the console relies on — `export(None,None,since)` global id-cursor,
  `thread_messages`, `inbox` (latest_disposition/actionable), `dispositions_for`,
  `occupancy`/`get_lease`, and the new `max_message_id` — and document a manual PG smoke
  procedure. No live PG required for v1 sign-off, but the checklist must be explicit.
- **Manual smoke:** seed a temp db with `telex send`, run `telex-console --db <tmp>`,
  verify backfill + live tail + navigation + filter, clean exit/terminal restore.

## Out of Scope (deferred follow-ups)
- Attention/kind filters and `/` full-text search (only address filter in v1).
- Install-script/release plumbing (`--with-tui`, release binary, install docs beyond
  the console README).
- Send/reply/disposition write actions.
- Postgres-specific performance polish.
