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

**State.** Main effort. Telex Watcher Wave 1 is merged and reconciled; its
builder-owned `viability-gate` is ready. Operator Station Wave 1 is implemented,
fully reviewed, and merge-ready in PR #104; its viability gate follows merge and
workstream reconciliation. The shared production application-client contract
remains downstream of both independent builder gates.

## Covering workstreams

| Workstream | Tracker | Outcome | Current first move |
|---|---|---|---|
| Operator Station | [#92](https://github.com/lossyrob/telex/issues/92) | Human-attended Telex endpoint plus an optional operator-agent filter and reply loop. | Merge [PR #104](https://github.com/lossyrob/telex/pull/104), reconcile Wave 1, then run the builder viability gate. |
| Telex Watcher | [#100](https://github.com/lossyrob/telex/issues/100) | Headless, provider-neutral deterministic detectors emit Telex messages without session-owned background tasks. | Wave 1 completed through [#101](https://github.com/lossyrob/telex/issues/101) / [PR #105](https://github.com/lossyrob/telex/pull/105) and reconciliation [PR #108](https://github.com/lossyrob/telex/pull/108); run the ready builder viability gate. |

## Shared seam

**Telex Application Client — [#12](https://github.com/lossyrob/telex/issues/12).**
Both production applications are long-lived non-agent stations. They need one
supported semantic client surface for process identity, attach/detach/recovery,
send, receive, reply, disposition, backend selection, and provenance.

The product spikes must not wait for this seam: they may use current CLI or Rust
library integration and must report every shortcut. After viability evidence is
available, #12 is revised and promoted as the single owner of the shared contract.
If implementation becomes workstream-sized, it is formed as a third enabling
workstream and exports an `application-client-ready` checkpoint consumed by both
product workstreams.

Neither Operator Station nor Telex Watcher may independently freeze a competing
public client API.

## Staging

### Stage 1 — Parallel operational-loop viability

The parallel Wave 1 implementation stage produced:

- Operator Station `operator-loop-spike`: complete implementation and approved
  live demonstration of worker → operator agent → human Station → reply →
  worker; PR #104 is awaiting merge.
- Telex Watcher `generic-watcher-spike`: merged and reconciled proof of external
  detector → Watcher → Telex → target agent with no originating session waiter.
  Evidence includes generic/custom GitHub, an authorized live Azure DevOps PR
  transition, occupied Copilot wakeup, durable unoccupied queueing, receipt-gated
  state, and isolated daemon-restart testing.

The spikes answer different questions and should not block each other:

- Is the mediated human interaction valuable and natural?
- Is generic external detector hosting reliable and broadly adaptable?

### Stage 2 — Independent viability gates

Each workstream has its own builder gate. The Watcher gate is ready now; the
Operator Station gate becomes ready after PR #104 merge/reconciliation. Either
may pass, reshape, or stop without forcing the other to the same conclusion.

Both gates produce evidence for #12:

- lifecycle and recovery needs;
- push/callback/poll requirements;
- service/application identity;
- cursor and restart behavior;
- provenance and metadata;
- supported IPC/binding ergonomics.

### Stage 3 — Shared application-client checkpoint

Consolidate the spike reports into #12. Accept one semantic Application Client
contract and, if needed, form its enabling workstream. Production app nodes wait
on the resulting `application-client-ready` checkpoint; product-specific contract
and UX/design nodes may continue in parallel while that checkpoint is built.

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
  reports exist. Watcher requirements are published in the issue; Operator
  Station requirements join them after Wave 1 reconciliation.

## Current next actions

1. Merge Operator Station PR #104 and complete its post-merge workstream
   reconciliation.
2. Run the independent Watcher and Operator Station builder viability gates,
   recording real dogfood observations without auto-passing either gate.
3. Consolidate both accepted gate outcomes and spike reports into #12; decide
   whether to form the shared Application Client enabling workstream.
4. Keep production `station-app` and `watcher-runtime` nodes blocked until their
   viability gate and the shared `application-client-ready` checkpoint permit
   promotion.
