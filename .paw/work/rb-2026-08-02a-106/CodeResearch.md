---
date: 2026-08-02T12:11:11-04:00
git_commit: 5a319b19128cbb207fda1653bd9bad7265c6b8fc
branch: feature/station-intent-reconciliation-106
repository: telex
topic: "Daemon drain/restart loses live push-station membership and on_deliver registration; durable/versioned station-intent reconciliation (issue #106)"
tags: [research, codebase, daemon, backend, copilot-bridge, station-intent, reconciliation, push-delivery]
status: complete
last_updated: 2026-08-02
---

# Research: Station-intent reconciliation for daemon drain/restart (issue #106)

## Research Question

Issue #106 (fetched via `gh issue view 106 --repo lossyrob/telex`) documents that a shared
local daemon drain/restart/upgrade discards in-memory push-station membership
(`MemberRecord`) and the harness-neutral `on_deliver` registration it carries, while the
Copilot bridge process and its heartbeat registry survive untouched and disconnected. The
issue itself is an investigation report (pinned to commit `754079755ae4f827113f616aa87e7cf3f2a9fc77`
on `main`) with precise file:line citations, 8 solution options (A-H), a staged recommended
direction (Stage 1 observability/intent, Stage 2 automatic reconciliation, Stage 3 graceful
handoff), explicit security/correctness constraints, non-goals, acceptance criteria, and a
20-row test matrix. This research re-verifies every citation against the current worktree
HEAD (61 commits ahead of the issue's pinned commit; `git merge-base HEAD
754079755ae4f827113f616aa87e7cf3f2a9fc77` == that commit, i.e. it is a strict ancestor with no
divergence) and extends it with implementation-adjacent detail (test harness shapes,
diagnostics field definitions, security primitives already present, documentation
infrastructure) needed for planning.

## Summary

- All file:line references in issue #106 remain accurate at HEAD to within a handful of
  lines (function/struct start lines matched or were within ~5 lines); none had moved to a
  different file or been removed. Citations below use current HEAD line numbers, called out
  explicitly wherever they differ from the issue's numbers.
- The core gap is a **deliberate design property, not a latent bug**: ADR 0023 in
  `docs/design/DECISIONS.md` and daemon.md §5/§14 establish "membership is explicit-only and
  in-memory... never rebuilt from history" as the intentional min-model contract, specifically
  to avoid resurrecting stale/removed stations. `NeedsAttach` + explicit re-`Register` is the
  *only* recovery path currently implemented, and it is only ever driven by an **active client
  op** (`wait` reconnect in `src/commands/wait.rs`, or `send`/`reply` in
  `src/commands/send.rs`) — there is no autonomous, idle-session-safe recovery path today, and
  none of those generic recovery call sites carry `on_deliver`, so they silently create a
  non-push member if used to "fix" a Copilot bridge.
- The Copilot bridge (`copilot/bridge/extension.mjs`) is a fully passive, self-contained
  in-session process: it never opens a connection to the daemon, never checks the daemon
  capability file's `instance_id`, and only writes a heartbeat-refreshed JSON registry file
  the CLI-side (`src/commands/copilot.rs`) reads for liveness (`bridge_is_live`, mtime-window
  based) and for the push transport (`BridgeRegistry`, secret + endpoint-by-convention). It has
  no knowledge of daemon epochs, `on_deliver` argv, store keys, delivery mode, or CC opt-in —
  exactly the "insufficient bridge files" gap issue #106 describes (option D's proposed fields).
- `daemon.rs`'s `drain_members` (~3492) explicitly calls `state.clear_members()` after
  releasing every epoch lease; `new_state`/`serve` (~2037-2072) construct a brand-new,
  empty `DaemonState` with no recovery/rehydration pass. The wire `Request::Drain` carries only
  an admin `proof` (`src/daemon_ipc.rs:287`), confirming no successor/state-transfer payload
  exists on the wire today.
- Durable backend state (`src/backend/mod.rs`, `sqlite.rs`) already has the primitives ADR
  0023/issue #106 build on: epoch-fenced leases (`leases` table, `lease_epoch`,
  `owner_instance_id`, `daemon_fence_token`), a durable `detach_tombstones` table
  keyed `(session_id, address)` written atomically with `release_epoch_lease_for_detach`, and
  `deliveries(message_id, recipient)` with `consumed_at_ms` for at-least-once dedup. None of
  this currently stores membership/on_deliver/CC/store identity — that is exactly the "durable
  station intent" surface issue #106's option D proposes adding.
- Existing PID+start-time liveness verification (`src/session_watch.rs`:
  `capture_process_start_time`, `process_alive_with_start_time`) and daemon capability-file
  peer verification (`src/daemon.rs` `cap_required_peer_identity`, `platform::verify_client_peer`)
  are reusable primitives for the "PID reused with different start time" / "proof of liveness"
  security constraints the issue calls out; they are not currently wired to bridge/intent
  liveness.
- Diagnostics fields already exist for related-but-distinct states (`PushDeliveryHealth`,
  `StationHealth::AttendedPush`/`CoverageConflict`, `MemberStatus.push_registered`,
  `push_deferred_count`, `push_suppressed_count`) but there is no field/state today for "a live
  bridge intent exists but no daemon member" or "protocol/version incompatible intent" — the
  three diagnostics issue #106 Stage 1 asks for (`live_intent_missing_member`,
  `member_missing_live_producer`, `intent_protocol_incompatible`) have no current counterpart.
- The turn guard (`evaluate_guard` in `src/commands/copilot.rs`) explicitly treats an empty
  `active_session_members` list as `"no_attended_stations"` -> allow, matching the issue's
  claim; `drain_deferred` (`src/daemon.rs:4182`) iterates `state.session_members_any_store`,
  which is empty post-restart, so it is a "successful zero-member sweep" as described.
- Tests already encode the *current*, explicit-only contract as intended behavior:
  `tests/daemon_core_sqlite.rs`'s `section17_04_restart_no_loss_no_resurrection` asserts empty
  membership + `NeedsAttach` + explicit re-`Register` after a simulated restart, and
  `tests/daemon_process_sqlite.rs`'s `real_process_drain_respawn_epoch_advances` only asserts
  epoch monotonicity across a real killed/respawned process — exactly what issue #106 cites as
  evidence that no reconciliation behavior is tested today.
- Verified, runnable commands: `cargo test` (SQLite conformance/unit, default features),
  `TELEX_PG_URL=...` env-gated Postgres conformance (`cargo test`), `cargo test
  --no-default-features --features sqlite --test daemon_process_sqlite copilot_fallback` (real
  child-process daemon tests), `cargo test --all-features --test conformance --test
  daemon_core_postgres`, `cargo fmt --check`, `cargo clippy --workspace -- -D warnings`, and
  `node --test copilot/bridge/busy-state.test.mjs` for the bridge's JS-side busy/idle state
  machine — all taken verbatim from `.github/workflows/ci.yml`.

## Documentation System

- **Framework**: Two-tier. (1) `docs/design/` is a **plain-Markdown design-decision corpus**
  (no static-site generator) — `DECISIONS.md` (ADR log, numbered `## NNNN — Title` entries),
  `daemon.md` (the normative daemon spec, section-numbered `## N. Title` /
  `### N.M Title` with anchor-style cross-links like
  `[daemon.md sec.14.3](daemon.md#143-crash-recovery-and-re-attach)`), `ARCHITECTURE.md`
  (mermaid sequence diagrams + "Governing spec" links into daemon.md), plus
  feature-specific design notes (`copilot-bridge-push.md`, `copilot-plugin-validation.md`,
  `application-client.md`, `watcher.md`). (2) `docs/guide/` is an **mdBook** user guide
  (`docs/guide/book.toml`, `src = "src"`).
- **Docs Directory**: `docs/design/` (internals/ADRs, linked from the book's
  `[Design and internals](internals.md)` page) and `docs/guide/src/` (user guide: concepts,
  getting-started, guides, reference).
- **Navigation Config**: `docs/guide/src/SUMMARY.md` (mdBook TOC). No navigation file for
  `docs/design/` — it is browsed directly on GitHub; `DECISIONS.md:22` documents its own
  "Conventions" for new ADR entries.
- **Style Conventions**: `docs/design/daemon.md` and `ARCHITECTURE.md` are dense, spec-grade
  prose with explicit invariants, `**bold**` for load-bearing terms, fenced ` ```text ` /
  ` ```sql ` blocks for wire/schema shapes, and numbered section anchors used for
  cross-references (e.g. `daemon.md#141-identity-and-in-memory-membership`). ADR entries in
  `DECISIONS.md` follow the template at `DECISIONS.md:32-35` (`Date`/`Status`/`Revises`/
  `Context`/`Decision`/`Consequences`/`Reopen conditions`). `docs/guide/src/guides/*.md` are
  short, task-oriented, command-first (fenced ` ```sh ` blocks then 1-3 sentence explanations).
- **Build Command**: `docs/guide` — `cargo install mdbook --locked --version 0.5.3`, then
  `bash docs/guide/generate-reference.sh ./target/release/telex` (regenerates
  `docs/guide/src/reference/cli.md` from `--help` output) followed by `mdbook build
  docs/guide` (from `.github/workflows/docs.yml`). `docs/design/` has no build step (plain
  Markdown, read on GitHub).
- **Standard Files**: `README.md` (repo root), `CONTRIBUTING.md` (repo root; backend-extension
  and conformance-suite instructions), `SECURITY.md`, `PRODUCT-THESIS.md`, `TELEX.md` (repo
  root). No `CHANGELOG.md` was found at the repo root; release notes are described in
  `docs/developing/releasing.md` (referenced from `CONTRIBUTING.md:36`).

## Verification Commands

(All copied verbatim from `.github/workflows/ci.yml`, which is the authoritative CI
definition; `CONTRIBUTING.md:19-32` documents the conformance-suite subset for backend
contributions.)

- **Test Command (default/SQLite)**: `cargo test` — `.github/workflows/ci.yml:44` (`cargo test
  --workspace --verbose`); runs the full unit + `tests/*.rs` battery against SQLite by
  default (temp-file DB per scenario, per `CONTRIBUTING.md:27`).
- **Test Command (Postgres conformance)**: `cargo test --all-features --test conformance
  --test daemon_core_postgres` — `.github/workflows/ci.yml:163`; requires `TELEX_PG_URL` (and
  optionally `TELEX_PG_PASSWORD`, `TELEX_PG_SCHEMA`, `TELEX_PG_REQUIRE=1`) per
  `CONTRIBUTING.md:24-31`; skips cleanly when `TELEX_PG_URL` is unset.
- **Test Command (real child-process daemon)**: `cargo test --no-default-features --features
  sqlite --test daemon_process_sqlite copilot_fallback` — `.github/workflows/ci.yml:72`; this
  is the suite that spawns a real `telex` daemon child process (`ProcessEnv` harness in
  `tests/daemon_process_sqlite.rs:53-64`) and kills/respawns it, which is the harness a
  reconciliation feature's "hard daemon crash" scenarios would extend.
- **Test Command (Copilot bridge JS)**: `node --test copilot/bridge/busy-state.test.mjs` —
  `.github/workflows/ci.yml:28`; the only JS-side automated test today, exercising
  `copilot/bridge/busy-state.mjs`'s state machine in isolation (no daemon/process
  integration).
- **Build Command**: `cargo build` (default features); feature-matrix builds are validated
  separately: `cargo build --no-default-features --features sqlite`,
  `--no-default-features --features postgres`, `--features entra`, `--no-default-features
  --features "sqlite,self-update"` (`.github/workflows/ci.yml:92-104`).
- **Lint Command**: `cargo fmt --check` and `cargo clippy --workspace -- -D warnings`
  (`.github/workflows/ci.yml:47,50`).
- **Type Check**: N/A (Rust; `cargo build`/`cargo clippy` serve this role). No separate
  type-check step for the Node.js bridge files (no TypeScript in this repo).

## Detailed Findings

### 1. Daemon singleton identity, endpoint stability, and capability-file rotation

The endpoint is derived from `(user_identity, config_root, protocol_major)` — stable across a
same-major restart — while the capability file's `instance_id`/`admin_cap`/PID/start-time are
freshly minted every process start.

- `SingletonKey` (`src/daemon.rs:178-224`): `material()`/`short_hash()` combine user identity,
  config root, and `proto::PROTOCOL_MAJOR`; `DaemonPaths::for_key` (`src/daemon.rs:255-276`)
  derives the named pipe (`\\.\pipe\telex-daemon-{hash}`) / UDS path and `cap_path` from that
  hash, so a same-major respawn binds the identical endpoint identity.
- `CapFile` (`src/daemon.rs:279-303`) carries `instance_id`, `admin_cap`, `singleton_hash`,
  `protocol_major`, `server_pid`, `server_start_time`; `new_state` (`src/daemon.rs:2072-2090`)
  mints a fresh `instance_id`/`admin_cap`/`server_start_time` and calls `write_cap_file` every
  process start (issue cited `src/daemon.rs:180-280`/`285-333`; current line numbers above).
- `new_state` (`src/daemon.rs:2072`) constructs `DaemonState` with `members:
  Mutex::new(BTreeMap::new())` and no recovery pass; `serve()` (`src/daemon.rs:2037-2065`)
  calls `new_state` once at process start and then only accepts connections / handles
  `ClientAction::Drain` — there is no startup membership-recovery hook point today (issue cited
  `src/daemon.rs:2037-2104`).

### 2. `MemberRecord`: in-memory-only membership + push-handler state

- Struct definition: `src/daemon.rs:458-486` (issue cited 458-495; the struct itself ends at
  486, the extra lines in the issue's range covered the following `WatchPidRecord` struct).
  Fields include `address`, `store_key`, `backend`, `session_id`, `occupant`, `host`,
  `waiters`, `watch_pids: Vec<WatchPidRecord>`, `lease_epoch`, `owner_instance_id`, idle/backlog
  timestamps, `last_waiter_*`, `last_delivered_message_id`, and critically
  `on_deliver: Option<Vec<String>>` (harness-neutral push handler argv),
  `on_deliver_wake_on_cc: bool`, `on_deliver_cc_after_ms: Option<i64>`.
- Held only in `DaemonState.members: Mutex<BTreeMap<MemberKey, MemberRecord>>`
  (`src/daemon.rs:324`); `DaemonState` struct at `src/daemon.rs:319-335`.
- Documented as intentionally non-persisted in `docs/design/daemon.md:403-431` (§5
  "Membership model and record shapes"): `MemberRecord { ... }` shape shown in a fenced block
  annotated "in-memory only; lost on daemon restart, rebuilt by explicit re-attach"; explicit
  statement "The `MemberRecord` is never persisted: it lives only in the serving daemon's
  memory and is recreated solely by an explicit `Register`."
- Durable lease-row columns are documented as carrying only ownership/fence data, explicitly
  *not* membership: `docs/design/daemon.md:432-480` (§5.1) states `occupant`/`session_id` on
  the `leases` row are "only a label: the exchange never uses it to rebuild the in-memory
  membership map after a restart," and enumerates that there is "no durable
  `session_incarnation`, no `tombstoned_at`, no attendance column on the lease row, and no
  `sessions` currency table."

### 3. `register_member`: explicit-provision vs. generic-refresh `on_deliver` handling

- `src/daemon.rs:3785-3934` (`async fn register_member`). Key behavior for issue #106 planning:
  - Rejects `Register` with `on_deliver.is_some()` while a live pull waiter is armed
    (`DeliveryModeConflict`, `src/daemon.rs:3818-3830`) — the existing "no push+pull conflict"
    guard that any recovery path must respect.
  - On refresh of an **existing** member (`src/daemon.rs:3830-3925`): a generic refresh that
    re-registers with `on_deliver: None` and `replace_on_deliver: false` **preserves** the
    existing handler (`refreshed.on_deliver = ... on_deliver.clone().or_else(|| existing
    .on_deliver.clone())`, `src/daemon.rs:3861-3865`) — but this preservation branch is only
    reachable when a `MemberRecord` for that key **already exists in this daemon's memory**
    (`state.get_member(...)`, `src/daemon.rs:3829`). After a restart there is no existing
    member, so this "preserve on refresh" path cannot fire — it falls through to fresh member
    creation with `on_deliver = None` (the "silently converting the station into an uncovered
    non-push member" issue #106 describes).
  - An **explicit re-provision** (`on_deliver.is_some()` or `replace_on_deliver: true`) resets
    push retry/backlog state and spawns a fresh backlog sweep (`src/daemon.rs:3906-3916`,
    `spawn_on_deliver_backlog`).
- Wire request shape: `Request::Register` at `src/daemon_ipc.rs:170-197`, carrying
  `recovery: bool`, `on_deliver: Option<Vec<String>>`, `replace_on_deliver: bool`,
  `on_deliver_wake_on_cc: bool`.

### 4. `drain_members`: the destructive drain path

- `src/daemon.rs:3492-3529` (issue cited 3492-3535; matches). Sequence: `state
  .begin_draining()` -> `state.members_snapshot()` -> per-member
  `backend.release_epoch_lease(...)` (non-deleting; retains `lease_epoch` high-water) ->
  `state.clear_members()` (drops the entire in-memory `BTreeMap`, including every
  `on_deliver` handler) -> returns `Ok(())` so the caller (`Request::Drain` handling,
  `src/daemon.rs:3634-3653`, matches issue's 3634-3653) replies `Ack{"draining"}` and returns
  `ClientAction::Drain`, which the `serve()` loop (`src/daemon.rs:2037-2065`) uses to break its
  accept loop and exit.
- Wire request: `Request::Drain { proof: Option<String> }` at `src/daemon_ipc.rs:287-290`
  (issue cited the identical range) — no successor id, no state-transfer payload, no
  recovery-generation field.
- `drain_deferred` (a *different*, non-destructive per-session sweep, not to be confused with
  full drain): `src/daemon.rs:4182-4214` (issue cited 4182-4214, exact match). Iterates
  `state.session_members_any_store(&session_id)` — empty after a real restart wiped
  `state.members` — so post-restart it is a genuine "successful zero-member sweep," matching
  issue point 6.
- Upgrade/rollback invocation of drain: `drain_daemon` in `src/commands/upgrade.rs:562-602`
  (issue cited `249-265`/`562-600`; the helper itself is at 562, call sites at 249 and 367);
  sends `Request::Drain` and interprets `Response::Ack`/`Error`/timeout; `unauthorized_drain
  _message` (`src/commands/upgrade.rs:547-560`) suggests `--skip-drain` on an auth failure.

### 5. Backend abstraction: durable epoch fence, tombstones, delivery buffer (no membership/intent)

- `Backend` trait: `src/backend/mod.rs:54-316` (issue cited 79-257 for the epoch/tombstone
  subset; current numbering above for the full trait). Notable methods:
  - `claim_epoch_lease` / `heartbeat_epoch` / `release_epoch_lease` (`src/backend/mod.rs:96-125`):
    epoch-fenced ownership CAS.
  - `release_epoch_lease_for_detach` (`src/backend/mod.rs:131-149`): atomically releases the
    epoch lease **and** records a durable detach tombstone via `record_detach_tombstone` — the
    default trait-level composition backends can override for atomicity.
  - `record_detach_tombstone` / `clear_detach_tombstone` / `detach_tombstone` (`src/backend
    /mod.rs:207-228`): default no-ops overridden by SQLite (see below); return type
    `Option<DetachTombstone>` (defined in `src/model.rs`).
  - `mark_consumed_if_current_owner` (`src/backend/mod.rs:159-170`): the epoch-guarded,
    atomic-with-ownership-check consumed-mark (the "no consume-on-handoff", "at-least-once"
    fence issue #106 requires be preserved).
  - `fetch_undelivered` (`src/backend/mod.rs:249`): the durable backlog re-scan any
    reconciliation must re-run rather than trusting in-memory push-attempt state.
- SQLite schema: `src/backend/sqlite.rs:884-1024` (issue cited 900-1024; matches closely).
  - `leases` table (`src/backend/sqlite.rs:908-923` fresh-DB path): `address PRIMARY KEY`,
    `occupant`, `host`, `principal`, `description`, `tags`, `scope`, `pid`, `since_ms`,
    `heartbeat_at_ms`, `lease_epoch NOT NULL`, `owner_instance_id`, `daemon_fence_token NOT NULL
    DEFAULT 0` — no columns for `store_key` beyond the address, no `on_deliver`, no delivery
    mode, no CC opt-in, no producer PID/start-time, no protocol-compat fields.
  - `detach_tombstones` table (`src/backend/sqlite.rs:1014-1022`, inside
    `ensure_v2_invariants`): `PRIMARY KEY(session_id, address)`, columns `reason`, `at_ms`; a
    supporting index `detach_tombstones_session_idx`. Populated by `record_detach_tombstone`
    (`src/backend/sqlite.rs:2042-2065`), read by `detach_tombstone`
    (`src/backend/sqlite.rs:2079-2100`), cleared by `clear_detach_tombstone`
    (`src/backend/sqlite.rs:2067-2077`).
  - A trigger `leases_reject_unfenced_update` (`src/backend/sqlite.rs`, inside
    `ensure_v2_invariants`) rejects any `UPDATE` on `leases` that does not strictly increase
    `daemon_fence_token` — a hard-fail guard against a non-epoch-aware (old) binary writing
    the fence table; directly relevant to issue #106's "upgrade skew"/"stale binary" security
    constraint.
  - `deliveries` table (`src/backend/sqlite.rs:884-892`): `(message_id, recipient)` unique,
    `consumed_at_ms` — the durable at-least-once buffer.

### 6. Postgres backend parity

- `src/backend/postgres.rs` mirrors the SQLite epoch/tombstone/delivery surface for the
  `shared_multi_writer` backend (not individually re-verified line-by-line in this pass, but
  the trait contract in `src/backend/mod.rs` is backend-agnostic and `tests
  /daemon_core_postgres.rs` runs the same `daemon_core_sqlite.rs`-style scenarios against it —
  see Testing section). `docs/design/daemon.md` §11.4-11.5 (`view_range 1329-1400` above)
  documents that the **live ordered handoff** (owner-directed atomic epoch transfer,
  `UPDATE leases SET owner_instance_id=:successor, lease_epoch=lease_epoch+1 ... WHERE
  lease_epoch=:E AND owner_instance_id=:predecessor`) applies only to Postgres, because SQLite's
  per-store advisory lock forbids a live two-daemon overlap; SQLite always uses
  "release + next-call respawn." Any reconciliation design that includes an "ordered handoff"
  (issue option E) needs to reconcile with this existing backend-scoped asymmetry.

### 7. Copilot bridge process (`copilot/bridge/extension.mjs`): passive, daemon-unaware

- Module docstring and imports: `copilot/bridge/extension.mjs:1-40` (issue cited 49-92 for a
  slightly later region; header/imports/constants sit earlier in the current file but content
  matches). Confirms design intent: "per-session OS named pipe (Windows) / unix domain socket
  (POSIX)... derived from the Copilot session id," secret-gated because "the default Windows
  named-pipe DACL grants Everyone READ."
- `joinSession` + registry/endpoint derivation + secret: `copilot/bridge/extension.mjs:56-92`.
  `endpoint` is derived purely from `sessionId` (`\\.\pipe\telex-bridge-{sessionId}` /
  `{sessionId}.sock`) — never from daemon state.
- `writeRegistry()` + 15s heartbeat interval + SIGTERM/SIGINT cleanup:
  `copilot/bridge/extension.mjs:246-317` (issue cited 246-310; matches). The registry JSON
  written is `{sessionId, endpoint, pid, secret, maxRequestBytes, createdAt, heartbeatAt,
  diagnostics: {busy...}}` — **no daemon instance id, no store key, no address list, no
  delivery mode, no CC opt-in, no protocol version** is ever written by the bridge itself (the
  separate `.bindings.json` list of addresses is written by the Rust CLI side, not the bridge —
  see next section).
- The bridge never calls back into the daemon, never reads the daemon capability file, and has
  no daemon-instance-change detection logic anywhere in the file (confirmed by absence — no
  `daemon`, `cap_path`, or IPC-socket references in `extension.mjs`). This directly substantiates
  issue point 4 ("The bridge survives, but it is not connected to the daemon").
- Busy/idle state machine used for deferred-push scheduling lives in a separate,
  independently unit-tested module: `copilot/bridge/busy-state.mjs` +
  `copilot/bridge/busy-state.test.mjs` (run via `node --test`). This module has no daemon or
  membership awareness either — it purely tracks root-turn boundaries for the "defer until
  idle" feature (ADR 0043), a distinct concern from station-intent recovery.

### 8. Rust-side bridge bookkeeping (`src/commands/copilot.rs`): the ".bindings.json" gap

- `bridge_bindings_path` (`src/commands/copilot.rs:172-176`): `~/.copilot/telex-bridge
  /{session_id}.bindings.json`. `read_bridge_bindings`/`write_bridge_bindings`
  (`src/commands/copilot.rs:192-227`) round-trip a **bare `Vec<String>` of addresses** — no
  store key, delivery mode, CC opt-in, metadata, or handler descriptor (issue point 5, confirmed
  verbatim: "a `.bindings.json` containing only `Vec<String>` addresses").
  `add_bridge_binding`/`remove_bridge_binding` (`src/commands/copilot.rs:230-259`) maintain this
  as a ref-count so the bridge extension is only torn down when the last bound address detaches.
- `BridgeRegistry` (Rust-side deserialize target for the bridge's own registry file):
  `src/commands/copilot.rs:418-427` — only `session_id`, `secret`, `max_request_bytes` are
  read; `bridge_registry_path`/`bridge_root_dir` at `src/commands/copilot.rs:457-464`.
- `bridge_is_live` (`src/commands/copilot.rs:471-483`): liveness is **solely** "registry file's
  mtime is within `BRIDGE_LIVENESS_WINDOW`" — explicitly documented in its own doc-comment as
  distinct from `push_registered` (daemon-side handler-registered flag). This is the "heartbeat
  mtime alone is not strong liveness proof" gap issue option D/security-constraints calls out;
  there is no PID/start-time check or endpoint-challenge/nonce check in this function today.
- `provision_bridge` (`src/commands/copilot.rs:327-359`): writes the extension files, records
  the binding, and returns the `on_deliver` argv (`bridge_handler_argv`,
  `src/commands/copilot.rs:288-306`: `<exe> [--backend B] [--db D] copilot push --session
  <id>`) that gets passed into `Register.on_deliver` by `attach`/`resume`.
- `attach`/`resume`: `src/commands/copilot.rs:970-1093`. `resume` (`:1077-1090`) is a thin
  wrapper that always calls `attach` with `copilot_bridge: true`. `attach` provisions the
  bridge (if `--copilot-bridge`), calls the generic `attach::run` (which issues `Register`),
  then **verifies** push was actually armed via `daemon_armed_push`
  (`src/commands/copilot.rs:1043-1057`) and **rolls back** the bridge on failure
  (`src/commands/copilot.rs:1061-1069`) — i.e. today's only complete, safe recovery path is this
  manual, session-triggered `attach`/`resume` flow (matches issue's "manual `copilot resume`"
  characterization exactly).
- `gc` (`src/commands/copilot.rs:1665-1735`) and `discover_bridge_sessions`
  (`src/commands/copilot.rs:1738-1769`): already enumerate bridge sessions from
  `~/.copilot/telex-bridge/*.bindings.json` / `*.json` and `~/.copilot/session-state/*/extensions
  /telex-bridge/`, but use that enumeration only for conservative stale-file cleanup (`keep`
  when live or bindings non-empty without `--force`), never to drive daemon-side recovery —
  matches issue point 5's characterization of `gc` precisely.

### 9. `agentStop`/`sessionEnd` hooks and turn guard: no repair path today

- `copilot/plugin/hooks.json:1-27`: `sessionEnd` installs only `telex copilot session-end`;
  `agentStop` installs `telex copilot turn-guard` then a PowerShell/bash launcher that runs
  `telex copilot drain` (idle-drain sweep) and returns a neutral/blocking decision depending on
  exit code and `TELEX_COPILOT_DRAIN` opt-out. Neither hook attempts membership
  reconciliation.
- `turn_guard` (`src/commands/copilot.rs:1477-1568`): fetches `DaemonStatus`, computes
  `active_session_members` (`src/commands/copilot.rs:1873-1886`, filters `!member.idle` for
  the session/store), and calls `evaluate_guard`.
- `evaluate_guard` (`src/commands/copilot.rs:1956-2089`): the very first check
  (`src/commands/copilot.rs:1961-1969`) is `if members.is_empty() { return
  GuardEvaluation { reason_code: "no_attended_stations", decision: Allow, ... } }` — an exact,
  verified match for issue point 6's "empty list returns 'No attended stations' and allows."
  It also computes `push_dead` (push-registered member whose bridge is not live, via
  `bridge_is_live`, `src/commands/copilot.rs:2000-2010`) and, when protocol >= 1.4, `conflicts`
  (push+pull both active) — these ARE surfaced when a member record exists, but cannot fire
  when the member itself is missing (the post-restart case).
- `drain_deferred` request handling confirmed empty-sweep behavior — see Finding 4 above.
- `PushDeliveryHealth::Probing` (`src/daemon.rs:2683-2726`, `push_delivery_health`): computed
  **per-member**, so it "cannot surface this orphan" when no member exists at all — confirmed
  exact match to issue point 6's closing claim.

### 10. Pull-waiter reconnect: the one existing generic recovery path, and its `on_deliver` gap

- `wait_loop` (`src/commands/wait.rs:132-256`, issue cited 132-390 for the whole file region):
  on any daemon-down/EOF/`NeedsAttach` response, calls `begin_reconnect`
  (`src/commands/wait.rs:289-301`) and, for `ERROR_NEEDS_ATTACH` specifically (unless
  `NeedsAttachReason::DeliberatelyDetached`), calls `register_for_retry`
  (`src/commands/wait.rs:351-383`).
- `register_for_retry` issues `Request::Register { recovery: true, on_deliver: None,
  replace_on_deliver: false, on_deliver_wake_on_cc: false, ... }` (`src/commands/wait.rs
  :351-368`) — **always** a pull-only, non-push registration. This is the generic recovery path
  issue point 8 and the "Generic recovery may restore membership but lose push intent" root
  cause describe verbatim; confirmed identically in `src/commands/send.rs:9-48` /
  `register_for_retry` (`src/commands/send.rs:118-141`), the `send`/`reply` recovery path.
- `NeedsAttachReason` enum: `src/daemon_ipc.rs:307-311` — exactly two variants,
  `RestartLost` and `DeliberatelyDetached`. Any reconciliation-aware `NeedsAttach` handling
  (e.g. distinguishing "recoverable via live intent" from "no intent") would need a new
  variant or an additional signal, since these two are all that exist today.

### 11. Diagnostics/status fields already defined (and the ones that are not)

- `MemberStatus` (`src/daemon_ipc.rs:456-537`): includes `push_registered`, `push_wake_on_cc`,
  `push_cc_after_ms`, `push_deferred_count`, `push_suppressed_count`, `station_health`,
  `delivery_mode`, `push_delivery` (typed `PushDeliveryHealth`), `unattended_since_ms`
  /`_for_ms`, `deaf_since_ms`/`_for_ms`/`deaf_warn`, `live_waiters: Vec<LiveWaiterStatus>`,
  `watch_pids`. All of these are **member-scoped** — they describe a station that already has
  an in-memory `MemberRecord`; none can represent "an intent/bridge exists but the member does
  not."
- `StationHealth` enum (`src/daemon_ipc.rs:551-570`): `Armed`, `RecentlyDelivered`,
  `Unattended` (default), `UnattendedWithBacklog`, `AttendedPush`, `CoverageConflict`, `Idle`,
  `Unknown` (forward-compat catch-all). No variant represents an orphaned/missing-member push
  intent.
- `PushDeliveryHealth` enum (`src/daemon_ipc.rs:588-609`): `NotRegistered` (default),
  `NoBacklog`, `Delivering`, `Probing`, `StaleAccepted`, `Failing`, `Unknown`. Computed by
  `push_delivery_health` (`src/daemon.rs:2676-2726`) purely from the daemon's own push-attempt
  outcome map (`self.on_deliver.pushed`) — explicitly documented as "harness-neutral: never
  reads the bridge registry" (doc-comment at `src/daemon.rs:2675-2676`).
- The three Stage-1 diagnostics issue #106 recommends (`live_intent_missing_member`,
  `member_missing_live_producer`, `intent_protocol_incompatible`) have **no existing
  representation** anywhere in `daemon_ipc.rs`/`daemon.rs`/`copilot.rs` — confirmed by absence
  (no matches for those literal strings or an equivalent concept in the codebase).

### 12. Existing PID/start-time and peer-verification primitives (reusable for liveness proof)

- `src/session_watch.rs`: `capture_process_start_time(pid)` (`:98-106`),
  `process_alive_with_start_time(pid, expected_start_time)` (`:107-...`) — the existing
  "PID + start-time reuse guard" used elsewhere (e.g. `WatchPidRecord.start_time` in
  `src/daemon.rs:489-493`, and `waiter_start_time` on `Request::Wait`/`WaiterRecord`). This is
  the same primitive issue #106's "PID reused with different start time" test scenario and
  "proof of liveness" constraint call for; it is not currently invoked for bridge-registry or
  station-intent liveness.
- `cap_required_peer_identity` (`src/daemon.rs:311-318`) and `platform::verify_client_peer`
  (referenced at `src/daemon.rs:3574` in `handle_client`) implement the existing daemon-side
  OS-peer-identity + PID/start-time verification model for the admin capability file; the
  same-user trust boundary is documented in `ARCHITECTURE.md:130-138` (§7, "v1 trust is
  same-user, user-private... Within the user, every process is trusted").

### 13. Governing design documents (ADR 0023 + daemon.md) establish the explicit-only model as deliberate

- `docs/design/DECISIONS.md:1078-1181` (ADR 0023, "Minimal session/presence/delivery model:
  supersede the incarnation-currency machinery," dated 2026-06-23, Status: Accepted (design)).
  Decision text explicitly states: "Membership is explicit-only and in-memory... the exchange
  returns a `NeedsAttach` error for an unknown session/address and never implicitly rebuilds
  membership from history — so a removed address is never silently resurrected, and
  tombstones are unnecessary (the over-correction guard the council required is satisfied by
  removing implicit rebuild rather than by adding durable tombstones)." Its own "Reopen
  conditions" section (`DECISIONS.md:1168-1180`) lists conditions under which this model would
  be revisited, none of which explicitly anticipate a durable station-intent surface — any
  ADR/plan proposing durable intent should reconcile with or supersede this ADR.
  (Detach tombstones were, in fact, subsequently added — see `docs/design/daemon.md:479-486`
  §5.1's "detach tombstone" paragraph and ADR references to `#66`/`#67`/`ADR 0042` — showing the
  design has already evolved once past ADR 0023's original "tombstones are unnecessary" claim.)
- `docs/design/daemon.md` §5 (`:403-431`) and §5.1 (`:432-480`): membership/record-shape
  contract described in Finding 2 above.
- `docs/design/daemon.md` §11.4 "Ordered handoff" (`:1329-1388`) and §11.5 "Postgres
  cross-machine reclaim" (`:1389-1400+`): documents that ordered handoff transfers **only**
  ownership/fence columns, explicitly not membership — "a daemon-to-daemon transfer carries
  delivery ownership, not a change of which session occupies the address... Membership... is
  in-memory and explicit-only... so whether a station is idle or attended is a function of
  in-memory waiters in the owning process, not of a durable column." This is the passage issue
  #106's "Design/implementation distinction" section cites as proof that implementing the
  existing epoch-transfer design alone would not preserve `on_deliver`.
- `docs/design/daemon.md` §13.2 "On-deliver push" (`:1460-1546` per issue's citation; verified
  content at `:1450-1560` in current numbering): full on-deliver contract — registration
  semantics, fire point (post-durable-commit, off ack path), liveness-only/never-consume
  guarantee, backoff/backstop/hard-cap re-push policy, and the **detach-tombstone trust model**
  paragraph explicitly noting tombstones are "keyed only by `(session_id, address)`" and can be
  created "without an ownership proof" on the no-active-member path — directly relevant to
  issue #106's "no stale resurrection" / "same-user trust" constraints.
- `docs/design/daemon.md` §14 "Session identity and explicit membership" (`:1643-1679` per
  issue citation; content on plugin/binary-owned Copilot compat sits nearby, verified at
  `:1620-1690`) and §14.1-14.5 (`:1730-1804`, matching issue's `1772-1804` citation for §14.5
  "Daemon-down and the TTL backstop"): the full "Crash recovery and re-attach" narrative
  (§14.3, `:1750-1768`) and "`wait` and re-attach on `NeedsAttach`" (§14.4, `:1770-1780`) — the
  existing, intentionally-manual recovery contract that any Stage-2 automatic reconciliation
  must extend without violating the "no implicit rebuild from history" invariant for
  addresses that were never re-declared.
- `docs/design/ARCHITECTURE.md` §4 "Restart & re-attach recovery" (`:142-168`, exact match to
  issue citation): the diagram and prose issue #106 also cites, confirming "restart loses ONLY
  the in-memory membership... nothing is rebuilt from history" and "a previously Detached
  address is NOT resurrected."
- `docs/design/ARCHITECTURE.md` §9 "Push delivery" (`:306-342`, exact match to issue citation):
  the on-deliver/bridge sequence diagram and prose, cross-linking `daemon.md §13.2` and
  `copilot-bridge-push.md`.
- `docs/design/copilot-bridge-push.md`: the original push-bridge design doc (issue #53);
  §"Liveness / self-stop hardening" and later addenda (issues #62/#64/#65/#66/#67) already
  document incremental hardening of the *existing* push/bridge model (busy/idle deferral,
  durable self-stop tombstone) but none address daemon-restart membership recovery.
- `docs/guide/src/guides/operating.md`: user-facing daemon lifecycle guide.
  "Recovering from a lost daemon" (`:129-134`) documents only the **pull** path (`telex attach`
  + re-arm `wait`); "Turn-end and resume reconciliation" (`:136-141`) documents
  `telex station status --session <id>` as the diagnostic tool. Neither section mentions
  Copilot push/bridge recovery — the doc gap issue #106's acceptance criteria (documented,
  bounded recovery) would need to fill. `docs/guide/src/guides/troubleshooting.md:37-45`
  ("Copilot: messages do not arrive as turns") currently instructs manually running
  `telex --address <addr> copilot resume` — the exact manual workaround issue #106 aims to
  make automatic.

### 14. Test harness shapes for daemon core / real-process / conformance testing

- `tests/daemon_core_sqlite.rs`: async in-process daemon tests using a `TestDaemon` harness
  (`TestDaemon::new(label)` at `src/daemon.rs:9950` — test-only helper reachable because
  `daemon.rs` has an internal `#[cfg(test)]` test module with client/action plumbing,
  `ClientAction::Drain` mapped at `src/daemon.rs:9944`, `TestDaemon::drain()` at
  `src/daemon.rs:10113-10120`). Representative restart-adjacent test:
  `section17_04_restart_no_loss_no_resurrection` (`tests/daemon_core_sqlite.rs:237-272`,
  matching issue's cited 238-272): seeds a durable message via a raw `SqliteBackend` open,
  constructs a **fresh** `TestDaemon` ("restarted"), asserts `status().await.members.is_empty()`,
  asserts `NeedsAttach` on `wait`, explicit `register`, then delivers/acks, then explicitly
  `drain()`s and constructs **another** fresh `TestDaemon` ("after_ack_restart"), asserting the
  consumed row is retained (`delivery_counts(&store).0 == 1`) and a subsequent `wait` times out
  (nothing to resurrect). This is the exact "test intentionally asserts empty membership and
  explicit re-registration" issue #106 cites.
- `tests/daemon_process_sqlite.rs`: spawns a **real** `telex daemon` child process via a
  `ProcessEnv` harness (`struct ProcessEnv` at `:53-64`, `ProcessEnv::new(name)` at `:65`,
  `.attach(session, address)` at `:149`, `.stop_daemon_best_effort()` at `:172`,
  `.wait_until_not_running(timeout)` at `:180`). Representative test:
  `real_process_drain_respawn_epoch_advances` (`:4081-4100`, issue cited
  `3621-3740` — content has shifted ~460 lines but the test's substance is unchanged):
  attaches once, records `lease_epoch`, kills the daemon (`stop_daemon_best_effort` +
  `wait_until_not_running`), attaches again under a **different** session/label, and asserts
  only that `second_epoch > first_epoch` — no assertion about membership/on_deliver
  reconciliation, matching issue's "process drain test only checks epoch advancement" claim.
  `terminate_pid` helpers (`SIGKILL` on Unix via `libc::kill`, `TerminateProcess` on Windows)
  are available in this file for "hard crash" scenarios from the test matrix.
- `tests/daemon_core_postgres.rs`: presumed structurally parallel to
  `daemon_core_sqlite.rs` (same `TestDaemon`-style harness pattern, gated by `TELEX_PG_URL`);
  not individually re-verified line-by-line in this pass but is the file the CI Postgres job
  (`.github/workflows/ci.yml:163`) exercises alongside `conformance.rs`.
- `tests/conformance.rs`: the shared, backend-parameterized battery
  (`CONTRIBUTING.md:15-24`) that any new `Backend` trait methods (e.g. a durable station-intent
  table) would need corresponding fixtures in to prove SQLite/Postgres parity.
- `tests/copilot_plugin.rs`: tests the **plugin manifest and shell-level drain-hook launcher**
  (`hooks.json` wiring, PowerShell/bash script behavior, neutral/blocking decision merging) via
  fake-`telex`-on-PATH process spawning (`write_fake_telex`, `run_drain_hook*` helpers,
  `:480-700`) — it does **not** exercise the real bridge/extension.mjs or a real Copilot
  session; JS-side bridge logic is only covered by `busy-state.test.mjs` (`node --test`, no
  process/daemon integration).
- `tests/release_upgrade.rs` / `tests/release_contract.rs`: cover the `telex upgrade`/
  `rollback` binary-swap + drain flow at a process/release-asset level; relevant to issue
  #106 Stage 3 ("graceful upgrade coordination") but not individually traced in this pass.

## Code References

- `src/daemon.rs:178-224` — `SingletonKey` (endpoint identity material/hash).
- `src/daemon.rs:255-276` — `DaemonPaths::for_key` (named-pipe/UDS + cap-file path derivation).
- `src/daemon.rs:279-303` — `CapFile` (rotating instance_id/admin_cap/pid/start_time).
- `src/daemon.rs:319-335` — `DaemonState` struct (in-memory `members`/`waiters`/etc.).
- `src/daemon.rs:458-486` — `MemberRecord` struct, including `on_deliver`.
- `src/daemon.rs:2037-2065` — `serve()` accept loop; drain-triggered exit.
- `src/daemon.rs:2072-2104` — `new_state` (fresh empty `DaemonState`, no recovery pass).
- `src/daemon.rs:2676-2726` — `push_delivery_health` (`PushDeliveryHealth` classification,
  harness-neutral, member-scoped only).
- `src/daemon.rs:3492-3529` — `drain_members` (release leases, `clear_members()`).
- `src/daemon.rs:3634-3653` — `Request::Drain` handling (`Ack{"draining"}` +
  `ClientAction::Drain`).
- `src/daemon.rs:3785-3934` — `register_member` (explicit-provision vs. generic-refresh
  `on_deliver` preservation logic; live-pull-waiter conflict guard).
- `src/daemon.rs:4182-4214` — `drain_deferred` (idle-drain sweep; zero-member-safe no-op after
  restart).
- `src/daemon_ipc.rs:170-197` — `Request::Register` wire shape (`recovery`, `on_deliver`,
  `replace_on_deliver`, `on_deliver_wake_on_cc`).
- `src/daemon_ipc.rs:287-290` — `Request::Drain { proof }` (no successor/state-transfer field).
- `src/daemon_ipc.rs:307-311` — `NeedsAttachReason` (`RestartLost`, `DeliberatelyDetached`).
- `src/daemon_ipc.rs:456-537` — `MemberStatus` (existing diagnostics fields).
- `src/daemon_ipc.rs:551-570` — `StationHealth` enum.
- `src/daemon_ipc.rs:588-609` — `PushDeliveryHealth` enum.
- `src/backend/mod.rs:54-316` — `Backend` trait (epoch fence, tombstone, delivery-buffer
  methods; no membership/intent methods).
- `src/backend/sqlite.rs:884-1024` — SQLite schema (`leases`, `detach_tombstones`,
  `deliveries`, `clock_hwm`, fence-rejection trigger).
- `src/backend/sqlite.rs:2042-2100` — `record_detach_tombstone` / `clear_detach_tombstone` /
  `detach_tombstone` SQLite implementations.
- `src/commands/copilot.rs:141-260` — bridge home/extension-dir/bindings-path helpers,
  `read_bridge_bindings`/`write_bridge_bindings`/`add_bridge_binding`/`remove_bridge_binding`
  (bare `Vec<String>` addresses, no store/mode/CC metadata).
- `src/commands/copilot.rs:288-359` — `bridge_handler_argv`, `provision_bridge`, `detach`
  (bridge teardown on last-binding detach).
- `src/commands/copilot.rs:418-427` — `BridgeRegistry` (session_id/secret/max_request_bytes
  only).
- `src/commands/copilot.rs:463-506` — `bridge_root_dir`, `bridge_is_live` (mtime-window
  liveness only), `bridge_endpoint_path`.
- `src/commands/copilot.rs:970-1090` — `attach`/`resume` (the one complete, manual recovery
  path; verifies `daemon_armed_push` and rolls back on failure).
- `src/commands/copilot.rs:1477-1568` — `turn_guard` (fetches status, computes
  `active_session_members`, calls `evaluate_guard`).
- `src/commands/copilot.rs:1665-1770` — `gc` / `discover_bridge_sessions` (conservative
  cleanup only, not recovery).
- `src/commands/copilot.rs:1873-1886` — `active_session_members` (session/store/non-idle
  filter).
- `src/commands/copilot.rs:1956-2089` — `evaluate_guard` (empty-members "no_attended_stations"
  allow path; `push_dead`/`conflicts` computation, member-scoped only).
- `src/commands/wait.rs:132-256` — `wait_loop` (reconnect-on-EOF/`NeedsAttach` state machine).
- `src/commands/wait.rs:289-301` — `begin_reconnect`.
- `src/commands/wait.rs:351-383` — `register_for_retry` (`recovery: true, on_deliver: None`).
- `src/commands/send.rs:9-68` — `send::run` (`NeedsAttach` retry loop).
- `src/commands/send.rs:118-141` — `send::register_for_retry` (`recovery: true, on_deliver:
  None`).
- `src/commands/upgrade.rs:249-265,367-375` — upgrade/rollback call sites for `drain_daemon`.
- `src/commands/upgrade.rs:547-602` — `unauthorized_drain_message`, `drain_daemon`.
- `src/session_watch.rs:98-`  — `capture_process_start_time`, `process_alive_with_start_time`
  (existing PID+start-time liveness primitive, reusable for intent liveness proof).
- `copilot/plugin/hooks.json:1-27` — `sessionEnd`/`agentStop` hook wiring (turn-guard +
  idle-drain only; no repair hook).
- `copilot/bridge/extension.mjs:1-100` — module docstring, transport/endpoint derivation,
  `joinSession`, per-session secret.
- `copilot/bridge/extension.mjs:246-317` — `writeRegistry`, heartbeat interval, SIGTERM/SIGINT
  cleanup-if-still-current-pid.
- `copilot/bridge/busy-state.mjs` / `copilot/bridge/busy-state.test.mjs` — independently
  unit-tested busy/idle scheduling state machine (unrelated to daemon reconciliation).
- `docs/design/DECISIONS.md:1078-1181` — ADR 0023 (explicit-only in-memory membership,
  deliberate design decision this issue's work must reconcile with/supersede).
- `docs/design/daemon.md:403-480` — §5/§5.1 membership model + durable lease-row columns.
- `docs/design/daemon.md:1329-1400` — §11.4/§11.5 ordered handoff (ownership-only, not
  membership) + Postgres cross-machine reclaim.
- `docs/design/daemon.md:1450-1560` — §13.2 on-deliver push full contract (registration,
  fire-point, liveness-only, backoff/backstop/hard-cap, detach-tombstone trust model).
- `docs/design/daemon.md:1730-1804` — §14.2-14.5 sessionEnd hook, crash recovery/re-attach,
  `NeedsAttach`, daemon-down TTL backstop.
- `docs/design/ARCHITECTURE.md:142-168` — §4 restart & re-attach recovery diagram/prose.
- `docs/design/ARCHITECTURE.md:306-342` — §9 push delivery diagram/prose.
- `docs/guide/src/guides/operating.md:129-141` — user-facing "Recovering from a lost daemon" /
  "Turn-end and resume reconciliation" sections (pull-only recovery documented today).
- `docs/guide/src/guides/troubleshooting.md:37-45` — "Copilot: messages do not arrive as
  turns" (documents the manual `copilot resume` workaround).
- `tests/daemon_core_sqlite.rs:237-272` — `section17_04_restart_no_loss_no_resurrection`.
- `tests/daemon_process_sqlite.rs:53-64,149-186` — `ProcessEnv` real-process test harness.
- `tests/daemon_process_sqlite.rs:4081-4100` — `real_process_drain_respawn_epoch_advances`.
- `tests/copilot_plugin.rs:1-30,480-700` — plugin manifest / shell drain-hook launcher tests
  (fake-telex-on-PATH harness).
- `.github/workflows/ci.yml:28,41-104,163` — authoritative verification commands.
- `CONTRIBUTING.md:1-31` — backend conformance-suite instructions and Postgres env vars.

## Architecture Documentation

- **Explicit-only, in-memory membership is a ratified design decision (ADR 0023), not an
  oversight.** Any reconciliation mechanism must be framed as either (a) a new durable
  *intent* layer that is explicitly distinct from membership and never auto-resurrects it
  (issue option D's framing, and consistent with how the codebase already treats
  `detach_tombstones` as durable-but-non-membership state), or (b) a formal revision/extension
  of ADR 0023 with its own dated entry in `docs/design/DECISIONS.md`, following the existing
  `Revises:`/`Reopen conditions` convention observed in ADRs 0023, 0039, 0041-0043.
- **The daemon's push-attempt/health tracking is intentionally harness-neutral and
  member-scoped** (`push_delivery_health`'s doc-comment: "never reads the bridge registry").
  A design that has the daemon read Copilot-specific bridge files directly (issue option C)
  would cross a boundary the codebase currently keeps clean; the existing pattern instead
  routes all harness-specific data through the generic `Register.on_deliver` /
  `MemberRecord.on_deliver` argv and the generic `NeedsAttach` recovery contract — any new
  "generic registration-intent" primitive (issue's Stage 1 recommendation) would naturally
  extend `Request::Register`/`MemberRecord` rather than add a Copilot-specific daemon code
  path, consistent with ADR 0039's "generic on-deliver exec, not a Copilot-coupled daemon"
  framing (see `docs/design/copilot-bridge-push.md` §"3. Generic daemon on-deliver exec, not a
  Copilot-coupled daemon").
- **Backend-scoped asymmetry is already load-bearing and documented.** SQLite forbids live
  two-daemon overlap (single-writer store lock) and uses release+respawn; Postgres supports a
  live owner-directed epoch transfer when a successor is ready. Any "ordered handoff carrying
  registration state" (issue option E) needs a design that is explicit about which backend
  path it targets, since the existing epoch-transfer machinery is Postgres-only.
- **Existing security/liveness primitives are reusable but not currently wired together**:
  PID+start-time verification (`session_watch.rs`) exists for watched session/waiter PIDs;
  OS-peer + admin-capability verification exists for the daemon's own trust boundary; neither
  is currently applied to bridge-registry or "intent" liveness/authenticity — a "live endpoint
  nonce/challenge" (issue's explicit requirement, "not registry mtime alone") has no existing
  analog in the bridge/copilot.rs code and would be new surface.
- **Diagnostics are member-scoped by construction** (`MemberStatus`/`StationHealth`
  /`PushDeliveryHealth` all key off an existing `MemberKey`). Surfacing "intent exists but
  member does not" requires either a new status projection independent of `MemberStatus`, or
  restructuring status to enumerate intents alongside members — a design decision point, not
  purely additive.

## Open Questions

- Where should durable station-intent rows live: a new table alongside `leases`/
  `detach_tombstones` in each backend (SQLite + Postgres, parity via `tests/conformance.rs`),
  or a separate owner-private local file (analogous to `.bindings.json` today) read only by
  the local daemon at startup? The issue's option D says "durable" and "owner-private,"
  which could map to either a backend table (durable across hosts on Postgres, consistent with
  ADR 0023's durable-tombstone precedent) or a local-only file (matching the issue's explicit
  non-goal "Cross-host Postgres: only local owner-private intents should restore local
  bridges; backend rows alone must not reconstruct remote session membership" — implying the
  intent itself may need to be local-file-based even on the Postgres backend, distinct from
  the backend-resident lease/tombstone rows). This choice materially affects schema/migration
  design and was not resolved by existing code.
- The bridge (`extension.mjs`) currently has zero daemon-awareness; Stage 2's "the live bridge
  periodically checks daemon instance identity and triggers reconciliation" requires the
  bridge to gain either a daemon IPC client capability or a new local-file-based
  daemon-instance-change signal it can poll — no existing precedent in `extension.mjs` for
  either approach (it has never read the daemon cap file or any daemon IPC in its current
  form), so this is new design surface without a directly reusable existing pattern beyond the
  bridge's own heartbeat-file idiom.
- ADR 0023 states "tombstones are unnecessary... satisfied by removing implicit rebuild," yet
  `detach_tombstones` were subsequently added (post-ADR-0023, per `daemon.md` §13.2's
  self-stop/tombstone paragraph and its `#66`/`#67` issue references) — confirming the design
  has already evolved past ADR 0023's original text once. Whether a new ADR should formally
  supersede 0023's membership-resurrection stance (vs. layering a new "intent" concept beside
  it, framed as compatible with 0023) is a planning-level decision, not resolved here.
- No test file currently exercises the Copilot bridge's `extension.mjs` against a real (or
  mocked) daemon process end-to-end (only `busy-state.test.mjs` unit-tests the JS busy/idle
  logic in isolation, and `tests/copilot_plugin.rs` only tests the shell-level hook
  launchers against a fake `telex` binary). A reconciliation feature touching the bridge would
  need a new test harness shape (real Node process + real/simulated daemon), which does not
  currently exist anywhere in the test suite; how such a harness should be structured relative
  to the existing `ProcessEnv` (Rust-only) pattern in `daemon_process_sqlite.rs` is unresolved.
