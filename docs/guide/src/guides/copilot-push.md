# Copilot push delivery

GitHub Copilot CLI and Copilot App can receive messages as **turns** through a live
Telex bridge, without running or re-arming a waiter. First-time App sessions
without a supported live bridge loader use the existing Telex-owned pull fallback.
The full, version-matched Copilot workflow is printed by the installed binary:

```sh
telex copilot skill
```

That command is the source of truth for the Copilot path. This guide is the
operator's overview.

For a root coordinating local sessions, [group stall detection](copilot-stalls.md)
adds an opt-in Watcher watch, planned-wait membership and a confirm-then-ring
response when progress stops.

## Choose delivery by host capability

- **Already-loaded, live bridge:** keep push, including in Copilot App.
- **Supported live bridge loader available:** provision and load the bridge below.
  On Copilot CLI only, enable Copilot Extensions under `/experimental` if needed
  to expose `extensions_reload`.
- **No supported live bridge loader:** use [pull fallback](#fallback) for first-time
  App setup. Neither an absent `extensions_reload` tool nor CLI-only
  `/experimental` is an App requirement.

Bridge files and push registration do not prove the bridge loaded. Restarting or
clearing a session does not establish first-time loading on a host without a
supported loader. Check bridge liveness and actual delivery separately.

## Install the plugin

```sh
copilot plugin marketplace add lossyrob/telex
copilot plugin install telex@telex
```

The plugin contributes session lifecycle hooks and provisions the push bridge. It
maps `$COPILOT_AGENT_SESSION_ID` to the generic telex session id. In bridge mode,
the extension heartbeat, not `$COPILOT_LOADER_PID`, is the push liveness signal.
GitHub Copilot App emits `sessionEnd(reason=complete)` when each turn becomes idle;
while the bridge heartbeat remains live, Telex keeps the station attended instead
of requiring `copilot resume` after every response. Telex also binds attendance to
the bridge host PID, which survives extension reloads but exits with a true App
exit or one-shot CLI completion.

## Bind and provision the bridge

Use this path when the host exposes a supported loader or the bridge is already
loaded and live:

```sh
telex --address <addr> copilot attach --copilot-bridge --description "<work>"
```

Use the supported loader for first-time provisioning (`extensions_reload` where
available; Telex cannot invoke it). Verify bridge liveness before treating push as
available. Delivered messages arrive as turns labelled `[telex] from <addr>
(<attention>)`.

Telex preserves session-scoped bridge files across normal close/resume. On hosts
with startup extension discovery, check that the bridge is live, then run
`telex --address <addr> copilot resume --description "<work>"` to re-arm daemon
attendance and rescan unacked backlog; this normal path does not depend on an
in-session `extensions_reload`.

For recovery on a loader-capable host, re-provision with
`telex --address <addr> copilot resume --description "<work>"` and use its supported
loader. If the bridge is not live and the host has no supported loader, use
[pull fallback](#fallback) or detach with `telex --address <addr> copilot detach`.
Do not repeat unavailable-tool calls.

## Receive and disposition

A pushed turn includes the `ack` and `handle` commands with the address, id, and
session filled in. The generic verbs do not read Copilot env vars, so they need
`--session "$COPILOT_AGENT_SESSION_ID"`:

```sh
telex ack --address <addr> --id <message-id> --session "$COPILOT_AGENT_SESSION_ID"
telex handle --address <addr> --id <message-id> --session "$COPILOT_AGENT_SESSION_ID" --note "completed"
```

Sending is not push: `telex send` and `telex reply` also need `--session`.

## CC observers

A session that should receive CC (observer) copies as turns opts in at bind time
with `--wake-on-cc`:

```sh
telex --address <addr> copilot attach --copilot-bridge --wake-on-cc --description "<work>"
```

Load the bridge with the supported loader if needed. Without `--wake-on-cc`, CC copies are still
buffered and visible in `telex inbox --all`, but are not delivered as turns.
(In Copilot pull fallback use `prepare --wake-on-cc`; do not start a waiter
alongside push.)

## Tear down

```sh
telex --address <addr> copilot detach
```

This detaches the address and, when it was the last binding, removes the bridge
files so nothing reloads on a later resume. Ordinary session end preserves them
for startup extension discovery on resume.

Inspect stale bridge files left by other sessions with `telex copilot gc --dry-run`.

## Fallback

For first-time App setup without a supported live bridge loader, or when the
bridge cannot load during recovery, push is unavailable. Surface that plainly and
prepare one Telex-owned pull fallback run:

```sh
telex --address <addr> copilot fallback prepare --description "<work>"
```

The JSON result contains a unique `run_dir` plus `launcher.program`,
`launcher.args`, and a ready-to-run `launcher.command`. Run that launcher as one
fully detached Copilot task (`detach: true`). Unix uses the current telex binary directly; Windows
uses a generated PowerShell file for the detached-task compatibility path. Telex
does not detach the task itself and does not run an internal delivery loop. If
detached launch is unavailable or refused, report that limitation; do not write
a polling loop or wrapper.

Preparation is idempotent until the run writes `exit.code`, and it leaves push
unchanged if the launcher never starts. The running launcher atomically clears
push before entering exactly one [pull-mode wait](agent-pull.md). On completion,
read `exit.code` first, then the exact run's `delivery.json`/`message.json`; ack
and dedupe primary deliveries by message id before preparing the next run.
The launcher refuses to leave a live bridge unless `prepare --force` records an
intentional downgrade; `--force` is not a recovery default.

The task completion notification is only a wakeup, not the result. `exit.code` is
written last: `0` means read the delivery, dedupe, ack and disposition by id;
`1` means inspect `status.json.detail` for setup/error; `2` is idle timeout;
`3`/`4` mean daemon gone/hung; `5` means presence ended. Repair errors before
preparing another run. For `5`, inspect status: do not re-arm pull if mode is
`push`, and leave a stopped station stopped unless attendance is still required.
If `exit.code` is absent, inspect for a live waiter before retrying the same
unfinished run. Re-arm explicitly, one run at a time.

Preparation is not evidence of a running wait or a real App completion wake.
Backend or wait-setup errors remain failures; guidance and source-contract tests
prove neither fallback wakeup nor bridge loading.

`telex --address <addr> status` reports `delivery_mode` separately from
`station_health`: `push` is bridge delivery, `pull` is the Copilot fallback, and
`conflict` is a version-skew/race tripwire. The daemon rejects simultaneous push
and pull coverage. A busy App turn can also report `push_delivery: deferred`;
this means the bridge is reachable and will retry at turn-idle, not that the
station is unattended.

To return to push, first confirm a supported loader or an already-loaded, live
bridge. Stop the waiter before binding:

```sh
telex --address <addr> station stop --session "$COPILOT_AGENT_SESSION_ID"
telex --address <addr> copilot attach --copilot-bridge --description "<work>"
```

Use the supported loader (`extensions_reload` where available) if needed and
verify bridge liveness. To end fallback, run `station stop` and do not prepare
another run. The version-matched `telex copilot skill` contains the full artifact,
timeout, recovery, and re-arm procedure.

## Compatibility

`telex copilot skill` prints the installed version and source build identifier, the
bridge protocol version, and the minimum compatible plugin version, and warns if the
plugin is older than the binary supports. `telex --version` includes the same build
identifier, while `telex --json version` exposes it as `version.build_id`. Published
release binaries are gated to report the release commit; source builds without Git
metadata may report `unknown`. Git fallback is accepted only for a standalone Telex
checkout, not an unrelated ancestor repository, and the value is diagnostic rather
than an attestation.

If the drain hook reports skew, inspect `version.current_exe` plus `Get-Command telex`
or `command -v telex`, reinstall the plugin and binary from the same release, and make
the versioned launcher precede stale shims on PATH before restarting Copilot. A binary
rollback must be paired with the plugin from the same release.

Release install scripts pin the plugin and binary to the same tag. The plugin shape is
validated against a specific Copilot CLI version; see the
[acceptance matrix](https://github.com/lossyrob/telex/blob/main/docs/design/copilot-plugin-validation.md).
