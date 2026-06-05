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

Postgres is the reference networked backend. It supports the strongest
cross-machine behavior without a custom hosted API:

- durable messages and audit history in tables;
- indexed inbox/thread queries;
- `LISTEN/NOTIFY` for push waiters;
- advisory locks for live address leases;
- transactional message append plus recipient creation;
- Entra credentials as a first-class authentication target for Azure Database for
  PostgreSQL Flexible Server.

The key design win is connection-bound liveness. A session holding a Postgres
advisory lock serves an address. If the session, terminal, or machine dies, the
connection drops and Postgres releases the lock. The address becomes unoccupied
without a reaper daemon guessing from stale heartbeats.

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

## Leases and answerback

A lease binds a live occupant to a durable address.

```text
address: workstream:dbagent/role:orchestrator
occupant: session:abc123
host: devbox-2
principal: rob@example.com
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

## CLI design direction

Exact verbs are deferred, but the likely shape is:

```text
telex attach
telex detach
telex send
telex inbox
telex wait
telex read
telex reply
telex ack
telex handle
telex defer
telex escalate
telex close
telex address list
telex address retire
telex export
```

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

- exact CLI verb surface and output contract;
- exact message/disposition schema;
- final address grammar;
- per-address ACLs and authorization;
- lease collision and takeover beyond the safe exclusive default;
- how thread summaries are generated, stored, and refreshed;
- how much profile validation belongs in Telex vs the consuming system;
- packaging name if bare `telex` package names are occupied;
- whether Redis or another backend should be added after SQLite/Postgres;
- how exports to Markdown/Git should be shaped for audit and review.
