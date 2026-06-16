---
name: telex
description: Use this skill when coordinating or messaging between AI agent sessions: attach a session to a durable address, wait for and disposition delivered messages, send or reply to operational messages, and find other live sessions through the self-registered address directory.
---

# Telex skill

## What telex is

Telex is a CLI-first message fabric for AI agent sessions. Ephemeral sessions attach to durable addresses, exchange typed operational messages with answerback liveness, and leave auditable disposition records. Use the single binary as `telex` (`telex.exe` on Windows).

Your operator will tell you which address to attach to. You can reload these instructions anytime with `telex skill`, or `telex skill --address <addr>` for instructions tailored to your assigned address.

## The core loop

Use Telex as a two-process loop, and run **both** processes as session-attached background tasks — never in the foreground, never as detached daemons. A supervisor (a background task or sub-agent) owns the holder and the `wait` loop and relays each delivered message to the foreground agent, which acts at its next turn. One resident holder keeps the address live; repeated `wait` calls deliver messages.

1. Start the holder as a **session-attached background task**. It must die with the session; never run it as a detached daemon. The holder owns the lease/heartbeat, so tying its lifetime to the session prevents stale liveness after the session dies.

   ```sh
   telex attach --address <addr> --description "<what this session is doing>"
   ```

   Optional attach flags:

   ```sh
   telex attach --address <addr> --description "<s>" --scope <s> --tags <a,b> --heartbeat-secs N --poll-secs N
   ```

2. Run the `wait` re-arm loop **inside the supervisor**, never in the foreground. Each `wait` connects to the holder, blocks until a message is delivered, prints it as JSON, and exits. The supervisor handles every exit code itself and **surfaces to the foreground only on a code-0 delivery**, carrying that JSON payload; codes 2/3/4 never reach the foreground.

   ```sh
   telex wait --address <addr>
   ```

   Exit codes — and who handles them:

   | Code | Meaning | Supervisor action | Reaches foreground? |
   |---:|---|---|:---:|
   | 0 | delivered | Emit the JSON payload to the foreground agent, then re-arm `wait`. | **yes** |
   | 2 | idle-timeout | Re-arm `wait` silently. Do **not** wake the foreground. | no |
   | 3 | holder-gone | Re-run `attach`, then re-arm `wait`. | no |
   | 4 | holder-hung | Re-run `attach`, then re-arm `wait`. | no |

   **In a background relay, prefer a blocking wait: omit `--timeout-ms` (or set it large) so `wait` completes exactly once, on real delivery.** A short timeout manufactures code-2 churn for no benefit — `holder-gone` is still detected promptly. Use `--timeout-ms N` only when your runtime caps individual command duration, forcing the supervisor to re-arm on a timer:

   ```sh
   telex wait --address <addr> --timeout-ms N   # only when commands are duration-capped
   ```

3. When the supervisor surfaces a delivery, the **foreground agent** acts and runs an appropriate disposition verb with the message id from the JSON.

   ```sh
   telex handle --id <message-id> --note "completed"
   ```

   Disposition verbs are `telex ack`, `telex handle`, `telex defer`, `telex reject`, `telex close`, and `telex escalate`; all take `--id <message-id>` and optional `--note <s>`. Non-terminal dispositions (`ack`, `defer`, `escalate`) still need a final terminal disposition later.

### Supervisor recipe

The supervisor owns `attach` plus the `wait` re-arm loop and is the only thing that blocks:

```text
telex attach --address <addr> --description "<s>"   # background; holds the lease
loop:
  run: telex wait --address <addr>
  exit 0      -> emit stdout (the JSON message) to the foreground agent; continue
  exit 2      -> continue                            # idle-timeout: never surfaces
  exit 3 or 4 -> telex attach --address <addr> ...; continue   # holder gone/hung: re-attach
```

This re-arm loop **is** the sanctioned pattern — it is not the "ad hoc shell polling" warned against here. The difference: the supervisor blocks in `wait` for push delivery from the holder and only re-issues on exit; ad hoc polling is the *foreground* agent repeatedly checking the inbox on its own timer. Do not do the latter — Telex owns long-duration waiting, and the holder keeps answerback live while the agent is between turns or handling a message.

Messages buffer in the holder, so re-arm timing is safe: a second message that arrives while the foreground is still handling the first is delivered by the next `wait`. Deliveries serialize at the agent's turn pace; none are dropped.

One-shot commands (`send`, `reply`, `resolve`, `address list`, `inbox`, and the disposition verbs) run directly from the foreground **while the background `wait` is blocked** — they reach the holder/backend independently and need no background task of their own.

> **Gotcha — the self-wake loop.** If the supervisor surfaces idle-timeouts (exit 2) to the foreground, the foreground wakes with no message, re-issues `wait`, and wakes again — a self-wake cycle that starves operator input and looks like a hung session. Cause: treating exit 2 as a foreground event. Fix: handle 2/3/4 entirely inside the supervisor and surface only exit 0.

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
| `--backend <name>` | Use a configured backend by name (or `$TELEX_BACKEND`); defaults to the configured default backend, or an implicit `default` sqlite store. |
| `--db <path>` | Override the SQLite path for this invocation (sqlite backends only; or `$TELEX_DB`). |
| `--address <addr>` | Default address or `$TELEX_ADDRESS`. |
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

Session A attaches to a durable address and waits. Run both `attach` and the `wait` loop as session-attached background tasks, never detached.

```sh
telex attach --address session:a --description "session A waiting for coordination" --scope project:telex --tags repo:telex,role:worker
```

Then A's supervisor runs the `wait` loop as a session-attached background task too (not the foreground agent), relaying each delivered message to A:

```sh
telex wait --address session:a
```

Session B also starts its own holder as a session-attached background task.

```sh
telex attach --address session:b --description "session B requesting status" --scope project:telex --tags repo:telex,role:requester
```

Then Session B finds A and sends a disposition-required message. One-shot commands like `resolve` and `send` run directly — no background task needed; only the holder and `wait` loop are backgrounded.

```sh
telex address list --scope project:telex --match "session A"
telex resolve --match "waiting for coordination" --scope project:telex
telex send --to session:a --subject "Status request" --body "Please send your current status." --attention interrupt --requires-disposition
```

A's background `wait` exits 0 with the delivered message as JSON; the supervisor relays it to A. A reads the id from the JSON, handles the work, dispositions it, and replies in the same thread.

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
