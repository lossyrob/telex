# Telex Design

## Status

Working design capture, kept current with the architecture. The normative
mechanism-level contracts for the local presence/transport layer (the daemon, its
IPC/attendance protocol, the lease-epoch fence, the lifecycle and recovery model) live
in [daemon.md](daemon.md); this document owns the architecture and framing and points
into daemon.md for the precise contracts. The decision trail is in
[DECISIONS.md](DECISIONS.md).

## Design center

Telex is a message fabric for AI agent sessions. Its design center is a
CLI-first, backend-pluggable protocol that lets ephemeral sessions attach to
durable addresses, exchange structured operational messages, prove liveness
through answerback, and leave an auditable record. Presence and delivery for
locally-attended addresses are supplied by an auto-spawned per-user **local
exchange** (a daemon) rather than by a resident per-session process — see
[Architecture overview](#architecture-overview) and [daemon.md](daemon.md).

The central model:

```text
durable address + ephemeral lease + structured message + disposition record
```

The durable address is the responsibility being served. The ephemeral lease is
the live session currently serving it, fenced by a monotonic lease epoch. The
message carries typed coordination text. The disposition records what happened to
the message after delivery.

## Product-language anchors

Historical telex is not just naming flavor. It provides design language that
maps directly to the system:

| Historical telex | Telex for agents | Design consequence |
|---|---|---|
| Telex number | Durable address | Address a responsibility, not a process |
| Telex exchange | Local exchange (per-user daemon) | One supervised presence/transport owner, not a per-session process |
| Answerback / WRU | Lease + attendance (the station) | Confirm identity/liveness without interrupting the agent |
| Teleprinter | CLI endpoint | Agents send, wait, read, report, and disposition |
| Switched text network | Pluggable backend | Same protocol locally and across machines |
| Store-and-forward relay | Queued delivery | Sender and recipient need not coexist |
| Printed record | Durable message history | Explainable provenance |
| Standard signaling | Thin semantic core | Shared meaning across backends |
| Operator discipline | Agent message protocol | Acknowledge, handle, defer, close, escalate |
| Expensive line time | Context budget pressure | Read newest/unseen content first; expand history on demand |

The metaphor should keep influencing names and behavior, especially around
answerback, store-and-forward, line-open receipts, and paper-trail auditability.

### Station: a registration in the local exchange

A **station** is a session's registration in the **local exchange** for a telex
address — the durable lease row plus the in-exchange attendance record that says "this
session attends this address." It is the umbrella noun for "the thing you set up to
serve an address": not the passive directory act of merely naming an address, and not a
metaphor-losing generic like "listener". A real telex **station** was the installation
that served a telex **number** through the **exchange**; here the per-user exchange (the
daemon) owns presence and transport for every locally-attended address, and a station is
the registration it holds on a session's behalf. The term still gives plain-language
invariants a clean noun — e.g. "two stations can't hold one number."

A station is **not** a resident per-session process. Earlier designs realized it as a
resident **holder** (lease + heartbeat + IPC server) plus a **waiter** loop; that
two-process, session-resident model is **superseded** by the local exchange (see
[Architecture overview](#architecture-overview) and decision 0014). The session now runs
**one-shot** verbs against the exchange; the exchange supplies the heartbeat, the IPC
endpoint, and the answerback continuously.

| Telex metaphor | Telex term | Meaning |
|---|---|---|
| Telex number | **address** | the durable responsibility |
| Telex exchange | **local exchange** | the per-user daemon owning presence + transport for all local addresses |
| Telex station | **station** | a session's registration in the exchange for an address (lease row + attendance record) |
| Holding the line | **lease** | the station's exclusive, epoch-fenced claim on the address |
| Answerback | **attendance** | the exchange-supplied proof the line is open (heartbeat + liveness) |

`attach` and `detach` remain the **lease verbs**, now **one-shot** against the exchange:
`attach` starts a station on the address (registering the lease with the exchange and
exiting); `detach` removes the station (releasing the lease). The CLI verbs are
unchanged — "station" is vocabulary and the exchange is implicit, not a new command.

## Architecture overview

Telex is a small client/library plus backend drivers, with an auto-spawned per-user
**local exchange** (a daemon) owning local presence and transport.

```text
CLI (one-shot verbs: attach / wait / detach / send / ...)
  uses
Telex core library
  owns semantic model, routing rules, state transitions, output shaping
  speaks
Local exchange (per-user daemon)        <-- presence + transport, single lease writer
  owns attendance, the durable buffer, the lease heartbeat + epoch, IPC, pid-watch
  calls
Backend driver
  owns durable writes, indexed reads, subscription, lease primitive
  maps to
SQLite / Postgres / later backends
```

There is **no required hosted Telex server** — the backend is the shared coordination
substrate and the exchange is a local, zero-config daemon (implicit, like
`rust-analyzer`/`gopls`), not a control plane. SQLite provides the local case; Postgres
provides the networked case. The exchange is a singleton per `(user SID, config root,
protocol-major)` and serves multiple stores; clients pass store identity explicitly. Its
mechanism-level contract — auto-spawn, the IPC/attendance protocol, the lease-epoch
fence, the lifecycle and crash-recovery model — is specified normatively in
[daemon.md](daemon.md).

Sessions self-attach by default. A supervising system such as Streamliner may pre-create
addresses, suggest attachment commands, or perform privileged lifecycle operations, but
messages do not flow through Streamliner and Telex does not require an orchestrator
process to assign every lease.

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
- persist delivery/disposition state.

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
- if push is unavailable, the exchange polls for undelivered messages;
- if leases are connection-bound, liveness can be exact;
- if leases are TTL/heartbeat-based, liveness is "last seen within lease
  window";
- if liveness is weak, receipts should say so honestly.

**v0 baseline (see decision 0005).** The portable v0 baseline uses
**poll** delivery and **TTL-heartbeat** liveness for *both* SQLite and
Postgres — a single code path on each axis. (Under the local exchange, decision 0017
narrows TTL-heartbeat to the *daemon-down backstop* role; live-session liveness is then
the **authoritative non-destructive hook + loader-pid (negative-only) + idle-TTL backstop**
model (decisions 0017, 0023) — see [daemon.md](daemon.md).) The exchange
polls the **undelivered set** keyed on
per-recipient delivery state rather than a monotonic id cursor (decision 0013, which superseded the
original poll-with-cursor mechanism). `LISTEN/NOTIFY` (native push) and
connection-bound advisory locks (exact liveness) are deferred to later, optional
Postgres-only upgrades behind these same capability flags, added only if a measured
need appears. The spike validated that poll + TTL is sufficient at agent-turn scale,
including live two-session messaging, so the Postgres-specific mechanisms are not v0
prerequisites.

### SQLite

SQLite is the local substrate. It should support the same semantic model but with
weaker liveness:

- messages, recipients, threads, addresses, delivery state, and dispositions live in
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

In v0, Postgres runs the same poll delivery (the undelivered-set drain of decision 0013) and
TTL-heartbeat liveness as
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

- **Occupant-declared (ephemeral):** what the current occupant says it is
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
deliberately deferred to active dispatch (see [DISPATCH.md](../../DISPATCH.md)).

Directory listings should respect the same scoping and lifecycle rules as the rest
of the address model: retired addresses drop out of normal listings, and on a
shared backend, directory visibility should be project-scoped rather than a global
enumeration of every principal's addresses.

## Leases and answerback

A lease binds a live occupant to a durable address, fenced by a monotonic epoch.

```text
address:           workstream:dbagent/role:orchestrator
occupant:          session:abc123
owner_instance_id: <the owning local exchange instance>
lease_epoch:       7
host:              devbox-2
principal:         rob@example.com
description:       dbagent orchestrator
last_confirmed:    ...
backendProof:      epoch-guarded heartbeat (single writer = the exchange)
```

The lease is Telex's answerback drum, written by the exchange as the single writer. It
gives senders two grades of confirmation:

1. **Line is open** - the address is currently attended by a live session, proven by the
   exchange's epoch-guarded heartbeat and liveness signals.
2. **Message was dispositioned** - the recipient acknowledged and handled, deferred,
   closed, rejected, or escalated the message.

The foreground agent should not answer liveness pings. The local exchange does that
continuously while the agent works.

### Lease collision and takeover

Leases are exclusive and epoch-fenced. The frozen rules (normative mechanism in
[daemon.md](daemon.md)):

- leases for role/node/session addresses are exclusive;
- `attach` fails if the address is already occupied by a live lease;
- the failure reports the current occupant and lease proof;
- a competing claim is resolved by the **lease epoch**, not by timing — the higher epoch
  wins and the loser self-demotes (see [daemon.md](daemon.md));
- shared visibility should use `cc`, `watchers`, or subscriptions, not multiple owners of
  one exclusive address.

Because loader-level liveness is a **negative-only signal** (a session can be dismissed
without its process tree dying), presence is handled **non-destructively**: the exchange
releases a station's blocked waiters and marks it **idle** on a definite signal (the
**authoritative `sessionEnd` hook** or **loader-pid** death), and a single **idle-TTL
(≥ 1 day)** backstops the rare unhooked-dismiss-with-loader-alive case. None of these ever
destroy a station or lose a message, so an idle-but-alive session stays instantly wakeable
**for days**. Identity is the **unique, stable `session_id`**; membership is **explicit-only**
(a one-off `attach`), and the exchange returns **`NeedsAttach`** for an unknown session rather
than implicitly rebuilding it — so a removed address is **never silently resurrected** (no
incarnation token, no tombstones). Delivery is durable **at-least-once + explicit agent ack +
`message_id` dedup**. The full model is normative in [daemon.md](daemon.md) §9–11, §14, and
recorded in decision 0023.

### Default `from` via daemon session ownership

`send`/`reply` are one-shot processes separate from the session that holds the lease, so
a sender needs a correct default `from`. Forcing every send to carry
`--from`/`$TELEX_ADDRESS` made un-repliable messages (`from = None`) an easy, silent
foot-gun.

The local exchange owns the authoritative `(store_key, session_id) -> addresses` map (see
[daemon.md](daemon.md), daemon-native session ownership), so `from` resolves with
precedence **`--from` > `$TELEX_ADDRESS`/`--address` > the exchange's
`ResolveFrom(store_key, session_id)`** against *that session's* registered addresses **for
that store only**: exactly one inferred succeeds, multiple refuses as `Ambiguous`, and
**none/unknown returns `NeedsAttach`** (the agent re-attaches its own address, then retries; if
still none the send **fails actionably** as `refused-unrepliable` — never a silent `from = None`,
mirroring [daemon.md](daemon.md) §14.6). The exchange **never** infers across all of its
addresses (it serves many sessions across many stores), so a multi-session, multi-store
exchange cannot misattribute a send; the harness propagates `store_key` + `TELEX_SESSION_ID`
to the `send`/`reply` process. Identity is
*defaulted, never forced*: explicit `--from`/env always win, preserving one-shot
reply-to senders, multi-address supervisors, and operator-as-system sends. A
disposition-required send that would still be un-repliable is refused outright. (This
supersedes the earlier local-holder-registry file mechanism of decision 0010, which keyed
off a resident holder that no longer exists.)

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

## Presence and delivery via the local exchange

Presence and action delivery are owned by the per-user **local exchange** (the daemon),
not by a resident per-session process. The mechanism-level contract is normative in
[daemon.md](daemon.md); this section gives the architectural shape and the reasoning.

**Telex owns long-duration waiting.** The blocking wait is a native Telex primitive
(`telex wait`), not something an agent reconstructs with generic loop skills, shell
polling, or repeated short-lived CLI invocations. This is a deliberate boundary with a
concrete technical reason: honest answerback requires a **single long-lived process** to
own the backend connection, the lease heartbeat, and the delivery buffer for the duration
of the mission. Repeated short-lived invocations open and close a connection each time and
structurally cannot keep a lease alive across turns; they would silently degrade
answerback. Previously that long-lived process was a per-session resident **holder**; it
is now the **local exchange**, shared across all of the user's sessions and addresses.

There is a real tension with how agent runtimes work: an agent can only reason about a
message once the call delivering it **returns**. Delivery to the reasoning layer requires
a process exit and a turn — the agent reads the message, acts, dispositions, and resumes
waiting. A single call cannot both block indefinitely and invoke agent turns mid-wait. If
the lease-owning process were the one that exits to deliver, the lease would release
during exactly the window when the agent is most alive — handling the message — which is
backwards.

Telex resolves this by separating the durable owner from the delivery courier across a
**process boundary that is no longer per-session**:

- the **local exchange** — long-lived, owns the backend connection(s), writes each lease's
  epoch-guarded heartbeat (single writer), drains the **undelivered set** keyed on
  per-recipient delivery state (decision 0013; `LISTEN/NOTIFY` is a later optional Postgres
  push upgrade, decision 0005), and buffers actionable messages. It never takes an agent
  turn, so it stays up across all turns and sessions. It is the literal answerback drum:
  it answers liveness automatically and continuously while agents work elsewhere.
- an **ephemeral delivery client** (`telex wait`) — a one-shot that blocks on the exchange
  over fast local IPC and **exits** the moment an actionable message is ready, handing it
  to the agent. The agent reasons, dispositions, and calls `telex wait` again.

The crucial property: the exit that hands a message to the agent happens at the *client*
layer, while the lease and its heartbeat live in the *exchange*. The agent's turn
therefore does **not** drop the backend connection or lapse the heartbeat, and the address
stays correctly `occupied` while the agent is actively handling a message. Because the
exchange outlives every session, this is strictly more robust than the old session-owned
holder: there is no per-session resident process to orphan, race on startup, or leave
attached on dismiss.

The exchange-to-client handoff follows one rule: **delivery is the exit trigger.** The
exchange never sends a separate "you should exit" signal; handing the client a message
*is* the instruction to exit. The exchange runs a daemon-scoped local IPC endpoint (a
named pipe on Windows, a unix socket elsewhere). `telex wait` connects, completes the
version handshake, sends a request describing what it is waiting for (store, address,
attention filter), then blocks on a socket read. The exchange emits a matching message
under an in-memory current-owner check and records the **durable** consumed mark only
**after** the **agent explicitly acks** it (`telex ack` — the at-least-once
`EMIT → print → agent Ack → MARK` fence; the stdout flush is transport-only, see
[daemon.md](daemon.md) §11.3); otherwise it registers the client as a waiter and stays silent.
The wakeup is push — the exchange's write releases the client's read — with no local
polling.

Delivery state and pending disposition live in the **exchange** (and ultimately the
backend), never in the ephemeral client, which is a stateless courier. A later
`telex ack`/`telex handle` is another short call that updates that state.

The client contract distinguishes outcomes by exit code: a delivered message (`0`); a
`--timeout` expiry with no message (`2`), so a supervisor can refresh without blocking
forever (agent runtimes cap tool-call duration); a daemon-gone error (`3`) **after** the
reconnect-on-EOF grace; a daemon-hung error (`4`); and **presence-ended (`5`)** when the exchange
reaps the waiter (sessionEnd hook / loader-pid death / idle-TTL — the agent re-attaches + re-waits).
Crucially, a daemon **restart or
ordered handoff is not a turn failure**: `telex wait` reconnects within a short grace
window and, on `NeedsAttach`, **explicitly re-attaches** the session from inherited environment
before it would return `3` (see [daemon.md](daemon.md)).

The agent's job is therefore to **supervise**, not to **be**, the waiter — but
supervision is now lighter, because there is no resident holder to launch and babysit. A
supervising sub-agent runs the `telex wait` delivery loop, relays actionable payloads to
the foreground, and refreshes the work-scope brief — the Plane A control role described in
[../../DISPATCH.md](../../DISPATCH.md). The exchange auto-spawns on first use; the agent
does not start it. Generic loop/skill mechanisms remain appropriate for dynamic,
agent-invented checks; they are simply not how Telex message-waiting and answerback are
implemented.

For a **push-capable harness** (e.g. a Copilot CLI session with the in-session bridge),
even that supervision is unnecessary: the agent registers a daemon **on-deliver exec** once
at attach, and a committed message is handed to the harness as a real turn without any
agent-owned `wait` loop to run or re-arm. This is a strict superset of the pull model — the
durable buffer and the agent-ack fence are unchanged, delivery stays at-least-once, and
`interrupt` still means "next turn boundary" (below) — it only removes the agent-managed
waiter as the wake path. It is distinct from the *transport* push of decision 0005 (the
exchange releasing a blocked read): here the daemon runs a harness-neutral handler that
injects the turn. Normative contract in
[daemon.md §13.2](daemon.md#132-on-deliver-push-opt-in-harness-neutral) / ADR 0039; the
`telex wait` loop below remains the harness-agnostic fallback.

The delivery loop should:

- `attach` (one-shot) to one or more addresses;
- let the exchange hold and refresh the lease;
- `wait` for actionable messages;
- wake only on messages that match the address and attention rules;
- emit a concise machine-readable payload that the foreground agent can inspect;
- avoid waking the agent for passive liveness checks;
- transparently resume across an exchange restart or handoff (reconnect-on-EOF).

If a message arrives while an agent is mid-task, the expected session protocol is not
"drop everything." The agent should inspect the summary. If it is not interrupt-grade, it
should create or remember a todo and finish the current work to a safe stopping point
before handling the message.

### A note on latency: "interrupt" means next turn

Spike measurements (see [spike/](../../spike/README.md)) decomposed end-to-end
delivery and found that the dominant term is **agent-wake latency** — the time the
host agent runtime takes to wake the foreground into a new turn after the exchange
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

- `--backend <name>`             use a configured backend by name (or `$TELEX_BACKEND`);
                                 defaults to the configured default, else an implicit
                                 `default` sqlite store
- `--db <path>`                  override the SQLite path for this invocation (or `$TELEX_DB`)
- `--address <addr>`             default address (or `$TELEX_ADDRESS`)
- `--json` / `--text`            output format; default JSON when stdout is not a TTY,
                                 text when interactive.
- Postgres connections are configured once as named backends (`telex backend add`), with a
  connection string and a password reference (`--password-env`/`--password-command`), not
  per-call environment variables. See "Backend profiles" below.

### Commands (grouped)

PRESENCE
- `telex attach --address <addr> [--description <s>] [--scope <s>] [--tags <a,b>]
     [--watch-pid <pid> ...] [--heartbeat-secs N] [--poll-secs N]`
  Become live occupant: one-shot — register a station on the address with the local
  exchange (which holds the lease and heartbeat), then exit. Exclusive: fails if the
  address is already occupied by a live lease (reports current occupant). Registers the
  directory description on attach. Auto-spawns the exchange on first use.
- `telex detach --address <addr>`  One-shot: remove the station and release the lease.

RECEIVE
- `telex wait --address <addr> [--timeout-ms N]`
  Block on the exchange; on delivery print one message as JSON and exit 0. Exit codes:
  0 delivered, 2 idle-timeout, 3 daemon-gone (after the reconnect-on-EOF grace),
  4 daemon-hung, 5 presence-ended (the exchange reaped the waiter — sessionEnd hook /
  loader-pid death / idle-TTL; the agent re-attaches + re-waits). A daemon restart/handoff is
  not a turn failure — `wait` reconnects and re-attaches on `NeedsAttach` within the grace window.
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
- `telex ack --id <id> [--address <addr>]`   acknowledged — the consume-ack carries the **delivered recipient address** (`--address`); the `wait` output's `to` field supplies it (else a flag/env), and a missing address **fails closed** rather than guessing ([daemon.md](daemon.md) §11.3)
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
- `telex init [--backend <name>] [--db <path>]`  Create `~/.telex/`, write a default
  sqlite backend, and initialize its schema.
- `telex status [--address <addr>]`  Show the resolved backend, address, exchange/IPC + occupancy.
- `telex skill [--address <addr>] [--raw]`  Print the agent usage instructions embedded in
  the binary (the `SKILL.md` for this build), optionally tailored to an assigned address.

BACKENDS (named profiles in `~/.telex/config.toml`)
- `telex backend add <name> --sqlite [--path <p>]`
- `telex backend add <name> --postgres <conn-string> [--schema <s>] [--password-env <VAR>] [--password-command <cmd>] [--default]`
- `telex backend list | show <name> | default <name> | remove <name> | kinds`

A **backend** is a named, configured store; the driver **kind** (sqlite/postgres) is set at
`add` time and reflects the build's compiled-in features (`telex backend kinds`). Selection
is by name: `--backend <name>` → `$TELEX_BACKEND` → the config `default` pointer → an implicit
`default` sqlite store, so a fresh machine works with zero setup. The first backend added
becomes the default; name and default pointer are orthogonal, so reassigning the default
never requires renaming. Postgres backends store a connection string (libpq URI or key=value
DSN) plus a password reference; secrets are not written to the config file. This is the
storage axis of the modular-backends model (see DECISIONS 0008); auth (`--entra`, AWS/GCP
IAM) is the second axis, layered behind features.

Two v0 details are settled: `attach` is the one-shot presence verb registering a station
with the local exchange — there is no separate `serve` verb, and there is no resident
per-session holder (the exchange owns presence; see decision 0014). Disposition verbs are
flat (`telex ack`, `telex handle`, ...), not nested under a `disp` parent. A hidden
`telex daemon` entrypoint runs the exchange (auto-spawned; not shown in normal help).

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
separately in [DISPATCH.md](../../DISPATCH.md), which carries its own open questions.
