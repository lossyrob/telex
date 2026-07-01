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
   `$COPILOT_AGENT_SESSION_ID` and `$COPILOT_LOADER_PID` for you.

2. **Load the bridge into the live session (one agent tool call).**

   Run the `extensions_reload` tool. telex cannot trigger a reload, so you do this once.
   After `/clear` (which reloads extensions), run it once more.

3. **Receive messages as turns.** A delivered telex message arrives as a new turn
   labelled `[telex] from <addr> (<attention>)`. An `interrupt` message is delivered as
   soon as possible (Copilot `immediate`, ahead of enqueued messages); every other
   attention level is `enqueue`d and arrives after your current turn. Neither preempts a
   turn already running. Read it, then record disposition **by id**:

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

4. **Tear down when done.**

   ```sh
   telex --address <addr> copilot detach
   ```

   This detaches the address and, when it was the last binding, removes the bridge files so
   it will not reload on a later resume. The already-loaded bridge stays live until the next
   `extensions_reload` or session end; run `extensions_reload` once after detach if you want
   it unloaded immediately. Session end also removes the files.

## Fallback: no bridge

If the bridge cannot be loaded (extensions disabled), telex push is unavailable.
**Surface that plainly** or fall back to generic pull mode (`telex wait`) -- do **not**
silently spin a waiter. `telex skill` documents the generic pull workflow, and
`telex wait --help` documents the waiter.

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
telex copilot detach --help
telex ack --help
telex handle --help
```

Use `telex <core-command> --help` (e.g. `telex send`, `telex status`, `telex wait`) for
the generic pull/send/status commands.
