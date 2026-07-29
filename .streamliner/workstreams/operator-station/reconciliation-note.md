# Operator Station — Reconciliation

## What changed

Wave 1 began as a deliberately temporary Windows vertical spike and merged as
PR #104. The product loop held, but plan review changed the live Station from
read-only inbox polling to an application-owned wait/read/ingest/ack courier so
the experiment proved honest attendance and consumption. Review also hardened
route-back recovery, disposition ordering, source identity, restart projection,
notification evidence, and the final repository layout.

The merged node proved implementation viability. The builder subsequently
passed `viability-gate` after guided dogfood exercised human escalation, routine
resolution, clarification, route-back, restart continuity, and notification
publication. The builder also chose to use the campaign orchestrator as the
live `attention:rob` mediator during normal workstream execution.

`station-contract` subsequently completed in PR #116. The accepted
`docs/design/operator-station.md`, ADR 0047, and ADR 0048 now promote the
mediated loop into intended product design. The corrected Operator Station
AC-01 through AC-15 export and merged-source addendum are published on issue
#12.

The campaign-owned `application-client-ready` checkpoint is now published.
Contract-convergence issue #118 completed through clean product-only PR #126,
merged at `62c2b23cc3d54877226f46df44d6036b7dffa380`. Polluted PR #123 was closed
without merge and preserved for protocol forensics.

The shared semantic dependency is satisfied, so `station-app` and
`operator-broker` are promotion candidates rather than externally blocked
sketches. They remain planned because the workstream has not yet decided how
their implementation should sequence against the still-pending shared client
core, binding, conformance, and consumer-integration work.

## Boundaries

- **Held:** The Station remained a separate optional application; filtering
  stayed in the operator agent; Telex core did not gain human UI or semantic
  routing; Windows/SQLite remained the spike boundary; raw and mediated threads
  stayed distinct.
- **Leaked:** Moving the report and Station late in review left authoritative
  task paths stale until the workstream orchestrator reconciled them.
- **Expanded:** Evidence grew to include Action Center publication, operator
  absence, route-back recovery, and unresolved-history stress beyond 1,000 newer
  messages. These strengthened the same viability question rather than creating
  production scope.

## Contracts and exports

The durable exports are the experimental Station and assignment under
`spike/operator-station/`, plus the evidence and Application Client requirements
in `docs/notes/operator-loop-spike-report.md`.

The accepted domain export is `docs/design/operator-station.md`, backed by ADR
0047/0048. The subprocess courier, full-history export, store fingerprint,
experimental namespace, current UI behavior, `attention.*` kinds, and
`campaignAttention` metadata remain explicitly outside the production contract.
Issue #12 remains the shared-client convergence authority.

## Context fitness

The original node boundary and engagement points were useful. Dual plan review
caught two misleading assumptions before implementation: attach plus inbox
polling was not healthy attendance, and inbox limits did not retrieve the newest
N actionable obligations. The mandatory live-demo review also caught evidence
that did not support its claims before the PR advanced.

The node spec should have named the intended final repository layout earlier.
That would have prevented the late shared-artifact path reconciliation.

For the contract wave, explicit campaign ADR allocation and exact-byte #12
draft approval were necessary because Watcher and Operator edited the same
design index, decision log, and shared-client tracker concurrently. The final
paired review also showed that replied-to findings and resolved GitHub threads
are separate merge-floor evidence.

The first Application Client PR exposed a context-authority failure: a worker
treated workstream gates, approval evidence, and workflow recovery as product
branch deliverables. The clean replacement proved that node missions must stay
separate from orchestration state, with PAW and approval evidence kept
off-branch unless the product explicitly requires it.

## Attention allocation

Operator attention was highest-leverage at plan review, the first live demo,
the domain-contract review, the exact #12 export review, and the
workstream-owned artifact reconciliations. The paired reviewer remained the
right owner for detailed lifecycle, provenance, recovery, and safety defects.

The builder's next attention belongs at production-node promotion and the later
usability gate. Campaign/operator attention is needed now to decide shared-client
implementation sequencing and to authorize the next launch. No worker launch is
requested by this reconciliation.

## Inspired vs. recovery interventions

- **Inspired:** Treating delivery/ack health and operator-agent occupancy as
  first-class Station state came from using the real loop and improved the
  product evidence.
- **Inspired:** Distinct raw and mediated threads made provenance and route-back
  behavior clearer than a single conversation model.
- **Recovery:** Replacing inbox polling with a real delivery consumer corrected
  a planning mistake that would have produced false viability evidence.
- **Recovery:** Late layout changes required an orchestrator-owned update to the
  authoritative node spec before merge.
- **Recovery:** Review found route-back/ack/disposition ordering gaps that the
  initial implementation evidence did not expose.
- **Inspired:** The cross-contract consistency pass preserved one coherent
  Watcher/Operator design layer and made the shared-client overlap explicit.
- **Recovery:** Fixed ADR assumptions and a stale #12 export required dynamic
  allocation, exact-byte approval, Class D correction, and publication
  verification before merge.
- **Recovery:** Paired review found per-recipient delivery identity,
  terminal route-back, restart-safe metadata operations, unresolved-work
  handoff, and inert-rendering gaps in the initial contract.
- **Recovery:** PR #123 mixed worker implementation with workstream gates,
  ledgers, and approval artifacts. Clean PR #126 restored a product-only change;
  future nodes must use the Streamliner launch broker and route new gates or
  workflow rewinds back to orchestration.

## Closeout observation dispositions

- Reply/disposition clarity: completed in
  [#114](https://github.com/lossyrob/telex/issues/114) at the contract level.
  `station-app` owns implementation and usability evidence.

Other deferred items remain production-contract or hardening concerns owned by
#12, #114, or `operational-hardening`.

## Promotion candidates

- Consolidate the spike's Application Client requirements with Telex Watcher
  evidence: target authority — issue #12
  - Disposition: landed as the corrected Operator Station domain export and
    merge-SHA addendum on #12; the shared semantic checkpoint is published and
    clean PR #126 is merged. Application Client orchestration still owns the
    non-semantic supporting-crosswalk link correction.
- Decide whether to promote, rename, or retire the experimental message/source
  convention: target authority — issue #114
  - Disposition: landed in `docs/design/operator-station.md`; the experimental
    namespace is retired, the production v1 application convention is accepted,
    and campaign-local conventions remain evidence only.
- Preserve the lesson that application attendance must prove delivery and
  consumption, not database visibility: target authority — workstream-design
  lesson (`project`)
  - Disposition: landed in this reconciliation note, the spike report, the
    Operator Station contract, and AC-04/AC-05.
- Keep worker missions separate from workstream gates and workflow evidence:
  target authority — workstream-design lesson (`streamliner`)
  - Disposition: landed in the campaign's updated launch protocol and v2
    Operator implementer/reviewer defaults; future launches use Streamliner's
    launch-preparations API.
