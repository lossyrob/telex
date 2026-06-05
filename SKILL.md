---
name: telex
description: Use this skill when coordinating or messaging between AI agent sessions: attach a session to a durable address, wait for and disposition delivered messages, send or reply to operational messages, and find other live sessions through the self-registered address directory.
---

# Telex skill

## What telex is

Telex is a CLI-first message fabric for AI agent sessions. Ephemeral sessions attach to durable addresses, exchange typed operational messages with answerback liveness, and leave auditable disposition records. Use the single binary as `telex` (`telex.exe` on Windows).

## The core loop

Use Telex as a two-process loop: one resident holder keeps the address live; repeated `wait` calls deliver messages to the agent turn.

1. Start the holder as a **session-attached background task**. It must die with the session; never run it as a detached daemon. The holder owns the lease/heartbeat, so tying its lifetime to the session prevents stale liveness after the session dies.

   ```sh
   telex attach --address <addr> --description "<what this session is doing>"
   ```

   Optional attach flags:

   ```sh
   telex attach --address <addr> --description "<s>" --scope <s> --tags <a,b> --heartbeat-secs N --poll-secs N
   ```

2. Loop on `wait`. `wait` connects to the running holder, blocks, prints one delivered message as JSON, and exits.

   ```sh
   telex wait --address <addr>
   ```

   Exit codes:

   | Code | Meaning | Agent action |
   |---:|---|---|
   | 0 | delivered | Read the JSON payload, act, disposition the message, then wait again. |
   | 2 | idle-timeout | No message before `--timeout-ms`; re-issue `wait`. |
   | 3 | holder-gone | Restart `attach`, then wait again. |
   | 4 | holder-hung | Restart `attach`, then wait again. |

   If your runtime caps command duration, use:

   ```sh
   telex wait --address <addr> --timeout-ms N
   ```

3. After handling a delivered message, run an appropriate disposition verb with the message id from the JSON.

   ```sh
   telex handle --id <message-id> --note "completed"
   ```

   Disposition verbs are `telex ack`, `telex handle`, `telex defer`, `telex reject`, `telex close`, and `telex escalate`; all take `--id <message-id>` and optional `--note <s>`. Non-terminal dispositions (`ack`, `defer`, `escalate`) still need a final terminal disposition later.

Do not rebuild this with ad hoc shell polling. Telex owns long-duration waiting; the holder keeps answerback live while the agent is between turns or handling a message.

## Sending and finding other sessions

Find targets by their self-registered attach descriptions, scope, or tags.

```sh
telex address list --scope <scope>
telex address list --match "<substring>"
telex resolve --match "<substring>"
telex resolve --tag <tag> --scope <scope>
```

Then send to the selected address.

```sh
telex send --to <addr> --subject "<subject>" --body "<body>"
```

Useful send flags:

```sh
telex send --to <addr> --subject "<s>" --body "<s>" --cc <a,b> --kind <s> --attention interrupt|next-checkpoint|background|fyi --requires-disposition --metadata <json>
```

`send` prints a receipt: `delivered`, `queued-unoccupied`, or `rejected-retired`, plus the new message id.

Reply inside an existing thread:

```sh
telex reply --to-message <message-id> --body "<body>"
```

Optional reply flags are `--attention interrupt|next-checkpoint|background|fyi` and `--requires-disposition`.

## Reading

List actionable and recent messages for an address:

```sh
telex inbox --address <addr>
telex inbox --address <addr> --all --limit N
```

Read a message, with compact thread context when useful:

```sh
telex read --id <message-id> --thread
```

Use `--full` only when you need full history:

```sh
telex read --id <message-id> --full
```

## Global options

Global options apply to all subcommands.

| Option | Purpose |
|---|---|
| `--backend <sqlite|postgres>` | Backend; default `sqlite` or `$TELEX_BACKEND`. |
| `--db <path>` | SQLite file; default `~/.telex/telex.db` or `$TELEX_DB`. |
| `--address <addr>` | Default address or `$TELEX_ADDRESS`. |
| `--json` / `--text` | Output format; default JSON when stdout is not a TTY, text when interactive. |

Postgres connection is configured with `TELEX_PG_HOST`, `TELEX_PG_USER`, `TELEX_PG_DB`, and `TELEX_PG_PASSWORD`.

## Command reference

### PRESENCE

| Command | Purpose | Key flags |
|---|---|---|
| `telex attach` | Become the live occupant, hold the lease, run the holder, and register the directory description. Blocks. Fails if the address is already occupied by a live lease. | `--address <addr>`, `--description <s>`, `--scope <s>`, `--tags <a,b>`, `--heartbeat-secs N`, `--poll-secs N` |
| `telex detach` | Release the lease and stop a running holder. | `--address <addr>` |

### RECEIVE

| Command | Purpose | Key flags |
|---|---|---|
| `telex wait` | Block on the holder; on delivery print one message as JSON and exit. | `--address <addr>`, `--timeout-ms N` |
| `telex inbox` | List actionable messages requiring disposition and recent messages for the address. | `--address <addr>`, `--all`, `--limit N` |
| `telex read` | Read a message. `--thread` shows compact thread context; `--full` shows full history. | `--id <message-id>`, `--thread`, `--full` |

### SEND

| Command | Purpose | Key flags |
|---|---|---|
| `telex send` | Send a message and print a delivery/queue/reject receipt plus message id. | `--to <addr>`, `--subject <s>`, `--body <s>`, `--cc <a,b>`, `--kind <s>`, `--attention interrupt|next-checkpoint|background|fyi`, `--requires-disposition`, `--metadata <json>` |
| `telex reply` | Reply under a parent message thread. | `--to-message <id>`, `--body <s>`, `--attention interrupt|next-checkpoint|background|fyi`, `--requires-disposition` |

### DISPOSITION

| Command | Purpose | Key flags |
|---|---|---|
| `telex ack` | Mark the message `acknowledged`. | `--id <message-id>`, `--note <s>` |
| `telex handle` | Mark the message `handled`. | `--id <message-id>`, `--note <s>` |
| `telex defer` | Mark the message `deferred`. | `--id <message-id>`, `--note <s>` |
| `telex reject` | Mark the message `rejected`. | `--id <message-id>`, `--note <s>` |
| `telex close` | Mark the message `closed`. | `--id <message-id>`, `--note <s>` |
| `telex escalate` | Mark the message `escalated`. | `--id <message-id>`, `--note <s>` |

### DIRECTORY

| Command | Purpose | Key flags |
|---|---|---|
| `telex address list` | Show addresses with description, occupancy, and liveness grade. | `--scope <s>`, `--match <substr>`, `--tag <t>`, `--all` |
| `telex address show` | Show detail for one address plus lease/occupancy. | `--address <addr>` |
| `telex address retire` | Retire an address so it drops from normal listings. | `--address <addr>` |
| `telex resolve` | Resolve target addresses by description substring or tag. | `--match <substr>` or `--tag <t>`, `--scope <s>` |

### AUDIT

| Command | Purpose | Key flags |
|---|---|---|
| `telex export` | Emit messages and disposition history as JSON lines for audit/provenance. | `--address <addr>`, `--thread <id>`, `--since <id>` |

### SETUP

| Command | Purpose | Key flags |
|---|---|---|
| `telex init` | Create `~/.telex/` and initialize schema. | `--backend <sqlite|postgres>`, `--db <path>` |
| `telex status` | Show config, backend, address, holder/IPC, and occupancy. | `--address <addr>` |

## Attention levels

Use exactly one of:

```text
interrupt | next-checkpoint | background | fyi
```

Meanings:

- `interrupt` — deliver as soon as possible, at the next agent turn boundary.
- `next-checkpoint` — handle after the current safe stopping point.
- `background` — visible in inbox, but should not derail current work.
- `fyi` — visible/auditable, non-actionable by default.

## Disposition states

States are:

```text
acknowledged | handled | deferred | rejected | closed | escalated
```

Terminal states, removed from the actionable inbox: `handled`, `rejected`, `closed`.
Non-terminal states, still needing final disposition: `acknowledged`, `deferred`, `escalated`.

## Latency: interrupt means next turn

Agent wake dominates perceived latency: measured waiter-exit-to-agent-turn time is roughly 6-26 seconds, while Telex backend delivery is sub-second. `interrupt` means the message should be handled at the next turn boundary; it is not preemption and cannot stop a model mid-turn. Shorter polling or push changes backend latency, not agent wake latency.

## Backends

SQLite is the zero-config default. If you do nothing, Telex uses `--backend sqlite` with database file `~/.telex/telex.db`. Override on any command with `--db <path>`, or set `TELEX_BACKEND=sqlite` and `TELEX_DB=<path>`.

For networked coordination, select Postgres on any command:

```sh
telex --backend postgres <command>
```

Configure Postgres with:

```text
TELEX_PG_HOST
TELEX_PG_USER
TELEX_PG_DB
TELEX_PG_PASSWORD
```

`TELEX_PG_PASSWORD` may be either a SQL password or an Entra access token.

## Worked example: two sessions

Session A attaches to a durable address and waits. Start `attach` as a session-attached background task, not detached.

```sh
telex attach --address session:a --description "session A waiting for coordination" --scope project:telex --tags repo:telex,role:worker
```

Then in Session A's foreground loop:

```sh
telex wait --address session:a
```

Session B also starts its own holder as a session-attached background task.

```sh
telex attach --address session:b --description "session B requesting status" --scope project:telex --tags repo:telex,role:requester
```

Then Session B's foreground finds A and sends a disposition-required message.

```sh
telex address list --scope project:telex --match "session A"
telex resolve --match "waiting for coordination" --scope project:telex
telex send --to session:a --subject "Status request" --body "Please send your current status." --attention interrupt --requires-disposition
```

A's `wait` exits 0 and prints the delivered message as JSON. A reads the id from the JSON, handles the work, dispositions it, and replies in the same thread.

```sh
telex handle --id <message-id-from-wait-json> --note "status prepared"
telex reply --to-message <message-id-from-wait-json> --body "Status: holder is live; continuing work." --attention next-checkpoint
telex wait --address session:a
```

Session B waits, receives A's reply, and closes or handles it.

```sh
telex wait --address session:b
telex handle --id <reply-message-id-from-wait-json> --note "reply received"
```
