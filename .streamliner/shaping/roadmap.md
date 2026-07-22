# Telex Campaign Roadmap

> Current campaign-level plan for Telex. The campaign concept is defined by
> Streamliner's `CAMPAIGNS.md`; this document is the project-local instance and is
> revised as workstreams pass gates or seams change.

## Current main effort

**Campaign — [Addressable Attention #102](https://github.com/lossyrob/telex/issues/102).**
Make Telex useful as a complete attention path: deterministic external
conditions and agent-generated obligations can reach the responsible agent or
human without session-bound polling, background waiters, or manual terminal
inspection.

## Campaign — Addressable Attention ([#102](https://github.com/lossyrob/telex/issues/102)) *(main effort)*

**Declared intent.** A Telex user can delegate long-duration observation and
human-attention routing to durable external applications. Agent sessions remain
free to reason and respond while Telex Watcher observes conditions outside the
session, Telex transports and wakes, an operator agent filters when desired, and
Operator Station gives the human an actionable inbox and reply surface.

**Review question.** Can external events and agent obligations reliably reach the
right agent or human, and receive a response, without manual tab polling or a
long-lived task occupying the session?

**Theater.** The Telex application layer: non-agent stations, deterministic event
producers, human recipients, and the shared programmatic client they consume.

**State.** Both builder viability gates and both production domain-contract
nodes are complete and reconciled. The shared
[Application Client workstream #117](https://github.com/lossyrob/telex/issues/117)
is formed, and contract-convergence node
[#118](https://github.com/lossyrob/telex/issues/118) is ready to reconcile the
merged requirements through issue #12 and publish `application-client-ready`.
The live campaign mediation path spans campaign control → `attention:rob` →
`operator:rob`. Production product nodes remain blocked on that checkpoint.

## Covering workstreams

| Workstream | Tracker | Outcome | Current first move |
|---|---|---|---|
| Operator Station | [#92](https://github.com/lossyrob/telex/issues/92) | Human-attended Telex endpoint plus an optional operator-agent filter and reply loop. | Contract #114 / PR #116 is merged and reconciled; `station-app` and `operator-broker` wait on `application-client-ready`. |
| Telex Watcher | [#100](https://github.com/lossyrob/telex/issues/100) | Headless, provider-neutral deterministic detectors emit Telex messages without session-owned background tasks. | Contract #110 / PR #115 is merged and reconciled; `watcher-runtime` and `detector-template-library` wait on `application-client-ready`. |
| Telex Application Client | [#117](https://github.com/lossyrob/telex/issues/117) | One supported semantic client contract and implementation for long-lived applications, without product-private forks. | Launch contract-convergence [#118](https://github.com/lossyrob/telex/issues/118), disposition both merged domain exports through #12, and publish `application-client-ready`. |

## Shared seam

**Telex Application Client — [#12](https://github.com/lossyrob/telex/issues/12).**
Both production applications are long-lived non-agent stations. They need one
supported semantic client surface for process identity, attach/detach/recovery,
send, receive, reply, disposition, backend selection, and provenance.

The product spikes must not wait for this seam: they may use current CLI or Rust
library integration and must report every shortcut. After viability evidence is
available, #12 is revised and promoted as the single owner of the shared contract.
The seam is now formed as the third enabling
[Application Client workstream #117](https://github.com/lossyrob/telex/issues/117).
Issue #12 remains the sole semantic owner. Node #118 first publishes the
API-neutral `application-client-ready` checkpoint; later workstream nodes
implement and validate the supported core and binding.

Neither Operator Station nor Telex Watcher may independently freeze a competing
public client API.

## Staging

### Stage 1 — Parallel operational-loop viability

The parallel Wave 1 implementation stage produced:

- Operator Station `operator-loop-spike`: merged and reconciled implementation
  plus approved live demonstration of worker → operator agent → human Station →
  reply → worker. Evidence includes honest wait/ack attendance, two auditable
  threads, source provenance, visible Windows notification, restart continuity,
  and recovery of an unresolved obligation beyond 1,000 newer message IDs.
- Telex Watcher `generic-watcher-spike`: merged and reconciled proof of external
  detector → Watcher → Telex → target agent with no originating session waiter.
  Evidence includes generic/custom GitHub, an authorized live Azure DevOps PR
  transition, occupied Copilot wakeup, durable unoccupied queueing, receipt-gated
  state, and isolated daemon-restart testing.

The spikes answer different questions and should not block each other:

- Is the mediated human interaction valuable and natural?
- Is generic external detector hosting reliable and broadly adaptable?

### Stage 2 — Independent viability gates

Each workstream has passed its independent builder gate:

- Watcher passed after scoped PR-lifecycle dogfood (~26-second merge detection,
  one snapshot plus one merge event, no duplicate/noisy events,
  canonical-checker agreement, clean watch removal, and reusable shared
  runtime).
- Operator Station passed after guided mediation dogfood covering escalation,
  source provenance, Windows notification, human reply/route-back, routine local
  handling, clarification, restart continuity, and terminal disposition
  recovery. The live campaign/operator/desktop path is now attended on the
  default coordination store.

Both gates produce evidence for #12:

- lifecycle and recovery needs;
- push/callback/poll requirements;
- service/application identity;
- cursor and restart behavior;
- provenance and metadata;
- supported IPC/binding ergonomics.

### Stage 3 — Contract convergence and shared application-client checkpoint

Watcher contract node #110 and Operator contract node #114 completed in
parallel, each exporting merged-source requirements without freezing a
competing shared API. Application Client node #118 now consolidates both
accepted contracts and spike/gate evidence into #12, records explicit
dispositions, and accepts one semantic contract. Product nodes wait on the
resulting `application-client-ready` checkpoint.

### Stage 4 — Parallel production applications

After the shared checkpoint:

- Operator Station builds the desktop app and reusable operator-agent role.
- Telex Watcher builds the production runtime and detector-template library.

Each retains its own usability and operational-hardening gates.

### Stage 5 — Campaign integration exercise

Before campaign close, exercise the full seam:

```text
external condition
      → Telex Watcher
      → operator agent or worker address
      → Operator Station when human attention is needed
      → human reply
      → responsible agent
```

Campaign closure checks both completed workstreams and the meaning at their seam:
source provenance remains intact, routing is predictable, notifications do not
collapse into noise, and no session-bound polling task is required.

## Coverage map

| Declared-intent slice | Covered by |
|---|---|
| External long-duration observation outside sessions | Telex Watcher |
| Agent-authored custom detector policy | Telex Watcher detector contract and templates |
| Durable event delivery and agent wakeup | Existing Telex local exchange and bridges |
| Filtering, aggregation, and human escalation | Operator Station operator-agent role |
| Human inbox, notifications, replies, and disposition | Operator Station |
| Supported long-lived application integration | Shared issue #12 / future Application Client checkpoint |
| End-to-end external-event-to-human-to-agent loop | Campaign integration exercise |

## Seams and ownership

| Seam | Owner | Consumers |
|---|---|---|
| `application-client-ready` | #12 or its promoted enabling workstream | Operator Station, Telex Watcher |
| Normalized watch event envelope | Telex Watcher | Agents, operator agent, Operator Station |
| Human escalation/source provenance | Operator Station | Human operator, originating agents |
| Durable address/message/disposition semantics | Telex core | All campaign workstreams |

## Boundary rules

- Telex core carries messages and liveness; it does not poll providers or run
  detector policy.
- Telex Watcher executes trusted observations and sends Telex; it does not run
  arbitrary trigger actions or own human UX.
- Operator Station presents and replies; it does not host detector scripts or
  become the availability boundary for watches.
- The operator agent reasons and filters; neither Telex core nor Watcher
  interprets what deserves human attention.
- Shared application-client semantics have one owner through #12.
- Destructive daemon, upgrade, handoff, and branch-binary tests use an isolated
  `TELEX_HOME`, `TELEX_DB`, `TELEX_INSTALL_ROOT`, absolute worktree binary, and
  disposable proof stations. The default local daemon and installed launcher are
  campaign coordination infrastructure and are never test targets.

## Side issue

- [#12](https://github.com/lossyrob/telex/issues/12) — revise the existing
  embeddable SDK design around the post-daemon reality and broaden it to desktop,
  headless service, and agent SDK application stations after the viability
  reports exist. Both viability decisions and contract-node promotions are now
  published; #12 remains the sole owner of shared client convergence.

## Current next actions

1. Launch Application Client contract-convergence node
   [#118](https://github.com/lossyrob/telex/issues/118) under workstream
   [#117](https://github.com/lossyrob/telex/issues/117).
2. Disposition every merged Watcher and Operator shared-client requirement,
   land the API-neutral design contract, and publish
   `application-client-ready` through issue #12.
3. Reconcile that checkpoint into both product workstreams, then promote/detail
   `station-app`, `operator-broker`, `watcher-runtime`, and
   `detector-template-library` for coordinated execution.
4. Detail the Application Client core, first binding, and conformance nodes from
   the accepted contract; do not allow a product-private fallback while shared
   implementation is in progress.
