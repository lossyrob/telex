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

   If you resume a Copilot session and the bridge files or heartbeat are gone, repair the
   station with the resume verb, then reload extensions:

   ```sh
   telex --address <addr> copilot resume --description "<what this session is doing>"
   ```

   `copilot resume` is an explicit re-provision of the same push bridge registration that
   `copilot attach --copilot-bridge` creates; it also re-scans queued unacked backlog.

2. **Load the bridge into the live session (one agent tool call).**

   Run the `extensions_reload` tool. telex cannot trigger a reload, so you do this once.
   After `/clear` (which reloads extensions and clears the conversation), **re-provision first** --
   re-run `telex --address <addr> copilot attach --copilot-bridge` and then `extensions_reload` --
   so the daemon re-delivers any message that was queued but not yet acked when you cleared. A
   re-attach (or a new session taking over the address) is what re-delivers unacked messages; while
   the same session stays continuously attached, an already-accepted message is **not** re-pushed on
   the fast cadence -- a long backstop may re-check it only every few minutes if it stays unacked.

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
   it unloaded immediately. Session end also removes the files.

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
env vars, and fail closed with `no session id available` without it:

```sh
telex --address <addr> send --to <peer> --body "..." --session "$COPILOT_AGENT_SESSION_ID"
```

On PowerShell use `$env:COPILOT_AGENT_SESSION_ID`. `telex reply` takes the same `--session`
(run `telex reply --help` for its exact flags). Only `telex copilot attach`/`detach` map the
Copilot session id for you; the generic verbs do not.

## Fallback: no bridge (pull mode)

If the bridge cannot be loaded (extensions disabled), telex push is unavailable.
**Surface that plainly** rather than silently spinning a waiter. If you must keep
receiving, fall back to generic pull mode with `telex wait`. `telex skill` documents
the generic pull workflow; the Copilot-specific mechanics for running that fallback
are below.

**Session id for pull commands.** Generic `telex wait`/`ack` do not read Copilot env
vars, so pass `--session "$COPILOT_AGENT_SESSION_ID"` (PowerShell:
`$env:COPILOT_AGENT_SESSION_ID`) on every invocation, or set `TELEX_SESSION_ID` in the
same shell. `telex copilot attach` maps `$COPILOT_AGENT_SESSION_ID` to the telex
session id and `$COPILOT_LOADER_PID` to the loader `--watch-pid` backstop for you; the
generic verbs do not.

**Detached waiter pattern (Copilot CLI / Windows).** Run the waiter as a single-shot,
**fully detached** background task (`detach: true`) so it does not spin the terminal
like foreground work; the Copilot session is still notified when the detached task
exits. Pass `--out-dir <dir>` and read the artifacts (`exit.code`, then
`delivery.json`/`message.json`) after the completion wake — detached stdout is not
returned to the agent. On Windows/Copilot CLI, wrap the call in a small `.ps1` and
detach `pwsh -File ...`: some Copilot CLI versions silently no-op a detached bare
external executable, while `pwsh -File` preserves PATH/environment. Keep the detached
command variable-free (pass literal paths/addresses as script arguments):

```powershell
pwsh -NoProfile -ExecutionPolicy Bypass -File "C:\path\to\telex-wait-once.ps1" `
  -Telex "telex" -Address "<addr>" -Session "<session-id>" `
  -OutDir "C:\path\to\telex-wait-<unique>"
```

```powershell
param(
  [Parameter(Mandatory)] [string]$Telex,
  [Parameter(Mandatory)] [string]$Address,
  [Parameter(Mandatory)] [string]$Session,
  [Parameter(Mandatory)] [string]$OutDir,
  [string]$MinAttention
)
if ([string]::IsNullOrWhiteSpace($MinAttention)) {
  & $Telex --json --address $Address wait --session $Session --timeout-ms 1800000 --out-dir $OutDir
} else {
  & $Telex --json --address $Address wait --session $Session --timeout-ms 1800000 --min-attention $MinAttention --out-dir $OutDir
}
exit $LASTEXITCODE
```

On the completion wake, read `<dir>\exit.code` (the completion marker — do **not**
trust the Copilot detached task's reported exit code). If it is `0`, parse
`<dir>\delivery.json` (or `<dir>\message.json`), then
`telex ack --address <addr> --id <message-id> --session "$env:COPILOT_AGENT_SESSION_ID"`
and dedupe by id before re-arming a fresh detached wait. If you see a completion
notification but `<dir>\exit.code` is missing, the waiter did not actually run — re-arm
via the `.ps1 -File` wrapper; do not infer an idle timeout. Do **not** use
`list_powershell` (or any task-list status) to decide whether the waiter is armed or
finished — a detached command can show `completed` while its child is still alive; the
completion wake plus the `exit.code` artifact are the signal. Never wrap `telex wait`
in an infinite shell loop. `telex wait --help` documents the waiter flags.

## Version and compatibility

The header printed above this document reports the installed `telex` version, the
Copilot **bridge protocol** version, and the **minimum compatible plugin** version. If
your plugin is older than the minimum, `telex copilot skill` prints a compatibility
warning: update the plugin (or the binary) rather than trusting stale instructions. You
can force the check explicitly:

```sh
telex copilot skill --plugin-version <your-plugin-version>
```

## Syntax is owned by the binary -- run help before using details

This document describes the workflow; the **exact** flags come from the installed
binary. Before relying on specific syntax, run:

```sh
telex --version
telex copilot skill
telex copilot --help
telex copilot attach --help
telex copilot resume --help
telex copilot detach --help
telex copilot gc --help
telex ack --help
telex handle --help
```

Use `telex <core-command> --help` (e.g. `telex send`, `telex status`, `telex wait`) for
the generic pull/send/status commands.
