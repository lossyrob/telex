# Copilot Plugin Validation Matrix

Validation target: issue #41, local-daemon `copilot-plugin` node; extended by issue #61,
`harness-skill-layout` node (nested Copilot plugin root), and issue #79 (drain-launch
plugin/binary skew detection).

Verified with GitHub Copilot CLI 1.0.66-1 (issue #41), re-verified with GitHub Copilot
CLI 1.0.69-2 (issue #61, nested marketplace `source`), and re-verified with 1.0.72-1
(issue #79, drain launchers copied from the nested plugin root) on Windows.

**Version floor for nested `source`:** nested-source marketplace install depends on the
Copilot CLI resolving a plugin `source` subdirectory (an external CLI capability). Treat
**1.0.69-2 as the known-good floor**. Determining the exact minimum supported version (an
oldest-supported-version row here) and adding a release-time install smoke against the
release tag are owned by the public-release gate (#59), which this node precedes.

| Acceptance / risk | Evidence |
|---|---|
| Marketplace installs and exposes a plugin skill | `.github/plugin/marketplace.json` declares marketplace `telex` and plugin `telex` with nested `"source": "copilot/plugin"`. Isolated verification: `copilot --config-dir <temp> plugin marketplace add <repo>` followed by `copilot --config-dir <temp> plugin install telex@telex` reports `Installed 1 skill`; the installed plugin skill resolves from the nested `copilot/plugin/skills/telex`. See the issue #61 evidence block below. |
| CLI and plugin skill stay non-divergent | The installed binary owns the version-matched instructions (`telex skill` / `telex copilot skill`); `copilot/plugin/skills/telex/SKILL.md` is a thin bootstrap that defers to the binary. `tests/copilot_plugin.rs` asserts it stays a small bootstrap (not a copy) and is the only plugin `SKILL.md`. This supersedes the former byte-identical mirror (PR #55 / ADR 0040). |
| Hooks are contributed by a real plugin manifest | `copilot/plugin/plugin.json` declares `hooks.json`; `copilot/plugin/hooks.json` declares `sessionEnd` and `agentStop` command hooks. `sessionEnd` and `turn-guard` invoke hidden Rust adapters directly. The drain entry uses inline PowerShell plus a small POSIX launcher to forward stdin to the binary-owned adapter and translate pre-dispatch failure into Copilot hook JSON. Inline PowerShell avoids a nested script execution-policy boundary. |
| Same-semver binary skew is diagnosable | `telex --version` includes a source build identifier, `telex --json version` exposes it as `version.build_id`, and versioned-install manifests retain it. Published release jobs set and verify the identifier against `GITHUB_SHA`. Git fallback is accepted only when the canonical Git top-level equals the Telex manifest directory; metadata-less copies, including copies nested under unrelated repositories, report the diagnostic fallback `unknown`. |
| Notification content enrichment is omitted | Detached waiter stdout is not delivered to the agent, so enrichment would be useful only if the notification hook could locate the completed `--out-dir`. A local spike found the notification hook payload exposes only notification metadata and not a stable `--out-dir` path; sync/agent-read shell completions already carry stdout in context. `hooks.json` intentionally does not install a notification hook. |
| Copilot env stays at plugin boundary | `src/identity.rs` resolves only `--session` / `TELEX_SESSION_ID`; `src/commands/copilot.rs` owns `COPILOT_AGENT_SESSION_ID` and `COPILOT_LOADER_PID` mapping. |
| Generic loop remains copy/paste executable after fallback removal | `SKILL.md`/README document that generic follow-up commands must pass `--session "$COPILOT_AGENT_SESSION_ID"` or set `TELEX_SESSION_ID`; process tests prove generic commands do not rely on the Copilot fallback. |
| `sessionEnd` healthy disconnect is non-destructive | `telex copilot session-end` calls daemon `SessionEnd` with cap proof, releases waiters and marks stations idle, and does not literal-Detach/drop membership. This resolves issue wording against `docs/design/daemon.md`. |
| `sessionEnd` is store-scoped | `real_process_copilot_session_end_is_store_scoped` attaches the same session to two stores and verifies a hook scoped to store A does not mark store B idle. |
| `sessionEnd` does not spawn a daemon just to reap | Command smoke covers daemon-down no-op logging; real command path uses `connect_existing_with_cap`, not `connect_or_spawn`. |
| Turn-end re-arm guard detects unarmed attendance | Isolated command smoke: after `copilot attach` and no live waiter, `copilot turn-guard` returns `{"decision":"block"}` naming `smoke:node (pending 0)`. |
| Guard covers delivered-but-unacked classic waiter work | The `agentStop` guard also nudges when a station has a prior waiter delivery still unacked (`pending_unconsumed_count > 0` and `last_waiter_outcome == message`), avoiding a naive in-flight backlog false positive. |
| Guard cap bounds ignored nudges | Isolated command smoke with `TELEX_TURN_GUARD_MAX_NUDGES=1`: first prior nudge is retained, subsequent `agentStop` invocations return `allow`; unit tests cover cap exhaustion and reset when the unresolved unarmed station set changes or clears. |
| Guard cap remains bounded in mixed armed/unarmed state | `real_process_copilot_turn_guard_caps_mixed_armed_unarmed_state` keeps one station armed while another remains unarmed and verifies repeated guard invocations reach cap exhaustion. |
| Structured hook observability exists | Isolated smoke writes `run/copilot/hook-events.ndjson` with `agentStop` and `sessionEnd` reason codes; live Copilot hook smoke also writes `agentStop` and `sessionEnd` entries. |
| Daemon-down guard failure is fail-open and observable | `real_process_copilot_turn_guard_daemon_down_fails_open_and_logs` returns `allow` and writes `daemon_unavailable` to hook logs when no daemon is running. |
| Real Copilot `agentStop` and `sessionEnd` hooks fire | Nested `copilot --plugin-dir <repo> -p "Respond with exactly: OK"` under isolated TELEX dirs produced hook log entries for `agentStop` and `sessionEnd` with the same Copilot session id. |
| Hidden adapter is not public CLI surface | CLI tests assert top-level help does not mention `copilot` and hidden subcommands still parse. |
| No PR #31 filesystem session registry authority | Plugin hooks call daemon/status/session operations only; the drain launchers are transport-only and do not inspect Telex state. Tests assert the direct hooks target `telex --json copilot ...`, the drain launchers target the same binary-owned adapter, and the only plugin skill is the thin bootstrap that defers to the binary. |
| Upgrade-facing constraints | Direct hooks and the drain launchers resolve `telex` by PATH name, not an absolute binary path, preserving the versioned install and seamless-upgrade launcher path. |
| Release install path points at the marketplace | Release archives contain the binary and license; install scripts print tag-pinned marketplace commands (`copilot plugin marketplace add lossyrob/telex#<tag>` then `copilot plugin install telex@telex`) so plugin assets are installed via the supported marketplace channel rather than deprecated direct installs. |
| Busy non-interrupt pushes defer instead of queueing (issue #65) | The bridge tracks busy from **root-agent** `assistant.turn_start`/`turn_end` (`agentId`-filtered) and returns `deferred_until_idle` without calling `session.send` while busy; `interrupt` still sends. `tests/.../drain_deferred_*` + `deferred_exit_records_deferred_outcome` cover the daemon `Deferred` outcome; `node --check` gates the bridge JS. |
| Idle drain does not resurrect a consumed message | `drain_deferred_skips_message_acked_before_idle`: a message deferred while busy then acked before turn-stop is re-validated out by the drain's `fetch_wait_candidates` sweep and never re-pushed. `drain_deferred_repushes_unacked_after_turn_stop` confirms a still-unacked deferred message is delivered after the drain. |
| Drain hook is dedicated and independently gated | `hooks.json` `agentStop` runs both `copilot turn-guard` and the drain launcher. `TELEX_COPILOT_DRAIN=off|0|false` skips launch and emits a neutral `{}` result, so it cannot cancel a turn-guard block. Once `copilot drain` starts, its bounded runtime/daemon handling remains fail-open (exit 0) and also returns neutral hook output. A missing/stale PATH binary or other nonzero pre-dispatch failure becomes one actionable `block` decision; the launcher itself exits 0 so Copilot processes that JSON. `DrainDeferred` is admin-capped (`drain_deferred_requires_admin_cap`). |
| Deferred is diagnosable and does not raise false degraded | Deferred attempts are surfaced as `push_deferred_count` in status and do not increment the degraded-status attempt counter (`deferred_attempt_holds_at_backstop_and_stays_off_degraded_counter`); backstop invariant `HEARTBEAT_INTERVAL <= deferred < accepted` is asserted (`on_deliver_backstop_invariants`). |

Live smoke summary:

- Command smoke used isolated `TELEX_HOME`, `TELEX_RUN_DIR`, `TELEX_DB`, and `LOCALAPPDATA`.
- `copilot attach` registered one station with `COPILOT_AGENT_SESSION_ID`.
- `copilot turn-guard` blocked once, then cap-exhausted to allow when max nudges was set to 1.
- `copilot session-end` marked the station idle and wrote hook logs.
- Nested Copilot CLI hook smoke exited 0, returned `OK`, and logged both `agentStop` and `sessionEnd`.

## Nested Copilot plugin layout (issue #61)

Layout: all Copilot-specific content is nested under `copilot/` — `copilot/COPILOT.md`
(binary-embedded skill body), `copilot/bridge/` (binary-embedded bridge source), and
`copilot/plugin/` (the marketplace plugin root: `plugin.json`, `hooks.json`, the POSIX
drain hook launcher, and `skills/telex/SKILL.md`). `.github/plugin/marketplace.json` sets the plugin
`"source": "copilot/plugin"`. The repository root is harness-neutral (`SKILL.md` carries
no Copilot mechanics), leaving room for future sibling harness plugin roots.

Empirical verification (nested-source behavior on GitHub Copilot CLI 1.0.69-2; plugin
contents re-verified on 1.0.72-1, Windows, isolated `--config-dir`):

| Check | Evidence |
|---|---|
| Nested `source` install (positive) | `copilot --config-dir <temp> plugin marketplace add <repo>` → `Marketplace "telex" added`; `plugin install telex@telex` → `Plugin "telex" installed successfully. Installed 1 skill.` |
| Installed skill resolves from the nested plugin root | The installed plugin tree contains `plugin.json`, `hooks.json`, `drain-hook.sh`, and `skills/telex/SKILL.md`, all copied from `copilot/plugin/`; Windows drain adaptation is inline in `hooks.json`. |
| Installed plugin is lean (no embedded sources shipped) | `copilot/COPILOT.md` and `copilot/bridge/` are siblings of `copilot/plugin/`, so they are NOT copied into the installed plugin. The POSIX launcher and inline PowerShell adapter remain small transport/error shims; binary-owned workflow content is not duplicated. |
| `source` is load-bearing (negative control) | Temporarily setting `"source": "copilot/plugin-DOES-NOT-EXIST"` makes install fail with `Failed to install plugin: Error: Plugin source directory not found: <repo>\copilot\plugin-DOES-NOT-EXIST`, confirming the marketplace resolves the plugin at `<repo-root>/<source>`. |
| GitHub-fetch install (production path) | Real `owner/repo#ref` fetch against a fresh isolated `--config-dir`: `copilot plugin marketplace add lossyrob/telex#feature/harness-skill-layout` → `Marketplace "telex" added`; `plugin install telex@telex` → `Installed 1 skill`. The nested source resolves from `copilot/plugin`; `COPILOT.md`/`bridge/` are not shipped. Issue #79 changes only the drain entry: `sessionEnd`/`turn-guard` remain direct binary adapters while drain runs the shipped platform launcher via `COPILOT_PLUGIN_ROOT`. |
