# Telex for Copilot CLI: push delivery

This is the Copilot-specific Telex workflow, printed by `telex copilot skill` so it
always matches the installed binary. The plugin skill is only a bootstrap; **this
document (from your installed `telex`) is the source of truth** for the Copilot path.
For exact flags, run the `--help` commands listed at the end rather than trusting any
static copy of the syntax.

## What changes for Copilot

In Copilot CLI, telex delivers messages to you as **turns**. You do **not** run,
re-arm, or babysit a `telex wait` waiter. You bind once, load an in-session bridge
once, then read messages as they arrive and record disposition by id. The bridge is a
liveness path only -- it never acks for you, so the durable consumed mark is still
yours to make.

## The bridge path

1. **Bind your address and provision the bridge (one command).**

   ```sh
   telex --address <addr> copilot attach --copilot-bridge --description "<what this session is doing>"
   ```

   **Prerequisite:** Enable **Copilot Extensions** under `/experimental`. Copilot exposes
   the `extensions_reload` tool only when that experimental feature is enabled; without it,
   Telex can provision and register the bridge files but cannot load them into the live session.

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

   On normal Copilot resume, the retained session-scoped extension is discovered during startup.
   Re-arm daemon attendance and rescan queued unacked backlog with:

   ```sh
   telex --address <addr> copilot resume --description "<what this session is doing>"
   ```

   `copilot resume` explicitly re-provisions the same push registration that
   `copilot attach --copilot-bridge` creates. It retains/re-writes the same bridge files and
   re-scans queued unacked backlog; it does not normally require an in-session extension reload.

2. **Load the bridge into an already-running session when provisioning or recovering.**

   After the first `copilot attach --copilot-bridge` in a live session, run the
   `extensions_reload` tool once. Telex cannot trigger a reload.

   A normal close/resume does not need this step because Copilot discovers the retained extension
   during startup. If the bridge heartbeat is missing after startup, run `copilot resume` and then
   `extensions_reload` as recovery provisioning into the already-running session.

   If `extensions_reload` is unavailable:

   1. Enable Copilot Extensions under `/experimental`.
   2. Re-provision the station with
      `telex --address <addr> copilot resume --description "<what this session is doing>"`.
   3. Run `extensions_reload`.

   If Copilot Extensions cannot be enabled, push is unavailable. Use the supported pull
   fallback below or detach the station with `telex --address <addr> copilot detach`.

   After `/clear` (which reloads extensions and clears the conversation), **re-provision first** --
   re-run `telex --address <addr> copilot resume` -- so the daemon re-delivers any message that was
   queued but not yet acked when you cleared. Run `extensions_reload` only if the retained extension
   is not live in the already-running session. A re-attach (or a new session taking over the
   address) is what re-delivers unacked messages; while the same session stays continuously
   attached, an already-accepted message is **not** re-pushed on the fast cadence -- a long
   backstop may re-check it only every few minutes if it stays unacked.

   Troubleshooting CC observer turns: if CC traffic is visible in `telex inbox` but is not arriving
   as Copilot turns, the bridge was probably bound without CC wake or the extension was not reloaded.
   Re-run the bind with the CC flag, then run `extensions_reload`:

   ```sh
   telex --address <addr> copilot attach --copilot-bridge --wake-on-cc --description "<what this session is doing>"
   ```

   Do not start `telex wait --wake-on-cc` for a Copilot plugin session; the bridge registration above
   is how the agent asks the daemon to watch live CC traffic.

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
   `extensions_reload` or session end; run `extensions_reload` once after detach if you want
   it unloaded immediately.

   Ordinary session end is resumable: it marks daemon attendance idle and clears transient
   turn-guard state, but retains the extension, bindings, and registry for startup discovery.
   If a closed session will not resume, explicitly detach before closing or use forced GC after
   verifying the session is gone.

   To inspect or clean orphaned bridge files from sessions that closed without detach:

   ```sh
   telex copilot gc --dry-run
   telex copilot gc --force
   ```

   GC is conservative: a live bridge heartbeat is kept, and bindings are kept unless you
   force cleanup after verifying the session is gone.

## Sending and replying

Receiving is push; **sending is not**. To start or continue a conversation, `telex send` and
`telex reply` need your session id -- exactly like `ack`/`handle`, they do **not** read Copilot
env vars, and fail closed with `no session id available` without it.

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

If the bridge cannot be loaded (extensions disabled), telex push is unavailable.
**Surface that plainly** rather than pretending push is live. Telex can prepare one
cross-platform, single-shot pull fallback without requiring an agent-authored script:

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
while a waiter is live. To return from fallback to push, stop pull first:

```sh
telex --address <addr> station stop --session "$COPILOT_AGENT_SESSION_ID"
telex --address <addr> copilot attach --copilot-bridge --description "<work>"
```

Then run `extensions_reload`. To end fallback without returning to push, run
`station stop` and do not prepare another run. Never wrap fallback launchers or
`telex wait` in an infinite shell loop.

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
telex ack --help
telex handle --help
```

Use `telex <core-command> --help` (e.g. `telex send`, `telex status`, `telex wait`) for
the generic pull/send/status commands.
