---
name: telex
description: Use this skill when coordinating or messaging between AI agent sessions: attach a session to a durable address, wait for and disposition delivered messages, send or reply to operational messages, and find other live sessions through the self-registered address directory.
---

# Telex skill

## What telex is

Telex is a CLI-first message fabric for AI agent sessions. Ephemeral sessions attach to durable addresses, exchange typed operational messages with answerback liveness, and leave auditable disposition records. Use the single binary as `telex` (`telex.exe` on Windows).

Your operator will tell you which address to attach to. You can reload these instructions anytime with `telex skill`, or `telex skill --address <addr>` for instructions tailored to your assigned address.

## The core loop

Use Telex as a two-process loop: a resident **holder** keeps the address live, and a single-shot `telex wait` delivers one message and completes. Both run as **background processes**, and each needs two independent properties — set both:

| Property | Holder + each `wait` | Why |
|---|---|---|
| Foreground or background? | **Background** (non-blocking) | so they don't consume your turns — the foreground stays free to act and take operator input |
| Session-bound or persistent? | **Session-bound** — killed when your session ends | so the lease releases promptly once you're gone; a process that outlives the session keeps answering liveness for a session that no longer exists |

So: **background and session-bound.** Never start them as persistent / standalone / daemonized processes that survive the session — that orphans the holder and corrupts liveness. You drive the loop one delivery at a time: each single-shot `telex wait` surfaces a message to you when the command **completes**, and then you re-arm a fresh one (see **The re-arm pattern** below).

> **Two unrelated meanings of "attach/detach" — don't conflate them.**
> - **telex `attach` / `detach`** are **lease** verbs: occupy or release an address. They say nothing about OS process lifecycle.
> - Your **agent runtime** separately decides whether a background process is **session-bound** (dies with the session — what you want) or **fully detached / persistent** (outlives it — never use this for the holder or a `wait`).
>
> The holder is long-lived, but unlike a typical server it must **not** be marked persistent. *(In Copilot CLI terms: start them async with `detach: false` — the default — never `detach: true`, even though they run long.)*

1. Start the holder in the **background** and **bound to your session** — it must be terminated when the session ends, never daemonized or left to outlive it. The holder owns the lease/heartbeat, so binding its lifetime to the session lets the lease release promptly when you're gone.

   ```sh
   telex attach --address <addr> --description "<what this session is doing>"
   ```

   Export `TELEX_ADDRESS=<addr>` for the session so every later command — `wait`, `inbox`,
   `send`, `reply` — defaults to this address, including the `from` stamped on what you send
   (see **Your identity** below).

   Optional attach flags:

   ```sh
   telex attach --address <addr> --description "<s>" --scope <s> --tags <a,b> --heartbeat-secs N --poll-secs N
   ```

2. Wait for one message with a **single-shot** background `telex wait` — **not** an internal loop. It connects to the holder, blocks until one message is delivered, prints it as JSON, and **completes**. The command *completing* is your wake signal (see the box below); you re-arm at your turn level, not inside a shell loop.

   ```sh
   telex wait --address <addr>          # one delivery, then the command completes
   ```

   Omit `--timeout-ms` so the command completes **only on a real delivery** — then every wake is an actual message. Use a timeout only if your runtime caps command duration; a timeout just completes the command with exit 2 ("nothing yet"), and you re-arm.

   When the `wait` command completes, read its exit code and output:

   | Exit | Meaning | What you do |
   |---:|---|---|
   | 0 | delivered | Read the JSON message, act on it, disposition it, then re-arm a fresh `wait`. |
   | 2 | idle-timeout | Nothing arrived before `--timeout-ms` (only if you set one); just re-arm. |
   | 3 | holder-gone | Restart the holder (`telex attach`), then re-arm. |
   | 4 | holder-hung | Restart the holder (`telex attach`), then re-arm. |

3. When a `wait` completes with a message (exit 0), act on it and run an appropriate disposition verb with the message id from the JSON.

   ```sh
   telex handle --id <message-id> --note "completed"
   ```

   Disposition verbs are `telex ack`, `telex handle`, `telex defer`, `telex reject`, `telex close`, and `telex escalate`; all take `--id <message-id>` and optional `--note <s>`. Non-terminal dispositions (`ack`, `defer`, `escalate`) still need a final terminal disposition later.

### The re-arm pattern (one wait per turn, not a shell loop)

Drive the loop from your own turn cycle, one delivery at a time:

```text
once:   telex attach --address <addr> --description "<s>"   # background, session-bound; holds the lease
then repeat, one per turn:
  1. start a SINGLE background command:  telex wait --address <addr>
  2. it blocks until one message, prints JSON, and COMPLETES -> your runtime notifies you
  3. read the completed command's output:
       exit 0   -> act on the JSON, then disposition it
       exit 3/4 -> restart telex attach (holder gone/hung)
       exit 2   -> (only if you set --timeout-ms) nothing arrived
  4. re-arm: start a fresh single `telex wait` background command
```

This is not "ad hoc shell polling": each `telex wait` blocks for push delivery from the holder, and you only relaunch after one completes. (Ad hoc polling would be repeatedly running `telex inbox` on a timer with no holder — don't do that.) Telex owns the long-duration waiting; the holder keeps answerback live between your turns.

Messages buffer in the holder, so re-arm timing is safe: a second message that arrives while the foreground is still handling the first is delivered by the next `wait`. Deliveries serialize at the agent's turn pace; none are dropped.

One-shot commands (`send`, `reply`, `resolve`, `address list`, `inbox`, and the disposition verbs) run directly from the foreground **while the background `wait` is blocked** — they reach the holder/backend independently and need no background task of their own.

> **Gotcha — the invisible-loop trap.** Do **not** wrap `telex wait` in an infinite background loop (`while true; do telex wait; done`). Many agent runtimes only surface a background command's output **when the command completes** — an internal loop never completes, so deliveries pile up in the loop's buffer and never wake you (you'd have to manually poll the background shell). `telex wait` exits on each delivery on purpose: let the **completion** drive a fresh single-shot re-arm at your turn level. Also avoid a short `--timeout-ms`, which wakes you on idle-timeouts carrying no message.

## Sending and finding other sessions

**Your identity — the `from` address.** Every `send` and `reply` stamps a `from` taken from
`--from`, else `$TELEX_ADDRESS` / the global `--address`. Set it to the address you serve (the
one you attached to) so replies route back to your inbox. If `from` is unset the message is
**un-repliable**: `telex reply` to it fails with "no from address," and any reply has nowhere to
go. So export `TELEX_ADDRESS=<your-addr>` once after attaching (recommended), or pass
`--from <your-addr>` on each send. Use a *different* `--from` only deliberately — e.g. a one-shot
sender declaring a reply-to it will attach to later; a `from` you don't actually serve means
replies queue in an inbox nobody is watching.

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
telex send --to <addr> --from <your-addr> --subject "<s>" --body "<s>" --cc <a,b> --kind <s> --attention interrupt|next-checkpoint|background|fyi --requires-disposition --metadata <json>
```

`send` prints a receipt: `delivered`, `queued-unoccupied`, or `rejected-retired`, plus the new message id.

Reply inside an existing thread:

```sh
telex reply --to-message <message-id> --body "<body>"
```

Optional reply flags are `--from <your-addr>`, `--subject <s>`, `--kind <s>`, `--attention interrupt|next-checkpoint|background|fyi`, and `--requires-disposition`. As with `send`, `--from` defaults to `$TELEX_ADDRESS` / `--address`; the reply's destination is taken from the parent message's sender (so the parent must itself have had a `from`).

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
| `--backend <name>` | Use a configured backend by name (or `$TELEX_BACKEND`); defaults to the configured default backend, or an implicit `default` sqlite store. |
| `--db <path>` | Override the SQLite path for this invocation (sqlite backends only; or `$TELEX_DB`). |
| `--address <addr>` | Default address (or `$TELEX_ADDRESS`) for commands that act on one address; also the default `from` for `send`/`reply`. |
| `--json` / `--text` | Output format; default JSON when stdout is not a TTY, text when interactive. |

Postgres connections are configured once as named backends with `telex backend add` (see Backends), not via per-call environment variables.

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
| `telex send` | Send a message and print a delivery/queue/reject receipt plus message id. | `--to <addr>`, `--from <addr>`, `--subject <s>`, `--body <s>`, `--cc <a,b>`, `--kind <s>`, `--attention interrupt|next-checkpoint|background|fyi`, `--requires-disposition`, `--metadata <json>` |
| `telex reply` | Reply under a parent message thread. | `--to-message <id>`, `--body <s>`, `--from <addr>`, `--subject <s>`, `--kind <s>`, `--attention interrupt|next-checkpoint|background|fyi`, `--requires-disposition` |

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
| `telex init` | Create `~/.telex/`, write a default sqlite backend, and initialize its schema. | `--backend <name>`, `--db <path>` |
| `telex status` | Show the resolved backend, address, holder/IPC, and occupancy. | `--address <addr>` |
| `telex skill` | Print these usage instructions (embedded in the binary). | `--address <addr>`, `--raw` |

### BACKENDS

| Command | Purpose | Key flags |
|---|---|---|
| `telex backend add <name>` | Add (or update) a named backend. | `--sqlite [--path <p>]` or `--postgres <conn-string> [--schema <s>]` plus auth: `--entra [--entra-cred auto\|cli\|managed]`, `--password-env <VAR>`, or `--password-command <cmd>`; `--default` |
| `telex backend list` | List configured backends and the default. | |
| `telex backend show <name>` | Show one backend's config (secrets redacted). | |
| `telex backend default <name>` | Set the default backend. | |
| `telex backend remove <name>` | Remove a backend. | |
| `telex backend kinds` | List backend kinds compiled into this build. | |

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

A **backend** is a named, configured store (a "key"). Selection is by name:
`--backend <name>` → `$TELEX_BACKEND` → the configured `default` → an implicit `default`
sqlite store at `~/.telex/telex.db`. So with no setup, telex just works on local SQLite.

Configure backends once with `telex backend add`. The first one added becomes the default;
`--default` (or `telex backend default <name>`) changes it.

```sh
# Local sqlite (usually unnecessary — it's the implicit default):
telex backend add local --sqlite

# Networked Postgres, password from an env var:
telex backend add staging --postgres "postgresql://app@staging-db:5432/telex?sslmode=require" \
  --password-env STAGING_PG_PASSWORD --schema telex

# Azure Postgres with Entra (token fetched on demand by telex itself):
telex backend add prod \
  --postgres "host=myserver.postgres.database.azure.com port=5432 user=me@example.com dbname=postgres sslmode=require" \
  --entra --schema telex --default
```

`--entra` requires a telex build with the `entra` feature (the published release binaries
include it). On a laptop it uses your `az login`; on a devbox/VM with a managed identity use
`--entra-cred managed` for zero-login setup. As an alternative on builds without `entra`, you
can supply the token yourself via `--password-command` (e.g. `az account get-access-token ...`).

Then select a backend per command, or rely on the default:

```sh
telex --backend staging inbox
telex send --to node:x --body "hi"     # uses the default backend
telex backend list
```

The Postgres connection string is a libpq URI or a key=value DSN. Provide the password by
reference (`--entra`, `--password-env`, or `--password-command`) rather than embedding it.
`--entra` (Azure SDK; supports `az login` and managed identity) is available in builds with
the `entra` feature — which the published release binaries include.

## Worked example: two sessions

Session A attaches to a durable address and waits. Run `telex attach` (the holder) and each single-shot `telex wait` in the background, bound to A's session — never as persistent processes that outlive it.

```sh
export TELEX_ADDRESS=session:a   # all of A's commands default to this address (and its from)
telex attach --address session:a --description "session A waiting for coordination" --scope project:telex --tags repo:telex,role:worker
```

Then A waits with a **single-shot** background `telex wait` (session-bound, not blocking the foreground); it completes on the next delivery, which notifies A:

```sh
telex wait --address session:a
```

Session B also starts its own holder in the background, bound to its session.

```sh
export TELEX_ADDRESS=session:b   # B's from; A's reply will route back here
telex attach --address session:b --description "session B requesting status" --scope project:telex --tags repo:telex,role:requester
```

Then Session B finds A and sends a disposition-required message. One-shot commands like `resolve` and `send` run directly — no background task needed; only the holder and each `telex wait` run in the background.

```sh
telex address list --scope project:telex --match "session A"
telex resolve --match "waiting for coordination" --scope project:telex
telex send --to session:a --subject "Status request" --body "Please send your current status." --attention interrupt --requires-disposition
```

A's `wait` command completes (exit 0) with the delivered message as JSON, which notifies A. A reads the id from the JSON, handles the work, dispositions it, replies in the same thread, and re-arms a fresh `wait`.

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
