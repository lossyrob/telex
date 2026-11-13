# Copilot push delivery

GitHub Copilot CLI and Copilot App can receive messages as **turns** through a live
Telex bridge, without running or re-arming a waiter. App's `install_extension`
can install and load a session bridge before first bind, without restarting.
On hosts with startup extension discovery, first-time App setup can load the
bridge after provisioning and a session restart, even without a live reload tool.
Verify liveness before
treating push as attended; the existing Telex-owned pull fallback remains available.
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
- **App `install_extension` available:** use the [session installer](#app-session-installer)
  before provisioning. It downloads the bridge and reloads extensions, but refuses
  an existing extension name.
- **App startup discovery without a live loader:** provision the bridge, restart
  the App session, then run `copilot resume` and verify liveness as described below.
- **Restart is unacceptable or liveness does not verify:** use
  [pull fallback](#fallback). Neither an absent `extensions_reload` tool nor
  CLI-only `/experimental` is an App requirement.

Bridge files and push registration do not prove the bridge loaded. A restart is
not proof by itself. Check bridge liveness and actual delivery separately; a
plain conversation clear is not an established first-time App loader.

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

## App session installer

When `install_extension` is available, install **before** the first
`copilot attach --copilot-bridge` or `copilot resume`. `telex copilot skill`
prints pinned tool arguments for a full Git build id:

```json
{
  "url": "https://github.com/lossyrob/telex/tree/<version.build_id>/copilot/bridge",
  "scope": "session",
  "name": "telex-bridge"
}
```

Replace `<version.build_id>` with the full commit from `telex --json version`.
Verify that this published commit matches the bridge embedded in the binary,
including any local source changes; the build identifier is diagnostic, not
provenance. Never use floating `main` or a guessed tag. If matching source is
unavailable or the build id is unknown, use supported reload or startup discovery.
Discover the tool schema before calling it. Use `scope: "session"` explicitly
(the default is user-wide) and `name: "telex-bridge"` exactly.

The installer downloads files and reloads extensions. After it succeeds, run
`copilot attach --copilot-bridge`, then the liveness checks below. A macOS App
1.1.35 experiment verified installer-first loading in a fresh child session and
an acknowledged pushed-message round trip over local SQLite with no pull waiters.
That observation does not establish support in every App build or on every OS;
an installer success response alone is not delivery proof.

**Existing extension:** installation refuses an existing `telex-bridge` name,
including files just provisioned by attach. Keep an already-loaded, live bridge;
otherwise prefer supported reload or startup discovery. Do not install a second
bridge under another name: both would target the same session socket.

If intentionally stopping all affected bindings, use `copilot detach` for each
address in this session, then install and reattach with the original backend and
options (including `--wake-on-cc`). Only the last detach removes bridge files.
Do not delete shared files or disrupt another binding to force installation.
These restrictions also apply to upgrades and recovery; this is not a
reload-only API.

### Windows inherited permissions and RC1 upgrades

RC2 uses the existing three-module Node bridge without a persistent helper.
Windows bridge directories, registry credentials and binding metadata inherit
the host profile's access policy. Telex no longer rejects these producer paths
based on owner or ACL entries, nor rewrites their permissions. Unix producer
checks and Telex-owned daemon capabilities/intents remain strict.

**Known limitation:** anyone able to read a Windows registry file learns the
bridge secret. With write access to that session's named pipe, they can submit
prompts as turns. Foreign-readable roots therefore do not provide credential
confidentiality or sandbox isolation. Foreign-writable producer files permit
disruption and, with control of a same-user server at the derived endpoint,
producer impersonation. Rust retains bounded regular/non-reparse file reads,
daemon-side containment, derived endpoint names and same-user/PID/start-time/
executable verification before sending credentials; none restores secrecy of a
readable file or protects against an already-compromised same-user process.

An already-loaded RC1 bridge needs no module rewrite or extension reload for
this correction. Upgrade/restart its owning daemon using the normal coordinated
upgrade procedure, then `copilot resume` with the original backend/options and
verify attended-push health and real delivery. An RC1 daemon retains the old
permission rejection; a CLI-only swap does not update that process.

Failed attach now removes only this attempt's binding and intent generation.
It never removes modules or the producer registry. An existing empty binding
file is retained; an attempt-created file can remain as `[]`, discoverable by
normal GC. Retained modules may later load an unbound producer; this does not
establish attended push. Do not use absent-binding detach as a cleanup shortcut.
Deliberate last detach/GC retain their separate teardown semantics.

Binding attach/finalize/rollback is serialized under one OS-released lock, so a
failed attempt cannot remove a later same-address adoption. Contenders wait at
most five seconds before an explicit retryable error. A rollback write failure
is reported and can leave this attempt's address recorded; inspect state before
deliberately detaching that address. The empty lock file remains between operations. On Windows a
live RC1 lock holder is excluded, and RC1 cannot delete a lock held by RC2.
RC1 may refuse updates because the retained file exists; use the matching CLI
for all writes instead of running mixed-version attach/detach commands. No
simultaneous mixed-version Unix writer interoperability is promised.
A legacy lock being deleted can cause an immediate access-denied refusal; retry
with the matching CLI after that operation ends.

## Bind and provision the bridge

Choose an already-loaded, live bridge, a supported live loader, or startup
discovery with an acceptable App session restart. For App's session installer,
complete the installation above **before** this bind. Provision with:

```sh
telex --address <addr> copilot attach --copilot-bridge --description "<work>"
```

Keep an already-loaded, live bridge. If the host exposes a supported live loader,
use it after provisioning (`extensions_reload` where available; Telex cannot
invoke it).

For **first-time App setup without an advertised `extensions_reload`**, use
the session installer before provisioning when available. Otherwise use
startup discovery when a restart is acceptable: after provisioning, restart the
App session, then run:

```sh
telex --address <addr> copilot resume --description "<work>"
telex --address <addr> --json status
```

`copilot resume` re-provisions the push registration, re-arms daemon attendance
and rescans unacked backlog; it does not load the extension itself. Telex
preserves session-scoped bridge files across normal close/resume, so hosts with
startup extension discovery can also load those files on resume.

After loading, verify `--json status` reports
`station_health: attended_push` and `push_registered: true` for the address.
Where available, call `telex_bridge_info` for supporting bridge evidence; do not
require that tool when it is absent. Restart, provisioning, registration and tool
availability alone are not end-to-end delivery proof. Delivered messages arrive
as turns labelled `[telex] from <addr> (<attention>)`.

[Issue #230](https://github.com/lossyrob/telex/issues/230) records startup loading
and a pushed message on one Windows App build. It does not establish support in
every App build or loading after a plain conversation clear. Liveness
verification, not an App version, is the gate.

For recovery on a loader-capable host, re-provision with
`telex --address <addr> copilot resume --description "<work>"` and use its supported
live loader, or use the attach/restart/resume path above. If a restart is
unacceptable or liveness does not verify, use [pull fallback](#fallback) or detach
with `telex --address <addr> copilot detach`. Do not repeat unavailable-tool calls.
For installer recovery, follow the existing-extension restrictions above rather
than calling `install_extension` after provisioning.

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

Use the loading path and liveness check above if needed. Without `--wake-on-cc`,
CC copies are still buffered and visible in `telex inbox --all`, but are not delivered as turns.
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

When a live bridge is unavailable and a restart is unacceptable or liveness does
not verify, use this path for first-time setup or recovery. Surface that push is
unavailable and prepare one Telex-owned pull fallback run:

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

`6` (`delivery-quarantined`) means read and preserve `status.json.quarantine`,
not `message.json`/`delivery.json`; do not ack the undelivered message. If
`may_continue` is true and attendance is still required, explicitly call
`fallback prepare` for the next single-shot run so later deliveries can progress.
Otherwise report the quarantine condition before continuing.

`7` (`backend-unavailable`) means inspect and report `status.json.detail` for
exhausted backend recovery, not delivery, idle timeout or daemon death.
Re-arm only after recovery, using `fallback prepare`. Older peers may report
this failure as exit `1`; the [exit-code reference](../reference/exit-codes.md)
describes recovery-capability compatibility.

Preparation is not evidence of a running wait or a real App completion wake.
Backend or wait-setup errors remain failures; guidance and source-contract tests
prove neither fallback wakeup nor bridge loading.

`telex --address <addr> status` reports `delivery_mode` separately from
`station_health`: `push` is bridge delivery, `pull` is the Copilot fallback, and
`conflict` is a version-skew/race tripwire. The daemon rejects simultaneous push
and pull coverage. A busy App turn can also report `push_delivery: deferred`;
this means the bridge is reachable and will retry at turn-idle, not that the
station is unattended.

To return to push, first choose an already-loaded, live bridge, a supported live
loader, or startup discovery with an acceptable App session restart. Stop the
waiter before binding:

```sh
telex --address <addr> station stop --session "$COPILOT_AGENT_SESSION_ID"
telex --address <addr> copilot attach --copilot-bridge --description "<work>"
```

For the App installer path, install before the attach command above; respect
the existing-extension restrictions. Use the supported live loader if needed,
or restart the App session and run
`copilot resume` for startup discovery. Then verify bridge liveness with
`--json status` as above. If liveness does not verify, return to the prepared
pull fallback; do not claim push is attended. To end fallback, run `station stop`
and do not prepare another run. The version-matched `telex copilot skill` contains
the full artifact, timeout, recovery, and re-arm procedure.

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
