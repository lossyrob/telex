# Telex Application Client - Reconciliation

## What changed

Client-core implementation completed through
[PR #132](https://github.com/lossyrob/telex/pull/132) at exact reviewed head
`6fd3c1133948f0cefe83948947f2d85d2db0a298`, merged to `main` as
`4ecbe84e99e00ab0cea3bcf3619d539c222746af` on 2026-08-27.
[Final delta review 5045578247](https://github.com/lossyrob/telex/pull/132#pullrequestreview-5045578247)
reported `+1` with no findings, all six exact-head checks succeeded, and no
review thread remained unresolved. Issues #129 and #124 closed as completed
with the merge.

The merged product implements the supported typed Application Client core
across SQLite and Postgres without a Watcher- or Station-private seam. The
regenerated `docs/design/application-client.bundle.json` is 2,423 bytes, Git
blob `231cfd231d2a343b59d8538a08002087b0f17aa8`, and SHA-256
`9dbc5cf90b917f602de8c2430438ed6f57d529893f8aaaa52f3996b51330252e`.

The first supported Rust binding completed through issue #149 and PR #151.
Exact reviewed head `c03db454781164f47a20e997665fe1251e07bd15`
merged as `ddedfab57cc305a1e91a81d7e49e712bb36d32fd`; issue #149
closed as completed at `2026-09-02T13:56:22Z`. Full PAW review
[5052221269](https://github.com/lossyrob/telex/pull/151#pullrequestreview-5052221269),
delta review
[5052369562](https://github.com/lossyrob/telex/pull/151#pullrequestreview-5052369562),
all six exact-head checks, zero unresolved review threads, and
[field report 5454362721](https://github.com/lossyrob/telex/issues/149#issuecomment-5454362721)
provide the completion evidence. Issue #12 publication revision 4 records the
binding.

Issue [#152](https://github.com/lossyrob/telex/issues/152) and its bundle-first
task now make `client-conformance` ready but unlaunched. One node and delivery
PR own the ten accepted conformance families across isolated SQLite and
credentialed Postgres, public-only Watcher send-only and Station bidirectional
fixtures, missing shared-semantic repair, and temporary-seam replacement
guidance. The issue body is ASCII and has SHA-256
`D37C020B141801A106B7089EC61748C9810AEC6049EFE58C80227C576FA66AD0`.

The accepted joint consumer-gate adjudication defines
`consumer-integration-gate` as a pre-product-integration exact-head
consumability attestation. After implementation review and required CI pass on
one conformance head, Watcher and Operator Station independently validate that
same public bundle against their merged contracts. Product implementation
evidence does not belong in the gate. Both consumer nodes retain direct
conformance dependencies and also wait on the gate, producing the acyclic order
`client-conformance -> consumer-integration-gate -> watcher-runtime-core` and
`client-conformance -> consumer-integration-gate -> station-app`. The gate,
`supported-client`, and every later checkpoint remain planned. Reconciliation
does not launch the node or pass the gate.

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

Issue #12 publication revision 4 preserves that checkpoint history and records
the supported Rust core and first supported Rust binding. It does not claim
completed conformance or authorize consumer integration.

Issue #124 completed within client-core issue #129 rather than becoming a
separate node. PR #132 documented the extensible typed membership-loss model,
aligned W-05 and AC-C05, regenerated the manifest metadata, and retained
explicit downstream conformance coverage.

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
- W-05 taxonomy wording: issue #124 completed within client-core issue #129; no
  separate node or session was created.
- First supported Rust binding: issue #149 completed through PR #151 and is
  published in issue #12 revision 4.
- Client conformance: issue #152 and its reviewed task make the node ready but
  unlaunched as one complete backend-parity and consumer-fixture bundle.
- Consumer integration: the gate remains planned and requires independent
  Watcher and Station attestations on the same reviewed and green conformance
  head; product implementation evidence remains downstream.
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
