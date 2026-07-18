# Operator Station (mediated human-attention loop)

## Purpose

Create an optional human-attended Telex station that lets agent sessions route
meaningful attention to a developer without requiring repeated terminal
inspection. Prove a mediated operating loop in which workers report to an
operator agent, the operator agent filters and escalates through a desktop
station, and the developer can reply through the same durable message fabric.

## Approach

The workstream starts with one deliberately narrow vertical spike rather than a
production architecture. The spike must make the complete loop real on Windows:
a worker messages a stable attention address, an operator agent attends that
address and sends a distilled message to a desktop-attended operator address,
the desktop surfaces the message, and a human reply reaches the operator agent
and can be routed back to the worker. The spike may use current CLI or library
surfaces and is not allowed to freeze the production client contract.

A builder viability gate follows the spike. The gate evaluates the loop through
real multi-session use: whether it reduces tab polling, whether the filtering
agent escalates the right amount, whether replies preserve enough context, and
whether the experience is valuable enough to productionize. A failed gate may
stop or reshape the workstream without carrying experimental integration choices
forward.

If the gate passes, a contract node establishes the production boundary before
implementation proceeds. The desktop app and reusable operator-agent role can
then execute in parallel under that contract, followed by an end-to-end usability
gate and an operational-hardening node covering Postgres, restart/offline
behavior, notification pressure, provenance, packaging, and recovery.

The richer rationale and early interaction model are preserved in
[`docs/initial-shaping.md`](docs/initial-shaping.md).

## Design References

- `telex:docs/design/index.md` - entry point for Telex's intended-system design.
- `telex:PRODUCT-THESIS.md` - durable responsibilities, store-and-forward
  delivery, auditable records, and the boundary against general chat.
- `telex:docs/design/daemon.md` - normative local-exchange and client/daemon
  contract that a production Station should reuse.
- `telex:docs/design/proposals/EXTENSIONS.md` - namespaced message kinds and
  opaque metadata conventions for typed operator requests and source references.
- `telex:docs/design/proposals/DISPATCH.md` - the reasoning-receptionist pattern
  and the rule that judgment remains in agents rather than Telex core.
- `telex:telex-console/README.md` - existing separate operator console and its
  feed, address, thread, delivery, and disposition concepts.

## Boundaries

- **In scope:** a Windows-first optional desktop Station; one or more
  human-attended operator addresses; Windows notifications; actionable feed and
  thread reading; reply and disposition; a reusable operator-agent role that
  filters, summarizes, resolves, and escalates worker messages; durable source
  provenance; direct and assisted address-routing modes; local SQLite and
  networked Postgres operation; restart/offline and noisy-workload validation.
- **Out of scope:** general-purpose human chat; contacts, rooms, typing
  indicators, reactions, or social presence; agent process supervision,
  launching, killing, or restart management; Streamliner-specific workflow
  semantics in Telex core; arbitrary command execution from message content;
  replacing `telex-console`; making the desktop app mandatory for Telex use.
- **Deferred:** multi-device fan-out for one operator address; macOS/Linux/mobile
  clients; inline structured decision controls beyond ordinary replies;
  cryptographically verified cross-principal sender identity; a general routing
  or alias engine in Telex core; rich session-opening or terminal-control
  integrations.

## Current State

The workstream is formed under parent issue
[#92](https://github.com/lossyrob/telex/issues/92). The first executable node is
the `operator-loop-spike`, tracked by
[#93](https://github.com/lossyrob/telex/issues/93): a single-session vertical
slice intended to create viability and operational-loop confidence rather than
production substrate. Later nodes remain sketches until the builder has
dogfooded the spike and passed the viability gate.

The current Telex local exchange, Postgres backend, attention levels, replies,
dispositions, and durable message record provide the substrate. The production
Station contract is intentionally unresolved until the spike shows what the
experience actually requires.

## Decisions

- **The spike is Wave 1, not an untracked side project:** the workstream preserves
  its purpose, boundary, and gate while allowing one session and one PR to move
  quickly.
- **The operator agent owns filtering policy:** Telex carries, routes, records,
  and dispositions messages but does not decide what deserves human attention.
- **The desktop app is a station, not a new protocol actor:** it attends durable
  responsibility addresses through existing Telex semantics.
- **The app is a control surface, not a control plane:** it sends instructions
  through messages but does not own session lifecycle or workflow execution.
- **Raw provenance survives mediation:** summaries and escalations identify their
  source messages and source addresses; the operator agent never impersonates a
  worker.
- **Windows is the first supported desktop target:** the existing Streamliner
  Tauri shell is reference implementation material, not a runtime dependency.
- **Experimental integration does not set production architecture:** the spike
  may use CLI subprocesses or in-process library access; the post-gate contract
  decides the supported daemon/client boundary.
- **Direct and assisted operation are routing configurations:** workers use
  stable responsibility addresses; which station attends an ingress address
  determines whether traffic reaches the desktop directly or passes through an
  operator agent.

## Open Questions

- What production client surface should the Station use: stabilized daemon IPC,
  an embeddable Rust client, or another thin supported API over the local
  exchange?
- How should a long-lived operator-agent role be launched, recovered, and
  rehydrated from unresolved threads after its session ends?
- Which message-kind and metadata conventions are sufficient for source
  references, recommendations, human-required bypasses, and eventual structured
  choices?
- Should replying to an escalation also disposition the human-facing message,
  and does that require an atomic higher-level operation?
- How should the UI switch between direct, assisted, and quiet operation without
  creating ambiguous or competing address occupancy?
- Which attention levels, kinds, and disposition requirements generate toasts by
  default, and how are noisy sources suppressed or summarized?
- What sender/principal assurance must be visible before a shared Postgres
  Station is safe for broader use?

## Imports and Exports

### Imports

- The local-daemon workstream's local-exchange lifecycle, durable delivery,
  attention, reply, disposition, and Postgres behavior.
- Existing Telex client/library and backend traits used only as experimental
  seams until the production contract is accepted.
- Streamliner Desktop's Tauri tray/feed/notification patterns as reference code,
  not as a package or service dependency.
- `telex-console` feed, address, thread, and provenance presentation concepts.

### Exports

- A demonstrated mediated human-attention loop and its viability evidence.
- An accepted production Station/client contract if the viability gate passes.
- A separately installable human Station that remains optional to Telex core.
- A reusable operator-agent role and routing convention that other orchestration
  systems can adopt without Telex-specific workflow logic.
- Dogfooding evidence and operational requirements for future portfolio-level
  attention surfaces.

## Closeout Observations

Parking lot for bounded polish, notification tuning, message rendering, and
operator-agent prompt improvements discovered during dogfooding. Anything that
changes Telex semantics, identity guarantees, routing architecture, or session
lifecycle belongs in its own node, candidate, or follow-on workstream.
