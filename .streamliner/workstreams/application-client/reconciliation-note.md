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

The `application-client-ready` export is now reconciled. Issue #12 publication
revision 2 points to clean PR #126 authority, the non-normative requirements
traceability location, manifest blob
`25f27401100a89b1e90dba46b44973a3e3d43908`, and SHA-256
`085deed89cef1741fb6967bbd9f5e87e4f9cf104917518a234006c35b0f62296`.

Issue #124 is folded into promoted client-core issue #129 rather than becoming a
separate node. The core must choose and document the extensible typed
membership-loss model, align W-05 and AC-C05, regenerate affected manifest
metadata, and preserve explicit downstream conformance coverage.

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

- Issue #12 stale supporting-traceability link and manifest identity: resolved in
  publication revision 2; `application-client-ready-gate` completed.
- W-05 taxonomy wording: issue #124 remains open but is folded into promoted
  client-core issue #129; no separate node or session.
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
