# Operator Station — Reconciliation

## What changed

Wave 1 began as a deliberately temporary Windows vertical spike and merged as
PR #104. The product loop held, but plan review changed the live Station from
read-only inbox polling to an application-owned wait/read/ingest/ack courier so
the experiment proved honest attendance and consumption. Review also hardened
route-back recovery, disposition ordering, source identity, restart projection,
notification evidence, and the final repository layout.

The merged node proves implementation viability only. Builder dogfood at
`viability-gate` remains the next confidence transition.

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
namespace, and current UI behavior are explicitly not production contracts.
Issue #12 and `station-contract` remain the promotion authorities.

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
builder's next attention belongs at the viability gate, using the merged loop
with real sessions rather than reviewing more implementation detail.

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

None. The current deferred items are production-contract, hardening, or
validation concerns owned by #12, `station-contract`, `operational-hardening`,
or the builder viability gate rather than bounded closeout polish.

## Promotion candidates

- Consolidate the spike's Application Client requirements with Telex Watcher
  evidence: target authority — issue #12
  - Disposition: deferred with rationale; campaign staging requires both
    viability reports and independent gate outcomes before accepting the shared
    client contract.
- Decide whether to promote, rename, or retire the experimental message/source
  convention: target authority — `station-contract`
  - Disposition: deferred with rationale; the builder viability gate must first
    establish that the mediated interaction is worth productionizing.
- Preserve the lesson that application attendance must prove delivery and
  consumption, not database visibility: target authority — workstream-design
  lesson (`project`)
  - Disposition: landed in this reconciliation note and the spike report;
    promote further only if another Telex application workstream corroborates it.
