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

`station-contract` is now ready as issue #114. It owns promotion from
experimental evidence into accepted product design.

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

The subprocess courier, full-history export, store fingerprint, experimental
namespace, current UI behavior, `attention.*` kinds, and `campaignAttention`
metadata are explicitly not production contracts. Issue #12 and #114 remain
the promotion authorities.

## Context fitness

The original node boundary and engagement points were useful. Dual plan review
caught two misleading assumptions before implementation: attach plus inbox
polling was not healthy attendance, and inbox limits did not retrieve the newest
N actionable obligations. The mandatory live-demo review also caught evidence
that did not support its claims before the PR advanced.

The node spec should have named the intended final repository layout earlier.
That would have prevented the late shared-artifact path reconciliation.

## Attention allocation

Operator attention was highest-leverage at plan review, the first live demo, and
the workstream-owned artifact reconciliation. The paired reviewer remained the
right owner for code-level lifecycle, provenance, and evidence defects. The
builder's next attention belongs at #114's product-contract reviews, especially
routing modes, provenance, notification posture, and the reply/disposition UX.
Campaign attention belongs at the #12 seam review.

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

## Closeout observation dispositions

- Reply/disposition clarity: promoted to
  [#114](https://github.com/lossyrob/telex/issues/114). The production contract
  must make terminal obligation state explicit and evaluate a combined
  **Reply & Handle** flow.

Other deferred items remain production-contract or hardening concerns owned by
#12, #114, or `operational-hardening`.

## Promotion candidates

- Consolidate the spike's Application Client requirements with Telex Watcher
  evidence: target authority — issue #12
  - Disposition: ready for promotion; both product viability gates have passed.
    #114 will publish the exact Operator Station requirements while campaign
    orchestration owns the shared contract/checkpoint.
- Decide whether to promote, rename, or retire the experimental message/source
  convention: target authority — issue #114
  - Disposition: promoted to #114; the contract node will accept, revise, or
    reject the spike and campaign-local conventions.
- Preserve the lesson that application attendance must prove delivery and
  consumption, not database visibility: target authority — workstream-design
  lesson (`project`)
  - Disposition: landed in this reconciliation note and the spike report;
    promote further only if another Telex application workstream corroborates it.
