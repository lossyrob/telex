# Telex for Copilot CLI and Copilot App: delivery

This is the Copilot-specific Telex workflow, printed by `telex copilot skill` so it
always matches the installed binary. The plugin skill is only a bootstrap; **this
document (from your installed `telex`) is the source of truth** for the Copilot path.
For exact flags, run the `--help` commands listed at the end rather than trusting any
static copy of the syntax.

## Choose delivery by host capability

Check the current session's capabilities before provisioning a bridge:

- **Already-loaded, live bridge:** keep push, including in Copilot App. App hosting
  alone is not a reason to downgrade to pull.
- **Supported live bridge loader available:** use the bridge path below. On
  Copilot CLI, enable **Copilot Extensions** under `/experimental` if needed to
  expose `extensions_reload`. That CLI setting is not an App prerequisite.
- **No supported live bridge loader:** for first-time setup in Copilot App, use
  the supported pull fallback below, starting with `copilot fallback prepare`.
  Do not require an absent `extensions_reload` tool or CLI-only `/experimental`.

Bridge files and push registration do not prove the bridge loaded. Restarting or
clearing a session is not a substitute for an unavailable loader or evidence of
first-time loading. Check bridge liveness and actual delivery separately.

With a live bridge, messages arrive as **turns** without a waiter to re-arm. With
pull fallback, you launch one prepared wait, process its completion artifacts, and
explicitly re-arm when needed. Neither path acks for you; record consumption and
workflow disposition by message id.

## The bridge path

1. **Bind your address and provision the bridge (one command).**

   ```sh
   telex --address <addr> copilot attach --copilot-bridge --description "<what this session is doing>"
   ```

   **Prerequisite:** a supported live bridge loader, or an already-loaded, live
   bridge. Telex can provision files and register push without loading the bridge;
   successful attach alone is not evidence of live delivery.

   This registers your session/address with the per-user local exchange and writes the
   telex bridge extension into this session's extension dir. The plugin adapter maps
   `$COPILOT_AGENT_SESSION_ID` for you; in bridge mode the extension heartbeat, not
   `$COPILOT_LOADER_PID`, is the push liveness signal.
   If this station is a deliberative-table observer/relay and should receive live CC
   traffic as turns, add `--wake-on-cc` to the bind:

   ```sh
   telex --address <addr> copilot attach --copilot-bridge --wake-on-cc --description "<what this session is doing>"
   ```

   CC push is opt-in, live-only, and notification-only: historical CC backlog is not
   replayed, and CC messages still do not require a terminal disposition from the observer.

   On hosts with startup extension discovery, normal close/resume can load the
   preserved session-scoped bridge files. Once the bridge is live, re-arm daemon
   attendance and rescan unacked backlog with:

   ```sh
   telex --address <addr> copilot resume --description "<what this session is doing>"
   ```

   `copilot resume` re-provisions the same push bridge registration that `copilot
   attach --copilot-bridge` creates; it also re-scans queued unacked backlog.

   GitHub Copilot App reports `sessionEnd(reason=complete)` after each completed
   turn even though the durable App session and its bridge remain live. Telex
   preserves attendance while the bridge heartbeat is fresh and binds the station
   to the bridge host process, avoiding a manual `copilot resume` after every
   response. The host remains stable across extension reloads. When a one-shot CLI
   run or the App session really exits, host-process death reaps attendance normally.
   Terminal reasons and a missing or stale bridge retain immediate session-end behavior.

2. **Load the bridge into the live session (one agent tool call).**

   Use the host's supported loader after first-time provisioning. Where available,
   run `extensions_reload`; Telex cannot invoke it for you. Verify bridge liveness
   rather than inferring it from provisioning or a reload request.

   If the bridge is not live, select recovery by the same host capability:
   on a loader-capable host, re-provision with
   `telex --address <addr> copilot resume --description "<what this session is doing>"`
   and use the supported loader. On Copilot CLI only, enabling Copilot Extensions
   under `/experimental` may expose `extensions_reload`. An App session without
   a supported live loader uses the supported pull fallback below, or detaches
   with `telex --address <addr> copilot detach`. Do not repeat unavailable-tool
   calls or claim push is live.

   On CLI hosts supporting `/clear` and `extensions_reload`, after `/clear`
   (which clears the conversation), **re-provision first** --
   re-run `telex --address <addr> copilot attach --copilot-bridge` and then `extensions_reload` --
   so the daemon re-delivers any message that was queued but not yet acked when you cleared. A
   re-attach (or a new session taking over the address) is what re-delivers unacked messages; while
   the same session stays continuously attached, an already-accepted message is **not** re-pushed on
   the fast cadence -- a long backstop may re-check it only every few minutes if it stays unacked.

   Troubleshooting CC observer turns: if CC traffic is visible in `telex inbox` but is not arriving
   as Copilot turns, the bridge was probably bound without CC wake or the extension was not reloaded.
   On a loader-capable host, re-run the bind with the CC flag, then load the bridge:

   ```sh
   telex --address <addr> copilot attach --copilot-bridge --wake-on-cc --description "<what this session is doing>"
   ```

   Do not start `telex wait --wake-on-cc` alongside push; the bridge registration
   above enables live CC turns. In pull fallback, use `prepare --wake-on-cc` instead.

3. **Receive messages as turns.** A delivered telex message arrives as a new turn
   labelled `[telex] from <addr> (<attention>)`. An `interrupt` message is delivered as
   soon as possible (Copilot `immediate`, ahead of enqueued messages). Every other
   attention level is **deferred** while a turn is running and delivered **after your turn
   stops** — it is not queued behind the current turn, so a message you read and
   disposition manually mid-turn is not re-injected as a stale turn later. Neither
   preempts a turn already running. Read it, then record disposition
   **by id**. For CC observer pushes, the prompt says `delivery_role: cc` and
   `requires_disposition: false`; ack it for transport consumption/dedupe, but do not
   treat the primary recipient's required-disposition flag as your own obligation.
   (Deferred delivery runs on a turn-stop drain hook; an operator can disable it with
   `TELEX_COPILOT_DRAIN=off`, independent of `TELEX_TURN_GUARD` — deferred messages then
   arrive on the daemon's slower backstop instead of promptly at turn-stop.)

   ```sh
   telex ack --address <addr> --id <message-id> --session "$COPILOT_AGENT_SESSION_ID"
   telex handle --address <addr> --id <message-id> --session "$COPILOT_AGENT_SESSION_ID" --note "completed"
   ```

   The pushed turn already includes these commands with your address, id, and session
   filled in -- prefer copying them. Generic `telex ack`/`handle` do not read Copilot env
   vars, so they need the session via `--session` (your session id is in
   `COPILOT_AGENT_SESSION_ID`; on PowerShell use `$env:COPILOT_AGENT_SESSION_ID`).

   `ack` is transport consumption for `(message_id, recipient-address)`. Terminal
   workflow disposition is still `handle`, `reject`, or `close`; `defer` and `escalate`
   are non-terminal. **Dedupe by id**: push is at-least-once, so a message may
   occasionally be delivered more than once (e.g. after a reconnect).

   In push mode, do **not** proactively drain unseen messages from `telex inbox` while the bridge is
   live just because status reports unacked backlog: enqueue-mode turns may already be queued behind
   the current turn, and acking them before the visible turn arrives creates duplicate work when that
   queued turn is later delivered. Use `inbox` for diagnostics/recovery (stale bridge, reload,
   backstop/degraded state), not as the normal receive path.

4. **Tear down when done.**

   ```sh
   telex --address <addr> copilot detach
   ```

   This detaches the address and, when it was the last binding, removes the bridge files so
   it will not reload on a later resume. The already-loaded bridge stays live until the next
   supported reload or session end; if `extensions_reload` is available, run it
   once after detach to unload immediately. Ordinary session end is resumable: it
   marks daemon membership idle and clears transient turn-guard state, but preserves
   the session bridge files for startup discovery on hosts that support it.

   To inspect or clean orphaned bridge files from sessions that closed without detach:

   ```sh
   telex copilot gc --dry-run
   telex copilot gc --force
   ```

   GC is conservative: a live bridge heartbeat is kept, and bindings are kept unless you
   force cleanup after verifying the session is gone.

## Sending and replying

Whether receiving through push or pull, **sending is explicit**. To start or
continue a conversation, `telex send` and `telex reply` need your session id --
exactly like `ack`/`handle`, they do **not** read Copilot env vars, and fail closed
with `no session id available` without it.

For every agent-authored operational `send` or new conversation, include a concise,
non-empty `--subject`. The subject is the human/operator scan surface in timelines,
operator views, and message lists, so make it communicate the outcome, requested action,
or topic at a glance rather than repeating an opaque kind or the body's first line. Good
subjects include `PR #123 ready for review`, `CI failure needs repair`,
`Issue #45 blocked on scope decision`, and `PR #123 merged; stand down`.

```sh
telex --address <addr> send --to <peer> --subject "PR #123 ready for review" --body "..." --session "$COPILOT_AGENT_SESSION_ID"
```

On PowerShell use `$env:COPILOT_AGENT_SESSION_ID`. `telex reply` takes the same `--session`
(run `telex reply --help` for its exact flags). Only `telex copilot attach`/`detach` map the
Copilot session id for you; the generic verbs do not.

Replies inherit `Re: <parent subject>` when `--subject` is omitted. That is sufficient when
the parent subject is already useful. If the parent subject is blank, vague, or misleading,
provide a meaningful replacement instead of perpetuating an unhelpful thread title:

```sh
telex --address <addr> reply --to-message <id> --subject "CI failure needs repair" --body "..." --session "$COPILOT_AGENT_SESSION_ID"
```

## Fallback: no bridge (pull mode)

For first-time App setup without a supported live bridge loader, or recovery when
the bridge cannot be loaded, use this path. **Surface that push is unavailable**
rather than pretending the bridge is live. Telex can prepare one cross-platform,
single-shot pull fallback without requiring an agent-authored script:

```sh
telex --address <addr> copilot fallback prepare --description "<what this session is doing>"
```

`prepare` maps `$COPILOT_AGENT_SESSION_ID`, creates a unique owner-private run
directory, and prints JSON containing:

- `run_dir` -- the exact artifact directory for this run;
- `launcher.program` and `launcher.args` -- structured launch data;
- `launcher.command` -- the platform-appropriate command to pass to the task runner;
- the exact `exit.code`, `status.json`, `delivery.json`, `message.json`, and
  `wait.pid` paths.

Preparation does **not** change delivery mode. If the launcher never starts, existing
push remains registered. Repeating `prepare` before `exit.code` exists returns the
same run and launcher instead of creating a competing waiter. Unix launchers invoke
the current telex binary directly. Windows launchers use a Telex-generated
PowerShell file, so the prompt no longer embeds a handwritten platform wrapper.

Run `launcher.command` as one **fully detached** task (`detach: true`). The Copilot
task runner supplies detachment; Telex does not spawn a background process, and you
must not append shell backgrounding or wrap the command in a loop. When the task
actually starts, it:

1. verifies that this is still the station's current run and that the daemon
   supports the atomic fallback transition;
2. refuses to leave a live push bridge unless `prepare --force` explicitly recorded
   an intentional downgrade;
3. clears and verifies push registration, removes this address's bridge binding,
   then enters exactly one `telex wait` using `run_dir`.

Use `--timeout-ms`, `--min-attention`, and `--wake-on-cc` on `prepare` to configure
that wait. `--force` is not a recovery default; use it only when deliberately
leaving a bridge whose heartbeat is still live.

Preparation is not evidence that the waiter started or that a completion wakes
this host. If detached launch is unavailable or refused, report that limitation;
do not substitute a custom polling loop. If wait setup or backend access fails,
report the failure and inspect the terminal artifacts below. Documentation and
source-contract tests prove neither real App wakeup nor bridge loading.

### Process a fallback completion

The task completion notification is only a wakeup. Read `run_dir/exit.code` first;
it is written last and is the durable completion marker. Do not trust the detached
task's reported process exit code.

- **`0` (message):** read `delivery.json` (or flat `message.json`), dedupe by
  message id, then ack and disposition it:

  ```sh
  telex ack --address <addr> --id <message-id> --session "$COPILOT_AGENT_SESSION_ID"
  telex handle --address <addr> --id <message-id> --session "$COPILOT_AGENT_SESSION_ID" --note "completed"
  ```

  PowerShell uses `$env:COPILOT_AGENT_SESSION_ID`. After processing the terminal
  artifacts, call `fallback prepare` again; it creates the next unique run.
  For `delivery_role: "cc"`, follow the recipient-specific metadata: the observer
  copy is notification-only and does not require the primary recipient's ack or
  terminal disposition.

- **`1` (setup/error):** read `status.json.detail`, repair the reported condition,
  then prepare a fresh run. An old running daemon fails closed here; restart/update
  it rather than bypassing the mode gate.
- **`2` (idle timeout):** prepare the next run if the station should remain reachable.
- **`3` / `4` (daemon gone/hung):** repair the daemon, then prepare the next run.
- **`5` (presence ended):** inspect `telex --address <addr> status`. If
  `delivery_mode` is `push`, do not re-arm pull. If it is `pull`, re-attach/prepare
  only if the station should still be attended. If no active member/mode is
  reported, the station is stopped; leave it stopped unless the workflow still
  requires attendance.

If a task completion arrives but `exit.code` is absent, inspect status before
launching anything: a live waiter means the original run is still active. Otherwise
run `fallback prepare` again; it returns the same unfinished run so its generated
launcher can be retried. Duplicate launcher starts are rejected without overwriting
the active run's artifacts.

### Switch modes and stop

Status separates configured delivery from health:

- `delivery_mode: push` -- on-deliver push is registered;
- `delivery_mode: pull` -- Copilot is in pull-fallback mode (a live waiter is shown
  separately by `live_waiters_count` / `station_health`);
- `delivery_mode: conflict` -- version-skew/race tripwire; stop one mechanism before
  continuing.

The daemon rejects a waiter while push is registered and rejects push registration
while a waiter is live. To return from fallback to push, first confirm a supported
loader is available or a bridge is already loaded and live, then stop pull:

```sh
telex --address <addr> station stop --session "$COPILOT_AGENT_SESSION_ID"
telex --address <addr> copilot attach --copilot-bridge --description "<work>"
```

Use the supported loader (`extensions_reload` where available) if needed and
verify bridge liveness. Do not leave working pull for a loader the host lacks.
To end fallback without returning to push, run
`station stop` and do not prepare another run. Never wrap fallback launchers or
`telex wait` in an infinite shell loop.

## Watch a local session group for stalls

Use one ordinary Watcher v2 watch to wake a group's root when its active members
have stopped progressing. This is opt-in, same-machine and same-user only.
`telex copilot stall-detect` is the detector: Watcher supplies its v2 request on
stdin and receives one v2 result. Do not write a log parser, detector script or
session-owned polling loop.

### Create and maintain membership

The root creates an absolute, private group directory and one
`<local-session-id>.json` file per member:

```json
{
  "schemaVersion": 1,
  "sessionId": "local-session-id",
  "address": "project:example/worker:one",
  "state": "active",
  "reason": "Working on the assigned item"
}
```

Use the **local Copilot CLI id**, available to that member as
`COPILOT_AGENT_SESSION_ID`, not the App session/project id. The filename must match
`sessionId`. Keep the local-id-to-App-id mapping in the root's coordination
context, outside these strict records. The root owns file creation/removal,
session identity and distinct Telex addresses; members change only their own
`state` and optional `reason`, preserving the other fields.

| Member state | When to use it |
| --- | --- |
| `active` | The member is expected to make progress; only active members count. |
| `parked` | A planned wait has a known wakeup, including a Watcher watch, App automation, or background shell/agent. Record what will wake it. |
| `operator-held` | Progress requires an operator decision or gate. Record the question/gate. |
| `done` | The member's assigned work is complete; the root can remove its file when it leaves. |

Before ending a turn to wait on a background shell or agent, set your own state
to `parked` with a reason. Return to `active` when woken to resume work. If rung
while legitimately waiting, reply with what you are waiting for and stay parked.
Background work is **unobserved** by this detector; both event kinds explicitly
carry `backgroundWork: "unobserved"`. The App can also show idle while background
work runs, so confirmation alone does not replace the member's parked state.

Write a temporary non-`.json` file in the same directory, then atomically replace
the member file. Never truncate it in place. Records are strict JSON: no extra or
duplicate fields. Limits are 32 members, 4096 bytes per record, 64 total directory
entries (including temporary files), and 256 UTF-8 bytes each for address/reason.
Addresses must be distinct and contain no whitespace/control characters.
Membership ids are 1-128 ASCII letters, digits, hyphens or underscores.

### Register the watch

Start from the root's **existing running Watcher installation**. Obtain the
trusted absolute `telex` and `telex-watcher` executable paths from its installation
record, and the absolute registry path, configured backend profile and send-only
sender from its launch/registration policy. Use the backend that holds the
members' Telex addresses, with absolute paths in profile configuration.
If any value is unknown, ask the operator through `ask_user`; do not guess a path
or default profile. The root's address must already receive Telex turns.

The per-group choices are group directory, root address and grace period. The
template below copies the installation values into the exact shipped v2 fields;
backend, registry and cadence each have one source. Choose a new permanent
lowercase watch id; removed ids cannot be reused. This example uses a 30-second
cadence and 30-second timeout, allowing the detector's 15-second observation
budget plus startup. Set `pollIntervalSeconds` to the registration's interval
(the detector default is 30); grace is required and ranges from 1 to 86400 seconds.

Run this inline template in **PowerShell 7**, replacing the angle-bracket values:

```powershell
$ErrorActionPreference = 'Stop'
$groupDirectory = '<absolute group directory>'
$rootAddress = '<root Telex address>'
$graceSeconds = 300
$telex = '<absolute installed telex executable>'
$watcher = '<absolute installed telex-watcher executable>'
$registry = '<absolute running Watcher registry>'
$backendProfile = '<existing configured backend profile>'
$sender = '<existing Watcher send-only address>'
$watchId = '<new permanent watch id>'
$pollSeconds = 30
$registrationFile = Join-Path $groupDirectory 'stall-watch-registration.txt'
$registration = @{
    schemaVersion = 2
    id = $watchId
    command = @($telex, 'copilot', 'stall-detect')
    intervalSeconds = $pollSeconds
    timeoutSeconds = 30
    backendProfile = $backendProfile
    sender = $sender
    target = $rootAddress
    environmentAllowlist = @('TELEX_HOME')
    parameters = @{
        groupDirectory = $groupDirectory
        backendProfile = $backendProfile
        watcherExecutable = $watcher
        watcherRegistry = $registry
        graceSeconds = $graceSeconds
        pollIntervalSeconds = $pollSeconds
    }
}
$registration | ConvertTo-Json -Depth 4 |
    Set-Content -LiteralPath $registrationFile -Encoding utf8NoBOM
& $watcher --registry $registry --json register $registrationFile
if ($LASTEXITCODE -ne 0) { throw 'Watch registration failed; inspect the error before retrying.' }
& $watcher --registry $registry --json show $watchId
```

This writes registration data, not a detector/wrapper script. Its `.txt` suffix
keeps it out of the membership scan. `workingDirectory` is captured from the
registration command's current directory. Allowlist `TELEX_CONFIG` and credential
environment names too if that installation uses them; values come from the
runtime's launch environment, not this registration shell. Do not put secrets in
parameters. The detector uses its explicit backend profile, ignoring `TELEX_DB`
and global database overrides.

Inspect `status $watchId` and `attempts $watchId` through the same Watcher binary
and registry to confirm that attempts are running. Registration alone does not
prove monitoring is healthy. The detector excludes exactly its own watch from
wake sources; every other active watch targeting a member counts, even if it
depends on another idle member. An empty/no-active-member group is not stalled.

### On a stall or attention advisory

1. Ack the delivered Telex message and dedupe by message id before handling it.
   Verify its registered sender/store and watch id; read the snapshot's membership,
   classifier reasons, activity timestamps, message counts, watch ids and background
   uncertainty. Events are once per unchanged episode, not repeated reminders.
2. **Before acting, call the App's `get_sessions_status`.** Map the snapshot's
   local ids to the App ids in the root's context. Recheck busy members, open
   questions, planned waits and already-completed work. Never assume these two
   kinds of session id are interchangeable.
3. For `copilot.group.stalled`, diagnose the confirmed lack of progress, then ring
   the affected members with a concrete next action or a request for their wait
   reason. Use the App's `send_session_message` for a not-loaded session that needs
   resuming; an unattended Telex address can retain a message without waking it.
   For an attended member, Telex `send` is also suitable: include a meaningful
   `--subject` and the root's explicit `--session`.
4. For `copilot.group.attention-needed`, the snapshot is **degraded, not a stall**.
   After App confirmation, inspect the named unknown members and their reasons;
   ask them what stopped progress or what they are waiting for. Do not declare
   them idle or restart their work based on this advisory. Quiet unknown evidence
   older than grace can trigger it; missing evidence may remain degraded silently.
5. A legitimately waiting member replies with its wait reason and stays parked;
   a member resuming work returns itself to active. The root changes membership
   identity/removal only. Record the diagnosis and terminally disposition the
   event with `handle`, `reject` or `close` as appropriate.

Known idle/not-loaded members with no observed wake source must remain unchanged
for strictly longer than grace before a stall event. Busy and operator-waiting
members prevent a stall; unknown is never idle. Membership changes or new activity
(other than usage accounting) start a new episode. Duplicate transport delivery
still needs message-id deduplication.

### Degraded does not mean healthy silence

A failed observation emits no stall event and does not advance detector state.
Use the same Watcher registry's `health $watchId` and `attempts $watchId`:
`diagnosticCategory: "detector-degraded"` needs inspection, not an assumption that
someone is busy. Default management output hides detector diagnostic text.

Check membership limits/format, local session ids, backend/environment access
and the running Watcher management endpoint. Inspect the member's inbox using
the same backend, including consumed history (`inbox --all`, newest 50 by default); use
`export --address <addr>` to review older disposition history.

While observations are unavailable, show monitoring coverage as incomplete and
use App inspection. Keep the existing routing identity and message history.
Do not mark active work parked/done merely to suppress degradation.

The [group stall guide](https://github.com/lossyrob/telex/blob/main/docs/guide/src/guides/copilot-stalls.md)
has a member-update example and the root's status-view checklist.

## Version and compatibility

The header printed above this document reports the installed `telex` version and
source build identifier, the Copilot **bridge protocol** version, and the **minimum
compatible plugin** version. `telex --version` also includes the build identifier,
and `telex --json version` exposes it as `version.build_id`. Official release builds
are gated so this value equals the release commit; source builds without Git metadata
may report `unknown`. Git fallback is used only for a standalone Telex checkout whose
Git top-level is the manifest directory, never from an unrelated ancestor repository.
The identifier is diagnostic, not cryptographic provenance.

If the drain hook reports plugin/binary skew, use `telex --json version` to inspect
`version.current_exe`, plus `Get-Command telex` on Windows or `command -v telex` on
POSIX, and confirm the versioned launcher wins PATH precedence over stale shims such
as a Cargo-installed copy. Reinstall the plugin and binary from the same release,
then restart Copilot. If intentionally rolling back the binary, roll back the plugin
to the same release first. `TELEX_COPILOT_DRAIN=off` is a temporary escape hatch,
not a completed repair.

If your plugin is older than the minimum, `telex copilot skill` prints a compatibility
warning: update the plugin (or the binary) rather than trusting stale instructions.
You can force the check explicitly:

```sh
telex copilot skill --plugin-version <your-plugin-version>
```

## Syntax is owned by the binary -- run help before using details

This document describes the workflow; the **exact** flags come from the installed
binary. Before relying on specific syntax, run:

```sh
telex --version
telex --json version
telex copilot skill
telex copilot --help
telex copilot attach --help
telex copilot resume --help
telex copilot detach --help
telex copilot fallback --help
telex copilot fallback prepare --help
telex copilot gc --help
telex copilot session-state --help
telex copilot stall-detect --help
telex ack --help
telex handle --help
```

Use `telex <core-command> --help` (e.g. `telex send`, `telex status`, `telex wait`) for
the generic pull/send/status commands.
