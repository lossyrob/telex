# Telex Design

## Status

Working design capture. This document intentionally preserves more design
thinking than a final design spec should. It will be whittled later and may feed
future design documentation, protocol specs, and decision records.

## Design center

Telex is a message fabric for AI agent sessions. Its design center is a
CLI-first, backend-pluggable protocol that lets ephemeral sessions attach to
durable addresses, exchange structured operational messages, prove liveness
through answerback, and leave an auditable record.

The central model:

```text
durable address + ephemeral lease + structured message + disposition record
```

The durable address is the responsibility being served. The ephemeral lease is
the live session or loop currently serving it. The message carries typed
coordination text. The disposition records what happened to the message after
delivery.

## Product-language anchors

Historical telex is not just naming flavor. It provides design language that
maps directly to the system:

| Historical telex | Telex for agents | Design consequence |
|---|---|---|
| Telex number | Durable address | Address a responsibility, not a process |
| Answerback / WRU | Lease + waiter loop | Confirm identity/liveness without interrupting the agent |
| Teleprinter | CLI endpoint | Agents send, wait, read, report, and disposition |
| Switched text network | Pluggable backend | Same protocol locally and across machines |
| Store-and-forward relay | Queued delivery | Sender and recipient need not coexist |
| Printed record | Durable message history | Explainable provenance |
| Standard signaling | Thin semantic core | Shared meaning across backends |
| Operator discipline | Agent message protocol | Acknowledge, handle, defer, close, escalate |
| Expensive line time | Context budget pressure | Read newest/unseen content first; expand history on demand |

The metaphor should keep influencing names and behavior, especially around
answerback, store-and-forward, line-open receipts, and paper-trail auditability.

## Architecture overview

Telex should be built as a small client/library plus backend drivers.

```text
CLI
  uses
Telex core library
  owns semantic model, routing rules, state transitions, output shaping
  calls
Backend driver
  owns durable writes, indexed reads, subscription, lease primitive
  maps to
SQLite / Postgres / later backends
```

There should not be a required Telex server in the first architecture. The
backend is the shared coordination substrate. SQLite provides the local case.
Postgres provides the networked case.

Sessions should be able to self-attach by default. A supervising system such as
Streamliner may pre-create addresses, suggest attachment commands, or perform
privileged lifecycle operations, but messages do not flow through Streamliner and
Telex does not require an orchestrator process to assign every lease.

## Thin semantic core

The thin semantic core belongs in the Telex client/library, not in Postgres
triggers or backend-specific behavior.

The core owns:

- message model;
- address model;
- lease model;
- receipt model;
- per-recipient delivery state;
- disposition state;
- attention rules;
- actionable inbox rules;
- thread summarization/read-shaping rules;
- validation of generic message fields;
- backend capability adaptation.

Backends provide primitives:

- append;
- query;
- transaction;
- subscribe or poll;
- claim/release lease;
- resolve address;
- persist cursor/disposition state.

Backend features may enforce mechanical invariants, but should not define the
product semantics. Postgres can use constraints, indexes, transactions,
`LISTEN/NOTIFY`, and advisory locks. It should not hide routing or disposition
policy inside trigger-heavy business logic that SQLite cannot reproduce.

## Backend strategy

### Initial backends

Telex should scope initial reference backends to:

- **SQLite** for local use, two-session workflows, tests, and low-friction
  adoption.
- **Postgres** for networked, cross-machine, multi-user-capable workflows.

Redis may be useful later. RabbitMQ should not be an initial target because it is
primarily a queue, not a durable threaded record plus lease/address store.

### Capability model

Backends should advertise capabilities rather than pretending they all offer the
same fidelity:

```ts
type BackendCapabilities = {
  durable: boolean;
  push: "native" | "poll";
  lease: "connection" | "ttl" | "advisory";
  transactions: "strong" | "best-effort";
  query: "sql" | "indexed";
};
```

The core adapts behavior:

- if push is native, `wait` subscribes;
- if push is unavailable, `wait` polls from a cursor;
- if leases are connection-bound, liveness can be exact;
- if leases are TTL/heartbeat-based, liveness is "last seen within lease
  window";
- if liveness is weak, receipts should say so honestly.

**v0 baseline (see decision 0005).** The portable v0 baseline uses
**poll-with-cursor** delivery and **TTL-heartbeat** liveness for *both* SQLite and
Postgres — a single code path on each axis. `LISTEN/NOTIFY` (native push) and
connection-bound advisory locks (exact liveness) are deferred to later, optional
Postgres-only upgrades behind these same capability flags, added only if a measured
need appears. The spike validated that poll + TTL is sufficient at agent-turn scale,
including live two-session messaging, so the Postgres-specific mechanisms are not v0
prerequisites.

### SQLite

SQLite is the local substrate. It should support the same semantic model but with
weaker liveness:

- messages, recipients, threads, addresses, cursors, and dispositions live in
  tables;
- waiting uses polling or file-change observation;
- leases use heartbeat rows and TTL windows;
- answerback means "last seen within lease window";
- it should be good enough for two local sessions and deterministic tests.

### Postgres

Postgres is the reference networked backend. It supports strong cross-machine
behavior without a custom hosted API:

- durable messages and audit history in tables;
- indexed inbox/thread queries;
- transactional message append plus recipient creation;
- Entra credentials as a first-class authentication target for Azure Database for
  PostgreSQL Flexible Server.

In v0, Postgres runs the same poll-with-cursor delivery and TTL-heartbeat liveness as
SQLite (decision 0005); it earns its place through durability, indexed queries, and
networked multi-machine access rather than through push or connection-bound liveness.

A later, optional upgrade can raise Postgres's fidelity: `LISTEN/NOTIFY` for push
waiters and session-scoped advisory locks for connection-bound liveness. The
potential design win there is that a session holding a Postgres advisory lock serves
an address, and if the session, terminal, or machine dies, the connection drops and
Postgres releases the lock — the address becomes unoccupied without a reaper daemon
guessing from stale heartbeats. This is a future enhancement, not a v0 requirement,
and v0 must remain correct without it.

### Entra authentication

For the author's expected networked setup, Postgres + Entra should be a
first-class path. This avoids shared database passwords and preserves principal
identity for future authorization.

V0 can treat access to the backend as trust. The schema should still record
principal metadata on:

- lease acquisition;
- message send;
- disposition;
- privileged actions such as takeover, retirement, or address management.

That preserves a path toward ACLs without requiring them immediately.

## Address model

An address names something durable enough to receive a message. A session is only
one possible occupant of an address.

Useful generic address categories:

- `session:<id>`;
- `role:<name>`;
- `user:<principal>`;
- `project:<id>` or another profile-defined project/scope handle;
- profile-specific qualified addresses.

For Streamliner-style work geometry, expected addresses include:

```text
project:<project-id>
workstream:<workstream-id>
workstream:<workstream-id>/role:orchestrator
workstream:<workstream-id>/node:<node-id>
workstream:<workstream-id>/checkpoint:<checkpoint-id>
session:<session-id>
user:<principal-id>
```

Address grammar can be finalized later. The important design rule is that the
address should express the responsibility being served, not the process currently
holding it.

## Address lifecycle

Addresses should have lifecycle state:

- `active` - appears in address resolution and can receive normal delivery;
- `retired` - address no longer appears in normal address lists and sends are
  rejected or forwarded according to policy.

Occupancy is a separate runtime dimension:

- `occupied` - a live lease currently serves the address;
- `unoccupied` - no live lease currently serves the address.

An `active` address can be occupied or unoccupied. Unoccupied active addresses
may queue messages according to policy. A `retired` address is administratively
closed regardless of whether some stale session still believes it serves it.

Retirement matters because old workstreams and nodes should not stay casually
messageable. A closed Streamliner workstream from last week should remain in
history but disappear from normal address completion and default resolution.

## Address directory and registration

Addressing only helps if a sender can find the address. The first and simplest
form of discovery is a **self-registered directory**: when a session attaches it
registers a short, human- and agent-readable description of what it is doing, and
that description travels with the address in directory listings.

This is a V0 concern and deliberately small. It needs no broadcast, no semantic
search, and no capability negotiation — only that an occupant describe itself on
attach and that listings return those descriptions. It is enough for a sender to
think "the session working on issue 215," scan the directory, and resolve the
target itself.

On attach a session should be able to declare:

- a one-line `description` — "session working on issue 215", "auth DB migration
  worker";
- optional `tags` — coarse labels such as `issue:215`, `repo:telex`, `db`;
- optional `scope` — the project or workstream the address belongs to.

Two layers of description exist, and V0 only requires the first:

- **Occupant-declared (ephemeral):** what the current lease holder says it is
  doing. It travels with the lease and changes when the occupant changes. This
  alone satisfies "find the session working on issue 215."
- **Address-declared (durable):** a stable description of the responsibility
  itself, independent of any occupant. Useful when a supervisor such as
  Streamliner pre-creates addresses from work geometry. Optional in V0.

`telex address list` should therefore return more than bare addresses. For each
active address it should show the description, occupancy, and liveness grade, so an
agent can scan the directory and resolve a target by reading descriptions:

```text
$ telex address list --scope project:telex
ADDRESS                                OCCUPANCY    DESCRIPTION
workstream:telex/node:issue-215        occupied     session working on issue 215
workstream:telex/role:orchestrator     occupied     telex orchestrator
workstream:telex/node:directory        unoccupied   passive directory design (queued)
```

A simple substring or tag filter (for example `telex address list --match 215` or
`--tag issue:215`) is enough for V0 resolution. Anything richer — natural-language
matching, broadcast "who can handle this?" enquiries, or capability bidding — is
deliberately deferred to active dispatch (see [DISPATCH.md](DISPATCH.md)).

Directory listings should respect the same scoping and lifecycle rules as the rest
of the address model: retired addresses drop out of normal listings, and on a
shared backend, directory visibility should be project-scoped rather than a global
enumeration of every principal's addresses.

## Leases and answerback

A lease binds a live occupant to a durable address.

```text
address: workstream:dbagent/role:orchestrator
occupant: session:abc123
host: devbox-2
principal: rob@example.com
description: dbagent orchestrator
since: ...
backendProof: heartbeat | advisory-lock | connection
```

The lease is Telex's answerback drum. It gives senders two grades of confirmation:

1. **Line is open** - the address is currently served by a live loop or
   connection.
2. **Message was dispositioned** - the recipient acknowledged and handled,
   deferred, closed, rejected, or escalated the message.

The foreground agent should not answer liveness pings. The background waiter or
backend connection does that.

### Lease collision and takeover

Detailed takeover policy can be deferred. The safe default should be:

- leases for role/node/session addresses are exclusive;
- attach fails if the address is already occupied;
- the failure reports the current occupant and lease proof;
- takeover is an explicit privileged operation;
- shared visibility should use `cc`, `watchers`, or subscriptions, not multiple
  owners of one exclusive address.

This default is safe enough to build while leaving room for supervisor authority,
stale takeover, worker pools, and delegation later.

## Messaging model

A Telex message is a typed operational record. It is not a transcript dump and
not a chat utterance.

Expected generic fields:

```yaml
id:
threadId:
parentId:
from:
to:
cc:
watchers:
kind:
attention:
requiresResponse:
requiresDisposition:
subject:
summary:
body:
metadata:
createdAt:
```

The exact schema is deferred, but the core should preserve these concepts:

- unique message IDs;
- single or multi-address delivery;
- per-recipient delivery and non-delivery reports;
- traceability after failures;
- answerback comparison or equivalent lease proof;
- threading without automatic full-thread replay;
- disposition as a first-class object.

ITU-T Rec. F.72, "International Telex Store and Forward," should be mined before
finalizing the message/disposition schema. It appears to specify many of these
requirements directly in historical telex terms.

## Delivery, attention, and disposition

Telex must separate delivery from attention.

Recipient roles:

- `to` - expected responder or accountable recipient;
- `cc` - visible recipient, not interrupting by default;
- `watchers` - passive subscribers or audit observers.

Attention levels:

- `interrupt` - wake as soon as possible;
- `next-checkpoint` - handle after current safe stopping point;
- `background` - visible in inbox but should not derail current work;
- `fyi` - visible/auditable, non-actionable by default.

Disposition states should include at least:

- `acknowledged`;
- `handled`;
- `deferred`;
- `rejected`;
- `closed`;
- `escalated`.

Delivery states are a separate axis and should include at least:

- `queued`;
- `delivered`;
- `seen`;
- `bounced`;
- `dead-letter` or equivalent derived stuck state.

Dead-letter should be a derived state, not a required daemon. If a message cannot
be delivered because a `session:<id>` target is gone, the sender should get a
bounce. If a durable role is unoccupied, the message can queue with an honest
receipt. If the address is retired, the send should be rejected or forwarded
according to the address policy.

## Threading and context budget

Threads should be navigable objects, not default read payloads.

Default agent reads should show:

- the new message;
- its subject/summary;
- sender and target;
- required disposition;
- compact thread context;
- commands or handles to expand history if needed.

Agents should be able to request:

- unread events only;
- latest message only;
- last N messages;
- compact thread summary;
- full thread history;
- message of record by ID.

This prevents long threads from repeatedly consuming context while preserving
auditability and explainability.

## Waiter loop behavior

The waiter loop is the current concrete mechanism for answerback and action
delivery.

**Telex owns long-duration waiting.** The blocking wait is a native Telex primitive
(`telex wait`), not something an agent reconstructs with generic loop skills, shell
polling, or repeated short-lived CLI invocations. This is a deliberate boundary with
a concrete technical reason, not just a consistency preference.

The strongest answerback grade — a connection-bound lease that releases the instant a
session dies — requires a single long-lived process to hold the backend connection
(and, on Postgres, the advisory lock) for the duration of the mission. A pattern of
repeated short-lived invocations (`check`, sleep, `check`) opens and closes a
connection each time and structurally cannot hold a connection-bound lease; it would
silently degrade answerback to the weaker heartbeat/TTL grade. So the long wait must
live inside one durable Telex process that holds the lease and blocks efficiently
(poll-with-cursor in the v0 baseline; `LISTEN/NOTIFY` is a later optional Postgres
push upgrade — see decision 0005).

This creates a real tension with how agent runtimes work: an agent can only reason
about a message once the call delivering it **returns**. Delivery to the reasoning
layer therefore requires a process exit and a turn — the agent reads the message,
acts, dispositions, and resumes waiting. A single call cannot both block indefinitely
and invoke agent turns mid-wait. If the lease-holding process were the one that exits
to deliver, the lease would release during exactly the window when the agent is most
alive — handling the message — which is backwards.

Telex resolves this by splitting the waiter into **two processes**:

- a **resident holder** — long-lived, holds the backend connection and writes the
  lease's TTL heartbeat, polls for actionable messages from a cursor, and buffers them
  locally (on the optional Postgres upgrade it can instead hold an advisory lock and
  run `LISTEN/NOTIFY`). It never needs to take an agent turn, so it can stay up for the
  whole mission. This is the literal answerback drum: it answers liveness
  automatically and continuously while the agent works elsewhere.
- an **ephemeral delivery client** (`telex wait`) — blocks on the resident holder
  over fast local IPC, and **exits** the moment an actionable message is ready,
  handing it to the agent. The agent reasons, dispositions, and calls `telex wait`
  again.

The crucial property: the exit that hands a message to the agent happens at the
*client* layer, while the lease (and its heartbeat) lives in the *holder*. The agent's
turn therefore does **not** drop the backend connection or lapse the heartbeat, and
the address stays correctly `occupied` while the agent is actively handling a message
— exactly when a naive single-process waiter would falsely report the line dead. This
also preserves the familiar exit-with-info-then-restart cadence of background-task
loops, but only
the cheap local delivery client exits per turn; the durable backend connection is
never disturbed.

The holder-to-client handoff follows one rule: **delivery is the exit trigger.** The
holder never sends a separate "you should exit" signal; handing the client a message
*is* the instruction to exit. Concretely, the holder runs a small local IPC endpoint
(a named pipe on Windows, a unix socket elsewhere) and acts as a local server.
`telex wait` connects, sends a request describing what it is waiting for
(address(es), attention filter, since-cursor), then blocks on a socket read. The
holder replies immediately if a matching message is already buffered; otherwise it
registers the client as a waiter and stays silent, leaving the read blocked. When an
actionable message arrives, the holder writes the framed payload to the waiting
client's socket; the read returns, the client prints the concise payload to stdout,
and exits. The wakeup is push — the holder's write releases the client's read — with
no local polling.

Delivery state, cursor position, and pending disposition live in the **holder** (and
ultimately the backend), never in the ephemeral client, which is a stateless courier.
A later `telex ack`/`telex handle` is another short call that updates that state.

The client contract distinguishes outcomes by exit code: a delivered message; a
`--timeout` expiry with no message, so a supervisor can refresh and re-issue without
blocking forever (agent runtimes cap tool-call duration); and a holder-gone error,
signalling the supervisor to restart and reconnect the holder.

For the spike, the local socket server can be skipped in favour of a local SQLite (or
file) buffer that the holder writes and `telex wait` blocks on via a short local poll
or file-change watch — enough to prove the two-process liveness property before
adding push IPC.

Because liveness is bound to the holder, the holder's lifecycle must track the
session's. It should be a session-owned process, **not** a fully detached daemon, so
that when the session, terminal, or machine dies, the holder dies with it and the
lease releases promptly. A fully detached holder would outlive a dead session and lie
about liveness — worse than no guarantee.

The agent's job is therefore to **supervise**, not to **be**, the waiter. A
supervising sub-agent launches and monitors the resident holder (restarting and
reconnecting it on failure), runs the `telex wait` delivery loop, relays actionable
payloads to the foreground, and refreshes the work-scope brief — the Plane A control
role described in [DISPATCH.md](DISPATCH.md). Generic loop/skill mechanisms remain
appropriate for dynamic, agent-invented checks; they are simply not how Telex
message-waiting and answerback are implemented.

The loop should:

- attach to one or more addresses;
- hold or refresh the lease;
- subscribe or poll for actionable messages;
- wake only on messages that match the address, attention, and cursor rules;
- emit a concise machine-readable payload that the foreground agent can inspect;
- avoid waking the agent for passive liveness checks;
- resume from a cursor after reconnects or process restarts.

If a message arrives while an agent is mid-task, the expected session protocol is
not "drop everything." The agent should inspect the summary. If it is not
interrupt-grade, it should create or remember a todo and finish the current work
to a safe stopping point before handling the message.

### A note on latency: "interrupt" means next turn

Spike measurements (see [spike/](spike/README.md)) decomposed end-to-end
delivery and found that the dominant term is **agent-wake latency** — the time the
host agent runtime takes to wake the foreground into a new turn after the waiter
delivers — at roughly **6–26 seconds**, one to two orders of magnitude larger than
Telex's own delivery (sub-second). No agent runtime today can preempt a foreground
agent mid-turn. Telex's `interrupt` attention level therefore means "deliver at the
next turn boundary," not "stop the running model now." Backend transport choices
(poll vs push, a shorter poll interval) move the sub-second term and matter for
machine-to-machine dispatch, but they do not change the perceived agent-to-agent
latency, which is governed by the runtime. The design and any receipts should be
honest about this: answerback proves the line is open and the message delivered, not
that the occupant has yet been woken to act.

## CLI design direction

The v0 command surface is settled:

### Global options (apply to all subcommands)

- `--backend <sqlite|postgres>`  default `sqlite` (or `$TELEX_BACKEND`)
- `--db <path>`                  SQLite file, default `~/.telex/telex.db` (or `$TELEX_DB`)
- `--address <addr>`             default address (or `$TELEX_ADDRESS`)
- `--json` / `--text`            output format; default JSON when stdout is not a TTY,
                                 text when interactive.
- Postgres connection via env (same as spike): `TELEX_PG_HOST`, `TELEX_PG_USER`,
  `TELEX_PG_DB`, `TELEX_PG_PASSWORD` (Entra access token OR SQL password).

### Commands (grouped)

PRESENCE
- `telex attach --address <addr> [--description <s>] [--scope <s>] [--tags <a,b>]
     [--heartbeat-secs N] [--poll-secs N]`
  Become live occupant; hold lease; run holder; blocks. Exclusive: fails if the address
  is already occupied by a live lease (reports current occupant). Registers the directory
  description on attach.
- `telex detach --address <addr>`  Release the lease (and stop a running holder).

RECEIVE
- `telex wait --address <addr> [--timeout-ms N]`
  Block on the holder; on delivery print one message as JSON and exit 0. Exit codes:
  0 delivered, 2 idle-timeout, 3 holder-gone, 4 holder-hung.
- `telex inbox [--address <addr>] [--all] [--limit N]`
  List actionable (requires-disposition, not yet terminally dispositioned) and recent
  messages for the address.
- `telex read --id <message-id> [--thread] [--full]`
  Read a message; `--thread` shows compact thread context; `--full` full history.

SEND
- `telex send --to <addr> [--subject <s>] --body <s> [--cc <a,b>] [--kind <s>]
     [--attention interrupt|next-checkpoint|background|fyi] [--requires-disposition]
     [--metadata <json>]`
  Send a message. Prints a receipt stating delivered/queued-unoccupied/rejected-retired
  plus the new message id.
- `telex reply --to-message <id> --body <s> [--attention ...] [--requires-disposition]`
  Reply; threads under the parent (inherits thread_id, sets parent_id).

DISPOSITION (flat verbs; all take `--id <message-id>` and optional `--note <s>`)
- `telex ack --id <id>`        acknowledged
- `telex handle --id <id>`     handled
- `telex defer --id <id>`      deferred
- `telex reject --id <id>`     rejected
- `telex close --id <id>`      closed
- `telex escalate --id <id>`   escalated

DIRECTORY
- `telex address list [--scope <s>] [--match <substr>] [--tag <t>] [--all]`
  Show addresses with description, occupancy, liveness grade.
- `telex address show --address <addr>`  Detail for one address + lease/occupancy.
- `telex address retire --address <addr>`  Retire (drops from normal listings).
- `telex resolve --match <substr> | --tag <t> [--scope <s>]`
  Resolve target(s) by description/tag; prints matching address(es) + descriptions.

AUDIT
- `telex export [--address <addr>] [--thread <id>] [--since <id>]`
  Emit messages + disposition history as JSON lines (jsonl) for audit/provenance.

SETUP
- `telex init [--backend ...] [--db ...]`  Create `~/.telex/` and initialize schema.
- `telex status [--address <addr>]`  Show config, backend, address, holder/IPC + occupancy.

Two v0 details are settled: `attach` blocks as the resident holder — there is no
separate `serve` verb; the holder IS `attach`. Disposition verbs are flat
(`telex ack`, `telex handle`, ...), not nested under a `disp` parent.

The CLI should optimize for agents:

- structured JSON output where useful;
- concise default text output;
- stable IDs/cursors;
- commands that can be embedded in loop waiters;
- receipts that honestly say delivered, queued-unoccupied, bounced, or
  rejected-retired.

## Streamliner reference profile

Telex should be Streamliner-informed but Streamliner-independent.

Streamliner needs:

- work-geometry addresses;
- actor/session attachment;
- field reports;
- blockers;
- split requests;
- scope-change requests;
- decision requests;
- closeout observations;
- checkpoint exports;
- reconciliation facts;
- cross-workstream coordination;
- address retirement when workstreams or nodes close;
- optional privileged supervision by Streamliner runtime code.

Telex should not mutate Streamliner artifacts directly. A Streamliner message can
carry profile metadata:

```yaml
profile: streamliner
kind: split-request
to: workstream:dbagent/role:orchestrator
requiresDisposition: true
attention: next-checkpoint
metadata:
  workstreamId: dbagent
  nodeId: currentvalue-baseline
  proposedMutation:
    type: split-node
    rationale: mixed-protocol-risk
  evidence:
    - issue: 486
    - file: path/to/artifact
```

The Streamliner orchestrator, reconciler, or operator decides whether that
message becomes graph, brief, tracker, or design-doc change.

## Session actor control plane relationship

Streamliner's Session Actor Control Plane candidate (captured in Streamliner PR
#103) validates Telex's direction but should not anchor Telex's scope.

That candidate asks for:

- selected sessions as ephemeral, role-bound runtime actors;
- structured messages;
- field reports;
- heartbeat/watch states;
- parent/child session relationships;
- loop-compatible message handling;
- actor identities attached to work geometry.

Telex provides the message fabric that such a candidate could consume. It should
not require Streamliner to implement a full actor model before it becomes useful.
Streamliner dogfood and Postgres/Entra backend work may be separate or parallel
workstreams later.

## Multi-user coordination

Telex should work when multiple people share a backend. A session is addressed
through its claimed responsibility, whether it belongs to one developer, another
developer, or automation.

V0 can trust backend access. The design should still keep principal metadata on
all significant records so future authorization can answer:

- who may claim this address?
- who may send to this address?
- who may retire or rehome this address?
- who may force takeover?
- who dispositioned this message?

Postgres with Entra is the likely first serious path here.

## Prior art and research notes

### ITU-T F.72

ITU-T Rec. F.72, "International Telex Store and Forward," is worth direct
research before protocol finalization. Early notes suggest it specifies:

- unique message IDs;
- single and multi-address delivery;
- delivery and non-delivery reports per address;
- traceability after failures;
- answerback comparison as a secure-delivery check.

Those concepts strongly align with Telex's intended message and disposition
model.

### MCP Agent Mail

`Dicklesworthstone/mcp_agent_mail` appears to be the closest known prior art:
an MCP/FastMCP server that gives coding agents identities, inboxes, searchable
threads, SQLite FTS, acknowledgements, advisory file leases, and Git-backed
Markdown.

Initial verdict: mine it for ideas, but do not depend on it. It appears
server-bound and not structurally aligned with Telex's wedge: CLI-first, no
required server, durable-address plus lease-backed answerback, and disposition
as a first-class object.

## Open design questions

Deferred questions:

- output contract details beyond the frozen v0 verb surface;
- exact message/disposition schema;
- final address grammar;
- per-address ACLs and authorization;
- lease collision and takeover beyond the safe exclusive default;
- how thread summaries are generated, stored, and refreshed;
- how much profile validation belongs in Telex vs the consuming system;
- packaging name if bare `telex` package names are occupied;
- whether Redis or another backend should be added after SQLite/Postgres;
- how exports to Markdown/Git should be shaped for audit and review;
- directory resolution fidelity for V0 (substring/tag) and when richer matching is
  justified;
- whether address descriptions live on the address, the lease, or both, and how
  the two stay in sync;
- directory visibility and scoping on a shared backend.

Active discovery, broadcast enquiries, and Contract-Net dispatch are explored
separately in [DISPATCH.md](DISPATCH.md), which carries its own open questions.
