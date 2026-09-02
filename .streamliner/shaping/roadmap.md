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
session, Telex transports and wakes, and Operator Station gives the human a
direct actionable inbox and reply surface. Users may layer their own mediation
agents over ordinary Telex messages, but that convention is not shipped or
required by the campaign products.

**Review question.** Can external events and agent obligations reliably reach the
right agent or human, and receive a response, without manual tab polling or a
long-lived task occupying the session?

**Theater.** The Telex application layer: non-agent stations, deterministic event
producers, human recipients, and the shared programmatic client they consume.

**State.** Both builder viability gates and both initial production
domain-contract nodes completed. Application Client contract convergence is
merged, the design-only `application-client-ready` checkpoint is published, and
Application Client core implementation is merged through PR #132. The first
supported Rust binding completed through issue
[#149](https://github.com/lossyrob/telex/issues/149) and PR #151 at exact reviewed
head `c03db454781164f47a20e997665fe1251e07bd15`, merged as
`ddedfab57cc305a1e91a81d7e49e712bb36d32fd`. Issue #12 publication revision 4
records that binding publication. Application Client `client-conformance` is
tracked by [#152](https://github.com/lossyrob/telex/issues/152), fully specified,
ready, and unlaunched. One bundle must prove the ten accepted semantic families
through the public Rust surface across SQLite and credentialed Postgres, provide
public-only send-only and bidirectional consumer fixtures, and repair any
missing shared semantic without splitting by backend or test family.
`consumer-integration-gate` remains planned until the same reviewed and green
conformance head receives independent Watcher and Operator Station
consumability attestations. No product implementation evidence belongs in that
gate. Watcher runtime and Operator Station retain direct conformance holds and
also wait on the gate. Later runtime/usability gates remain planned. Operator
Station's direct-Station contract reset
completed through issue #134 and PR #136, merged as
`e071e3170c19ab1b8a753b502c67be2ee80688ec`. The builder accepted the
direct human-attended product boundary, ADR 0051 supersession, external-only
mediation, shared-client dependency, and downstream geometry at
`direct-station-direction-gate`. This closes the design checkpoint without
launching `station-app`, which remains planned and waits directly on Application
Client `client-conformance` and on `consumer-integration-gate`. A separately
prepared and authorized launch is still required after both complete. Telex
Watcher's minimal v2
authoring/registration reset completed through issue #133 and PR #135, merged as
`b91e8301899351c0411d6e2e9ac5290af8a3cb4c`; its builder-owned
`dumb-watcher-contract-gate` and `minimal-contract-accepted` checkpoint are
complete. Issue [#144](https://github.com/lossyrob/telex/issues/144) and its
bounded task specification now provide the optional example pack's launch
prerequisite. The node remains ready but unlaunched pending separate campaign
authorization. Runtime remains planned and waits on completed Application
Client `client-conformance` and the pre-integration
`consumer-integration-gate`, which attests the exact public conformance revision
without requiring Watcher runtime implementation.
Operator Station issue #146 separately preserves
[PR #143 Postgres and UI/UX lessons](../workstreams/operator-station/docs/postgres-dogfood-evidence.md)
as completed, non-gating evidence. The report does not promote spike mechanisms,
change direct-Station authority, or advance any production node or dependency.
Local Daemon release-confidence validation completed, but issue #106 exposed a
daemon-replacement push-intent gap. Existing PR #138 is adopted as the
in-progress repair ahead of the still-unaccepted hardening gate; its proposed
station-intent contract is not current authority until repaired, reviewed, and
merged.

## Covering workstreams

| Workstream | Tracker | Outcome | Current first move |
|---|---|---|---|
| Operator Station | [#92](https://github.com/lossyrob/telex/issues/92) | Direct human-attended Telex desktop endpoint for inbox, notification, reply, disposition, health, and recovery. | The builder accepted the direct contract and downstream geometry at `direct-station-direction-gate`, closing the design checkpoint. `station-app` remains planned and unlaunched with direct `client-conformance` and `consumer-integration-gate` holds; launch still requires separate preparation and authorization. |
| Telex Watcher | [#100](https://github.com/lossyrob/telex/issues/100) | Headless, provider-neutral execution of trusted agent-authored observations with fixed Telex delivery and no session-owned background tasks. | Issue #144 and its task specification prepare the ready optional example pack; launch still requires separate campaign authorization. Runtime remains planned and waits on completed Application Client `client-conformance` and the pre-integration `consumer-integration-gate` over the same exact public revision; the gate does not require Watcher runtime implementation. |
| Telex Application Client | [#117](https://github.com/lossyrob/telex/issues/117) | One supported semantic client contract and implementation for long-lived applications, without product-private forks. | Issue #152 and its bundle-first task make `client-conformance` ready but unlaunched. The consumer gate remains planned until both product authorities attest the same reviewed and green conformance head without product implementation evidence. |
| Local Daemon | [#32](https://github.com/lossyrob/telex/issues/32) | Reliable local presence and transport across SQLite/Postgres, Copilot push delivery, daemon replacement, upgrade, and restart. | Adopt issue #106 / PR #138 as `station-intent-reconciliation`; integrate current `main`, resolve blocking review, and present isolated both-backend evidence before the hardening gate. |

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

- Operator Station `operator-loop-spike`: merged and reconciled historical proof
  of a human-attended Station, notifications, durable reply, honest wait/ack
  attendance, provenance, restart continuity, and unresolved-history recovery.
  Its worker → operator agent → human topology remains evidence, not current
  product authority.
- Telex Watcher `generic-watcher-spike`: merged and reconciled proof of external
  detector → Watcher → Telex → target agent with no originating session waiter.
  Evidence includes generic/custom GitHub, an authorized live Azure DevOps PR
  transition, occupied Copilot wakeup, durable unoccupied queueing, receipt-gated
  state, and isolated daemon-restart testing.

The spikes answer different questions and should not block each other:

- Is a human-attended Telex inbox, notification, and reply surface valuable and
  natural?
- Is generic external detector hosting reliable and broadly adaptable?

### Stage 2 — Independent viability gates

Each workstream has passed its independent builder gate:

- Watcher passed after scoped PR-lifecycle dogfood (~26-second merge detection,
  one snapshot plus one merge event, no duplicate/noisy events,
  canonical-checker agreement, clean watch removal, and reusable shared
  runtime).
- Operator Station passed after guided dogfood proved the human inbox,
  provenance, Windows notification, durable reply, restart continuity, and
  disposition experience. Later direction review retained those product
  findings while externalizing mediation policy.

Both gates produce evidence for #12:

- lifecycle and recovery needs;
- push/callback/poll requirements;
- service/application identity;
- cursor and restart behavior;
- provenance and metadata;
- supported IPC/binding ergonomics.

### Stage 3 — Contract convergence and shared application-client checkpoint

Watcher contract node #110 and initial Operator contract node #114 completed in
parallel, each exporting merged-source requirements without freezing a
competing shared API. Application Client node #118 consolidated both accepted
contracts and spike/gate evidence into #12, recorded explicit dispositions, and
accepted one semantic contract. The resulting `application-client-ready`
checkpoint is complete.

Application Client convergence, client-core implementation, and the Rust-first
binding are complete. Issue #149 and PR #151 landed the binding at
`telex::application_client`; issue #12 publication revision 4 records the
transition. Issue #152 and its reviewed bundle-first task make
`client-conformance` ready but unlaunched. The node must deliver all ten
conformance families, public-only Watcher and Station fixtures, and
temporary-seam replacement guidance in one PR. The planned
`consumer-integration-gate` then requires independent exact-head attestations
before product implementation continues. Operator issue #134 and PR #136
completed the direct product contract reset without changing the generic
Application Client ownership boundary.

### Stage 4 — Production applications under accepted contracts

After the shared semantic checkpoint:

- Operator Station completed its design reset around direct human attendance.
  The builder accepted the direction gate and closed the design checkpoint.
  The desktop app remains planned and unlaunched while it waits on Application
  Client `client-conformance` and `consumer-integration-gate`; it retains the
  direct conformance dependency, and launch still requires separate preparation
  and authorization after both holds complete.
- Telex Watcher's minimal command-plus-policy contract and builder usability
  gate are accepted. Issue #144 and its task specification prepare the optional
  examples, which remain ready but unlaunched pending separate campaign
  authorization. Runtime and CLI remain planned until Application Client
  `client-conformance` completes and `consumer-integration-gate` accepts the
  same exact public revision without a private seam. The gate requires no
  Watcher runtime implementation.

Each retains its own usability and operational-hardening gates.

### Stage 5 — Campaign integration exercise

Before campaign close, exercise the full seam:

```text
external condition
      → Telex Watcher
      → responsible agent or directly attended Station address
      → Operator Station when the destination is human-attended
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
| Agent-authored custom detector policy | Telex Watcher minimal detector contract and optional examples |
| Durable event delivery and agent wakeup | Existing Telex local exchange and bridges |
| Optional filtering/aggregation conventions | External user-developed agents over ordinary Telex |
| Direct human inbox, notifications, replies, and disposition | Operator Station |
| Supported long-lived application integration | Shared issue #12 / completed Application Client core and Rust binding; conformance pending |
| End-to-end external-event-to-human-to-agent loop | Campaign integration exercise |

## Seams and ownership

| Seam | Owner | Consumers |
|---|---|---|
| `application-client-ready` | #12 or its promoted enabling workstream | Operator Station, Telex Watcher |
| Normalized watch event envelope | Telex Watcher | Agents, Operator Station, optional external mediation |
| Human-attended message/reply provenance | Operator Station | Human operator, originating agents |
| Durable address/message/disposition semantics | Telex core | All campaign workstreams |

## Boundary rules

- Telex core carries messages and liveness; it does not poll providers or run
  detector policy.
- Telex Watcher executes trusted observations and sends Telex; it does not run
  arbitrary trigger actions or own human UX.
- Operator Station presents and replies; it does not host detector scripts or
  become the availability boundary for watches or ship semantic agent policy.
- Optional user-authored mediation may reason and filter outside the product;
  Telex core, Watcher, and Station do not require or interpret that convention.
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

1. Reconcile and repair adopted Local Daemon PR #138 for issue #106 without
   weakening explicit membership, fencing, or merged Copilot App lifecycle
   semantics; keep the hardening gate separate from merge.
2. Keep issue #144's `minimal-example-pack` ready but unlaunched; launch only
   after separate campaign authorization.
3. Prepare issue #152's ready but unlaunched `client-conformance` node for one
   bundle-first implementation PR. Keep the consumer gate and `supported-client`
   checkpoint planned; reconciliation does not launch the node.
4. Keep `watcher-runtime-core` planned until `client-conformance` uses the
   completed first binding to prove merged exact-store/exact-operation
   `NotRecorded`, exact-same-operation retry, and retention-boundary failure
   across both backends, and `consumer-integration-gate` accepts the same exact
   public revision for Watcher without a private seam. The gate requires no
   Watcher runtime implementation; recovery remains reconciliation-first and
   query-only under uncertainty.
5. Keep `five-minute-custom-watch-gate` planned for later operational proof, and
   preserve the campaign integration exercise and no-private-client boundary.
