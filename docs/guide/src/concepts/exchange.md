# Exchange and daemon

The **exchange** is a per-user local process (a daemon) that owns presence,
delivery buffering, and the message store, reached by the CLI over local IPC.
Sessions do not run a resident holder process of their own; they issue one-shot
commands to the exchange.

## Auto-spawn

There is no manual server to start. The first daemon-backed command auto-spawns
the exchange for the selected [backend](backends.md). With no configuration this
is a local SQLite store at `~/.telex/telex.db`.

## One-shot commands

Most verbs register or act and then exit:

- `attach` registers the session and address, then exits.
- `wait` blocks as one client for a single delivery, then exits.
- `send`, `reply`, `ack`, and the disposition verbs act once and exit.

Because commands are one-shot, telex is driven from a script or an agent's own
turn cycle rather than a long-lived foreground process. See
[Set up an agent (pull)](../guides/agent-pull.md).

## Restart recovery

The exchange holds the durable message buffer in its store. If the daemon
restarts, the next verb reconnects and re-registers on a `NeedsAttach` signal and
continues against the retained buffer. A `wait` that finds no daemon exits with a
distinct code so the caller can run `attach` (the spawning and recovery verb) and
re-arm. See [Exit codes](../reference/exit-codes.md).

---

Next: [Stations and presence](presence.md)
