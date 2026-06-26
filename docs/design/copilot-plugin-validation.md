# Copilot Plugin Validation Matrix

Validation target: issue #41, local-daemon `copilot-plugin` node.

Verified with GitHub Copilot CLI 1.0.66-1 on Windows.

| Acceptance / risk | Evidence |
|---|---|
| Plugin installs and exposes a plugin skill | `copilot --config-dir <temp> plugin install <repo>` reports `Installed 1 skill`; `copilot --config-dir <temp> skill list --json` includes plugin skill `telex` from `skills/telex`. |
| CLI and plugin skill stay non-divergent | `tests/copilot_plugin.rs` asserts root `SKILL.md` and `skills/telex/SKILL.md` are byte-identical and are the only `SKILL.md` files in the package. |
| Hooks are contributed by a real plugin manifest | `plugin.json` declares `hooks.json`; `hooks.json` declares `sessionEnd` and `agentStop` command hooks that invoke hidden Rust adapter commands, not shell scripts. |
| Copilot env stays at plugin boundary | `src/identity.rs` resolves only `--session` / `TELEX_SESSION_ID`; `src/commands/copilot.rs` owns `COPILOT_AGENT_SESSION_ID` and `COPILOT_LOADER_PID` mapping. |
| `sessionEnd` healthy disconnect is non-destructive | `telex copilot session-end` calls daemon `SessionEnd` with cap proof, releases waiters and marks stations idle, and does not literal-Detach/drop membership. This resolves issue wording against `docs/design/daemon.md`. |
| `sessionEnd` does not spawn a daemon just to reap | Command smoke covers daemon-down no-op logging; real command path uses `connect_existing_with_cap`, not `connect_or_spawn`. |
| Turn-end re-arm guard detects unarmed attendance | Isolated command smoke: after `copilot attach` and no live waiter, `copilot turn-guard` returns `{"decision":"block"}` naming `smoke:node (pending 0)`. |
| Guard cap bounds ignored nudges | Isolated command smoke with `TELEX_TURN_GUARD_MAX_NUDGES=1`: first prior nudge is retained, subsequent `agentStop` invocations return `allow`; unit tests cover cap exhaustion and reset when any live waiter is observed. |
| Structured hook observability exists | Isolated smoke writes `run/copilot/hook-events.ndjson` with `agentStop` and `sessionEnd` reason codes; live Copilot hook smoke also writes `agentStop` and `sessionEnd` entries. |
| Real Copilot `agentStop` and `sessionEnd` hooks fire | Nested `copilot --plugin-dir <repo> -p "Respond with exactly: OK"` under isolated TELEX dirs produced hook log entries for `agentStop` and `sessionEnd` with the same Copilot session id. |
| Hidden adapter is not public CLI surface | CLI tests assert top-level help does not mention `copilot` and hidden subcommands still parse. |
| No PR #31 filesystem session registry authority | Plugin hooks call daemon/status/session operations only; tests assert hook manifest targets `telex --json copilot ...` and no copied plugin skill other than byte-identical mirror. |
| Upgrade-facing constraints | Hooks invoke `telex` by PATH name, not an absolute binary path, preserving room for the later seamless-upgrade launcher shim. |

Live smoke summary:

- Command smoke used isolated `TELEX_HOME`, `TELEX_RUN_DIR`, `TELEX_DB`, and `LOCALAPPDATA`.
- `copilot attach` registered one station with `COPILOT_AGENT_SESSION_ID`.
- `copilot turn-guard` blocked once, then cap-exhausted to allow when max nudges was set to 1.
- `copilot session-end` marked the station idle and wrote hook logs.
- Nested Copilot CLI hook smoke exited 0, returned `OK`, and logged both `agentStop` and `sessionEnd`.
