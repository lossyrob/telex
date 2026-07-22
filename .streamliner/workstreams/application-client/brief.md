# Telex Application Client (supported long-lived application integration)

## Purpose

Create one supported Telex Application Client for long-lived non-agent
applications. The workstream owns contract convergence and implementation of the
shared lifecycle, capability, messaging, identity, recovery, and backend
semantics required by Operator Station and Telex Watcher without creating
product-specific client forks.

Issue [#12](https://github.com/lossyrob/telex/issues/12) remains the sole
semantic contract owner. This workstream is the execution geometry around that
authority.

## Approach

The first confidence transition is contract convergence, not implementation.
The `contract-convergence` node reconciles the merged Watcher and Operator
requirements, records accepted/deferred/rejected dispositions for every input,
lands one API-neutral normative contract, and publishes the
`application-client-ready` checkpoint through issue #12.

That checkpoint means the semantic contract is accepted. It unblocks detailed
promotion and coordinated execution of product nodes, but it does not claim the
supported client implementation is complete and never permits a private
fallback. Later waves implement a shared client core, the first supported
binding, and a conformance harness before consumer integration and operational
hardening.

The richer formation rationale and requirement-family map are preserved in
[`docs/initial-shaping.md`](docs/initial-shaping.md).

## Design References

- `telex:docs/design/index.md` - intended-system design entry point.
- `telex:docs/design/daemon.md` - membership, liveness, delivery, receipt,
  restart, authorization, and backend authority.
- `telex:docs/design/watcher.md` - accepted Watcher domain contract and
  shared-client requirements.
- `telex:docs/design/operator-station.md` - accepted Station/operator domain
  contract and AC-01 through AC-15.
- `telex:docs/design/DECISIONS.md` - ADR 0046 through ADR 0048 and the
  campaign-allocated ADR 0049 once the accepted Application Client contract
  lands.
- `telex:PRODUCT-THESIS.md` - durable responsibility, store-and-forward, and
  workflow-engine boundaries.
- `telex:.streamliner/shaping/roadmap.md` - Addressable Attention campaign
  staging and shared-seam ownership.
- `telex:.streamliner/workstreams/telex-watcher/brief.md` - send-only consumer
  and no-private-fallback requirement.
- `telex:.streamliner/workstreams/operator-station/brief.md` - bidirectional
  human-loop consumer and exact-delivery/recovery requirements.

## Boundaries

- **In scope:** one API-neutral semantic contract; stable application
  responsibility and ephemeral runtime identity; send-only and bidirectional
  capability; attach/reconcile/detach and compensation; typed membership loss;
  process liveness; explicit sender selection; typed receipts; receive and
  exact-delivery acknowledgment; reply and per-recipient disposition;
  retry-safe operations; unresolved/history queries; logical-store and source
  identity; health and delta ordering; backend/profile selection; first
  supported core and binding; conformance and packaging.
- **Out of scope:** Operator Station UX, notification, or mediation policy;
  Watcher detector, provider, scheduler, or event-state behavior; general
  workflow execution; a daemon wire redesign unless the accepted semantics prove
  a missing core primitive; product-specific private client forks.
- **Deferred:** additional language bindings beyond the first supported
  consumer set; remote hosted client gateways; multi-host active/active
  responsibility; cryptographic cross-principal identity beyond backend
  provenance; broad SDK ergonomics unrelated to the accepted application
  contract.

## Current State

The workstream is part of the
**[Addressable Attention campaign #102](https://github.com/lossyrob/telex/issues/102)**
and is tracked by parent issue
[#117](https://github.com/lossyrob/telex/issues/117).

Both consumer domain contracts are merged and durably reconciled:

- Watcher #110 / PR #115, ADR 0046, four canonical schemas, requirements
  [export](https://github.com/lossyrob/telex/issues/12#issuecomment-5042702401),
  and merged-source
  [addendum](https://github.com/lossyrob/telex/issues/12#issuecomment-5043498697).
- Operator Station #114 / PR #116, ADR 0047/0048, corrected requirements
  [export](https://github.com/lossyrob/telex/issues/12#issuecomment-5042612298),
  and merged-source
  [addendum](https://github.com/lossyrob/telex/issues/12#issuecomment-5044388908).

The builder explicitly approved retaining this workstream and
`contract-convergence` node
[#118](https://github.com/lossyrob/telex/issues/118) after the campaign's scope
review. The node is active in planning. Planning-reviewed plan revision 14 is
committed at `626a80a`; requests sent before the scope pause were closed, and a
fresh exact-plan request must receive both Application Client and campaign
approval before contract work begins.

Campaign orchestration allocated ADR 0049 for the shared API-neutral semantic
boundary. The number is reserved but the ADR is not yet landed; use remains
gated on exact plan approval and latest-main collision revalidation. Issue #12
remains the sole semantic owner and has not published
`application-client-ready`.

The `application-client-ready-gate` is pending. All later client, binding,
conformance, integration, and hardening nodes remain planned and blocked on
that semantic checkpoint.

## Decisions

- **Issue #12 remains the sole contract owner:** the workstream executes and
  maintains that issue's authority rather than replacing it with a competing
  tracker.
- **One semantic contract with explicit capabilities:** send-only and
  bidirectional applications share lifecycle, identity, receipt, recovery, and
  backend semantics while exposing only supported operations.
- **Contract before bindings:** the first node is API-neutral. The old
  TypeScript sketch on issue #12 is historical input until implementation work
  selects supported surfaces.
- **Independent domain exports remain evidence:** convergence preserves their
  provenance and stronger product pressure instead of rewriting history.
- **`application-client-ready` is a semantic checkpoint:** it unblocks detailed
  product-node promotion and coordinated implementation, not a claim that the
  client library is already shipped.
- **No private fallback:** a deferred or rejected required semantic blocks the
  affected consumer; CLI parsing, raw daemon IPC, spike helpers, and
  product-private clients are not allowed substitutes.
- **Consumer review is mandatory:** both Operator Station and Watcher
  orchestrators review the final contract bundle before campaign acceptance.
- **Builder approved the workstream shape:** #117 and #118 remain the execution
  geometry around issue #12; the scope pause is closed, but no pause-era plan
  approval carries forward.
- **ADR 0049 is allocated, not accepted:** campaign reserved the number for the
  shared semantic boundary; the worker may use it only after the exact plan gate
  and latest-main collision check.
- **Shared artifacts follow primary-main ownership:** workers use feature
  worktrees; only the Application Client workstream orchestrator reconciles this
  brief, graph, and related Streamliner state from the primary main checkout.

## Open Questions

- After semantic acceptance, should the supported Rust client core and first
  binding be one PAW-sized node or sequential nodes with a stable core export?
- Is TypeScript/napi-rs the first required binding for both Station and SDK use,
  or should the first implementation expose Rust plus a narrower TypeScript
  surface and defer other languages?
- Which conformance evidence is required before product integration PRs may
  merge, beyond the earlier semantic `application-client-ready` checkpoint?

## Imports and Exports

### Imports

- Merged Watcher and Operator contracts, requirements exports, and addenda.
- Daemon/local-exchange membership, liveness, receipt, restart, authorization,
  and backend semantics.
- Campaign authority to disposition issue #12 requirements and publish
  `application-client-ready`.

### Exports

- One accepted API-neutral Application Client semantic contract.
- Per-requirement dispositions and crosswalk on issue #12.
- The `application-client-ready` checkpoint consumed by Operator Station and
  Telex Watcher.
- A supported client core, first binding, and conformance evidence for later
  product integration.
- Explicit migration guidance away from temporary CLI, raw-IPC, and spike
  integration seams.

## Closeout Observations

Keep API convenience, additional bindings, and consumer-specific ergonomics out
of the contract node unless they expose a missing semantic. Any requirement that
cannot be accepted must name its blocked consumer and owner rather than being
softened into ambiguous shared wording.
