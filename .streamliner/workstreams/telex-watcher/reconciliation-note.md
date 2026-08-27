# Telex Watcher — Reconciliation

## 2026-08-27 minimal v2 contract and usability acceptance

Issue #133 completed through
[PR #135](https://github.com/lossyrob/telex/pull/135), merged from exact reviewed
head `a9a832b940ac95b8eb51fe04b456902d0c74d251` as
`b91e8301899351c0411d6e2e9ac5290af8a3cb4c`. The merge had an exact-head PAW
`+1`, six successful checks, clean mergeability, and zero unresolved review
threads. It promoted the minimal v2 Watcher contract, three v2 schemas, and ADR
0050 into project design authority.

The canonical integrated workstream design is now
[`design/current-design.md`](design/current-design.md). The
`minimal-watcher-authoring-contract` node is completed. The operator separately
accepted `dumb-watcher-contract-gate` as contract-usability acceptance, so the
`minimal-contract-accepted` checkpoint is completed. `minimal-example-pack` is
next-ready but has not been launched. `watcher-runtime-core` remains planned and
blocked specifically on Application Client `client-conformance`; the completed
internal gate remains in its graph dependencies as history.

The builder decision used the ordinary authoring flow in merged `watcher.md`:
write or copy a detector, optionally exercise it, register it, and inspect
diagnostics if needed. Registration requires only command, cadence, timeout,
backend, sender, and target, while other generic fields have explicit defaults.
Mandatory manifests, pinning, kind allowlists, provider preflight, downtime
declarations, and template conformance are absent from v2 registration/runtime
semantics. PR #135 carried PAW `+1` review 4825216100 and green CI. The focused
design-steward follow-up in PR #141 carried `+1` review 5044005481 and green CI.
This accepts contract usability only: it is not production runtime or proof that
a watch can be created, registered, left running, and diagnosed in five minutes.
That operational proof remains owned by the planned
`five-minute-custom-watch-gate`.

One controlled shared-client gap remains explicit: Watcher requires
authoritative exact-store/exact-operation `not-recorded` evidence and
identity-checkable same-operation retry, while the current Application Client
AC-C14 does not yet state the former. Application Client owns promotion and
conformance proof. Watcher runtime must remain query-only/blocked under
uncertainty and must not create a private fallback.

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

Application Client convergence subsequently completed through clean replacement
PR #126 and merge `62c2b23cc3d54877226f46df44d6036b7dffa380`.
The original PR #123 was closed unmerged and preserved for protocol forensics
after its worker branch accumulated PAW plans, approval ledgers, source
snapshots, and other orchestration evidence. The clean replacement retained only
the six intended product-documentation paths and moved requirements traceability
from the normative design set to `docs/notes/application-client/`.

Issue #12 now publishes `application-client-ready`, but explicitly as a
design-only checkpoint. This resolves the semantic convergence prerequisite and
permits detailed node promotion; it does not provide the supported client core,
binding, conformance, or consumer-integration export that production
`watcher-runtime` needs.

The first template-library implementation then reached a technically
merge-ready state in PR #131 after three substantive review repairs. Operator
use and document review exposed a more important product issue: the accepted
contract and node geometry had converted optional agent hardening into mandatory
Watcher ceremony. PR #131 and issue #127 were closed without merge as
superseded, preserving their branch, reviews, and implementation evidence.

The approved redesign makes Watcher a dumb persistent execution/delivery
mechanism. Registration contains an agent-authored command, generic execution
bounds, and fixed Telex routing. The script owns provider semantics and event
content. Pinning, manifests, kind policies, provider preflight, downtime
declarations, fixtures, and deep tests become optional user/project choices.
Watcher retains opaque state, diagnostics, receipt-gated commit, and owns
runtime-generated event sequence identity.

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
- **Leaked:** The first Application Client implementation session treated
  workstream gates, exact-byte approvals, evidence accounting, and workflow
  recovery as worker-owned product-branch tasks. PR #123 was closed unmerged;
  the replacement path restored the worker/orchestrator authority boundary.
- **Held:** The Watcher consumer review caught the W-15 status-provenance gap,
  and the repaired contract preserved the full send-only requirement set without
  creating a Watcher-private seam.
- **Leaked:** The production contract and template node made safety mechanisms
  that can be useful in mature projects mandatory for ordinary agent-authored
  watches. That boundary optimized a framework before proving the simplest
  authoring experience.
- **Held:** PR #131 did not leak provider semantics into the runtime and produced
  reusable examples, ordering/size fixes, and review evidence. Closing it
  unmerged is a product-direction reset, not a rejection of implementation
  quality.

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
semantics.

The Application Client semantic export is available through issue #12 and clean
PR #126. Production runtime still needs the explicit `client-conformance`
export. The minimal authoring contract and optional examples do not require the
implemented client and can proceed first.

PR #131 remains a source of optional examples and implementation learning:
provider scripts, bounded process helpers, deterministic provider ordering,
bounded metadata, recurrence regression tests, and cross-platform fixes. Its
manifest schema, required digests, paired registrations, universal checklist,
and large conformance suite are not accepted as mandatory product surface.

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

The Application Client incident exposed a larger context failure: the worker
prompt let workstream-stage mechanics become implementation scope and allowed
off-branch evidence to enter a product PR. The campaign's updated v2 launch
profiles and launch-broker protocol now make the node mission and authority
boundary explicit. Future Watcher nodes must be launched through Streamliner
rather than hand-written terminal prompts.

The Watcher reset exposed a shaping failure rather than a worker failure. The
node faithfully implemented the accepted design and issue, but those artifacts
overfit hardening evidence and did not preserve the operator's desired
"write a small script, register it, and move on" workflow. Future contract
shaping must test the shortest user journey before promoting optional safety
recipes into required schemas and gates.

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

During shared-contract convergence, Watcher attention was correctly spent on
the W-15 status-provenance omission. Campaign/operator attention then shifted to
the PR #123 protocol failure, froze the campaign, and authorized a clean
product-only repair. That was recovery work caused by authority and context
leakage, not additional Application Client product scope.

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
- **Recovery:** PR #123 mixed worker implementation with workstream gates,
  approval ledgers, exact-byte evidence, and workflow recovery. Closing it
  unmerged and rebuilding PR #126 from current `main` restored a product-only
  branch and preserved the polluted branch separately for forensics.
- **Inspired:** The repaired consumer review made logical-store identity explicit
  on Watcher status, receipt/result, and receive surfaces, closing W-15 without
  expanding the shared client into Watcher policy.
- **Inspired:** Operator feedback after the template framework became concrete
  clarified that Watcher's value is persistent execution/delivery and repairable
  diagnostics, not authoring governance.
- **Recovery:** Issue #127 and PR #131 encoded mandatory manifests, provenance,
  pinning, kind policy, downtime, preflight, and conformance because the prior
  design treated mature-project hardening as the ordinary path. The workstream
  now resets the contract before implementation continues.

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
    and ADR 0046; all 15 shared requirements are accepted by issue #12 and clean
    PR #126. Production runtime integration remains deferred until the supported
    Application Client implementation and conformance export exists.
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
  - Disposition: accepted by the builder viability gate. PR preflight remains an
    optional example or project hardening choice rather than mandatory ordinary
    registration behavior.
- Implementation workers must not own workstream gates, approval/evidence
  ledgers, or workflow rewinds: target authority — workstream-design lesson
  (`streamliner`).
  - Disposition: landed in the campaign protocol update, the Streamliner
    launch-preparation API requirement, and the configured Watcher implementer
    and reviewer v2 profiles. Future Watcher launches adopt those surfaces.
- A design-only semantic checkpoint is not an implemented consumer dependency:
  target authority — Watcher graph dependency and Application Client export.
  - Disposition: runtime now depends on Application Client
    `client-conformance`; contract and example nodes do not.
- Watcher authoring should optimize for the shortest useful loop before
  optional hardening: target authority — `docs/design/watcher.md`, a new
  superseding ADR, and v2 schemas.
  - Disposition: completed through issue #133 and merged PR #135; the builder
    accepted contract usability after PAW review 4825216100 and focused
    design-steward review 5044005481. The optional example pack is next-ready
    but unlaunched; operational proof remains a later gate.
- PR #131 implementation evidence: target authority — minimal examples and
  optional hardening recipes.
  - Disposition: preserved on the closed unmerged PR/branch. Extraction may
    proceed through the now-ready but unlaunched minimal example pack.
