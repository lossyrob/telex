# Initial shaping: Operator Station

> Support document. This preserves the product reasoning that produced the
> workstream brief and graph. It is background, not authoritative current state.
> Promote changes to intended Telex behavior into the project design layer.

## Origin

The motivating problem is the attention cost of running several agent sessions.
The developer repeatedly inspects terminal tabs to discover which sessions are
blocked, waiting for input, finished, or in need of review. Process dashboards
can expose activity, but they do not necessarily explain what decision is needed
or provide a durable conversational path back to the session.

Telex already gives sessions durable addresses, store-and-forward delivery,
attention levels, replies, dispositions, Postgres connectivity, and an auditable
record. The seed idea was to let a desktop application attend an address so an
agent could send a meaningful operational message to a human and receive a reply.

The idea became stronger when a filtering agent was inserted between workers and
the developer. Instead of every worker addressing the human directly, workers
report to one operator agent. That agent can resolve routine questions, ask for
missing evidence, aggregate related reports, suppress noise, and escalate only
the decisions or events that deserve human attention.

## Core operating loop

```text
worker sessions
      |
      | raw reports, blockers, questions, completion notices
      v
attention:rob
      |
      | attended by an operator agent
      | filters, investigates, aggregates, recommends
      v
operator:rob
      |
      | attended by the desktop Station
      | feed, toast, reply, disposition
      v
developer
```

The return path is equally important:

```text
developer reply
      -> operator:rob thread
      -> operator agent
      -> worker reply or new directed instruction
```

The developer can continue conversing with the operator agent when deeper
context is needed. The operator agent may query one or more workers before
answering, so the desktop thread becomes a single conversational surface over
many raw agent interactions.

## Direct, assisted, and quiet operation

The protocol does not need a special human actor or routing engine. Stable
responsibility addresses and ordinary occupancy are enough:

- **Direct:** the desktop Station attends the ingress address and receives raw
  messages.
- **Assisted:** an operator agent attends the ingress address and sends filtered
  messages to the desktop's private operator address.
- **Quiet:** the operator agent handles ordinary traffic, sends scheduled
  digests, and escalates only urgent or explicitly human-required messages.
- **Per-project:** different operator addresses can use different modes at the
  same time.

An urgent bypass can be represented through a separate address, CC delivery, or
a convention interpreted by the operator agent. The first spike should not add a
general router to Telex core.

## Why this remains Telex-shaped

The desktop application is a station attending durable responsibilities. The
human is not introduced as a new transport primitive, and messages still target
addresses rather than terminal processes or transient sessions.

The operator agent is a reasoning receptionist. Its judgment is deployment
logic layered on top of the fabric, consistent with the dispatch proposal's
guardrail: Telex carries facts and obligations but does not score, summarize,
route, or execute them semantically.

The application is also a control surface rather than a control plane. It can
send an instruction such as "stop after this phase" to an orchestrator address,
but Telex does not itself stop the process or mutate the workflow.

## Source and thread model

Mediation creates two related but distinct conversations:

1. Worker-to-operator-agent threads contain raw reports and follow-up questions.
2. Operator-agent-to-human threads contain summaries, recommendations, and
   decisions.

The operator agent must preserve links between them. A human-facing escalation
should identify its source message IDs and addresses in opaque metadata while
keeping the body understandable without tooling. The Station can initially
offer only "reply to operator agent"; later it may offer "view sources" or
"contact source directly."

The operator agent should disposition raw messages according to what actually
happened:

- handled when it resolved the matter itself;
- deferred while waiting on a worker or human;
- escalated when it created a human-facing obligation;
- closed after the result was routed back and no further action remains.

## Notification posture

The Station should map Telex attention and actionable state into human attention
without turning every message into a toast.

Initial policy direction:

| Message posture | Default human behavior |
|---|---|
| Decision request, blocker, urgent failure | Toast and actionable feed |
| Review request | Actionable feed, optional toast |
| Completion | Configurable toast plus feed |
| Progress or background status | Feed only |
| FYI or audit record | Quiet history |

The operator agent is the first filter. The Station remains the second filter
through local policy by address, kind, attention, and disposition requirement.

## Why a workstream with a spike first

The complete outcome crosses product UX, Telex client integration, agent role
design, routing conventions, provenance, Postgres, packaging, restart behavior,
and operational validation. That is workstream-sized.

The largest uncertainty is experiential rather than basic technical feasibility:
whether the mediated loop meaningfully reduces attention cost and whether the
operator agent escalates at the right level. A large production-first design
would freeze architecture before earning that confidence.

The first node is therefore one experimental vertical slice followed by a
builder gate. It may use temporary integration seams, but it must exercise the
real operating loop. If the experience is not useful, the workstream can stop
without creating a supported desktop/client surface.

## Candidate workstream shape

### Confidence transition 1: operational-loop viability

One worker can report to an operator agent, the operator agent can escalate to a
desktop Station, the developer can reply, and the response can reach the worker
with source provenance intact.

### Confidence transition 2: accepted product and client contract

Dogfooding evidence has resolved the Station boundary, client/daemon integration,
address-routing modes, provenance conventions, notification policy, and
reply/disposition expectations.

### Confidence transition 3: first usable Station

The production desktop Station and reusable operator-agent role work together
under the accepted contract and pass a builder usability gate.

### Confidence transition 4: operational confidence

The loop remains dependable under Postgres, restarts, offline periods, noisy
message loads, operator-agent replacement, packaging, and security/provenance
constraints.

## Alternatives considered

### Send every worker message directly to the human

Useful as an optional direct mode, but it recreates attention overload and makes
the desktop a raw transport feed rather than a higher-value control surface.

### Build a process/session dashboard instead

A dashboard can show idle or waiting processes but does not inherently carry the
reason attention is needed, preserve an obligation record, or provide a durable
reply path to a responsibility after a particular session ends.

### Put summarization and routing inside Telex core

Rejected. It would make the fabric interpret application semantics and drift
toward an orchestration framework. Filtering belongs in the operator agent;
notification preferences belong in the Station.

### Extend `telex-console`

Rejected as the primary path. The console is deliberately read-only and
dependency-light. Its feed and thread concepts are useful references, but a
writable, persistent, notification-capable human endpoint is a distinct product.

### Productionize before dogfooding

Rejected. The product loop and attention policy should be proven before choosing
the supported desktop/client architecture.

## Key risks

- **Bad filtering:** the operator agent may hide something important or escalate
  too much. Raw messages and explicit source references must remain inspectable.
- **Broker outage:** messages queue durably while the operator address is
  unoccupied, but urgent bypass and recovery behavior still need definition.
- **Stale requests:** a human may answer after the originating worker or response
  window is gone. The durable responsibility address and eventual response-window
  conventions must make this legible.
- **Identity and spoofing:** a human-facing app increases the cost of misleading
  sender addresses, links, or metadata on a shared backend.
- **Notification collapse:** completion and progress traffic can overwhelm the
  desktop without strong defaults and aggregation.
- **Experimental architecture leakage:** the fastest spike integration may be the
  wrong production boundary.
- **Implicit command execution:** source-supplied links or actions must never
  become arbitrary command execution without an explicit trusted policy.

## Spike demonstration

The minimum convincing walkthrough is:

1. A worker sends a blocker or decision request to the attention ingress.
2. The operator agent reads the raw request and either resolves it or creates a
   distilled human escalation with source references and a recommendation.
3. The desktop Station backfills or receives the escalation and surfaces it.
4. The developer replies from the Station.
5. The operator agent receives the reply and sends an appropriate response to
   the worker.
6. The worker proceeds, and the original and mediated threads remain auditable.
7. The Station is restarted and still shows the unresolved/recent conversation.

The viability gate should then use the spike with several real sessions during a
focused work period, not merely watch a scripted demo.
