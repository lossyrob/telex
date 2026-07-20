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

## Attention allocation

Operator attention was correctly concentrated on the reviewed detector protocol,
the first GitHub/Azure DevOps events, the ADO mutation-authority decision, the
shared-daemon incident, and merge readiness. The paired reviewer found a real
Windows spawn-before-Job containment race. The multi-specialist review process
also produced avoidable noise around redacted credential-bearing source and
review-object counting; future review prompts should require raw-token/AST
verification for secret-adjacent findings and identify the expected review by
marker or review ID.

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

## Closeout observation dispositions

None yet. The brief's `Closeout Observations` section remains a parking lane for
later viability and production dogfood.

## Promotion candidates

- Destructive daemon/upgrade tests must never use the default coordination
  plane: target authority — brief decision and future node-spec/launch guidance.
  - Disposition: landed in this brief; campaign-wide launch guidance remains
    owned by campaign orchestration.
- Long-lived applications need explicit stable-address, process-incarnation,
  strict recovery, receipt, and sender-only/bidirectional semantics: target
  authority — issue #12 / future Application Client contract.
  - Disposition: exported through the spike report; campaign consolidation into
    #12 is required before production implementation.
- External-provider proof requires mutation authority as well as credentials:
  target authority — workstream-design lesson (`project`).
  - Disposition: deferred with rationale; apply to the next live-provider node
    spec and promote as a project habit if the pattern recurs.
- Dual workstream/campaign plan approval caught real cross-seam issues before a
  high-autonomy node started: target authority — workstream-design lesson
  (`streamliner`).
  - Disposition: deferred with rationale pending comparison with the parallel
    Operator Station spike.
