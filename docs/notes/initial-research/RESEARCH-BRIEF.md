# Telex — Research Brief for Prior-Art Discovery

## What Telex is

Telex is a **message fabric for AI agent sessions**: a small, durable, addressable
way for autonomous (or semi-autonomous) AI coding/agent sessions to send each
other coordination messages — questions, handoffs, field reports, blockers,
evidence, and decisions that require disposition — across local and networked
execution environments.

It is delivered primarily as a **CLI utility** that agent sessions invoke, backed
by a pluggable datastore. It is explicitly **not** a chat app, not a human
team-messaging product, and not a general multi-agent orchestration framework. It
is the thin coordination layer *between* sessions, while the durable source of
truth (code repos, design docs, work plans, issue trackers) lives elsewhere.

The name comes from the historical **telex** network — the first widely adopted
machine-to-machine text network, notable for stable machine addresses, automatic
identity confirmation ("answerback"), store-and-forward delivery, and a durable
printed record. Those four properties are the conceptual anchors of the project.

## The problem it solves

Modern AI agent/coding sessions are **capable but ephemeral, context-limited, and
isolated**:

- The session that plans work is often not the session that implements it.
- Each session sees only its own context window; there is no shared memory.
- Sessions start cold, crash, get interrupted, and are replaced.
- Work increasingly spans multiple concurrent sessions, multiple machines, and
  long-running loops that wait on external events (PRs, CI, reviews).

Today, coordination between such sessions is **carried by hand**. A typical
real-world workflow: a developer asks session A to write a message file, commits
it to a repo, pushes, pulls on another machine, and asks session B to read it and
respond. Git history and scratch notes become an improvised runtime coordination
layer they were never designed to be. This is slow, manual, lossy, and does not
scale to many parallel sessions.

These are **coordination-infrastructure problems, not intelligence problems**. The
agents are smart enough; they lack a disciplined protocol to address one another,
deliver messages reliably, confirm a live recipient, and leave an auditable record.

Concretely, sessions need to be able to:

1. **Address a durable responsibility** (a role, a work node, a workstream),
   independent of which ephemeral process currently serves it.
2. **Send a typed message** to that address and have it delivered or safely queued.
3. **Wait** efficiently for actionable messages (loop-friendly), without being
   spammed by traffic that isn't for them.
4. **Confirm liveness** of a target before depending on it — without interrupting
   the working agent.
5. **Disposition** messages: acknowledge, handle, defer, close, escalate.
6. **Audit** the full history later to explain *why* an agent acted.

## Intended design (current thinking)

This is the design direction, not a frozen spec. Researchers should treat it as the
shape to find prior art for and pressure-test.

### Identity: durable address + ephemeral lease

- An **address** is a durable target in "work geometry," e.g.
  `workstream:<id>/role:orchestrator`, `workstream:<id>/node:<id>`,
  `session:<id>`, `project:<id>`.
- A **lease** binds an ephemeral session to an address for the duration of its
  mission. Sessions **self-attach** by default. A session releases the lease
  politely on exit, but the system must not depend on that (terminals crash).
- Addresses have a **lifecycle**: `active`, `dormant` (no occupant, still
  deliverable/queued), `retired` (closed; dropped from the address book; sends
  rejected). Closed workstreams retire their addresses so stale nodes are not
  messageable.

### Answerback: liveness without interrupting the agent

- In historical telex, the receiving **machine** answered an identity query
  automatically while the operator kept working. The equivalent here: a
  **background waiter loop** holds the lease and a live connection and answers
  liveness/identity automatically, so the **foreground agent is never
  interrupted**.
- Two grades of liveness: (a) **line open** — a session is attached and its loop
  is alive; (b) **message received and dispositioned** — acknowledged and handled,
  not merely delivered.
- Fidelity depends on the backend (see below).

### Messages: thin semantic core

- Generic, typed messages with fields like: `id`, `threadId`, `parentId`, `from`,
  `to`, `cc`, `watchers`, `kind`, `attention`, `requiresResponse`,
  `requiresDisposition`, `subject`, `body`, opaque `metadata`, `createdAt`.
- **Disposition state machine** per recipient, e.g.
  `queued → delivered → seen → acknowledged → handled | deferred | rejected → closed`.
- **Attention** (`interrupt`, `next-checkpoint`, `background`, `fyi`) is separated
  from **delivery** (`to` vs `cc` vs `watchers`) so large threads don't wake the
  wrong sessions.
- Message *kinds* (question, handoff, decision-request, split-request,
  scope-change-request, blocker, checkpoint-export, evidence, field-report,
  closeout-observation, reconciliation-fact) carry domain meaning; domain-specific
  payloads ride in opaque `metadata` so the core stays general.
- **Context-budget discipline:** the default read returns the newest/unseen
  message(s); full thread history is expandable on demand to avoid reloading text
  the model has already seen.

### Transport: pluggable backend, thin-semantic core in the client

- Business/semantic logic lives in the **client library**, not in datastore
  triggers, so behavior is consistent across backends.
- Backends advertise **capabilities** (durable storage, push vs poll, lease
  mechanism, transaction strength) and the core adapts.
- **Scope to two reference backends initially:**
  - **SQLite** — local single-machine use, tests, two-sessions-talking case.
    Push = poll/file-watch; lease = heartbeat row + TTL/reaper.
  - **Postgres** — networked, cross-machine reference. Push = `LISTEN/NOTIFY`;
    lease = **session-scoped advisory lock** that auto-releases on connection drop
    (real crash-safe liveness). First-class target: **Postgres Flexible Server
    with Entra (Azure AD) auth**, to avoid shared passwords.
  - Redis is a possible later backend; RabbitMQ is likely out (queue bus, weak on
    threads/audit/leases).
- **No bespoke server in V0.** "The backend is the broker." Cross-machine
  coordination is achieved by pointing multiple machines at one datastore
  (e.g. a hosted Postgres), not by hosting a custom relay service.
- **Audit/export:** the message store is the paper trail; it can be exported to
  Markdown/files (e.g. for Git check-in) without being the runtime substrate.
- Dead-letter is a **derived state**, not a required running daemon; senders get
  honest receipts (`delivered`, `queued-unoccupied`, `bounced`, `rejected-retired`).

### Deployment tiers (same protocol, different transport)

- **Local:** SQLite, single machine.
- **Networked:** shared Postgres (incl. hosted/managed), multi-machine,
  multi-user (any principal with DB access can be addressed/address others).
- A fully hosted, authenticated service is a *possible future*, but should be a
  redeploy of the same core rather than a rewrite.

### Relationship to a host system (Streamliner)

- The first real consumer is "Streamliner," a tool for designing and operating
  parallel AI "workstreams" (bounded bodies of work with nodes, gates,
  checkpoints, and orchestrator/worker roles). Telex addresses map naturally onto
  that "work geometry."
- Telex is designed to be **Streamliner-informed but Streamliner-independent**: a
  general utility that two arbitrary agent sessions can use with zero Streamliner
  involvement. Streamliner is an *optional privileged supervisor* (it can
  pre-create/retire addresses and force-detach sessions it observed crash), never
  the transport itself.

## Non-goals / boundaries

- Not a human chat or team-messaging product.
- Not a general autonomous multi-agent orchestration/planning framework.
- Not a replacement for durable artifacts (code, design docs, trackers).
- Not (initially) a hosted service, distributed actor router, or remote relay.
- Not trying to make every session an addressable actor — addressing is opt-in per
  mission.


