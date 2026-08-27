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

The contract-convergence node is complete. Clean replacement
[PR #126](https://github.com/lossyrob/telex/pull/126) merged to `main` as
`62c2b23` and closed
[#118](https://github.com/lossyrob/telex/issues/118). The merged product state:

- adds the API-neutral Application Client contract and ADR 0049;
- preserves the exact pre-convergence issue #12 body;
- keeps `docs/design/application-client.md` as the sole normative semantic
  authority;
- stores the 30-row requirements mapping under
  `docs/notes/application-client/requirements-crosswalk.md` as durable,
  non-normative traceability/provenance;
- preserves 30 accepted, 0 deferred, 0 rejected mappings and the W-15
  status-provenance repair;
- adds the design-only bundle manifest with SHA-256
  `085deed89cef1741fb6967bbd9f5e87e4f9cf104917518a234006c35b0f62296`.

Polluted [PR #123](https://github.com/lossyrob/telex/pull/123) was closed
without merge and remains preserved for protocol forensics. Its branch and
dirty forensic worktree must not be removed without explicit cleanup
authorization.

Issue [#12](https://github.com/lossyrob/telex/issues/12) remains the semantic
contract owner. Publication revision 2 now points to clean PR #126 authority:
the normative contract and ADR 0049 at merge `62c2b23`, non-normative
traceability at `docs/notes/application-client/requirements-crosswalk.md`,
manifest blob `25f27401100a89b1e90dba46b44973a3e3d43908`, and SHA-256
`085deed89cef1741fb6967bbd9f5e87e4f9cf104917518a234006c35b0f62296`.
The `application-client-ready` checkpoint and gate are complete.

The supported `client-core` node completed through
[#129](https://github.com/lossyrob/telex/issues/129),
[#124](https://github.com/lossyrob/telex/issues/124), and
[PR #132](https://github.com/lossyrob/telex/pull/132). Exact reviewed head
`6fd3c1133948f0cefe83948947f2d85d2db0a298` merged as
`4ecbe84e99e00ab0cea3bcf3619d539c222746af` after
[delta review 5045578247](https://github.com/lossyrob/telex/pull/132#pullrequestreview-5045578247),
six successful checks, and zero unresolved review threads. Both issues closed
as completed.

The merged core implements AC-C01 through AC-C20 across SQLite and Postgres and
retains one supported typed boundary without a product-private fallback. The
regenerated `docs/design/application-client.bundle.json` is 2,423 bytes, Git
blob `231cfd231d2a343b59d8538a08002087b0f17aa8`, and SHA-256
`9dbc5cf90b917f602de8c2430438ed6f57d529893f8aaaa52f3996b51330252e`.

`first-binding` is selected, tracked by
[#149](https://github.com/lossyrob/telex/issues/149), ready, and focus-level but
unlaunched. The operator selected a Rust-first binding in the root `telex`
crate, preserving `telex::application_client`. The task must publish supported
`default-features = false` consumer profiles and define compatibility, runtime,
and cancellation behavior without widening the stable surface beyond
contract-bearing Rust types. `client-conformance`, the consumer integration
gate, operational hardening, and closure gate remain unchanged on their declared
dependencies. Tracker creation alone does not authorize launch. The existing
operator decision authorizes routine launch only after the reviewed Tier B
packet lands on `main` and launch preparation validates the exact main, tracker,
task, and session inputs. Reconciliation itself does not launch.

## Decisions

- **Issue #12 remains the sole contract owner:** the workstream executes and
  maintains that issue's authority rather than replacing it with a competing
  tracker.
- **One semantic contract with explicit capabilities:** send-only and
  bidirectional applications share lifecycle, identity, receipt, recovery, and
  backend semantics while exposing only supported operations.
- **Contract before bindings:** the first node is API-neutral. The historical
  TypeScript sketch on issue #12 remains evidence, not contract authority.
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
- **ADR 0049 is accepted:** clean PR #126 landed the shared API-neutral semantic
  boundary and issue #12 publication revision 2 cites that clean authority.
- **Shared artifacts follow primary-main ownership:** workers use feature
  worktrees; only the Application Client workstream orchestrator reconciles this
  brief, graph, and related Streamliner state from the primary main checkout.
- **Traceability is supporting evidence, not design authority:** the requirements
  crosswalk lives under `docs/notes`; `docs/design/application-client.md` alone
  defines the intended Application Client semantics.
- **Workers execute node missions, not workstream gates:** checkpoint, campaign
  approval, issue-publication, human-attention, and workflow-rewind state remain
  orchestrator-owned. PAW plans, ledgers, snapshots, transcripts, and approval
  evidence stay off product branches unless explicitly named as deliverables.
- **Future node launches use the Streamliner launch broker:** the workstream's v2
  implementer/reviewer defaults are resolved and launched through
  `/api/launch-preparations/runs`, not hand-written terminal prompts.
- **Core and binding remain sequential:** `client-core` completed first;
  `first-binding` is selected, tracked by #149, and ready but unlaunched. The
  existing operator decision authorizes routine launch after the reviewed Tier B
  packet lands on `main` and launch preparation validates the exact main,
  tracker, task, and session inputs; no new operator decision is required.
- **The first binding is Rust-first:** the supported binding remains in the root
  `telex` crate at `telex::application_client`. It does not introduce a second
  crate or a language-translation boundary.
- **Application consumers opt out of package defaults:** supported consumers use
  `default-features = false` with one documented backend profile: SQLite
  (`sqlite`), Postgres (`postgres`), Postgres with Entra (`entra`, which includes
  `postgres`), or dual backend (`sqlite` plus `postgres`, optionally with
  `entra`). The `self-update` feature is not part of an application-consumer
  profile.
- **The caller owns the async runtime:** the Rust binding runs in a
  caller-provided Tokio runtime and must not create a hidden runtime, daemon, or
  sidecar. Exact ownership and API mechanics remain worker decisions.
- **Cancellation preserves durable uncertainty:** cancellation never proves that
  an operation was not accepted. Callers persist prepared `RecoveryHandle`
  evidence and reconcile uncertain operations; cancelled receive work does not
  acknowledge a delivery. Exact cancellation API shape remains a worker
  decision.
- **Only semantic Rust types stabilize:** compatibility commitments cover public
  types and behavior that carry the accepted Application Client contract. They
  do not promote backend records, daemon frames, CLI types, or consumer DTOs into
  the supported surface. Version and deprecation mechanics remain worker
  decisions within the root crate's compatibility contract.
- **External boundaries remain deferred:** napi-rs/TypeScript, a separate client
  crate, C ABI, public socket or sidecar protocols, and product DTOs require
  later decisions. This deferral does not permit a private consumer fallback.
- **Conformance remains a separate gate:** first-binding must preserve the full
  AC-C01 through AC-C20 model. It does not complete cross-backend conformance,
  consumer integration, packaging, upgrade readiness, or production hardening.
- **Issue #124 completed within client-core:** PR #132 closed both #129 and #124
  and reported the regenerated manifest identity. This factual reconciliation
  does not mutate issue #12.

## Open Questions

- What concrete Rust API, ownership, feature-alias, deprecation, and cancellation
  mechanics best satisfy the approved compatibility, runtime, and recovery
  contracts?
- Which conformance evidence is required before product integration PRs may
  merge, beyond the earlier semantic `application-client-ready` checkpoint?
- Which external language or process boundary, if any, should follow Rust after
  consumer architecture is authoritative?

## Imports and Exports

### Imports

- Merged Watcher and Operator contracts, requirements exports, and addenda.
- Daemon/local-exchange membership, liveness, receipt, restart, authorization,
  and backend semantics.
- Campaign authority to disposition issue #12 requirements and publish
  `application-client-ready`.

### Exports

- One accepted API-neutral Application Client semantic contract.
- Per-requirement dispositions in a durable, non-normative traceability note.
- The `application-client-ready` checkpoint consumed by Operator Station and
  Telex Watcher.
- A supported client core, Rust-first binding, and conformance evidence for later
  product integration.
- Explicit migration guidance away from temporary CLI, raw-IPC, and spike
  integration seams.

## Closeout Observations

- Issue #12 publication revision 2 now reflects clean PR #126 authority; the
  `application-client-ready` checkpoint and gate are complete.
- W-05 taxonomy wording from issue #124 completed within client-core issue #129
  and merged through PR #132. The Rust-first `first-binding` task is selected,
  tracked by #149, and ready but remains unlaunched.
- Polluted PR #123 and its dirty worktree are deferred with rationale for
  protocol forensics; cleanup requires explicit operator authorization.
- The clean #118 implementation worktree is also deferred for cleanup until the
  operator requests it.
- Keep API convenience, additional bindings, and consumer-specific ergonomics
  out of contract work unless they expose a missing semantic. Any requirement
  that cannot be accepted must name its blocked consumer and owner rather than
  being softened into ambiguous shared wording.
