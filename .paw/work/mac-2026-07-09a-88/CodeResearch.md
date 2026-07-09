---
date: 2026-07-09T17:02:04-04:00
git_commit: 97f596d402983b8e8100a2d567ea27ddd11e8c32
branch: feature/mac-2026-07-09a-88
repository: telex
topic: "Copilot delivery: first-class cross-platform detached-waiter fallback when extensions are unavailable"
tags: [research, codebase, copilot-bridge, telex-wait, turn-guard, station-stop, fallback]
status: complete
last_updated: 2026-07-09
---

# Research: Copilot detached-waiter fallback when extensions are unavailable (issue #88)

## Research Question

Issue #88 asks to keep the Copilot extension push bridge as the preferred delivery mode
while adding a **supported cross-platform single-shot detached `telex wait` fallback** for
when extensions/`extensions_reload` are unavailable. Map the existing implementation so a
plan can address: end-to-end macOS + Windows fallback, status/guard visibility of `push`
vs `pull-fallback` coverage, explicit safe mode transitions (stop one consumer before
starting the other), duplicate-waiter rejection/supersession, agent-owned durable
ack/disposition, at-least-once + message-id dedupe, no infinite polling loop, and
worker/orchestrator usability without platform-specific ad hoc scripts.

Workflow Mode is `custom` (WorkflowContext.md); per Custom Workflow Instructions the issue
itself is the spec — no `Spec.md` exists. This document builds directly on the Issue URL as
the requirements source.

## Summary

Telex is a single-binary Rust CLI (`src/`) plus a Copilot CLI marketplace plugin
(`copilot/plugin/`) and a binary-embedded, version-matched skill body (`copilot/COPILOT.md`).
The concrete building blocks the issue needs **already exist and are wired**:

- **Push (preferred):** `telex copilot attach --copilot-bridge` provisions an in-session
  bridge extension + registers a daemon `on_deliver` handler; the agent runs
  `extensions_reload` once; messages arrive as turns via `telex copilot push`. Liveness is a
  heartbeat-refreshed registry file (`bridge_is_live`), not a waiter.
- **Pull fallback (already documented, but Windows-centric and manual):** `telex wait
  --out-dir <dir>` is a single-shot, detached-friendly waiter that writes durable artifacts
  (`message.json`/`delivery.json`, `status.json`, `exit.code` last, `wait.pid` at start). The
  Copilot-specific fallback recipe currently lives in `copilot/COPILOT.md` and is expressed
  as a Windows `pwsh -File .ps1` wrapper; there is no macOS/Linux-native equivalent recipe
  and no Telex-owned helper that "prepares a unique out-dir and runs exactly one wait."
- **Guardrails already present:** the daemon rejects a concurrent second waiter per
  `(store_key, session_id, address)` returning `PresenceEnded` + `ConcurrentWaiter` audit
  (ADR 0029). `telex station stop` drains live waiters before durable detach (ADR 0027).
  The turn guard already treats a live push bridge OR a live waiter as covered and gives
  mode-specific remediation, surfacing a stale/dead bridge and unarmed pull stations.
  Status/guard already expose `push_registered`, `push_delivery`, `station_health`
  (`attended_push`), `live_waiters`, and `last_waiter_outcome`.

Neutral observations that inform planning (evidence-cited below, no solution proposed):
the root `SKILL.md`/`telex skill --address` preamble is **contractually harness-neutral**
(a test forbids Copilot mechanics, ADR 0044); the Copilot fallback recipe is embedded in
`copilot/COPILOT.md` as a Windows-only `pwsh` wrapper; the daemon does not itself forbid a
member from being simultaneously `on_deliver`-registered (push) and having a live waiter
(pull) — "exactly one active coverage mechanism" is coordinated by the agent/skill via the
attach/detach and station-stop verbs, not enforced in one daemon call.

## Documentation System

- **Framework**: mdBook (user guide) + hand-maintained Markdown ADR log.
- **Docs Directory**: `docs/guide/` (mdBook: `docs/guide/book.toml`, sources under
  `docs/guide/src/`); design/decisions under `docs/design/`.
- **Navigation Config**: `docs/guide/src/SUMMARY.md` (mdBook nav). New guide pages must be
  added here to appear.
- **Style Conventions**: Concise, task-oriented guide pages; "operator's view" prose with
  fenced `sh`/`powershell` examples; PowerShell env-var variants shown inline
  (`$env:VAR`). ADRs in `docs/design/DECISIONS.md` follow a fixed
  `## NNNN — Title / Date / Status / Context / Decision / Consequences` shape.
- **Build Command**: `mdbook build docs/guide`; the CLI reference page is generated (never
  hand-edited) by `docs/guide/generate-reference.sh <telex-binary>` from `--help` output
  (`.github/workflows/docs.yml`, ADR 0040 single-source principle).
- **Standard Files**: `README.md`, `CONTRIBUTING.md`, `SECURITY.md`, `SKILL.md` (root,
  embedded), `TELEX.md`, `PRODUCT-THESIS.md` at repo root. No `CHANGELOG.md`.
- **Source-of-truth note**: `telex skill` embeds root `SKILL.md`
  (`src/commands/skill.rs:10`); `telex copilot skill` embeds `copilot/COPILOT.md`
  (`src/commands/copilot.rs:58-59`). Editing the skill body means editing these embedded
  Markdown files; CI regenerates `docs/guide/src/reference/cli.md` from the binary.

## Verification Commands

- **Build**: `cargo build --workspace` (root `telex` + `telex-console`; `spike/` excluded —
  `Cargo.toml:11-14`). Feature combos also built in CI: `--no-default-features --features
  sqlite` / `--features postgres` / `--features entra` (`.github/workflows/ci.yml`).
- **Test**: `cargo test --workspace`. Bridge JS is separately checked in CI with
  `node --check copilot/bridge/extension.mjs` and `node --test
  copilot/bridge/busy-state.test.mjs` (`.github/workflows/ci.yml:23-27`).
- **Lint**: `cargo clippy --workspace -- -D warnings` (CI `continue-on-error: true`).
- **Format**: `cargo fmt --check` (CI `continue-on-error: true`).
- **Type check**: covered by `cargo build` / `cargo test` (Rust; no separate typecheck).
- **CI matrix**: `ubuntu-latest` and `windows-latest` (`.github/workflows/ci.yml:14-18`) —
  **no macOS runner**; macOS behavior is not currently exercised in CI.

## Detailed Findings

### 1. CLI surfaces (arg definitions)

`src/cli.rs` defines the clap command tree.

- `WaitArgs` (`src/cli.rs:174-206`): `--session` (env `TELEX_SESSION_ID`), `--timeout-ms`,
  `--min-attention`, `--wake-on-cc`, deprecated `--since`/`--hang-ms`/`--stale-heartbeat-ms`,
  `--reconnect-grace-ms` (env `TELEX_RECONNECT_GRACE_MS`), and `--out-dir <PathBuf>` (writes
  `message.json` on delivery, `status.json` always, `exit.code` last as completion marker).
- `CopilotCmd` (`src/cli.rs:442-466`): `attach`, `resume` (alias `repair`), hidden
  `session-end`, hidden `turn-guard`, `skill`, hidden `push`, hidden `drain`, `detach`, `gc`.
- `CopilotAttachArgs` (`src/cli.rs:544-568`): `--session`, `--description`, `--scope`,
  `--tags`, `--occupant`, `--copilot-bridge` (provision push), `--wake-on-cc`.
- `CopilotResumeArgs` (`src/cli.rs:570-590`), `CopilotDetachArgs` (`src/cli.rs:621-626`),
  `CopilotSkillArgs` (`src/cli.rs:641-647`: `--plugin-version`, env `TELEX_PLUGIN_VERSION`).
- `SkillArgs` (`src/cli.rs:649-657`): generic `telex skill` with `--address`, `--raw`.
- `StationCmd` (`src/cli.rs:215-241`): `status` (`--all-sessions`) and `stop`
  (`--wait-grace-ms`, default 3000).

### 2. Skill / docs source-of-truth and the harness-neutral boundary

- `telex skill` prints embedded root `SKILL.md` (`src/commands/skill.rs:10,17-40`).
  `telex skill --address <addr>` prepends `assignment_preamble` (`src/commands/skill.rs:46-99`),
  which describes the generic single-shot detached `telex wait --out-dir` loop, `ack`, dedupe
  by id, re-arm, `telex station stop` teardown, and "don't wrap wait in an infinite shell
  loop." It routes harness specifics via a **neutral pointer** ("run `telex <harness> skill`").
- **Contract constraint (planning-relevant):** `assignment_preamble` is asserted
  harness-neutral by test `assignment_preamble_is_harness_neutral`
  (`src/commands/skill.rs:132-157`), which **forbids** strings like `detach: true`,
  `pwsh -File`, `list_powershell`, `COPILOT_AGENT_SESSION_ID`, `copilot attach/detach`,
  `extensions_reload`. Copilot-specific detached-waiter mechanics may not live here (ADR 0044).
- `telex copilot skill` embeds `copilot/COPILOT.md` (`src/commands/copilot.rs:58-59`) and
  wraps it with a version/compat header (`render_copilot_skill`,
  `src/commands/copilot.rs:1229-1257`; `plugin_compat_warning`,
  `src/commands/copilot.rs:1205-1227`). Constants: `COPILOT_BRIDGE_PROTOCOL = 1`
  (`src/commands/copilot.rs:62`), `MIN_COMPATIBLE_PLUGIN_VERSION = "0.1.0"`
  (`src/commands/copilot.rs:65`).
- **Where the current fallback recipe lives:** `copilot/COPILOT.md` §"Fallback: no bridge
  (pull mode)" — it explicitly documents the **Windows/Copilot** detached waiter pattern
  as a `pwsh -NoProfile -ExecutionPolicy Bypass -File ...telex-wait-once.ps1` wrapper with
  an inline `.ps1` body, plus the completion-artifact reading protocol (`exit.code` first,
  then `delivery.json`/`message.json`, ack, dedupe, re-arm), the "don't trust the detached
  task's reported exit code," "don't use `list_powershell`," and "never wrap in an infinite
  shell loop" rules. There is **no macOS/Linux-native shell recipe** and **no Telex-owned
  helper command** that prepares a unique out-dir and runs one wait.
- Plugin bootstrap: `copilot/plugin/skills/telex/SKILL.md` is a thin bootstrap that defers to
  `telex copilot skill` (push) and `telex skill` (pull); `copilot/plugin/plugin.json`
  (version `0.1.0`) and `copilot/plugin/hooks.json` wire `sessionEnd` →
  `telex copilot session-end` and `agentStop` → `telex copilot turn-guard` + `telex copilot
  drain` (bash + powershell command variants each).

### 3. `telex wait` and `--out-dir` artifacts (the fallback primitive)

`src/commands/wait.rs`:

- Entry `run` (`src/commands/wait.rs:14-75`): resolves address/store/session, builds
  `WaitLoopConfig` capturing `waiter_pid = std::process::id()` and
  `waiter_start_time = capture_process_start_time(...)` (`:44-46`), writes start artifacts
  when `--out-dir` set (`:48-54`), runs `wait_loop`, then `emit_outcome`.
- `wait_loop` (`src/commands/wait.rs:129-259`): single blocking daemon `Wait` request with
  reconnect/re-register grace on daemon restart; returns on `Message | Timeout |
  PresenceEnded`, or `DaemonGone`/`DaemonHung`. It is **single-shot** — no internal polling
  loop over deliveries.
- `WaitOutcome` → exit codes (`src/commands/wait.rs:426-497`): `0` message, `2` idle-timeout,
  `3` daemon-gone, `4` daemon-hung, `5` presence-ended, `1` error (also documented at
  `docs/guide/src/reference/exit-codes.md`).
- `write_wait_artifacts` (`src/commands/wait.rs:532-573`): writes `message.json` +
  `delivery.json` (delivery only; envelope has `message`/`delivery`/`status`), always
  `status.json`, and `exit.code` **last** as the completion marker; clears stale
  `message.json`/`delivery.json` on a non-delivery outcome when out-dir is reused.
- `write_wait_start_artifacts` (`src/commands/wait.rs:578-590`): publishes `wait.pid` and a
  `status.json` `{outcome:"armed"}` at start, after removing stale completion artifacts.
- `ensure_out_dir`/`atomic_write` (`src/commands/wait.rs:606-651`): Unix dir `0700`, files
  `0600`; atomic temp+rename. Cross-platform via `#[cfg(unix)]`/`#[cfg(not(unix))]`.
- Artifacts are **transport only**, not the consumed mark (ADR 0026); ack is still the fence.

### 4. Copilot push bridge: provisioning, attach/detach/resume/session-end

`src/commands/copilot.rs`:

- Bridge files: extension dir `~/.copilot/session-state/<session>/extensions/telex-bridge/`
  (`bridge_extension_dir`, `:93-99`); embedded `extension.mjs` + `busy-state.mjs` written by
  `write_bridge_extension` (`:109-116`; embedded at `:82-84`). Bindings ref-count file
  `~/.copilot/telex-bridge/<session>.bindings.json` (`bridge_bindings_path`, `:101-107`) with
  lock+atomic write (`add_bridge_binding`/`remove_bridge_binding`, `:159-186`).
- `provision_bridge` (`:255-286`): fail-closed — writes extension, records binding, returns
  the `on_deliver` handler argv (`telex ... copilot push --session <id>`,
  `bridge_handler_argv`, `:216-239`); rolls back on any failure.
- `attach` (`:903-991`): if `--copilot-bridge`, ignores `COPILOT_LOADER_PID` (bridge heartbeat
  is the liveness signal), provisions bridge, calls generic `attach` with
  `no_session_bind: true` + `on_deliver: Some(argv)`, then **verifies push actually armed**
  via `daemon_armed_push` and **rolls back** a half-armed bridge (Namra #5). Non-bridge attach
  anchors `COPILOT_LOADER_PID` as a `--watch-pid`.
- `resume` (`:992-1006`): re-provisions the same bridge registration (alias `repair`).
- `detach` (`:287-312`): generic detach + bridge teardown only when this was the last binding.
- `session_end` (`:1008-1089`): non-destructive daemon SessionEnd; removes bridge files on
  success; cleans turn-guard state.
- `push` (`:564-724`) / `drain` (`:740-860`): `push` delivers a descriptor (stdin) into the
  session over the bridge endpoint; `drain` (agentStop hook) flushes messages deferred while
  the session was busy (ADR 0043; `drain_enabled` honors `TELEX_COPILOT_DRAIN=off`, `:725-739`).
- `gc` (`:1268-1373`): garbage-collect orphaned bridge files; keeps live heartbeats.

### 5. Bridge liveness heartbeat vs. registration

- `bridge_is_live(session_id)` (`src/commands/copilot.rs:402-419`): true iff the registry
  file `~/.copilot/telex-bridge/<session>.json` (`bridge_registry_path`, `:385-391`) was
  modified within `BRIDGE_LIVENESS_WINDOW` (60s, `:43`). This is the "bridge loaded and
  reachable" signal, distinct from daemon `push_registered` ("handler registered"). Endpoint
  derivation is per-platform: Windows named pipe `\\.\pipe\telex-bridge-<session>`
  (`:423-425`), Unix socket `~/.copilot/telex-bridge/<session>.sock` (`:427-435`).

### 6. Turn guard + coverage model (agentStop hook)

`src/commands/copilot.rs`:

- `turn_guard` (`:1090-1181`): resolves session, loads `GuardSettings`
  (`TELEX_TURN_GUARD`, `TELEX_TURN_GUARD_MAX_NUDGES` default 3; `:1480-1511`), reads daemon
  status, computes `active_members`, reads `bridge_is_live`, calls `evaluate_guard`, persists
  nudge state under a `StateLock`, logs to `hook-events.ndjson`, emits an allow/block JSON.
- `evaluate_guard` (`:1547-1655`) partitions members into:
  - `unarmed`: `live_waiters_count == 0 && !push_registered` (needs a waiter or detach),
  - `delivered_unacked`: pull member with pending + `last_waiter_outcome == Message`,
  - `push_dead`: `push_registered` members **only when `!bridge_live`** (stale/crashed bridge
    surfaced as uncovered),
  - a live push member with unacked backlog is deliberately **not** nudged (enqueue turns may
    be queued behind the current turn; ADR 0041).
  Emits `covered` (allow) when all sets empty, else a `coverage_gap` block with a
  mode-specific `guidance` string: push-dead → "run `extensions_reload` ... or `telex detach`";
  unarmed → "Re-arm `telex wait ... --out-dir <dir>` ... or `telex detach`"; unacked → "Ack
  ... before ending the turn." Nudge cap then allows (`cap_exhausted`).
- `coverage_summary`/`coverage_issue_key` (`:1657-1720`) build the human summary and a stable
  key so repeated nudges for the same gap are counted together.
- **Planning-relevant:** the guard already implements the acceptance item "either a live push
  bridge or a live fallback waiter counts as covered, with mode-specific remediation." A
  `pull-fallback` waiter is covered via `live_waiters_count > 0`.

### 7. Daemon: waiter registry, one-waiter-per-station, station stop, push health

`src/daemon.rs` (+ `src/daemon_ipc.rs` types):

- **One live waiter per station (duplicate rejection):** on `Wait`, if
  `has_live_waiter_for(store_key, session_id, address)` the daemon records a
  `ConcurrentWaiter` recent-error and returns `Response::PresenceEnded` without emitting a
  message (`src/daemon.rs:4372-4381`; ADR 0029, `has_live_waiter_for` at `:1082-1090`). This
  is the existing "duplicate waiter launches are rejected/safely superseded" primitive.
- **Live waiter registry:** waiters recorded by daemon-assigned `waiter_id` carrying pid +
  start_time (`add_waiter`/`remove_waiter`/`prune_dead_waiters`, `:1091-1205`;
  `WaiterGuard` RAII `:1470-1523`). Status exposes top-level `live_waiters` and per-member
  `live_waiters` (`LiveWaiterStatus`, `src/daemon_ipc.rs:592-616`).
- **Station stop drains before detach:** `station_stop` (`src/daemon.rs:4098-4166`) snapshots
  `waiters_before` + `push_registered`, marks the member idle so a blocked waiter returns
  `PresenceEnded`, calls `wait_for_waiters_to_drain` (grace loop, `:4162-4189`), then durably
  detaches; returns typed `StationStopped { waiters_before, waiters_after, live_waiters,
  push_registered, ... }` (ADR 0027). CLI `station stop` prints a warning when
  `push_registered` that membership was released but the in-session push producer may still be
  loaded, pointing at `telex copilot detach` (`src/commands/station.rs:186-193`).
- **Push registration vs. waiter coexistence (constraint, no critique):** a member's
  `on_deliver` (push) is stored on the same member record that also tracks live waiters; the
  daemon's only per-station consumer exclusivity check is the *concurrent-waiter* guard above.
  There is no single daemon call that atomically forbids a member from being both
  `on_deliver`-registered and waiter-armed. `station_health` for an `on_deliver` member is
  computed from push health and reported `AttendedPush` (never `unattended`) regardless of
  waiter presence (`src/daemon.rs:1384-1424`; ADR 0042). "Exactly one active coverage
  mechanism" is therefore coordinated by the agent/skill via attach(`--copilot-bridge`) /
  `copilot detach` / `station stop` transitions, not by a single enforcing daemon primitive.
- **Push health types:** `PushDeliveryHealth` (`NotRegistered|NoBacklog|Delivering|Probing|
  StaleAccepted|Failing|Unknown`, `src/daemon_ipc.rs:568-590`) and `StationHealth`
  (`Armed|RecentlyDelivered|Unattended|UnattendedWithBacklog|AttendedPush|Idle|Unknown`,
  `:544-566`).

### 8. Status surfaces (mode + coverage visibility)

- `telex status --address` (`src/commands/status.rs:31-118`) emits `station_health`,
  `push_registered`, `push_delivery`, `push_wake_on_cc`, `push_deferred_count`,
  `push_suppressed_count`, `live_waiters`/`live_waiters_count`, `last_waiter_outcome`/
  `..._exit_code`/`..._detail`/`..._pid`, and a human line showing `push=<delivery>` and
  waiter count. Also cross-backend `also_active_on` warning.
- `telex station status` (`src/commands/station.rs:16-141`) emits the same per-member fields
  including `push_registered`/`push_delivery`/`live_waiters`.
- **Planning-relevant:** there is currently a `push=<health>` indicator and a
  `live_waiters` count, but no single explicit `mode: push | pull-fallback` label field.

### 9. Cross-platform process/helper patterns

`src/session_watch.rs`:

- `process_alive` (`:90`), `capture_process_start_time` (`:98`),
  `process_alive_with_start_time` (`:107`) with `#[cfg(unix)]` (Linux `/proc/<pid>/stat`,
  macOS `sysctl KERN_PROC`) and `#[cfg(windows)]` (`OpenProcess`/`GetProcessTimes`)
  implementations (`:121-263`). This is the established pattern for pid+start-time identity
  used to track/verify detached waiters (also consumed by `wait.rs` and the daemon registry).
- Bridge endpoints and out-dir creation similarly branch on `#[cfg(windows)]`/`#[cfg(unix)]`
  (`src/commands/copilot.rs:421-435`, `src/commands/wait.rs:606-651`).
- The `spike/` directory holds throwaway PowerShell probes (`spike/*.ps1`) and is **excluded
  from the workspace** (`Cargo.toml:13-14`); not shipped/tested. The only shipped `.ps1` is
  `install.ps1`. No Telex-owned cross-platform "run one wait into a unique out-dir" helper
  script or generated command exists today.

### 10. Tests and fixtures

- `tests/copilot_plugin.rs`: asserts plugin manifest/hooks/marketplace source
  (`copilot/plugin`), the thin-bootstrap skill byte ceiling, and that hooks wire
  `session-end`/`turn-guard`/`drain`.
- `src/commands/skill.rs:110-157` (unit tests): `assignment_preamble_is_harness_neutral`
  enforces the neutral-boundary forbidden-strings list — a plan touching the generic skill
  must keep it harness-neutral.
- `src/commands/wait.rs:637+` unit tests script the wait loop (reconnect/PresenceEnded/timeout)
  via `WaitConnector`/`WaitClient` trait fakes.
- `src/daemon.rs` async tests include `station_stop_drains_live_waiter_and_status_lists_pid`
  (`:8147+`), concurrent-waiter behavior, and `session_end_marks_idle_releases_waiter...`.
- `src/daemon_ipc.rs` tests cover `PushDeliveryHealth` serde forward-compat (`:990+`).
- `copilot/bridge/busy-state.test.mjs` (`node --test`) exercises the bridge busy/idle contract.
- No end-to-end macOS/Windows fallback integration test exists; CI runs on ubuntu + windows
  only (no macOS), and the detached-waiter fallback is exercised by documentation/runbook, not
  an automated cross-platform test.

### 11. Design docs / ADRs directly governing this work

- `docs/design/DECISIONS.md`:
  - **0003** telex owns long waiting (no agent-authored loops); **0004** holder+ephemeral
    client split.
  - **0026** `telex wait --out-dir` outcome artifacts for detached delivery (the artifact
    contract, transport-only, why the `.ps1 -File` wrapper was needed on Windows).
  - **0027** station stop + live waiter registry + status reconciliation.
  - **0028** only `attach` auto-spawns the daemon (`wait` exits 3 if none).
  - **0029** one live waiter per station (concurrent `Wait` → PresenceEnded).
  - **0030** attention-gated waits (`--min-attention`).
  - **0037** wait out-dir flat + enveloped delivery artifacts.
  - **0039** push delivery via generic `on_deliver` exec + Copilot bridge; `telex wait`
    retained as the harness-agnostic pull fallback.
  - **0040** Copilot skill is binary-owned; plugin skill is a bootstrap.
  - **0041** on-deliver re-delivery is re-provision-triggered (at-least-once, dedupe by id).
  - **0042** bridge-aware station health + durable self-stop.
  - **0043** bridge defers non-interrupt pushes until turn-stop, drained by agentStop.
  - **0044** harness-neutral root skill; per-harness content nested under `<harness>/`; the
    root skill "now also owns the pull-mode fallback recipe" is assigned to `copilot/COPILOT.md`
    for Copilot mechanics. **Adding cross-harness or code/CI touchpoints has a recorded
    convention here** (module `src/commands/<harness>.rs`, marketplace entry, CI `node --check`).
- Deeper design: `docs/design/copilot-bridge-push.md`, `docs/design/daemon.md`
  (§13.2 on-deliver push, §11.3 delivery fence).
- Runbook: `docs/developing/runbooks/copilot-session-validation-runbook.md`;
  plugin acceptance matrix: `docs/design/copilot-plugin-validation.md`.

## Code References

- `src/cli.rs:174-206` — `WaitArgs` (`--out-dir`, `--min-attention`, `--timeout-ms`, session).
- `src/cli.rs:442-466,544-590,621-647` — `CopilotCmd` and attach/resume/detach/skill args.
- `src/cli.rs:215-241` — `StationCmd` status/stop args.
- `src/commands/wait.rs:14-75` — wait entry, out-dir start artifacts, waiter pid/start-time.
- `src/commands/wait.rs:426-497` — `WaitOutcome` and exit-code mapping.
- `src/commands/wait.rs:532-590,606-651` — artifact writing (`exit.code` last), start artifacts, atomic/owner-only IO.
- `src/commands/skill.rs:10,17-40,46-99` — embedded `SKILL.md`, generic assignment preamble (detached wait loop).
- `src/commands/skill.rs:132-157` — harness-neutral forbidden-strings test (boundary contract).
- `copilot/COPILOT.md` — Copilot workflow; §"Fallback: no bridge (pull mode)" is the current Windows `pwsh -File` detached-waiter recipe (no macOS/Linux-native recipe, no Telex-owned helper).
- `src/commands/copilot.rs:58-65` — embedded `COPILOT.md`, `COPILOT_BRIDGE_PROTOCOL`, min plugin version.
- `src/commands/copilot.rs:93-186,255-312` — bridge file layout, bindings ref-count, provision/detach.
- `src/commands/copilot.rs:402-435` — `bridge_is_live` heartbeat + per-platform endpoint derivation.
- `src/commands/copilot.rs:903-1006` — `attach`/`resume` (bridge arm + fail-closed verification/rollback).
- `src/commands/copilot.rs:1090-1181,1547-1720` — turn guard + coverage model (push-live vs waiter-live).
- `src/commands/copilot.rs:1229-1257` — `render_copilot_skill` header/compat.
- `src/commands/status.rs:31-118` — status fields (push_registered/push_delivery/live_waiters/last_waiter_outcome).
- `src/commands/station.rs:143-200` — `station stop` typed summary + push-registered warning.
- `src/daemon.rs:4098-4189` — `station_stop` drain-then-detach; `wait_for_waiters_to_drain`.
- `src/daemon.rs:4372-4381,1082-1090` — concurrent-waiter rejection (`ConcurrentWaiter`/`PresenceEnded`).
- `src/daemon.rs:1384-1424` — push-member `station_health` = `AttendedPush` (no waiter expected).
- `src/daemon_ipc.rs:456-616` — `MemberStatus`, `WaiterOutcome`, `StationHealth`, `PushDeliveryHealth`, `LiveWaiterStatus`.
- `src/session_watch.rs:90-263` — cross-platform process alive / start-time helpers.
- `copilot/plugin/{plugin.json,hooks.json,skills/telex/SKILL.md}` — marketplace plugin + hooks + bootstrap.
- `.github/workflows/ci.yml:14-46` — ubuntu+windows matrix, bridge `node --check`/`node --test`, cargo build/test.
- `docs/guide/src/guides/{agent-pull.md,copilot-push.md}` — pull loop guide; Copilot push guide + `## Fallback`.
- `docs/guide/src/reference/exit-codes.md` — wait exit-code reference.
- `docs/design/DECISIONS.md` — ADRs 0026/0027/0028/0029/0030/0039/0040/0041/0042/0043/0044.

## Architecture Documentation

- **Harness boundary (ADR 0039/0044):** the Rust daemon/core is harness-neutral (a single
  opaque `on_deliver` argv + stdin descriptor); all Copilot specifics live in
  `src/commands/copilot.rs` + `copilot/`. The root `SKILL.md`/`telex skill` output is
  contractually harness-neutral (test-enforced); Copilot mechanics are reached via the
  `telex <harness> skill` pointer.
- **Skill single-source (ADR 0040):** skill bodies are `include_str!`-embedded Markdown
  (`SKILL.md`, `copilot/COPILOT.md`) so `telex skill` / `telex copilot skill` always match the
  binary; the mdBook CLI reference is generated from `--help` on every docs build.
- **Delivery invariants (ADR 0026/0029/0041):** `--out-dir` artifacts are transport-only; ack
  is the durable consume fence; one live waiter per station; at-least-once with dedupe-by-id;
  re-delivery is re-provision-triggered. Skills instruct: read → ack → dedupe by id → re-arm,
  and never wrap `wait` in an infinite shell loop.
- **Teardown/transition mechanics (ADR 0027):** `station stop` = mark idle → drain live
  waiters → durable detach; `copilot detach` additionally tears down the bridge on last
  binding. These are the existing verbs a safe push↔pull-fallback transition composes from.
- **Cross-platform conventions:** `#[cfg(unix)]`/`#[cfg(windows)]` branching for endpoints,
  out-dir permissions, and process identity; pid+start-time is the process-identity primitive.

## Open Questions

1. **Coverage-exclusivity enforcement point.** "Exactly one active coverage mechanism"
   (acceptance) is today composable from existing verbs (`copilot attach --copilot-bridge`,
   `copilot detach`, `station stop`, the concurrent-waiter guard) but is **agent/skill-
   coordinated**, not enforced by one daemon primitive; a member can hold `on_deliver` and a
   waiter simultaneously without a dedicated rejection. Whether the plan should add explicit
   enforcement or rely on documented transitions is a design decision.
2. **Mode label surface.** Status exposes `push_registered`/`push_delivery`/`live_waiters` but
   no single explicit `mode: push | pull-fallback` field; the plan will decide whether a new
   surfaced field is desired or whether existing fields satisfy "mode visible in status/guard."
3. **Home of the cross-platform fallback recipe/helper.** The neutral-boundary test forbids
   Copilot mechanics in the generic skill; the current detached recipe (Windows `pwsh -File`)
   lives in `copilot/COPILOT.md`. Whether a "small Telex-owned helper or generated command"
   (issue proposed-shape #3) becomes a new subcommand, a generated script, or expanded
   embedded recipe — and how a macOS/Linux shell-native path is expressed without violating
   the harness-neutral contract — is unresolved.
4. **macOS end-to-end verification.** CI has no macOS runner; the fallback is validated by
   docs/runbook rather than automated tests. How macOS acceptance ("exercised end-to-end on
   macOS and Windows") is demonstrated/gated is undefined by the current test infrastructure.
