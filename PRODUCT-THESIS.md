# Telex Product Thesis

## Thesis

**Telex makes agent sessions addressable without making them permanent.**

AI agent sessions can reason, write, run tools, and execute work, but they are
ephemeral, context-limited, and isolated. The session doing work is rarely the
durable thing that other agents should address. The durable thing is usually a
responsibility: a role, node, workstream, question, checkpoint, review lane, or
other mission boundary.

Telex is a message fabric for that gap. It gives agents a disciplined way to
address durable responsibilities, deliver structured messages, confirm liveness
through infrastructure, preserve an auditable record, and disposition messages
without turning sessions into permanent teammates or replacing the artifacts of
record.

In short: **Telex is a modern answerback network for AI agents.**

## Problem

Agent coordination today often looks like pre-telex relay work:

1. Ask one session to write a message file.
2. Commit or copy the file somewhere.
3. Move to another machine or terminal.
4. Ask the target session to read it.
5. Manually carry the reply or status update back.

This works only because a human stands in the relay center tearing tape by hand.
Git commits, scratch notes, and chat transcripts become an improvised runtime
coordination layer even though they were not designed for liveness, delivery
receipts, address resolution, actionable inboxes, or disposition tracking.

The missing capability is not more model intelligence. It is coordination
infrastructure:

- sessions need to message responsibilities instead of fragile process IDs;
- recipients may be offline, busy, not yet started, or on another machine;
- senders need honest receipts when an address is unoccupied, retired, or served;
- messages need disposition, not just delivery;
- agents need to explain later why they acted;
- runtime coordination must not pollute or replace project artifacts.

## Product promise

Telex provides a small, durable, addressable message fabric for agent sessions.
It carries coordination messages, field reports, questions, handoffs, blockers,
evidence, map corrections, and disposition-required decisions across local and
networked execution environments.

It is not casual chat. It is operational messaging for agents that are doing
work.

## Who benefits

Telex is for developers and agent systems that run multiple AI sessions and need
those sessions to coordinate without constant human relay.

The first reference customer is Streamliner-style work geometry:

- a workstream orchestrator needs to receive worker field reports;
- a node worker discovers that the map is wrong and sends a split request;
- an adjacent workstream exports a checkpoint that unblocks another worker;
- a session on one machine needs to send a scoped message to a role on another;
- the developer later asks why a session made a decision and expects a message
  of record.

The tool should remain general enough for two arbitrary terminal sessions to
talk through a local SQLite store, while being strong enough for Streamliner to
use as its message-passing substrate.

## Core invariants

### Address responsibilities, not processes

An address names a durable responsibility. A lease names the ephemeral session
currently serving that address. Messages target the address, not the terminal,
host, process, or transcript that happens to occupy it.

### Delivery is not disposition

A delivered message has reached an address or queue. A dispositioned message has
been acknowledged, handled, closed, deferred, rejected, or escalated by the
responsible recipient. Telex must preserve that distinction.

### Answerback is infrastructure-supplied liveness

A working agent should not be interrupted by "are you there?" pings. An auto-spawned,
per-user **local exchange** — a small background daemon that owns presence and delivery
for the user's locally-attended addresses — supplies answerback automatically by holding
each address lease and proving whether the line is open, so the agent never has to.

### Store and forward is required

The sender and recipient do not need to exist at the same time. Messages can be
accepted, queued, delivered later, and reported honestly as delivered, queued,
bounced, or rejected.

### Context budget matters

Agents should see the newest actionable message first. Thread history is
expandable, not automatically replayed. The protocol should support unread-only
reads, compact summaries, and explicit expansion.

### Messages coordinate work; artifacts remain authoritative

Telex carries operational text between agents. It does not replace source
repositories, workstream graphs, design docs, trackers, PRs, or other durable
artifacts. Messages may request or report changes to those artifacts, but the
owning system applies and records those changes.

### Backends vary in fidelity; semantics stay stable

SQLite and Postgres can offer different liveness and push guarantees, but the
client-facing semantic contract should stay stable. Backends implement
capabilities; they do not redefine what a message, lease, address, receipt, or
disposition means.

## Product wedge

The initial wedge is a CLI-first utility:

- no required hosted control plane;
- no mandatory MCP server;
- no chat UI dependency;
- an auto-spawned, zero-config **local exchange** for presence and delivery — implicit
  and unmanaged, like `rust-analyzer` or `gopls`, not a server the user runs;
- local SQLite for simple use and tests;
- Postgres for networked, cross-machine use;
- Entra credentials as a first-class Postgres authentication target;
- a thin semantic core implemented in the client/library, not hidden in one
  backend's triggers;
- auditability through durable message records and exportable history.

The command users type should be `telex`, even if packaging later requires a
scoped or qualified package name.

## Reference profile: Streamliner

Streamliner should be treated as Telex's reference profile, not as a dependency.

Streamliner needs Telex to support:

- durable addresses for projects, workstreams, nodes, checkpoints, roles, and
  sessions;
- ephemeral leases for launched, resumed, manual, and ad-hoc sessions;
- actionable inboxes for orchestrators and workers;
- field reports carrying outcome, evidence, blockers, downstream impacts, map
  corrections, and escalation conditions;
- split requests, scope-change requests, decision requests, closeout
  observations, checkpoint exports, and reconciliation facts;
- address lifecycle states so closed workstreams and obsolete nodes disappear
  from normal address resolution;
- loop-compatible waiters that wake on actionable work without interrupting
  foreground reasoning for passive liveness checks.

Streamliner remains the owner of workstream artifacts and reconciliation
decisions. Telex is the message fabric that lets sessions coordinate around that
work geometry.

## Multi-user direction

Telex should not assume that all sessions belong to one human. In the networked
case, sessions are addressable whether they are Rob's sessions, another
developer's sessions, or automation, as long as they share access to the backend
and are authorized to claim or message the relevant addresses.

V0 can be trust-on-backend-access. Postgres with Entra credentials gives a path
to stronger identity and authorization later by recording principals on leases,
messages, dispositions, and privileged operations.

## Non-goals

Telex is not:

- a general chat app;
- an anthropomorphic agent-team simulator;
- a replacement for durable project artifacts;
- a required hosted service;
- a workflow engine that mutates downstream systems directly;
- a universal actor framework;
- a transcript synchronization system;
- a guarantee that every backend can provide identical real-time liveness.

## Success criteria

Telex is successful when:

- an agent can attach to a durable address and receive only actionable messages;
- another agent can send to that address without knowing which session currently
  occupies it;
- the sender gets an honest receipt when the address is live, unoccupied,
  retired, or unknown;
- a waiting loop can provide answerback without disturbing the foreground agent;
- messages can be acknowledged, handled, deferred, closed, or escalated with a
  durable record;
- a later agent can inspect the record and explain why a decision or action
  happened;
- the same semantic model works locally on SQLite and across machines on
  Postgres;
- Streamliner can use Telex for work-geometry coordination without making Telex
  Streamliner-specific.

