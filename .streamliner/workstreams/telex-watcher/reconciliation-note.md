# Telex Watcher — Reconciliation

## What changed

Wave 1 began as a narrow proof that trusted local detector scripts could run
outside agent sessions and emit one normalized Telex event. The merged result in
PR #105 preserved that boundary, but real provider and lifecycle exercises made
the application-station surface substantially more concrete: one ephemeral
runtime session serves stable sender addresses, sender membership is bound to
the Watcher PID, event state follows durable Telex acceptance, and duplicate
event IDs never authorize a new transition.

The spike also produced sharper evidence than the original formation artifacts
anticipated. A live Azure DevOps snapshot was insufficient until the campaign
authorized a disposable PR-created transition. A shared-daemon restart disrupted
campaign communications and established a mandatory isolated test-plane rule.
Final review exposed and fixed a Windows process-containment race.

The builder subsequently passed the viability gate after scoped PR lifecycle
dogfood on Operator Station PR #104. The shared Watcher runtime detected merge
in about 26 seconds, emitted one initial snapshot and one merge event with no
duplicates/noise, agreed with the canonical checker, removed the watch, and
remained live for reuse. That moves the workstream from experimental viability
into production contract definition.

Production contract node #110 then landed in PR #115. It promoted the proven
domain semantics into `docs/design/watcher.md`, four canonical schemas, and ADR
0046. The byte-exact Watcher requirements export is published on issue #12.
Downstream implementation is now waiting on campaign acceptance of the shared
Application Client rather than on unresolved Watcher-domain design.

Post-approval Watcher-sentry preflight also caught that PR #115 had merged before
its state/activity watches were registered. No watches or Loop fallback were
started, and the shared runtime remained reusable. Template guidance must make
terminal state the final check immediately before registration.

## Boundaries

- **Held:** Provider semantics stayed in editable scripts; Watcher remained
  provider-neutral and send-only; detector output could not reroute or request
  actions; no public Application Client or stable strict-send API was created.
- **Leaked:** The first daemon restart proof used the campaign coordination
  plane. The stable launcher and bridges were restored, and all later destructive
  evidence moved to isolated `TELEX_HOME`, database, install root, and binary.
- **Expanded:** The experimental node needed process-bound multi-sender
  membership, restart reconciliation, and sender-only receipt semantics to make
  the external runtime honest. These are #12 requirements, not accepted
  production architecture.
- **Held:** The production contract remained design-only and did not promote CLI
  subprocess parsing, raw daemon IPC, the private send-once environment
  contract, or sender occupancy into a supported Application Client.

## Contracts and exports

The viability gate can rely on:

- the version-1 JSON detector request/result protocol;
- `idle`, `event`, `terminal`, and `degraded` state semantics;
- receipt-before-state ordering with a visible at-least-once duplicate window;
- stable watch, event, state, script, sender, target, attempt, and message
  provenance;
- PID-bound sender attachment and restart reconciliation;
- generic GitHub, customized GitHub, Azure DevOps, and non-PR templates;
- `docs/generic-watcher-spike-report.md` as the evidence and #12 requirements
  handoff.

Production work cannot treat the spike-private send-once environment contract,
CLI subprocess lifecycle, or sender-only occupancy behavior as a supported
client API.

The passed gate exports a product decision: external Watcher hosting is useful
enough to continue. Node #110 now owns the accepted production Watcher contract;
production runtime/template work remains blocked on that contract and the
campaign-owned Application Client checkpoint.

PR #115 completed that domain contract. Runtime/template workers can rely on
`docs/design/watcher.md`, ADR 0046, and the four schemas without reopening
detector, state, lifecycle, trust, failure, provenance, health, or message
semantics. They cannot launch until #12 dispositions produce
`application-client-ready`.

## Context fitness

The outcome and boundary sections were strong enough to prevent workflow-engine,
production packaging, and public SDK scope drift. The two-orchestrator plan gate
was valuable: six revisions resolved unsafe duplicate state advancement,
premature hardening, live-provider evidence, service-station lifecycle, and the
shared-client ownership boundary before implementation.

The launch context missed two operational prerequisites. Destructive daemon
tests needed isolation from the coordination plane from the start, and meaningful
live provider transitions needed an owned or explicitly authorized disposable
resource rather than credentials alone.

Parallel contract work added two useful controls: campaign-allocated ADR numbers
prevented shared decision-log collisions, and a byte-exact dual-orchestrator
draft gate kept the #12 export aligned with the final reviewed Watcher contract.

## Attention allocation

Operator attention was correctly concentrated on the reviewed detector protocol,
the first GitHub/Azure DevOps events, the ADO mutation-authority decision, the
shared-daemon incident, and merge readiness. The paired reviewer found a real
Windows spawn-before-Job containment race. The multi-specialist review process
also produced avoidable noise around redacted credential-bearing source and
review-object counting; future review prompts should require raw-token/AST
verification for secret-adjacent findings and identify the expected review by
marker or review ID.

The builder gate required little additional intervention: a scoped Watcher-backed
PR sentry run produced timely, quiet, canonical-checker-consistent evidence and
clean watch removal. That is the intended gate shape—real use and judgment rather
than another implementation review.

Contract review focused on genuine semantic gaps. The paired reviewer required a
pre-send ledger fence, defined event-producing results without `nextState` as
unchanged prior state, and made actionable inbound backlog force
`productionReady = false`. Those changes strengthened the domain contract
without changing the already-approved shared-client requirements.

## Inspired vs. recovery interventions

- **Inspired:** Real sender lifecycle evidence produced the stable-address /
  ephemeral-session model and explicit acceptance-versus-consumption
  requirements for #12.
- **Inspired:** Rejecting an initial ADO snapshot led to a reproducible,
  authorized PR-created transition rather than a weak provider claim.
- **Recovery:** The shared coordination daemon was restarted because the launch
  prompt lacked a test-plane isolation rule.
- **Recovery:** Live abrupt-death evidence exposed stale runtime and unfinished
  attempt rows; startup reconciliation was added before broader dogfood.
- **Recovery:** Final review found the Windows process could spawn descendants
  before Job assignment; suspended creation and resume-after-assignment closed
  the race.
- **Inspired:** Dynamic ADR allocation and byte-exact #12 draft approval allowed
  parallel domain contracts to advance without pre-allocating shared design
  numbers or accepting a shared API prematurely.
- **Recovery:** Final contract review found three underspecified semantics:
  duplicate evidence needed a pre-send fence, omitted event `nextState` needed a
  defined committed state, and send-only inbound backlog needed an explicit
  production-readiness consequence.

## Closeout observation dispositions

- Test-support helper binaries: promoted through #110 into the
  `watcher-runtime` packaging acceptance checklist.
- PR-sentry merge-during-preflight: promoted to
  `detector-template-library` bootstrap guidance and lifecycle tests.

## Promotion candidates

- Destructive daemon/upgrade tests must never use the default coordination
  plane: target authority — brief decision and future node-spec/launch guidance.
  - Disposition: landed in this brief; campaign-wide launch guidance remains
    owned by campaign orchestration.
- Long-lived applications need explicit stable-address, process-incarnation,
  strict recovery, receipt, and sender-only/bidirectional semantics: target
  authority — issue #12 / future Application Client contract.
  - Disposition: Watcher-specific semantics landed in `docs/design/watcher.md`
    and ADR 0046; exact shared requirements are published in
    [issue #12 comment](https://github.com/lossyrob/telex/issues/12#issuecomment-5042702401).
    Campaign consolidation remains required before production implementation.
- External-provider proof requires mutation authority as well as credentials:
  target authority — workstream-design lesson (`project`).
  - Disposition: deferred with rationale; apply to the next live-provider node
    spec and promote as a project habit if the pattern recurs.
- Dual workstream/campaign plan approval caught real cross-seam issues before a
  high-autonomy node started: target authority — workstream-design lesson
  (`streamliner`).
  - Disposition: deferred with rationale pending comparison with the parallel
    Operator Station spike.
- Shared Watcher supervision can replace a session-owned PR sentry loop for
  scoped PAW dogfood while a one-shot canonical checker remains authoritative:
  target authority — detector-template guidance and Watcher viability evidence.
  - Disposition: accepted by the builder viability gate; #110 owns the production
    contract, and the template node must check terminal state immediately before
    watch registration so merge-during-preflight creates no stale supervisor.
