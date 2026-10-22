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
contract owner. Publication revision 4 preserves the revision 2
`application-client-ready` checkpoint, records the revision 3 supported Rust
core, and publishes the first supported Rust binding merged through PR #151.
The `application-client-ready` checkpoint and gate remain complete.

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

The Rust-first `first-binding` completed through
[#149](https://github.com/lossyrob/telex/issues/149) and
[PR #151](https://github.com/lossyrob/telex/pull/151). Exact reviewed head
`c03db454781164f47a20e997665fe1251e07bd15` merged as
`ddedfab57cc305a1e91a81d7e49e712bb36d32fd`. Full PAW review
[5052221269](https://github.com/lossyrob/telex/pull/151#pullrequestreview-5052221269)
and delta review
[5052369562](https://github.com/lossyrob/telex/pull/151#pullrequestreview-5052369562)
covered the binding; all six exact-head checks succeeded and no review thread
remained unresolved. The
[field report](https://github.com/lossyrob/telex/issues/149#issuecomment-5454362721)
records the implementation and validation evidence. Issue #149 closed as
completed at `2026-09-02T13:56:22Z`.

`client-conformance` is tracked by
[#152](https://github.com/lossyrob/telex/issues/152) and is in progress as the
campaign main effort. One node, tracker, and delivery PR must prove all ten
conformance
families through the public Rust surface across SQLite and credentialed
Postgres. The same PR owns public-only Watcher send-only and Operator Station
bidirectional fixtures, any missing shared-semantic repair, and guidance for
replacing temporary consumer seams. Runtime fixtures use production
`InstalledCurrent { trusted_root }` daemon bootstrap under the Local
Daemon-owned selector-admission and process-authentication contract; compile-only
fixtures are insufficient.

`consumer-integration-gate` remains planned. After conformance review and
required CI pass on one exact head, Watcher and Operator Station independently
attest that same bundle against their merged contracts and confirm no private
seam is required. Product implementation, usability, packaging, and operational
evidence remain downstream. Watcher runtime and Station app retain direct
conformance dependencies and also wait on the consumer gate. No checkpoint or
gate advances with this shaping reconciliation.

On 2026-10-08 the operator approved issue #152 as the campaign main effort,
recorded in campaign directive
`campaign-postrelease-execution-directive-20261008.json` (SHA-256
`a4d98af7d891efe7164839c79b4431d07acfc1ceef606c54e46b82ab7eb5613d`). The
schema-3 recovery release that had excluded this node is complete, so that
exclusion has ended. The same issue #152 writer resumed on
`feature/client-conformance` at preserved clean head
`5fdd95c2c32cde08a6c936171b208bbf14f6677c`. That head holds the 2026-09-02
implementation and review-fix commits and merges main `ed417c6b`. No PR existed
then. The writer is merging current main
`2c084873719080b812e0d0c4bf92125afa751499` into the branch; full validation,
exact-head review, required CI, and design inspection follow on the integrated
head. The accepted trusted `InstalledCurrent` policy is unchanged and gains no
executable-digest requirement or unsafe fallback. This records activation only;
no new source, proof, review, or merge evidence is claimed.

Later on 2026-10-08 the operator chose "Authorize source-only 0.3.0
transition in #152 (Recommended)", recorded in campaign decision
`client-conformance-semver-decision-20261008.json` (SHA-256
`ec6fadc0a99628bd6bcedee912bf2c15ed7d20768d6c17bd2c72117913d0a8fc`).
Campaign verified that published v0.2.1 source
`212b76a4c586101bdc2a53264e2a4c3e2326671a` has root
package version 0.2.1, an exhaustive `ApplicationClientError` in
`src/application_client.rs`, and a `docs/application-client-core.md` that
still says no published release contains the binding. The issue #152 PR
therefore moves the root package and its lockfile entry to 0.3.0, updates
every surface that existing lockstep tests couple to that version, corrects
the public documentation to name v0.2.1 as the released baseline, and
documents migration for typed bootstrap errors and non-exhaustive matching.
This records authorization, not completion: no final 0.3.0 source head, PR,
merge, tag, or release is established or claimed here. No node, dependency,
gate, or protocol or schema version changes.

On 2026-10-22 (UTC) the issue #152 writer published PR #161 from
`feature/client-conformance` at exact head
`9cc23e67a99440ee51e7b60fcf8d68f6b84edef3` (tree
`b678e0cc4eca2b598e5a7cae0fe02488523532b4`), based on main `2c2b9fb5`. The PR
is open, non-draft, and mergeable, and its body has no closing reference.
Two retained independent reviews, each run with `gpt-6-astra`, high
reasoning, and long context, covered the full diff from merge base `e4f585a7`
at that head and reported no actionable findings; the review synthesis records
all 14 earlier findings resolved. Main moved from `e4f585a7` to `2c2b9fb5`
only in `.streamliner/**`. Local's separate trust and spawn review of the same
head then required changes, and required CI run 39716184110 completed with
failure there: 11 jobs passed and 5 failed, including both Build and test jobs
and Live Postgres parity.

The same writer repaired PR #161 through heads `d08e764c`, `5efe5bf9`,
`20fe13b4`, and `8bd3c330`. Local's bounded reviews raised five source
findings: a probe deadline that did not bound pipe draining while the selector
lock was held (L161-1), stale capability evidence treated as a foreign daemon
(L161-2), a Linux process-group signal that could follow the leader's reap
(L161-3), a restart classification that bypassed deliberate detach (L161-4),
and a PostgreSQL migration lock that was not cancellation-safe (L161-5).
Earlier retained approvals of `5efe5bf9` and `20fe13b4` were superseded by
later findings. At exact head `8bd3c330393409a79e88338d24e1174b1c7407b6` (tree
`4d06daec9200db8de5196d85d5eb47a7d8f1a96a`), Local's source review approved
with no findings, and the retained `gpt-6-astra` review of
`5efe5bf9..8bd3c330` approved with no actionable findings. These are source
approvals only. CI runs at `d08e764c` and `5efe5bf9` failed and are
diagnostic. CI run 39730569101 at `8bd3c330` also failed, in 5 of 16 jobs,
including both Build and test jobs, Live Postgres parity, and the Windows
`InstalledCurrent` Postgres process proofs.

The writer then submitted source changes intended to address those failures
in exact head `2b8ade1d23214fcaffef68f6d2b0143977b9a7a5` (tree
`db4a8ce7a8a8988b4ec276366932f113be6ef8fa`), a five-path test and fixture
change of +93/-66 lines. Windows test roots now sit directly below the volume
root, and production authority validation is unchanged. Local's bounded
review approved that head with no findings. The retained review sequence of
five `gpt-6-astra` invocations, run with high reasoning and long context,
found defects that the writer corrected, then concluded with an approval of
`8bd3c330..2b8ade1d`. These are also source approvals only. The writer
reports that full-workspace all-target Clippy is blocked by warnings in
unchanged targets, so it is not green.

Required CI run 39734925308 at `2b8ade1d` completed with failure at
06:50:27Z. Three required Windows jobs failed: `InstalledCurrent` Postgres
process proofs, Copilot fallback end-to-end, and Build and test. The other 13
jobs passed, including Live Postgres parity. Each failure reports unsafe
install authority. The consumer fixtures' restart-sender and watcher attaches
were rejected, the station-intent upgrade test failed at pre-upgrade
install-root validation, and `daemon_bootstrap` unit tests failed. The
`2b8ade1d` source approvals stand as source review only, and the head has no
CI or readiness credit.

The writer then submitted a two-path, Windows-only source change at exact head
`ec8b88f71ddb5aa8bea79704806e7895d7b4ad3a` (tree
`a1a73573150fc8371553f09ba85b3d0776b9181b`), +48/-95 lines, intended to address
that failure class. Windows may give objects created by an elevated process the
token's default owner, often Administrators, rather than the token user, and
bootstrap accepted only the token user. Bootstrap now reuses the self-owner set
already defined in `platform_fs`: the token user, the token owner, and groups
the token may assign as owner. Deny-only groups are excluded from that set; this
does not newly reject every access-control entry that names such a group.
Administrators remains an independently trusted access-control trustee, as
before, and there is no new broad Administrators exemption for object owners.
Local's bounded review approved `ec8b88f7` with no findings, and two retained
`gpt-6-astra` reviewers, run with high reasoning and long context, approved
`2b8ade1d..ec8b88f7` with no actionable findings. The earlier source resolutions
of L161-1 through L161-5 are conserved. These are source approvals only. CI run
39737943130 was started at `ec8b88f7`; its result is not recorded here.

Required CI run 39737943130 at `ec8b88f7` completed with failure at 07:19:29Z.
Its only failed job was Build and test on Windows. The
`application_client_conformance` test binary started four tests: one was ignored
and two reported ok. The process then exited with `STATUS_HEAP_CORRUPTION`
(0xc0000374) before printing a result summary. The other 15 jobs passed,
including the required Live Postgres parity and Windows `InstalledCurrent`
Postgres process proof jobs. Those two results are diagnostic evidence only: the
run failed, and the head has no CI or readiness credit. The `ec8b88f7` source
approvals stand as source review only.

The writer then submitted head `7505568c`, a one-path change to
`src/daemon_bootstrap.rs` (+42/-76) intended to address that failure. Before
`ec8b88f7`, bootstrap already compared Windows SIDs by converting the string
forms of its fixed trusted principals into native binary SID buffers (`SidBuf`,
allocated by `ConvertStringSidToSidW` and freed on drop). `ec8b88f7` did not
introduce that lifecycle; it routed the expanded self-owner set through it.
`7505568c` removes the string-to-binary SID round trip from bootstrap authority
checks. Bootstrap now converts each borrowed owner or access-control-entry SID
returned by Windows to a string with `ConvertSidToStringSidW`, copies the result
into a Rust string, immediately frees only the returned UTF-16 allocation, and
compares that string with the unchanged cached self-owner set and the fixed
trusted principals. SID-to-string conversion and security-descriptor acquisition
still allocate native memory. Trusted principals, access masks, ancestor-owner
allowances, and the `platform_fs` self-owner set are unchanged.

Local's bounded review approved `7505568c` with no findings, and two retained
`gpt-6-astra` reviewers, run with high reasoning and long context, approved
`ec8b88f7..7505568c` with no actionable findings. Neither review shows that the
earlier SID lifecycle caused the heap corruption, that `7505568c` fixes it, or
that it reduces runtime risk; only CI can establish the runtime outcome. The
earlier source resolutions of L161-1 through L161-5 are conserved. These are
source approvals only. CI run 39740654750 was started at `7505568c`; its result
is not recorded here.

Required CI run 39740654750 at `7505568c` completed with success at 07:45:05Z.
All 16 jobs passed, including the required Live Postgres parity and Windows
`InstalledCurrent` Postgres process proof jobs. The CI gate is met for
`7505568c` only; any later head needs its own source review and passing required
CI. Passing CI does not resolve incident I161-1 or certify proof isolation. App
reports that consumer attestations have started; none is recorded as passed, and
the formal consumer gate stays planned. Design inspection, campaign
authorization, and merge remain outstanding.

Both pre-merge consumer attestations approved exact head `7505568c` (tree
`f532f986`) on 2026-10-22, and neither found a private fallback or an unaccepted
dependency. The Watcher attestation found that a public-only consumer, built
with default features disabled, connects through the `InstalledCurrent` trusted
root and that attach, restart, durability, recovery, send-only, health, and
backend-parity semantics align. The Operator Station attestation found that
multi-address attach, readiness, compensation, exact-recipient ingest before
acknowledgment, restart reconciliation, reply, disposition, source, health, and
backend-parity semantics align through public surfaces. These attestations meet
the task's attestation requirement for `7505568c` only, and any head movement
invalidates them. After merge they become evidence for
`consumer-integration-gate`, which stays planned; they do not pass that gate or
authorize consumer launch.

The Application Client design inspection approved `7505568c` against merge base
`e4f585a7` on 2026-10-22. It ran `gpt-5.6-sol` with high reasoning, and its
context tier was not recorded. The task specifies a Claude Opus 4.8,
high-reasoning, long-context inspector, so whether this inspection satisfies the
fresh design inspection requirement needs a campaign disposition; it receives no
design credit until then. The inspection judged that incident I161-1 does not
block its design verdict, and it states that the incident remains unresolved,
with isolation acceptance, causality, and closure withheld.

The Local Daemon design inspection required changes at `7505568c` on 2026-10-22.
It found four reachable design-boundary defects by static analysis; none was
reproduced at runtime.

- D161-1: upgrade and rollback drain the daemon that the calling executable
  authenticates, not the current selection of the install root they have locked.
- D161-2a: conformance setup drops other runs' PostgreSQL schemas based only on
  name prefix and age.
- D161-2b: forced fixture teardown can kill a process by numeric PID without
  proof that the PID still belongs to the original daemon.
- D161-3: on non-Linux Unix, the bootstrap exact-executable check reads metadata
  from the executable's path instead of failing closed when the running image's
  identity is unavailable.

The inspection also found that D161-2a and D161-2b independently prevent
accepting the current proof-isolation design. I161-1 remains a separate,
unresolved external operator incident; no restoration, proof replay, or cleanup
is authorized. The source review, CI, and attestation evidence for `7505568c`
stays valid in its original scope but does not waive these findings, and that
head receives no design credit. A successor head is assigned to make the
corrections. Any head movement invalidates all `7505568c` evidence, and the
successor needs fresh source review, passing required CI, both consumer
attestations, and both design inspections. The campaign disposition on the
Application Client inspector profile remains open. Merge authorization, merge,
and closure remain outstanding.

Successor head `f02f0ee9` (tree `f402c866`) moved PR #161 from `7505568c`
through 13 commits that change eight paths (+562/-120) and are intended to
address D161-1, D161-2a, D161-2b, and D161-3. The retained gpt-6-astra delta
review approved the full range from `7505568c` with no remaining findings. It
reports that drain now authenticates the locked root's selected predecessor,
conformance setup drops only its own run's schema, fixture teardown no longer
kills by numeric PID and keeps roots when process exit is uncertain, and
non-Linux Unix bootstrap refuses before connecting or spawning. Its validation
was compile-only because local runtime is frozen under I161-1. This is a source
approval only; it does not resolve the design findings, which need fresh design
inspections. Required CI run 39755917809 started at `f02f0ee9` and Local's
source review is assigned; neither result is recorded here. All `7505568c` CI,
consumer, and design evidence is historical and gives the successor no credit.
The successor has no gate credit yet: before merge it needs Local source
approval, passing required CI, both consumer attestations, both design
inspections, a disposition of I161-1, and campaign authorization.

The released-v0.2.1 upgrade proof is not accepted. At `d08e764c`, the official
v0.2.1 executable staged the candidate into a fresh fixture root and public
attachment succeeded. The proof script, however, also rewrote the access control
list of the operator's real `AppData\Local` directory in several runs, and no
prior descriptor or backup exists. That incident, I161-1, withholds isolation
certification and awaits an operator-impact disposition. No restoration or
cleanup is authorized, and local runtime and proof replay are frozen, so runtime
validation relies on CI. Design inspection acceptance, merge authorization, and
merge remain outstanding. The node stays in progress, and issue #152 stays open.

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
  `first-binding` then completed through issue #149 and PR #151. Conformance
  remains a separate node and gate boundary.
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
  caller-provided Tokio runtime and does not create a hidden runtime,
  application-specific daemon, or sidecar. Selecting the shared installed Telex
  daemon does not transfer daemon lifecycle ownership to the consumer.
  Lifecycle operation objects preserve completed, uncertain in-flight,
  untouched, and compensation evidence across cancellation.
- **Production consumers use trusted installed-current daemon selection:**
  `ApplicationDaemonBootstrap::InstalledCurrent { trusted_root }` and additive
  `ApplicationClient::connect_with_daemon` preserve existing config literals.
  The client holds Local Daemon's shared selector admission from
  manifest-validated target resolution through authenticated `HelloAck`;
  upgrade and rollback use exclusive admission across drain and selector
  publication. A spawned daemon independently holds shared admission through
  endpoint and readiness publication. Existing current-executable connect and
  explicit exact-target selection are subordinate development/test support,
  not production fallback.
- **Local Daemon owns install and process authority:** Local Daemon validates the
  trusted root, selector, manifest, immutable versioned target, ownership and
  writability, platform file identity, persistent selector-lock admission,
  captured selection token, protocol/capabilities, and readiness publication.
  Application Client owns the public policy,
  config-compatible constructor, typed failure projection, and runtime consumer
  evidence.
- **Cancellation preserves durable uncertainty:** cancellation never proves that
  an operation was not accepted. Callers persist prepared `RecoveryHandle`
  evidence and reconcile uncertain operations; cancelled receive work does not
  acknowledge a delivery.
- **Only semantic Rust types stabilize:** compatibility commitments cover public
  types and behavior that carry the accepted Application Client contract. They
  do not promote backend records, daemon frames, CLI types, or consumer DTOs into
  the supported surface. Published v0.2.1 contains the binding and is the
  released compatibility baseline, superseding the earlier rationale that no
  release contained it. Consumers of unpublished source, including 0.3.0 before
  any release, use an exact full commit SHA; unpinned Git dependencies remain
  outside the compatibility promise.
- **Issue #152 makes a source-only 0.3.0 transition:** published v0.2.1
  (source `212b76a4`) exposes an exhaustive `ApplicationClientError`. Adding
  `DaemonBootstrap(DaemonBootstrapFailure)` and marking the enum
  `#[non_exhaustive]` breaks Rust source compatibility, so the operator
  authorized the root package to move to unpublished 0.3.0 within the same
  PR, with genuinely release-coupled metadata and source-consumer migration
  guidance. This is not a release, tag, or publication, and it changes no
  daemon protocol or schema version.
- **External boundaries remain deferred:** napi-rs/TypeScript, a separate client
  crate, C ABI, public socket or sidecar protocols, and product DTOs require
  later decisions. This deferral does not permit a private consumer fallback.
- **Conformance remains a separate gate:** first-binding must preserve the full
  AC-C01 through AC-C20 model. It does not complete cross-backend conformance,
  consumer integration, packaging, upgrade readiness, or production hardening.
- **Conformance is one complete delivery bundle:** issue #152 owns all ten
  semantic families, SQLite and credentialed-Postgres parity, public-only
  send-only and bidirectional fixtures, missing shared-semantic repair, and
  temporary-seam replacement guidance in one PR. It also absorbs
  `application-client-daemon-bootstrap-gap-v1`: installed-current runtime
  bootstrap, Windows/Linux proof, selector upgrade/rollback races, and product
  design promotion stay in the same PR. Backend, test-family, fixture,
  migration, bootstrap, and reviewability splits are not independent confidence
  transitions.
- **Consumer attestations are pre-integration gate evidence:** after
  implementation review and required CI pass on one exact conformance head,
  Watcher and Operator Station independently validate the supported public seam.
  Their attestations do not implement either product or pass the gate by
  themselves.
- **Material discoveries receive durable disposition:** required conformance
  work stays in issue #152; real prerequisites name their owning dependency and
  exact resume condition; product-only work routes to its owning workstream.
  No material item may remain untriaged at merge readiness.
- **Issue #124 completed within client-core:** PR #132 closed both #129 and #124
  and reported the regenerated manifest identity. This factual reconciliation
  does not mutate issue #12.

## Open Questions

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
- Exact-head public-seam attestations from Watcher and Operator Station for the
  planned `consumer-integration-gate`.

## Closeout Observations

- Issue #12 publication revision 4 records the design checkpoint, supported Rust
  core, and first supported Rust binding without claiming conformance.
- W-05 taxonomy wording from issue #124 completed within client-core issue #129
  and merged through PR #132.
- The Rust-first `first-binding` completed through issue #149 and PR #151.
  Issue #152 `client-conformance` is in progress as the campaign main effort;
  the consumer gate and `supported-client` remain planned.
- Polluted PR #123 and its dirty worktree are deferred with rationale for
  protocol forensics; cleanup requires explicit operator authorization.
- The clean #118 implementation worktree is also deferred for cleanup until the
  operator requests it.
- Keep API convenience, additional bindings, and consumer-specific ergonomics
  out of contract work unless they expose a missing semantic. Any requirement
  that cannot be accepted must name its blocked consumer and owner rather than
  being softened into ambiguous shared wording.
