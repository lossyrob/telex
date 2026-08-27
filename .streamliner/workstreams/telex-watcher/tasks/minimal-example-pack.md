# Minimal optional detector examples

- **Workstream:** `telex-watcher`
- **Node:** `minimal-example-pack`
- **Type:** implementation
- **Status:** ready, unlaunched
- **Attention:** focus
- **Depends on:** completed `dumb-watcher-contract-gate`
- **Tracker:** [lossyrob/telex#144](https://github.com/lossyrob/telex/issues/144)
- **Parent workstream:** [lossyrob/telex#100](https://github.com/lossyrob/telex/issues/100)
- **Campaign:** [Addressable Attention #102](https://github.com/lossyrob/telex/issues/102)

## Outcome

Ship small, copyable Watcher detector examples for GitHub, Azure DevOps,
HTTP/JSON, file, and command conditions. Each example helps an agent start from
the accepted minimal command-plus-policy v2 contract without introducing a
mandatory template framework.

This node produces optional teaching material. It does not prove production
runtime behavior or satisfy the five-minute operational gate.

## Current authority

- The [canonical workstream design](../design/current-design.md) integrates the
  accepted Watcher boundaries and downstream gates.
- The [Watcher contract](../../../../docs/design/watcher.md) defines the
  provider-neutral runtime, trusted same-user detector boundary, fixed routing,
  receipt-gated state, runtime-owned event identity, and optional-example role.
- [ADR 0046](../../../../docs/design/DECISIONS.md#0046--watcher-runs-provider-neutral-trusted-local-detectors-with-receipt-gated-state)
  retains the provider-neutral, trusted-local, fixed-route, receipt-gated
  architecture.
- [ADR 0050](../../../../docs/design/DECISIONS.md#0050--watcher-v2-uses-minimal-command-registration-and-runtime-owned-event-identity)
  supersedes mandatory authoring ceremony and detector-authored event identity.
- The canonical v2 contracts are
  [registration](../../../../docs/design/schemas/watcher-registration-v2.schema.json),
  [detector request](../../../../docs/design/schemas/watcher-detector-request-v2.schema.json),
  and
  [detector result](../../../../docs/design/schemas/watcher-detector-result-v2.schema.json).
- The [Application Client contract](../../../../docs/design/application-client.md)
  owns the supported Telex integration seam. This node does not implement or
  replace it.
- [PR #142](https://github.com/lossyrob/telex/pull/142), merged as
  `c13bad6441e74771280770793e76a302aafc6388`, records the completed
  `dumb-watcher-contract-gate` dependency and current workstream state.

Closed [issue #127](https://github.com/lossyrob/telex/issues/127) and closed
unmerged [PR #131](https://github.com/lossyrob/telex/pull/131) are learning
sources only. They are not product or node authority. Extract useful examples,
bounded helpers, tests, or cross-platform fixes selectively; do not restore
their mandatory template, provenance, kind-policy, preflight, downtime, or
test-framework obligations.

## Deliverables

- Small examples for GitHub, Azure DevOps, HTTP/JSON, file, and command
  conditions.
- Short guidance for copying, configuring, running, and testing each example.
- Examples that use the accepted v2 detector request/result schemas and keep
  provider policy in trusted same-user scripts.
- Focused tests or fixtures that prove the shipped examples follow the public
  detector contract.

## Boundaries

### In scope

- Provider-specific observation logic in editable local scripts.
- Opaque detector state and provider-owned cursor or replay choices where an
  example needs them.
- Honest credential, timeout, and failure guidance specific to each example.
- Selected implementation learning from PR #131 that remains valid under the
  merged v2 design.

### Out of scope

- Mandatory manifests, script pinning, digest policy, kind allowlists, provider
  preflight, downtime declarations, or a prescribed template/test framework.
- Watcher runtime, registry, scheduler, lifecycle, CLI, packaging, or service
  installation.
- Production-runtime or five-minute operational acceptance.
- Workflow actions, provider mutation, remote executable registration, hosted
  ingestion, or sandbox claims.
- A private Watcher-specific Telex client or fallback around the shared
  Application Client.
- Streamliner state, approval records, or orchestration artifacts as product
  deliverables.

## Required invariants

- Watcher routing remains fixed by registration; detector output cannot choose
  sender, target, attention, or workflow action.
- Runtime-owned permanent watch identity and monotonic event sequence remain
  authoritative.
- Event-producing state remains gated on durable acceptance. Uncertain
  acceptance uses exact-store/exact-operation reconciliation, authoritative
  `not-recorded`, identity-checkable exact-same-operation retry, and query-only
  recovery.
- The shared Application Client boundary remains mandatory; examples must not
  introduce a private fallback.
- Repeated identical observations are new occurrences after prior state commits;
  examples must not silently impose product-wide deduplication policy.

## Success criteria

- Each required category has a concise, copyable example and matching usage
  guidance.
- An agent can adapt an example without changing Watcher runtime code or
  adopting optional hardening ceremony.
- Examples and fixtures conform to the merged v2 schemas and preserve opaque
  state, fixed routing, runtime-owned identity, and receipt-gated state
  semantics.
- Provider-specific cursor, replay, credentials, and test choices remain local
  to the example instead of becoming registration requirements.
- The change introduces no runtime implementation, private client seam,
  workflow action surface, or operational-gate claim.
- Directly related documentation accurately presents examples as optional
  teaching material.

## Validation and review

Run the smallest existing checks that cover the example formats, fixtures,
links, and any executable scripts. Keep generated PAW, Streamliner, and scratch
artifacts out of the product diff. Apply the campaign clear-writing policy to
the PR description, durable documentation, and field report. Final acceptance
requires exact-head review, green required checks, resolved actionable threads,
and separate operator merge authorization.

The node remains ready but unlaunched until the campaign separately authorizes
launch. Its implementation must not change `watcher-runtime-core`,
Application Client `client-conformance`, or
`five-minute-custom-watch-gate` status.
