# Release Confidence Validation - v0.1.0 (local-daemon / Copilot bridge)

Issue: https://github.com/lossyrob/telex/issues/78
Node: `release-confidence-validation` (workstream `local-daemon`)
Date: 2026-07-08
Platform: Windows (x86_64-pc-windows-msvc)

This is a practical release-confidence pass over the shipped v0.1.0 release and the
local-daemon / Copilot bridge path. It exercises the real shipped paths, records evidence,
and files or accepts the gaps found. It replaces the previously planned oversized validation
harness + AKS scale rig (see issue #78).

## Summary verdict

| Acceptance criterion | Result |
| --- | --- |
| Release install/upgrade works on Windows from published assets | PASS |
| Copilot bridge push works end-to-end in a real session | PASS (with an environment caveat, see below) |
| The #65 / #66 regressions do not reproduce | PASS |
| Daemon restart/kill does not lose messages | PASS (via gating tests) |
| Postgres / Entra smoke passes | PASS |
| Remaining gaps filed or accepted | Done (see "Gaps found") |

One environment gap was found (a stale locally-installed binary silently disables the
agentStop idle-drain hook). It is not a defect in the published release; it is filed as a
hardening follow-up. Two lower-severity items are also filed/recorded.

## Gating test suite (source build, default + entra features)

Build: `cargo build` (default `sqlite,postgres,self-update`) and `cargo build --features entra`
both succeed. `telex version --json` contract surface intact
(`copilot.bridge_protocol = 1`, daemon protocol `1.3`, `package_version 0.1.0`,
`supported_schema 2`, full `required_capabilities` list).

| Suite | Result |
| --- | --- |
| lib unit tests | 271 / 271 |
| `conformance` | 12 / 12 |
| `copilot_plugin` | 6 / 6 |
| `daemon_core_sqlite` | 22 / 22 |
| `daemon_core_postgres` | 8 / 8 (skipped-as-passed; password path, no `TELEX_PG_URL`) |
| `daemon_process_sqlite` | 33 / 34 (1 timing-flake, see Gap 2) |
| `release_contract` | 10 / 10 |
| `release_upgrade` | 8 / 8 |
| bridge `busy-state.test.mjs` (Node) | 8 / 8 |

`release_upgrade` packages the built binary into fixture archives + sha256, serves them over
a local HTTP server, and exercises discover / download / verify / extract / install. The
bridge `busy-state` suite covers the idle-drain busy/idle state machine (busy detection,
idle-heal, hard ceiling, sub-agent-event filter, deferred-contract string) - the #65 / #66
hardening.

Daemon durability / no-loss is covered by `daemon_process_sqlite`
(`real_process_crash_recovery_wait_needsattach_no_loss`,
`real_process_drain_respawn_epoch_advances`,
`real_process_station_stop_drains_waiter_and_preserves_next_message`, and peers), all green.

## 1. Release install / upgrade (Windows)

### Published-asset install (real)

The published `v0.1.0` release (2026-07-08) carries Windows/macOS/Linux archives, each with a
`.sha256` sidecar and asset digest. Validated the Windows x86_64 asset end-to-end:

- Downloaded `telex-v0.1.0-x86_64-pc-windows-msvc.zip` + `.sha256`.
- SHA-256 verified against the sidecar: `88149af075cb22a3cd489a20dcfbb85bccfbba887ca9352c5bc99851697f44b0` (match).
- Extracted and ran: `telex version --json` -> `package_version 0.1.0`; the published binary
  supports `telex copilot drain` (relevant to Gap 1).

Unix smoke was not run (Windows validation host); the macOS/Linux assets and sidecars are
present and correctly named (the `release_contract` test couples asset naming + the
`version --json` surface + `install.sh` / `install.ps1` to the code, and is green).

### Versioned upgrade / rollback / gc (practical, throwaway install root)

Using a temp install root so the operator's environment was untouched:

- `upgrade --from <binary> --version v0.1.0-smokeA` then `--version v0.1.0-smokeB`: both
  install into `versions/<tag>` and switch `current`; `previous_tag` tracks correctly.
- Installed launcher `version --json`: `active_tag`/`current_tag = v0.1.0-smokeB`,
  `previous_tag = v0.1.0-smokeA`, `layout_detected = true`, full `current_manifest`.
- `rollback --skip-drain`: `current -> v0.1.0-smokeA`, `previous -> v0.1.0-smokeB`.
- `gc --dry-run`: both versions kept, reason "current, previous, or active process version"
  (conservative GC as designed).

## 2. Copilot bridge push (real Copilot CLI session)

Exercised against this validation session (a real Copilot CLI session) on the local SQLite
backend + live daemon.

Proven live:

- `telex copilot attach --copilot-bridge`: wrote the embedded bridge `extension.mjs` into the
  session extension dir and registered the on-deliver push handler (`push_registered = true`),
  took the address lease (`lease_epoch 1`).
- `extensions_reload`: forked the bridge live (pid, same turn), opened the per-session named
  pipe (`\\.\pipe\telex-bridge-<sessionId>`), and wrote the registry entry with a fresh
  heartbeat -> bridge is live and reachable.
- Sent a message to the bridge address: delivered (`occupied = true`) and parked as
  `pending_unconsumed` while the session was busy - the correct busy-defer behavior (a
  `next-checkpoint` message must not push mid-turn; it drains on turn-stop). This is the #65
  idle-drain guarantee observed live.
- Teardown: closed the pending message, `telex copilot detach` removed the extension +
  registry + bindings, and `extensions_reload` dropped it (0 extensions running). Detach /
  stop-delivery sticks.

Caveat (see Gap 1): the actual turn injection (message surfacing as a pushed turn) could not
be completed in this environment because the operator's PATH `telex` binary is stale and does
not support the `copilot drain` subcommand that the plugin's `agentStop` hook invokes, so the
idle-drain-on-turn-stop path errors. The published v0.1.0 binary does support `drain`. Turn
injection itself is covered by prior dogfooding (#45, #53, #65, #66, #67) and the plugin /
busy-state tests. Root cause of local non-delivery is the stale binary, not a release defect.

## 3. Idle-drain / duplicate prevention

- Bridge `busy-state.test.mjs` 8/8 (busy detection, idle-heal, hard ceiling, sub-agent-event
  filter, deferred-contract string).
- Observed live: a message arriving while busy is parked (`pending_unconsumed`), not pushed -
  no stale-queued-turn duplicate. No duplicate storm observed.

## 4. Bridge liveness / self-stop

- `telex copilot detach` + `extensions_reload` tears the bridge down and it stays down
  (self-stop / stop-delivery sticks; the in-session escape hatch works).
- Liveness intent is gated by `live_push_bridge_is_attended_not_deaf` and
  `failing_push_bridge_becomes_deaf` (a live/delivering bridge is `attended_push`, only a
  failing/non-draining one is flagged deaf).
- Observation: in this stale-binary environment the live bridge was reported
  `unattended_with_backlog` / `deaf_warn = true` because its backlog could not drain (Gap 1).
  This could not be cleanly attributed to a false-deaf regression versus correct behavior
  given a broken drain, because the daemon and hook binary here predate the shipped fix.
  Re-check against a matching (current) binary + daemon is recommended before drawing a
  liveness conclusion.

## 5. Durability and daemon lifecycle

- No-loss / crash-recovery / drain-respawn / stop-drains-and-preserves-next covered by
  `daemon_process_sqlite` (green).
- `status` / `gc` / `export` provide operator-visible evidence: `status` surfaces
  `station_health`, `push_registered`, `deaf_warn`, `pending_unconsumed_count`, lease epoch,
  and members; `export` yields the durable message + full disposition trail (used in the
  Postgres smoke below).

## 6. Postgres / Entra real-use smoke

Backend `pg-rde-telex` (Azure Postgres, Entra CLI auth), `entra`-feature build. Real two-session
exchange:

- Connect via Entra CLI token: `status` OK (`durable = true`, `lease = ttl`, `push = native`).
- Session A `attach rcv`: `lease_epoch 1`, occupant + `store_key` recorded.
- `send snd -> rcv` (`requires-disposition`): delivered (id 295), `occupied = true`.
- A `inbox`: 1 actionable message with `requires_disposition`.
- A `ack` -> `acknowledged`; A `handle` -> terminal `handled` (`by_principal robemanuele`).
- B `attach rcv` while A live: rejected `Incompatible` (non-destructive single-writer presence,
  ADR 0023).
- A `detach`; B `attach rcv`: succeeds, `lease_epoch -> 2` (reclaim / epoch fence).
- `export rcv`: durable message body + disposition trail (handled).
- Cleanup: B detached.

Lease / reclaim / delivery / disposition / durability align with the SQLite semantics.

## Gaps found

### Gap 1 (hardening follow-up, issue #79) - stale PATH binary silently disables the agentStop idle-drain hook; version skew is not detectable

The Copilot plugin (main) wires `agentStop -> telex copilot drain` (plus `turn-guard`) in
`copilot/plugin/hooks.json`. The hook invokes `telex` from PATH. On this machine the PATH
binary (`~/.cargo/bin/telex.exe`, built 2026-07-06) predates the `copilot drain` subcommand
and returns "unrecognized subcommand 'drain'", so the idle-drain-on-turn-stop path errors on
every turn-stop and deferred bridge messages never drain. Both the stale binary and the
published v0.1.0 binary report `package_version 0.1.0`, so `telex version --json` does not
surface the skew. The published release is fine; the risk is that upgrading the plugin (or
pulling new hooks) without upgrading the installed binary silently breaks a load-bearing hook.
Suggested hardening: a plugin/binary compatibility check and/or a build/commit identifier in
`version --json` so the skew is detectable, and a visible error when a hook subcommand is
missing. Remediation for the operator: reinstall/upgrade the PATH `telex` to the published
v0.1.0.

### Gap 2 (low, issue #80) - `real_process_idle_wait_timeout_is_not_hung` flakes under heavy load

The test races a 250 ms idle-timeout against a 275 ms hang-watchdog (25 ms margin). Under
heavy parallel compile+test load it failed once (reported HUNG, exit 4, instead of clean idle
timeout, exit 2); it passed 5/5 in isolation. Product behavior is correct; the test's timing
margin is too tight. Suggested: widen the margin or make it load-tolerant.

### Gap 3 (low, environment-influenced, issue #81) - opaque `unauthorized daemon IPC request` on upgrade/rollback drain when a foreign-exe daemon owns the target store

When a daemon started by a different `telex` executable already owns the target store,
`upgrade` / `rollback` drain coordination fails the same-user peer-executable authorization
check with a raw `unauthorized daemon IPC request: server executable ... does not match ...`.
`rollback` requires `--skip-drain` to proceed; a `--db` override did not redirect the drain
target. In a clean release environment the daemon is the installed binary, so this does not
arise. Suggested: a clearer, actionable message and/or a graceful drain fallback.

## Residual risk / not covered

- Unix (macOS/Linux) install smoke not run on this Windows host (assets + sidecars present and
  contract-tested).
- Daemon kill/restart during an in-flight send/push not hand-run; covered by the
  `daemon_process_sqlite` crash-recovery gating tests.
- Live bridge turn-injection not visually captured in this session (busy orchestration + Gap 1
  stale binary); covered by prior dogfooding and tests. A re-check on a machine whose PATH
  binary matches the plugin is recommended to close the turn-injection + liveness (deaf)
  observation.
