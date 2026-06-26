---
name: telex
description: Use this skill when coordinating or messaging between AI agent sessions: attach a session to a durable address, wait for and disposition delivered messages, send or reply to operational messages, and find other live sessions through the self-registered address directory.
---

# Telex skill

## What telex is

Telex is a CLI-first message fabric for AI agent sessions. Ephemeral sessions attach to durable addresses, exchange typed operational messages with answerback liveness, and leave auditable disposition records. Use the single binary as `telex` (`telex.exe` on Windows).

Your operator will tell you which address to attach to. You can reload these instructions anytime with `telex skill`, or `telex skill --address <addr>` for instructions tailored to your assigned address.

## The core loop

Use Telex as a **one-shot command loop** backed by an auto-spawned per-user local
exchange (daemon). Sessions no longer run a resident holder process. `attach`
registers your session/address with the local exchange and exits; `wait` blocks
as one daemon client for one delivery and exits; `ack` is the explicit durable
consumed mark for the message you just received.

Before attaching, make sure the session has a stable identity. Telex uses
`--session`, `$TELEX_SESSION_ID`, or (in Copilot) `$COPILOT_AGENT_SESSION_ID`.
It fails closed rather than guessing.

1. Register your session's address once:

   ```sh
   export TELEX_SESSION_ID=<stable-session-id>   # often already supplied by the harness
   telex attach --address <addr> --description "<what this session is doing>"
   ```

   Optional metadata:

   ```sh
   telex attach --address <addr> --session <id> --description "<s>" --scope <s> --tags <a,b> --watch-pid anchor:<pid>
   ```

   `--watch-pid` is a non-destructive liveness backstop. The v1 floor is a loader
   `anchor` pid with a start-time reuse guard; when it dies, blocked waits return
   `PresenceEnded` but the station and durable message buffer remain.

2. Wait for one message with a single-shot **fully detached** background
   `telex wait` (`detach: true` in Copilot CLI). This is for **UX**, not just
   shutdown survival: a session-attached async waiter keeps the terminal busy /
   spinner-like and competes with normal user-agent interaction. A fully detached
   background task still wakes the session when it completes; that completion
   notification is the waiter loop's wake signal. Pass `--out-dir <dir>` so the
   waiter writes its result to files; the woken agent reads those instead of
   relying on captured stdout:

   ```sh
   telex wait --address <addr> --out-dir <dir>
   ```

   On exit, `telex wait --out-dir` writes into `<dir>`:
   - `message.json` — the delivered message (only on exit `0`);
   - `status.json` — `{ outcome, exit_code, detail, address, written_at_ms }`, always;
   - `exit.code` — the integer exit code, written **last** as the completion marker.

   This matters because some runtimes (e.g. Copilot CLI) do not return fully
   detached shell stdout, and the detached task's reported exit code may describe
   only the launcher/wrapper. Do not fall back to `detach:false` just to recover
   stdout; that regresses the waiter into foreground-ish UX. Trust the artifact
   `exit.code`, not the shell task exit code. The detached completion notification
   is the wake signal; after it arrives, read
   `exit.code` (then `message.json` if it is `0`), `ack` the delivered message and
   dedupe by id, then re-arm a fresh detached `wait` before longer processing.
   Do not hide `wait` inside an infinite shell loop.
   `wait` does **not** spawn the daemon. If the daemon is gone, `wait` exits 3 so
   the agent can run `telex attach` (the spawning/recovery verb) and then re-arm.
   If a replacement daemon already exists, `wait` can reconnect/re-register during
   its bounded reconnect grace.

   | Exit | Meaning | What you do |
   |---:|---|---|
   | 0 | delivered | Read `message.json` (or stdout JSON), `ack` + dedupe by id, then re-arm a fresh `wait` before longer processing. |
   | 2 | idle-timeout | Nothing arrived before `--timeout-ms`; re-arm if still attending. |
   | 3 | daemon gone / not running | Run `telex attach` and re-arm. |
   | 4 | daemon hung / no response after a finite wait's `--timeout-ms + --hang-ms` watchdog | Re-arm or restart the daemon if repeated. |
   | 5 | presence ended | Non-destructive reap; live sessions should `attach`/`wait` again. |

3. After reading the delivered JSON, explicitly ack it, then apply the workflow
   disposition that reflects the actual outcome:

   ```sh
   telex ack --address <addr> --id <message-id>
   telex handle --address <addr> --id <message-id> --note "completed"
   ```

   `ack` is transport consumption for `(message_id, recipient-address)`. Terminal
   workflow disposition is still `handle`, `reject`, or `close`; `defer` and
   `escalate` are non-terminal. Dispositions default to the current `--address`
   recipient; pass `--recipient` only when intentionally recording for another
   recipient.

### The re-arm pattern (one wait per delivery, not a shell loop)

Drive the loop from your own turn cycle:

```text
once:   telex attach --address <addr> --description "<s>"
then repeat:
  1. start one detached background command named `TELEX MESSAGE WAITER`:
     while focused on other work: `telex wait --address <addr> --min-attention interrupt --out-dir <dir>`
     while idle/ready for anything: `telex wait --address <addr> --out-dir <dir>`
  2. it blocks until one message, exits, and the runtime completion wakes you
  3. read `<dir>\exit.code` (not the shell task exit code):
     0 -> parse `message.json`, run `telex ack`, dedupe by id, then start a fresh wait before longer processing
     5 -> attach/wait again if the session is still live
     2/3/4 -> re-arm or restart as indicated above (see `status.json` for detail)
```

### Two-phase attention loop

When you are actively working, arm a phase-1 waiter with
`--min-attention interrupt`. It wakes only for urgent messages; `next-checkpoint`,
`background`, and `fyi` messages stay durably buffered for your next checkpoint.
When you finish the current unit of work or reach a natural checkpoint, do phase
2: inspect `telex inbox --all --address <addr>`, read/ack/disposition the
pending messages you are ready to handle, then either continue with an
interrupt-only waiter or, if you are idle, arm an unfiltered waiter.

Do not run an interrupt-only waiter and an unfiltered waiter at the same time:
the daemon permits only one live waiter per station. To switch modes, let the
current waiter complete, or stop the station (`telex station stop`), re-attach,
and arm the new mode.

> **Gotcha — the invisible-loop trap.** Do **not** wrap `telex wait` in an
> infinite background loop (`while true; do telex wait; done`). Many agent
> runtimes surface output only when the command completes; an internal loop hides
> delivered messages in a background buffer.

### Copilot CLI detached waiter pattern

Use a fully detached shell task (`detach: true`) for the waiter UX. This is the
standard pattern: it does **not** spin the terminal like foreground work, and the
Copilot CLI session is still notified when the detached task exits. Do **not** use
session-attached async (`detach:false`) as the normal waiter mode just because it
returns stdout; use `--out-dir` artifacts instead. Let `telex wait --out-dir
<dir>` write the result to files you read after the detached completion
notification. Name the background task **`TELEX MESSAGE WAITER`** so it is
obvious in `/tasks`.

On Windows/Copilot CLI, use a small `.ps1` file and detach `pwsh -File ...` as
the primary reliable pattern. Some Copilot CLI versions silently no-op when a
detached task is a bare external executable (`telex wait ...`) even though the
same command works attached; wrapping the same call in `pwsh -File` preserves
PATH/environment and reliably runs the child. Keep the detached command itself
variable-free: pass concrete literal paths/addresses as script arguments.

```powershell
pwsh -NoProfile -ExecutionPolicy Bypass -File "C:\path\to\telex-wait-once.ps1" `
  -Telex "telex" `
  -Address "<addr>" `
  -Session "<session-id>" `
  -OutDir "C:\path\to\telex-wait-<unique>"
```

The script body can be minimal:

```powershell
param(
  [Parameter(Mandatory)] [string]$Telex,
  [Parameter(Mandatory)] [string]$Address,
  [Parameter(Mandatory)] [string]$Session,
  [Parameter(Mandatory)] [string]$OutDir,
  [string]$MinAttention
)
if ([string]::IsNullOrWhiteSpace($MinAttention)) {
  & $Telex --json --address $Address wait --session $Session --out-dir $OutDir
} else {
  & $Telex --json --address $Address wait --session $Session --min-attention $MinAttention --out-dir $OutDir
}
exit $LASTEXITCODE
```

If you see a detached completion notification but `<dir>\exit.code` is missing,
the waiter process did not actually run (or the harness failed before launching
it); do not infer a Telex idle timeout. Re-arm using the `.ps1 -File` wrapper and
inspect the task/stdout log if your runtime exposes one.

On the detached completion notification:

1. Read `<dir>\exit.code` (the completion marker); do not trust the Copilot detached task's exit code.
2. If it is `0`, parse `<dir>\message.json`. Otherwise read `<dir>\status.json` and the exit-code table.
3. Run `telex ack --address <addr> --id <message-id>` and dedupe by `message_id`.
4. Arm the next detached wait in the right mode: interrupt-only if resuming focused work, unfiltered if idle/ready for anything.

`wait` also writes `<dir>\wait.pid` at startup. If you need to tear down the
station before a message arrives, prefer `telex station stop --address <addr>`:
it releases the station and waits for the live waiter to exit. The PID file is a
diagnostic fallback only; do not hunt OS process lists unless `station stop`
reports a still-live waiter after its grace window.

Do **not** use `list_powershell` (or any task-list status) as the source of truth
for whether the waiter is armed or finished — a detached command can show as
`completed` while its child is still alive. The runtime completion notification
plus the `exit.code` artifact are the wake signal.

### Teardown and upgrade

Use `telex station stop --address <addr>` as the symmetric inverse of the
`attach` + detached-wait loop. It marks the station non-attending, releases
membership durably, and waits for tracked live waiters to exit. After it returns
with `waiters_after: 0`, a later message to the address remains queued until a
future attach/wait; it is not consumed by an orphan waiter.

For a local binary upgrade on Windows, use this order:

```sh
telex station stop --address <addr>
telex daemon stop --drain
# replace telex.exe
telex attach --address <addr> --description "<s>"
telex wait --address <addr> --out-dir <dir>
```

If the session resumes without an armed waiter, recovery is durable: inspect
`telex inbox --address <addr>` and `telex read --id <id>`, then arm a fresh
detached wait.

## Sending and finding other sessions

**Your identity — the `from` address.** Every `send`/`reply` stamps a `from` so replies can route
back to you. It resolves inside the local exchange from explicit `--from`, else
`$TELEX_ADDRESS` / the global `--address`, else the single address your
`TELEX_SESSION_ID` currently attends. If your session attends multiple addresses,
the send is refused as ambiguous until you pass `--from`.

Guardrails the binary enforces:

- **Unknown session/address returns `NeedsAttach`.** If the daemon does not know
  your `(store_key, session_id, from-address)`, the CLI re-registers when it has
  enough identity, otherwise it fails actionably.
- **Ambiguous inference is refused.** If your session attends more than one address
  and pass no `--from`/env, the send is refused rather than guessing.
- **Explicit `--from` must be attended.** A same-user process can operate under
  the v1 trust model, but the daemon still validates that the named session
  attends the explicit sender address before using it.

Use explicit `--from` when you attend multiple addresses or when the harness needs
to re-register after a daemon restart. Do not rely on a silent `from = None`.

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
telex send --to <addr> --subject "<subject>" --body-file <path>   # body from a UTF-8 file (`-` = stdin)
```

Useful send flags:

```sh
telex send --to <addr> --from <your-addr> --subject "<s>" --body "<s>" --cc <a,b> --cc <c> --kind <s> --attention interrupt|next-checkpoint|background|fyi --requires-disposition --metadata <json>
```

`--body` and `--body-file` are mutually exclusive and exactly one is required. Prefer
`--body-file` for non-trivial or multiline content — Markdown, code blocks, JSON, quoted command
output — to avoid shell quoting headaches and command-line length limits. The file is read as
UTF-8 and sent exactly as written (no trimming, so trailing newlines are preserved);
`--body-file -` reads the body from stdin.

```sh
# Recommended for multiline/structured messages:
telex send --to <addr> --subject "Status" --body-file message.md --requires-disposition
```

`send` prints a receipt: `delivered`, `queued-unoccupied`, or `rejected-retired`, plus the new message id.

A `queued-unoccupied` receipt is **durable**: the message is persisted and
delivered by a later `telex wait` once a station re-attends the address. Delivery
is at-least-once: printing a message is transport only, and the message remains
eligible until the agent runs `telex ack --id <id> --address <recipient>`. Ack is
per recipient, so acking a message for address A never consumes the same
`message_id` for cc recipient B. Terminal workflow dispositions (`handle`,
`reject`, `close`) remain the way to close the work after ack/processing.

Reply inside an existing thread:

```sh
telex reply --to-message <message-id> --body "<body>"
telex reply --to-message <message-id> --body-file <path>   # reply body from a UTF-8 file (`-` = stdin)
```

Optional reply flags are `--body-file <path>` (UTF-8 file body, `-` for stdin; mutually exclusive with `--body`, exactly one of the two required), `--from <your-addr>`, `--subject <s>`, `--cc <a,b>` / repeated `--cc <c>`, `--kind <s>`, `--attention interrupt|next-checkpoint|background|fyi`, and `--requires-disposition`. As with `send`, `--from` defaults to `$TELEX_ADDRESS` / `--address`; the reply's destination is taken from the parent message's sender (so the parent must itself have had a `from`).

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
| `--address <addr>` | Default address (or `$TELEX_ADDRESS`) for commands that act on one address; also a `from` fallback for `send`/`reply` (which otherwise default `from` to the live local station you hold). |
| `--json` / `--text` | Output format; default JSON when stdout is not a TTY, text when interactive. |

Postgres connections are configured once as named backends with `telex backend add` (see Backends), not via per-call environment variables.

## Command reference

### PRESENCE

| Command | Purpose | Key flags |
|---|---|---|
| `telex attach` | One-shot register: attach this session to the address through the local exchange, claim the epoch lease, and register directory metadata. Exits immediately. | `--address <addr>`, `--session <id>` (or `$TELEX_SESSION_ID` / `$COPILOT_AGENT_SESSION_ID`), `--description <s>`, `--scope <s>`, `--tags <a,b>`, `--watch-pid anchor:<pid>` |
| `telex detach` | One-shot detach: drop this session's in-memory membership and release epoch ownership non-destructively. | `--address <addr>`, `--session <id>` |

### RECEIVE

| Command | Purpose | Key flags |
|---|---|---|
| `telex wait` | Block on the local exchange; on delivery print one message as JSON and exit. Does not spawn a missing daemon; run `attach` first or after exit 3. Use `--min-attention interrupt` while focused. | `--address <addr>`, `--session <id>`, `--timeout-ms N`, `--min-attention <level>`, `--reconnect-grace-ms N` |
| `telex inbox` | List actionable messages requiring disposition and recent messages for the address. | `--address <addr>`, `--all`, `--limit N` |
| `telex read` | Read a message. `--thread` shows compact thread context; `--full` shows full history. | `--id <message-id>`, `--thread`, `--full` |

### SEND

| Command | Purpose | Key flags |
|---|---|---|
| `telex send` | Send through the local exchange and print a delivery/queue/reject receipt plus message id. `from` must be an attended address for the session (or unambiguous from membership). `--cc` accepts repeated flags and comma-separated values. | `--session <id>`, `--to <addr>`, `--from <addr>`, `--subject <s>`, `--body <s>`, `--body-file <path>`, `--cc <a,b>`, `--cc <c>`, `--kind <s>`, `--attention interrupt|next-checkpoint|background|fyi`, `--requires-disposition`, `--metadata <json>` |
| `telex reply` | Reply under a parent message thread through the local exchange, optionally with CC visibility recipients. | `--session <id>`, `--to-message <id>`, `--body <s>`, `--body-file <path>`, `--from <addr>`, `--subject <s>`, `--cc <a,b>`, `--cc <c>`, `--kind <s>`, `--attention interrupt|next-checkpoint|background|fyi`, `--requires-disposition` |

### DISPOSITION

| Command | Purpose | Key flags |
|---|---|---|
| `telex ack` | Explicitly mark the delivered `(message_id, recipient address)` consumed in the daemon delivery buffer. | `--address <addr>` or `--recipient <addr>`, `--session <id>`, `--id <message-id>` |
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
| `telex status` | Show the resolved backend/address projection. Use hidden `telex daemon status` for daemon internals. | `--address <addr>` |
| `telex skill` | Print these usage instructions from the embedded single source. | `--address <addr>`, `--raw` |

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

Session A attaches to a durable address and waits. `attach` is one-shot; only
`wait` blocks in the background.

```sh
export TELEX_ADDRESS=session:a   # optional: defaults A's wait/inbox to this address
export TELEX_SESSION_ID=session-a # often supplied by the agent harness
telex attach --address session:a --description "session A waiting for coordination" --scope project:telex --tags repo:telex,role:worker
```

Then A starts a **single-shot** background `telex wait`; it completes on the next delivery, which notifies A:

```sh
telex wait --address session:a
```

Session B also registers its address.

```sh
export TELEX_SESSION_ID=session-b
telex attach --address session:b --description "session B requesting status" --scope project:telex --tags repo:telex,role:requester
# B's send below stamps from=session:b automatically because B attends that address.
```

Then Session B finds A and sends a disposition-required message. One-shot commands like `resolve` and `send` run directly — no background task needed.

```sh
telex address list --scope project:telex --match "session A"
telex resolve --match "waiting for coordination" --scope project:telex
telex send --to session:a --subject "Status request" --body "Please send your current status." --attention interrupt --requires-disposition
```

A's `wait` command completes (exit 0) with the delivered message as JSON, which notifies A. A saves the JSON, **acks + dedupes by message id**, immediately re-arms a fresh background `wait` before longer processing, then handles the work, dispositions it, and replies in the same thread.

```sh
telex ack --address session:a --id <message-id-from-wait-json>
telex wait --address session:a   # start this as a fresh background wait after ack/dedupe
telex handle --id <message-id-from-wait-json> --note "status prepared"
telex reply --to-message <message-id-from-wait-json> --body "Status: holder is live; continuing work." --attention next-checkpoint
```

Session B waits, receives A's reply, and closes or handles it.

```sh
telex wait --address session:b
telex ack --address session:b --id <reply-message-id-from-wait-json>
telex handle --id <reply-message-id-from-wait-json> --note "reply received"
```
