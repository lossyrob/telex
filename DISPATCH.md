# Telex — Active Discovery & Dispatch

Broadcast enquiries, answerback bidding, and Contract-Net dispatch for finding the
right *agent* on the network — not just the right address.

## Status

Exploratory design capture, and a **forward-looking layer** rather than a V0
requirement. Telex is useful with direct addressing and a passive, self-registered
directory alone (see [DESIGN.md](docs/design/DESIGN.md)). This document describes what becomes
possible *after* that foundation exists: a network where a session can ask "who is
best placed to handle this?" and live agents answer for themselves.

Treat everything here as a destination to build toward incrementally, reusing the
existing message fabric rather than introducing new transport.

## Where this sits: the discovery ladder

Discovery spans a ladder from cheap-and-exact to powerful-and-fuzzy. Each rung
builds on the ones below it.

1. **Convention / derivation** — well-known structured addresses
   (`workstream:foo/role:orchestrator`) and graph-derived node IDs. No lookup.
2. **Directory listing** — enumerate active addresses with their descriptions and
   liveness. The passive "telex directory book."
3. **Attribute resolution** — filter the directory by tag, scope, or substring.
4. **Semantic resolution** — natural-language match against registered
   descriptions (FTS or embeddings), returning ranked candidates.
5. **Answerback confirmation** — verify a resolved occupant's self-description and
   liveness before committing a directed, interrupt-grade message.
6. **Active dispatch** — broadcast an enquiry to a scoped set of addresses and let
   live agents reason about fit and bid. This is Contract Net.

V0 (in DESIGN.md) covers rungs 1–3 passively. This document is mainly about rung 6,
with 4 and 5 as supporting machinery.

## The core idea: broadcast, bid, award

"Who is best placed to handle this?" is the **Contract Net Protocol** from
multi-agent systems — a familiar four-move interaction:

```text
announce  →  bid  →  award  →  (decline)
```

A session broadcasts a scoped **enquiry** describing a problem. Addresses that
match evaluate their own fit and return a **bid**. The asker picks a winner and
sends a directed **award**; everyone else is declined or times out. The award — not
the broadcast — is what actually assigns work.

### Peer-to-peer is the Telex capability

The Telex capability is **peer-to-peer**: any session can announce, and any session
can bid. The Streamliner orchestrator is just another session; Telex does not bake
in a privileged announcer. Whether a given deployment restricts announcing to an
orchestrator (mediated dispatch) is a **Streamliner rollout choice layered on top**,
not a Telex limitation. Telex provides the general peer capability and lets
consumers narrow it.

## The key principle: broadcasts target the waiter, not the agent

A naive broadcast spams every working agent, breaking Telex's core "do not
interrupt the foreground" invariant. The resolution is that an enquiry is delivered
to the address's **answerback drum — the background waiter of its station — and the
waiter reasons about it autonomously.** (The station is the running presence serving
the address: its holder holds the lease; its waiter answers and, here, reasons.)

Historical answerback automatically answered *who are you?* The waiter now
automatically answers *are you a good fit for this?* — light reasoning, no
foreground interruption. It escalates to the foreground only if it decides to
**accept** real work. Active dispatch is answerback extended, not chat bolted on.

## The waiter's three responsibilities

This promotes the waiter from a heartbeat into a tiered responder:

1. **Liveness pings** — answer automatically, never escalate. *(existing
   answerback)*
2. **Enquiries / calls-for-bids** — reason against current work-scope, bid or stay
   silent, escalate only on accept. *(new)*
3. **Directed obligations** (handoff, award, blocker) — escalate to the foreground
   per attention level. *(existing)*

The waiter graduates from an answerback drum into a **reasoning receptionist** for
the responsibility it serves.

## Two planes, and the waiter as bridge

Active dispatch spans two distinct communication planes that should not be
conflated:

- **Plane A — intra-session control.** The foreground agent and its own waiter
  sub-agent. Local, ephemeral, carried by the host agent platform's
  sub-agent-steering mechanism (for example, updating a running sub-agent in
  Copilot CLI). This is how the waiter *learns the current work scope*.
- **Plane B — inter-session fabric.** Waiter to backend to other waiters. Durable,
  networked, audited. **This is Telex.**

The waiter sits on the seam. The strategic rule: **Telex standardizes Plane B and
treats Plane A as a platform integration detail.** Different agent runtimes will
steer sub-agents differently; if the Telex protocol depended on one runtime's
mechanism, it would lose the vendor-neutrality the product depends on. Specify the
work-scope brief format and the enquiry/bid/award messages; leave "how a foreground
pushes its brief to its waiter" to per-platform adapters.

## The work-scope brief

A waiter can only reason well about enquiries if it knows what its session is
currently doing. The foreground periodically pushes a compact **work-scope brief**
to its waiter (over Plane A):

- mission and current focus;
- active files, artifacts, issues, or nodes;
- tags and scope;
- known blockers;
- an **availability posture** — for example `focused / not-accepting` versus
  `open-to-handoffs`.

The posture is the cheapest, highest-value field: it gates whether the waiter bids
at all, with no reasoning required.

Mechanics:

- **Refresh at safe stopping points,** not mid-thought — the same discipline as the
  `next-checkpoint` attention level. The foreground updates the brief when it
  reaches a checkpoint.
- **Sync across planes:** the foreground sends the full brief to its waiter
  (Plane A); the waiter publishes a redacted, compact summary to the **lease-level
  description** on the backend (Plane B). That keeps passive directory resolution
  working even when the waiter is momentarily busy.
- **Staleness is a real risk:** a stale brief makes a waiter bid confidently on
  abandoned work. Therefore **bids are provisional, and an award still routes to
  the foreground for final acceptance** (rung 5, answerback confirmation). An
  autonomous bid must never auto-commit the foreground to interrupt-grade work.

## Message kinds

Active dispatch adds a small family of message kinds to the thin semantic core.
They reuse existing delivery, threading, and disposition machinery.

- **`enquiry`** (call-for-bids): a scoped recipient set, a problem description,
  required capabilities/tags, and a response **deadline (TTL)**. Delivered as
  *background* to foregrounds but *actionable* to waiters.
- **`bid`** (proposal): a cheap, structured response —
  `{ confidence, oneLine, evidence[], canStartBy, costHint }`. Bids should cite
  evidence from the work-scope brief ("I own `auth/migrate.py` in this
  workstream"), not vibes.
- **`award`** / **`decline`**: a directed message to the chosen bidder with
  `requiresDisposition: true` — now it is a real obligation, governed by the lease
  and claim discipline. Non-winners are declined or simply time out.

A broadcast is **multi-address delivery to a computed recipient set** — the result
of a scoped directory query. The messaging model already anticipates "single or
multi-address delivery"; active dispatch is that capability aimed at a query result
rather than a hand-listed set.

## Where it gets hard

An honest list of the problems active dispatch introduces:

- **Scope and spam.** Unbounded broadcast across a shared backend is noise and a
  privacy leak. Enquiries must be scoped, and addresses should opt in to receiving
  them.
- **Fan-in / answer storms.** N bids each cost the asker context. Make bids
  **two-phase**: a cheap structured bid first, full reasoning pulled only from top
  candidates — the same context-budget discipline as "newest/compact first, expand
  on demand."
- **Per-node reasoning cost.** Do not run model reasoning at every node for every
  broadcast. **Cheap directory pre-filter (tags/FTS) → only matched live waiters
  reason.** The directory triages; agents judge.
- **Liveness and TTL.** Enquiries are time-sensitive. Only *occupied* addresses bid
  with live reasoning; unoccupied addresses may match on static description only,
  clearly flagged as not live.
- **Award authority and races.** The broadcast discovers; it does not assign.
  Assignment is the directed `award`, and the lease/claim primitive must prevent
  the same node being awarded conflicting work twice.
- **Trust and gaming.** Agents self-assess fit and may over-bid. In a single-user,
  trusted deployment this is fine; across principals it needs evidence-backed bids,
  asker validation, or reputation. Defer it, but design bids to carry evidence from
  the start.

## Guardrail: discovery, not orchestration

Telex must stay **lower and dumber** than the agents using it. It carries the
`enquiry`, the `bid`, the `award`, the liveness, and the audit trail. It does **not**
run the auction, score bids, or manage task lifecycle — that judgment lives in the
asking agent or a Streamliner orchestrator. The moment Telex starts scoring bids it
drifts into the orchestration framework the project explicitly is not. Active
dispatch is a switchboard, not a planner.

## Vision

Carried far enough, the waiter becomes a **reasoning receptionist**: it holds the
lease, answers liveness, screens enquiries against a live brief, drafts
evidence-backed bids, and escalates only real obligations. Across a scoped network,
that is a **capability switchboard** for agent work — the most fitting thing a
system named after the telex exchange could grow into.

Further-out extensions, each with real cost:

- **Referral / gossip** — a waiter that is not a fit points to one that might be.
- **Standing subscriptions** — beyond one-shot "who can handle this now?", a node
  subscribes to "notify me of any enquiry matching tag X." Pub/sub on the discovery
  plane; Redis Streams or NATS are natural later transports for it.

## Relationship to historical telex

The directory and answerback parts of this are squarely in the metaphor: telex had
**directory books** and a **directory-enquiry** service, and operators read the
**answerback** to confirm they had reached the right party before transmitting.
Resolution-then-confirmation is genuine telex discipline.

Broadcast bidding reaches slightly *beyond* point-to-point telex into modern
service-discovery and multi-agent territory, and that is acceptable for a
forward-looking layer — provided it keeps the discipline that made the original
trustworthy: address a responsibility, confirm the line, leave a record, and never
mistake "line open" for "work understood and handled."

## Open questions

- whether deployments default to peer-to-peer or orchestrator-mediated dispatch,
  and how Streamliner chooses (a Streamliner rollout question, not a Telex
  capability question);
- how semantic resolution is stored and refreshed (FTS versus embeddings) and where
  it runs;
- how enquiry scope is expressed and enforced, and how addresses opt in;
- trust, reputation, and anti-gaming for bids across multiple principals;
- how `award` interacts with the lease/claim primitive to prevent double
  assignment;
- referral depth and loop prevention if waiters can refer enquiries onward;
- the subscription model for standing enquiries, and which backend provides it;
- cleanup of expired enquiries and stale bids, as derived state rather than a
  required daemon.
