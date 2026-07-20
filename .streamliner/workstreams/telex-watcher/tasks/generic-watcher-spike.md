# Vertical spike: generic external detector runner

- **Workstream:** telex-watcher
- **Type:** research (product/technical spike)
- **Status:** completed
- **Depends on:** none
- **Blocks:** viability-gate
- **Tracker:** [lossyrob/telex#101](https://github.com/lossyrob/telex/issues/101)
- **Parent workstream:** [lossyrob/telex#100](https://github.com/lossyrob/telex/issues/100)
- **Campaign:** [Addressable Attention #102](https://github.com/lossyrob/telex/issues/102)
- **Result:** [PR #105](https://github.com/lossyrob/telex/pull/105) merged;
  evidence is recorded in
  [`docs/generic-watcher-spike-report.md`](../../../../docs/generic-watcher-spike-report.md).

## Outcome

A developer can register trusted local detector scripts with a persistent
headless process, leave the originating agent session free of background tasks,
and later receive a durable Telex message at a configured address when a
detector reports an event. The experimental runner is provider-neutral and is
demonstrated with editable GitHub and Azure DevOps detectors plus one
repository-specific customization.

## Design References

- [`../docs/initial-shaping.md`](../docs/initial-shaping.md) - detector/runtime
  boundary, protocol sketch, transaction, risks, and viability scenarios.
- `telex:PRODUCT-THESIS.md` - durable addresses, store-and-forward, auditability,
  and the workflow-engine non-goal.
- `telex:docs/design/daemon.md` - local exchange lifecycle and durable message
  acceptance.
- `telex:docs/design/DESIGN.md` - long-duration delivery and wakeup ownership.
- `telex:docs/design/proposals/EXTENSIONS.md` - namespaced kinds and opaque
  metadata conventions.
- Lossyrob Loop skill `references/pr-polling.md` and detector scripts - domain
  behavior to adapt without retaining the owner-bound loop/waiter runtime.

## Exports

- A runnable in-repo experimental Telex Watcher application and local management
  CLI suitable for multi-day dogfooding.
- A versioned experimental detector input/output contract.
- Persistent watch registration, opaque detector state, sent-event provenance,
  and restart recovery sufficient for the viability gate.
- Editable GitHub PR and Azure DevOps PR detector templates, including a
  demonstrated custom author/comment policy.
- `docs/generic-watcher-spike-report.md`, recording the exercised scenarios,
  integration shortcuts, detector-authoring experience, failures, security
  observations, and production contract requirements.

## Boundaries

### In scope

- A separately runnable Watcher process outside all agent sessions.
- Local CLI operations sufficient to add, inspect, pause, resume, update, and
  remove experimental watches.
- Trusted local command execution with timeout, non-overlap, bounded output,
  retry/backoff, and diagnostic logs.
- Versioned structured detector request/result exchange with opaque state.
- Fixed sender/target registration and normalized Telex event emission.
- State commit after durable Telex send receipt for event-producing results.
- Stable watch and event IDs in message metadata.
- GitHub and Azure DevOps templates that agents can copy and customize quickly.
- Watcher restart recovery and an end-to-end session wakeup demonstration.

### Out of scope

- A production embeddable client or stable public SDK; owned by campaign seam
  [issue #12](https://github.com/lossyrob/telex/issues/12).
- General workflow actions, PR mutation, auto-merge, repository edits, or agent
  launching.
- Remote registration of commands, script sandboxing, signed catalogs, hosted
  webhooks, or multi-host failover.
- Production installer, automatic upgrades, comprehensive security hardening, or
  long-term template compatibility; owned by later workstream nodes.
- Operator Station UI or watch-management UX.

## Inherited decisions

- **Provider-neutral runtime, editable provider examples.** The spike proves a
  generic detector boundary rather than compiling GitHub or Azure DevOps policy
  into Watcher.
- **One reaction only: send Telex.** There is no configurable action command after
  an event. Consequential action belongs to the woken agent.
- **Trusted local execution.** The spike is explicit that detector commands run as
  the current user. Registration is local-only and no sandbox guarantee is made.
- **Watcher owns routing and process policy.** Scripts cannot override target,
  sender, cadence, timeout, or environment policy through event output.
- **Structured result semantics.** Exit codes express process success or failure;
  JSON expresses `idle`, `event`, `terminal`, or `degraded`.
- **Safe state ordering.** Event-producing state is committed only after Telex
  accepts the event. Stable IDs expose possible at-least-once duplicates.
- **Temporary Telex integration is acceptable.** CLI subprocesses or current Rust
  library reuse may be used for the spike, but the report must identify every
  shortcut and must not present it as the #12 application-client contract.
- **Follow-path development is allowed.** The spike may optimize for rapid agent
  editing while recording the executed script digest on attempts and events.

## Design-impact expectation

No project design update is required merely to land the experiment. Record
potential Telex, application-client, message-envelope, or trust-model changes in
`docs/generic-watcher-spike-report.md`; `watcher-contract` and the campaign's #12
seam own promotion after the viability gate.

## Success criteria

- A watch continues operating after the registering agent turn and session task
  complete, with no attached loop/waiter task remaining in that session.
- A detector can retain opaque state across attempts and Watcher restart.
- When a detector reports an event, Watcher sends it to the registration's fixed
  Telex address and commits the proposed state only after a successful receipt.
- A Telex plugin-backed agent is woken by the event, or the message remains
  durably queued when the address is unoccupied.
- Agents can create or materially customize a detector without changing Watcher
  runtime code.
- The GitHub example handles a repository-specific author/comment policy.
- The Azure DevOps example proves the same detector protocol against a distinct
  provider.
- Hung, malformed, overlapping, and repeatedly failing detector executions are
  bounded and visible rather than blocking the scheduler.
- The Watcher can restart and continue registered watches without duplicating all
  prior observations.
- The spike report gives `watcher-contract`, issue #12, and the Operator Station
  workstream actionable production requirements.

## Engagement

- Review the worker's plan and detector protocol before implementation so the
  generic boundary stays narrow and no workflow-action surface is introduced.
- Review the first GitHub and Azure DevOps end-to-end events before broad
  dogfooding.
- Conduct the multi-watch viability exercise separately at `viability-gate`; the
  worker prepares the scenarios and report but does not self-pass the gate.
