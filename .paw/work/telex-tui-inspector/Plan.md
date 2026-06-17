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
Arc<dyn Backend>`, both public). The console reuses these directly; we add only a thin
`telex::open_backend(selector, db)` convenience that both `Ctx::backend()` and the
console call, to avoid duplicating the two-step dance. No behavior change to the CLI.

Live tail is pull-only cursor polling: the cursor is the max message `id` seen; each
tick calls `export(None, None, cursor)` (global) or `export(Some(addr), None, cursor)`
(scoped) and advances the cursor. The console never holds a lease or heartbeats, and
contains **no blocking `telex wait`**.

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

### 1. `scaffold` — workspace + crate + backend-open helper
- Add `members = ["telex-console"]` to root `[workspace]` (keep `exclude=["spike"]`).
- Create `telex-console/Cargo.toml` + minimal `src/main.rs` that parses args, calls
  `telex::open_backend(...)`, prints backend kind, and exits.
- Add `pub async fn open_backend(selector: Option<&str>, db: Option<&str>) ->
  Result<Arc<dyn Backend>>` to the lib (wrap `profiles::resolve`+`profiles::build`);
  refactor `Ctx::backend()` to delegate to it (no behavior change).
- **Success:** `cargo build` (workspace) and `cargo build -p telex` both green;
  `cargo tree -p telex` shows **no** `ratatui`/`crossterm`; `telex-console --help` runs.

### 2. `app-shell` — terminal, event loop, router, chrome
- Panic-safe terminal init/restore (raw mode, alternate screen; restore on
  Drop/panic hook). Select-loop in `event.rs`. `AppState` + `View` router. Header bar
  (backend kind, target, ● LIVE/⏸, counts) and footer keybinding hints. `q` quits.
- `Store` in `data.rs` wrapping `Arc<dyn Backend>` (methods may start as stubs).
- **Success:** console launches against a temp sqlite db, shows empty chrome, `q`
  exits cleanly with terminal restored; no panics on resize.

### 3. `feed-view` — global live tail
- `Store::feed_since(cursor, filter)` via `export(None,None,cursor)`; cursor advance;
  bounded in-memory list; auto-scroll when tailing; row format
  `time attn from→to kind subj/body`; `theme` symbols/colors; `t` toggle tail.
- Selecting a row populates the shared detail pane (lazy `dispositions(id)` +
  pretty `metadata`).
- **Success:** seeding messages via the `Backend` (temp db) makes them appear within
  one poll interval; tail toggle stops/starts auto-scroll; detail renders body +
  metadata + dispositions.

### 4. `addresses-view` — Miller-column drill-down
- Left: `addresses_with_occupancy()` (`list_addresses` + per-address `occupancy`/
  `get_lease`) with ●live/○idle. Center: messages for selected address
  (`address_messages` via `export(Some(addr),...)` or `inbox`). Right: shared detail.
- **Success:** addresses list with correct occupancy dots; selecting an address lists
  its messages; detail pane works; `Enter` opens the message's Thread.

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
- **Lint:** `cargo clippy --workspace` clean (or no new warnings).
- **Unit tests (logic, headless):** seed a temp `SqliteBackend` via the trait and test
  `Store` reads; test cursor-advance/feed-merge, address-filter predicate, and thread
  grouping. Pure-logic functions kept UI-free for testability.
- **Postgres parity:** confirm `export(None,None,since)` id-cursor semantics match
  SQLite (code review of `backend/postgres.rs`); no live PG required for v1.
- **Manual smoke:** seed a temp db with `telex send`, run `telex-console --db <tmp>`,
  verify live tail + navigation, clean exit/terminal restore.

## Out of Scope (deferred follow-ups)
- Attention/kind filters and `/` full-text search (only address filter in v1).
- Install-script/release plumbing (`--with-tui`, release binary, install docs beyond
  the console README).
- Send/reply/disposition write actions.
- Postgres-specific performance polish.
