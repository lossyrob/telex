# Telex Application Client - Reconciliation

## What changed

Formation expected one contract-convergence node to establish the shared semantic
boundary, publish `application-client-ready`, and unblock later implementation.
The product contract did land, but the first PR path mixed worker implementation
with workstream approval, publication, and human-floor machinery. Polluted PR #123
was closed without merge. Clean replacement PR #126 rebuilt the intended six product
documents from current `main` in one product-only commit and merged as `62c2b23`.

The merged design keeps `docs/design/application-client.md` as the sole normative
semantic authority. Requirements mapping moved to
`docs/notes/application-client/requirements-crosswalk.md` as durable,
non-normative traceability/provenance.

## Boundaries

- **Held:** One shared API-neutral contract serves Watcher and Operator Station;
  product-specific policy stays outside the client; no private fallback seam
  landed.
- **Leaked:** The worker session took ownership of workstream gates, exact approval
  state, publication, sentry, and human-floor routing, then committed that evidence
  to the product branch.
- **Expanded:** Repository information architecture was clarified: long-lived
  intended-system design remains under `docs/design`, while workstream
  requirements traceability lives under `docs/notes`.

## Contracts and exports

The contract, ADR 0049, 30-row traceability note, W-15 repair, historical issue
snapshot, and manifest landed cleanly. The bundle manifest is
`085deed89cef1741fb6967bbd9f5e87e4f9cf104917518a234006c35b0f62296`.

The `application-client-ready` export is not fully reconciled yet. Issue #12 still
points at the superseded crosswalk path and prior manifest identity. That checkpoint
update belongs to the workstream/gate authority, not an implementation worker.

Issue #124 preserves the non-blocking W-05 taxonomy wording follow-up. Its dependency
on core or conformance work remains to be shaped.

## Context fitness

The original worker context over-specified gate mechanics and blurred Streamliner
state with PAW implementation state. `commit-and-clean` did not protect the branch
after later planning recovery recommitted `.paw` artifacts. Long-running lifecycle
waiters also conflicted with Telex push delivery and deferred a large message backlog.
The legacy #118 sessions were launched before the broker protocol, so no matching
Application Client run appears in the Streamliner runtime registry.

The configured v2 launch profiles now state the boundary explicitly: workers execute
the selected node mission, keep workflow evidence off the product branch, and route
gates, publication, human attention, and workflow rewinds to orchestration.

## Attention allocation

Operator attention was pulled into recovery from approval-loop recursion, branch
pollution, stale routing, and Telex queue congestion instead of judging the product
contract. The clean repair succeeded once the operator reviewed a normal product-only
PR. Future attention should concentrate on design shape, checkpoint acceptance, and
downstream node geometry.

## Inspired vs. recovery interventions

- **Inspired:** Moving the crosswalk to `docs/notes` came from concrete operator
  review of the repository's information architecture and improved the durable
  product shape.
- **Recovery:** Freezing the campaign, replacing PR #123, and updating worker
  protocols corrected a work-design failure: implementation sessions had been given
  workstream-state authority they should not hold.
- **Recovery:** Replacing background PR/lifecycle waiters with Telex-driven review
  handshakes avoided starving the same push channel used for coordination.

## Closeout observation dispositions

- Issue #12 stale supporting-traceability link and manifest identity: promoted to
  the pending `application-client-ready-gate`; operator/orchestrator action required.
- W-05 taxonomy wording: promoted to issue #124; open.
- Polluted PR #123 forensic branch/worktree: deferred with rationale until explicit
  operator cleanup authorization.
- Clean #118 worktree/branch cleanup: deferred with rationale until explicit
  operator cleanup authorization.
- Legacy #118 implementer/reviewer Telex stations remain attached after completion:
  runtime detach/closeout is pending.
- Worker-gate boundary and branch-evidence pollution: reflected in v2 launch
  profiles and the orchestrator node-launch protocol.

## Promotion candidates

- Node workers must never satisfy or simulate workstream gates inside an
  implementation mission. Target authority - workstream-design lesson
  (`streamliner`).
  - Disposition: landed in configured v2 launch profiles and launch protocol.
- Approval, publication, human-floor, and reconciliation evidence should remain
  off product branches unless the product explicitly requires it. Target authority -
  node-spec guideline.
  - Disposition: landed in configured v2 launch profiles; apply to future node specs.
- Workstream design should review intended-system documentation placement before
  final checkpoint publication. Target authority - workstream-design lesson
  (`project`).
  - Disposition: landed for Application Client in PR #126; broader promotion
    deferred pending corroboration from another workstream.
