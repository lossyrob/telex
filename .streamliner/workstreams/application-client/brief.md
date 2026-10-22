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

`client-conformance` completed through
[#152](https://github.com/lossyrob/telex/issues/152) and PR #161 at exact
reviewed head `52abb30e49dff54f9a9ce7b31c64584a871d11c3`, merged as
`4d25fdff375dcfd55b4a7b4613a5b5ecb8241bdb` on 2026-10-22. One node, tracker, and
delivery PR covers all ten conformance families through the public Rust surface
across SQLite and credentialed Postgres. The same PR delivered public-only
Watcher send-only and Operator Station bidirectional fixtures, shared-semantic
repairs, and guidance for replacing temporary consumer seams. Runtime fixtures
use production `InstalledCurrent { trusted_root }` daemon bootstrap under the
Local Daemon-owned selector-admission and process-authentication contract.
Issue #152 remains open; this record does not close it.

`consumer-integration-gate` passed on merged main `4d25fdff`. Watcher and
Operator Station each attested the reviewed head before merge and then passed
the formal post-merge gate, confirming that their consumers need no private
seam; the receipts are in the PR #161 record below. The `supported-client`
checkpoint is complete. Product implementation, usability, packaging, and
operational evidence remain downstream. Watcher runtime's Application Client
hold is discharged, but it needs a reviewed and reconciled runtime tracker
before launch. The Station app's Application Client dependencies are satisfied,
but it remains planned and unlaunched until separately prepared and authorized.

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
context tier was not recorded. The task then specified a Claude Opus 4.8,
high-reasoning, long-context inspector, so whether this inspection satisfied the
fresh design inspection requirement needed a campaign disposition. The campaign
later declined to credit it, as recorded below. The inspection judged that
incident I161-1 does not block its design verdict, and it states that the
incident remains unresolved, with isolation acceptance, causality, and closure
withheld.

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

Local's source review approved `f02f0ee9` against baseline `7505568c` with no
findings. It accepted the source corrections for all four design findings. For
D161-1, upgrade and rollback now drain only after resolving the target root's
selected predecessor under the exclusive admission they already hold, and a
versioned CLI launch uses a selection token. For D161-2a, the cross-run
PostgreSQL sweep is removed. For D161-2b, fixture teardown stops the daemon
gracefully through its own executable, checks the exact process generation, and
keeps roots when exit is uncertain; the review does not cover explicit
crash-induction helpers that still signal by PID. For D161-3, unsupported
non-Linux Unix bootstrap and the defensive peer check fail closed, while legacy
connections without an expected file identity are unchanged. The earlier L161-1
through L161-5 repairs, the SID authority, PostgreSQL transaction-scoped
migration locking, and the public API are conserved. Local's review was
read-only: it ran no compilation, tests, or runtime proof, and it did not
independently audit the writer's compile-only validation. This is a source
approval only. It gives no runtime, consumer, design, merge, or closure credit,
and it does not accept the design findings as resolved. Required CI run
39755917809 is still in progress; its result is not recorded here. Incident
I161-1 and the withholding of E161-1 are unchanged.

Required CI run 39755917809 at `f02f0ee9` completed with failure at
2026-10-22T09:48:54Z. Two of its 16 jobs failed. In the required Live Postgres
parity job (119747762473), the PostgreSQL leg of the Application Client
conformance battery panicked during the strict-recovery case: while restarting
the daemon, the daemon for `appconf-postgres` did not stop within the deadline.
The SQLite leg of the same battery passed. The non-required macOS Copilot
fallback E2E job (119747762373) failed to compile a test under `-D warnings`
because the method `set_daemon_executable` is never used. The other 14 jobs
succeeded, including the required Windows InstalledCurrent Postgres process
proofs (119747762498). Because a required job failed, `f02f0ee9` receives no CI
credit, and the passing jobs give no readiness credit. The Local and retained
delta-review approvals remain source approvals only, and Local's acceptance of
the D161 source corrections is unchanged; neither is design acceptance. A
further successor head is assigned to address these failures. Any head movement
invalidates all `f02f0ee9` evidence. Incident I161-1 and the withholding of
E161-1 are unchanged.

Residual successor head `689883e272fbb17d82546d7883bf6df36ca93a6d` (tree
`32f1b60d964813c5ae2c2ab4bf8259c6db3bc8bc`) is now the PR #161 head. Its review
baseline is `f02f0ee9`, an ancestor five commits back; its immediate parent is
`53a669ae`. The product merge base remains `e4f585a7`. The delta from `f02f0ee9`
changes two paths, +114/-8: `src/backend/postgres.rs` and
`tests/daemon_process_sqlite.rs`. It is intended to address both CI failures at
`f02f0ee9`. The test helper `set_daemon_executable` now has the same platform
gate as its only caller, which is intended to fix the macOS compile failure. For
the Live Postgres restart timeout, drain now releases its lease through a
distinct PostgreSQL client instead of the shared client, with bounded background
repair of that client and driver abort ownership held through initialization.
This is intended to keep drain from queuing behind ordinary work on the shared
client. The retained delta review approved the change. Its reviewers, agents
`99ae208c` and `2488698f`, are recorded as gpt-6-astra with high reasoning and
long context. Its validation was compile-only under the I161-1 freeze:
formatting, all-features no-run compilation, and targeted Clippy on Rust 1.99
and 1.98. The review identifies the timeout cause from source only and states
that CI must confirm it. Required CI run 39762898070 started at the exact head,
and Local's source review is assigned; neither has a result. The head has no
gate credit yet. Evidence from `f02f0ee9`, including its source approvals and
failed CI, is historical and does not carry forward. Incident I161-1 and the
withholding of E161-1 are unchanged.

Local's source review of `689883e2` against baseline `f02f0ee9` required
changes. Finding L161-6 (P2) concerns the new auxiliary PostgreSQL repair task.
The task waits for the release-client lock before its five-second timeout
starts, and it waits for the publication locks after that timeout ends. Lease
release holds the same release-client lock while its query runs. A PostgreSQL
row-lock wait or stalled I/O can therefore keep the repair task, its shared
state, and its single-flight flag alive indefinitely, which prevents any later
repair from being scheduled. Healthy foreground calls still do not wait for the
repair. Local established this wait path from source and did not reproduce it.
The requested change is one absolute deadline over the whole repair attempt, or
nonblocking inspection and publication that safely defers when busy. The
single-flight flag must be released on completion, timeout, and cancellation,
and any unpublished replacement connection must be cleaned up. Foreground calls
must stay nonblocking, and drain must stay free of credential discovery. Local
found no other source issue: it accepted the macOS cfg change in source and
raised no further finding on the dedicated release client, credential handling,
driver cancellation, or migration locking. Local reports the earlier D161 and
L161-1 through L161-5 source repairs conserved and the public client source
unchanged. It also states that no runtime evidence shows that client-lock
contention caused the earlier Live Postgres failure or that this head fixes it.
The retained delta review's approval is historical and does not override Local's
finding. Local's review was read-only and did not run the writer's compile-only
validation. Required CI run 39762898070 at `689883e2` later completed with
failure at 2026-10-22T10:35:37Z. Two of its 16 jobs failed: the required Live
Postgres parity job (119771171825) and the Windows build-and-test job
(119771171806). The run is superseded by head movement and is diagnostic only.
`689883e2` received no source, CI, readiness, consumer, design, merge, or
closure credit, and the move to `0490f5e5` invalidates all `689883e2`
current-head evidence.

Successor head `0490f5e5936484868d03d19f70daf7b9e2ef4df3` (tree
`61cdd28b44d71df1d02333f5f6eefbc09ffe207c`) is one commit directly on `689883e2`
and became the PR #161 head. The product merge base remained `e4f585a7`. It
changes only `src/backend/postgres.rs`, +97/-15, and was intended to address
L161-6. One five-second budget covers the whole auxiliary repair attempt, a drop
guard was intended to release the single-flight flag on every exit, and an
unpublished replacement is dropped on timeout. Foreground calls stay
nonblocking, and drain stays free of credential discovery. The head added
deterministic unit tests, compiled but not run under the I161-1 freeze. The
retained delta review approved it. Its reviewers, agents `2887b2b4` and
`0d21e13e`, are recorded as gpt-6-astra with high reasoning and long context,
and its validation was compile-only. Local's source review then required changes
again. It accepted the whole-attempt timeout, cleanup once the repair task has
started, and the new tests for those paths. It found the foreground, credential,
fencing, and migration code otherwise unchanged. A narrow part of L161-6
remained: the single-flight flag was set when the repair was scheduled, but the
guard that clears it was created only when the spawned task first ran. If the
queued task was dropped before then, for example during runtime shutdown, the
flag stayed set and no later repair could be scheduled. Local required the guard
to be created synchronously when the flag is acquired and moved into the task,
plus a test that drops the unpolled task and confirms that the flag clears and
can be acquired again. This finding is static and was not reproduced. Local
reported D161, L161-1 through L161-5, the SID authority, the public API, and
migration locking conserved. The retained delta review's approval does not
override Local's finding. Local's review was read-only and did not run the
writer's validation. Required CI run 39763861998 at `0490f5e5` was superseded by
head movement while still in progress, after its required Live Postgres parity
job had already failed. It is diagnostic only, and no terminal result is
recorded here. `0490f5e5` received no source, CI, readiness, consumer, design,
merge, or closure credit, and the move to `745eaf28` invalidates all `0490f5e5`
current-head evidence.

Successor head `745eaf28bb9d0abe70a79f8023dc7a98fa66d5da` (tree
`6e61c22b1116b68965554da1bdafe5c36372b016`) is one commit directly on `0490f5e5`
and is now the PR #161 head. The product merge base remains `e4f585a7`. It
changes only `src/backend/postgres.rs`, +42/-13, and is intended to close the
remaining part of L161-6. The repair guard is now created synchronously as soon
as the single-flight flag is acquired and is moved into the scheduled task, so
dropping the task before it first runs clears the flag. A new unit test drops
the unpolled repair future and checks that the flag clears and can be acquired
again. The retained delta review approved it. Its reviewers, agents `9e3f3795`
and `1cea26a6`, are recorded as gpt-6-astra with high reasoning and long
context, and its validation was compile-only. Local's source review approved
`745eaf28` against baseline `0490f5e5` with no findings. Local accepted L161-6
as resolved in source, covering both the whole-attempt deadline and guard
ownership before the first run. The repair body, connection setup, foreground
path, reconnect, and backend implementation are otherwise unchanged. Local
reports D161, L161-1 through L161-5, the SID authority, the public API, and
transaction-scoped migration locking conserved. Local inspected the new test but
did not run it; the test covers the repair future directly, and Local verified
the outer task from source. Local's review was read-only and did not run the
writer's validation. Local states that the approval does not show that the Live
Postgres timeout is fixed or that CI will pass. This is a source approval only.
It gives no CI, runtime, consumer, design, merge, or closure credit. Required CI
run 39764752556 then ran at the exact head; its result follows. Evidence from
`0490f5e5` does not carry forward. Incident I161-1 and the withholding of E161-1
are unchanged.

Required CI run 39764752556 at `745eaf28` completed with failure at
2026-10-22T10:57:59Z. Two of its 16 jobs failed, and the other 14 succeeded. In
the required Live Postgres parity job (119777319781), the PostgreSQL leg of the
Application Client conformance battery failed: the daemon for `appconf-postgres`
did not stop within the test deadline while the strict-recovery check restarted
it, the same symptom seen at `f02f0ee9`. In the required Windows
InstalledCurrent Postgres process-proof job (119777319882), the concurrent
first-use test failed because a concurrent connect returned Unavailable after a
backend operation failed (diagnostic `442fe890f96b80f8`). The log shows these
symptoms but not their cause. Because required jobs failed, `745eaf28` receives
no CI or readiness credit, and the Live Postgres timeout is not shown to be
fixed. The Local and retained delta-review approvals of `745eaf28` remain
source-only. A further successor was assigned; its head needs its own source
review and passing required CI. Incident I161-1 and the withholding of E161-1
are unchanged.

Successor head `0486beeb027ba787465a860f94eacc06a26f8170` (tree
`d0052264b1d403ddfa31c5855f6f94cc6c827509`) is now the PR #161 head. It adds
four commits on `745eaf28`; its immediate parent is `e6eb4a40`. The product
merge base remains `e4f585a7`. Against baseline `745eaf28` it changes four
paths, +188/-39: `src/backend/postgres.rs`, `src/daemon.rs`,
`tests/application_client_conformance.rs`, and
`tests/installed_current_process.rs`. It is intended to address both failures in
run 39764752556. First, each conformance scenario, including the auxiliary
migration store, now stops its daemon before its PostgreSQL schema is discarded.
Second, the InstalledCurrent Postgres test writes the shared profile only once,
under its process lock, so concurrent first-use tasks no longer truncate and
rewrite the configuration file while peers read it. Third, the drain-release
connection is now prepared eagerly and maintained proactively only for
daemon-owned profile backends; public construction and ordinary activity use one
connection, although an explicit lease release can still open the release
connection lazily. The release connection owns its driver, uses server-side
statement and lock timeouts, and keeps one absolute five-second budget for the
fenced release. Its invalidation is guarded, and repair with fresh credentials
is bounded and runs outside drain. Only daemon activity checks its health
proactively. The retained delta review approved it. Its reviewers, agents
`fd257b51` and `fb340fb6`, are recorded as gpt-6-astra with high reasoning and
long context. Its validation was formatting, compilation, and lint only under
the I161-1 freeze, with no runtime or database test. The review links the two
failures to these causes from source and the CI log, and it states that CI must
confirm both repairs. Local's source review then approved `0486beeb` against
baseline `745eaf28` with no findings. It accepted the source as follows. Only
the daemon's profile backends open the release connection eagerly; public
construction and ordinary activity use one connection, although an explicit
lease release can still open the release connection lazily. The connection
driver is abort-owned through setup and publication. Each lease-release call has
one absolute five-second deadline, with fencing, invalidation, and repair
preserved; the deadline applies per call, not across a whole multi-member drain.
Each scenario stops its daemon before its exact PostgreSQL schema is discarded,
and the test profile is published once under a lock. Local recorded three
qualifications. Dropping a connection is local cleanup and does not guarantee
synchronous remote cancellation. The absence of a daemon capability file, which
the teardown uses to skip the stop, does not prove that no daemon endpoint
exists. Local did not verify the review's claim that these changes caused or fix
the CI failures. Local reports L161-6 still resolved in source and D161 and
earlier source authority conserved. Local's review was read-only and did not run
the writer's validation. This is a source approval only. It gives no CI,
runtime, consumer, design, merge, or closure credit. Required CI run 39770808804
then ran at the exact head; its result follows. The move from `745eaf28`
invalidates all `745eaf28` current-head evidence, and earlier approvals and CI
results are historical. Incident I161-1 and the withholding of E161-1 are
unchanged.

Required CI run 39770808804 at `0486beeb` completed with success at
2026-10-22T11:48:08Z. All 16 jobs succeeded, including the required Live
Postgres parity job (119797559542) and the required Windows InstalledCurrent
Postgres process-proof job (119797559651). At completion, PR #161 was open and
not a draft at the exact head, its base was `main` at `2c2b9fb5`, and GitHub
reported the merge state as clean. `0486beeb` therefore receives required-CI
credit for this exact head only; any head movement invalidates it. The passing
run shows that the two PostgreSQL failures from run 39764752556 did not recur at
this head; it does not by itself establish their cause. Passing CI is not
isolation certification: incident I161-1 remains unresolved, and E161-1 remains
withheld. App reported that fresh Watcher and Operator Station pre-merge
consumer attestations at `0486beeb` had started; their results follow.
`0486beeb` has no consumer, design, merge, or closure credit, and the formal
post-merge consumer gate remains planned.

Both fresh attestations at `0486beeb` returned on 2026-10-22, with different
verdicts. Watcher APPROVED. It found that Watcher can consume the client through
supported public surfaces only, and that the D161 and L161 repairs and the
PostgreSQL lifecycle changes add no Watcher-private seam and no new
compatibility or design dependency. Operator Station returned CHANGES REQUIRED.
Station found the public API and its fixture behavior compatible and found no
private seam or new design dependency. Its blocker, OS-PR161-0486BEE-1, is a
teardown-order defect: line 281 of `tests/application_client_fixture.rs` drops
the PostgreSQL fixture schema after the probes without first stopping the
InstalledCurrent daemon. That file was not part of the `0486beeb` delta; the
earlier stop-before-discard change covered the conformance scenario store and
the auxiliary migration store. In passing Live Postgres job 119797559542, the
fixture binary started at 11:31:36Z and its PostgreSQL test reported ok at
11:33:42Z. The PostgreSQL service log records 71 `relation "leases" does not
exist` errors from 11:33:22 to 11:39:12 UTC, about every five seconds, ending
just before the container was removed. The job passed because no assertion
checks for daemon termination or database activity after the drop. Station
requires the fixture to stop its exact daemon through the existing
identity-bound stop path before dropping the schema, a rerun of the required
PostgreSQL and InstalledCurrent proofs at the new head showing no surviving
daemon or continued queries, and a fresh Station attestation at that head. Run
39770808804 remains valid run evidence for `0486beeb`, but it does not waive
this finding. App then reported that a surgical successor was assigned.
Watcher's approval is bounded pre-merge evidence for `0486beeb` only, and head
movement invalidates it. `0486beeb` has no combined consumer credit and no
design, merge, or closure credit. I161-1 remains unresolved, E161-1 remains
withheld, and the formal post-merge consumer gate remains planned.

Successor head `4cd34938` (tree `16456905`), a direct child of `0486beeb`, is
intended to address OS-PR161-0486BEE-1. Its only change adds 20 lines to
`tests/application_client_fixture.rs`: the fixture keeps its capability-presence
observation, calls the existing identity-bound `iso.stop_daemon()`, and only
then runs the final `DROP SCHEMA`; a stop that cannot be verified panics before
the drop. It also adds a static source-order regression check. The retained
gpt-6-astra exact-delta review approved it with no findings, using static
inspection only. Local's exact-head source review also approved it with no
findings and accepted OS-PR161-0486BEE-1 as resolved in source. Local qualified
that approval: the later destructor stop remains best effort and is not a
numeric-PID kill or root cleanup; the regression check is textual and protects
only the current straight-line function, not arbitrary future control flow;
Local inspected the check but did not run it; and source review does not
establish runtime daemon termination. Local found all other tracked files
identical to `0486beeb`, so the earlier D161, L161, SID-authority, public-API,
and PostgreSQL lifecycle source authority is conserved. Both approvals are
source-only. Required CI run 39775306925 then ran at the exact head; its result
follows. The head movement invalidated all `0486beeb` current-head evidence,
including its CI credit and Watcher's approval, so `4cd34938` started with no
CI, consumer, design, merge, or closure credit. I161-1 remains unresolved,
E161-1 remains withheld, and the formal post-merge consumer gate remains
planned.

Required CI run 39775306925 at `4cd34938` completed with success at
2026-10-22T12:22:46Z. All 16 jobs succeeded, including the required Live
Postgres parity job (119812625200) and the required Windows InstalledCurrent
Postgres process-proof job (119812625123). At completion, PR #161 was open and
not a draft at the exact head, its base was `main` at `2c2b9fb5`, and GitHub
reported the merge state as clean. `4cd34938` therefore receives required-CI
credit for this exact head only; any head movement invalidates it. In the Live
Postgres job, the consumer fixture's PostgreSQL test reported ok at 12:09:21Z,
and the PostgreSQL service log contains no `relation "leases" does not exist`
errors. Its only errors are 30 `isolated wait recovery fault` entries, the last
at 12:06:03 UTC, before that test finished. This is consistent with the intended
fix, but whether it satisfies OS-PR161-0486BEE-1 is for Operator Station's fresh
attestation to decide. Passing CI is not isolation certification: incident
I161-1 remains unresolved, and E161-1 remains withheld. App reported that fresh
Watcher and Operator Station pre-merge consumer attestations at `4cd34938` had
started; their results follow.

Both fresh attestations at `4cd34938` returned APPROVED on 2026-10-22. Watcher
found that the cleanup-order delta establishes verified daemon termination
before the run-owned schema is dropped, without changing the public Application
Client, the public Watcher fixture, the daemon, or the PostgreSQL backend, and
that no private fallback or unaccepted Watcher compatibility or design
dependency is required. Operator Station found that it can consume this exact
revision through supported public surfaces only, with no private fallback, and
closed OS-PR161-0486BEE-1 in source and runtime. Station's evidence: the fixture
stops the exact daemon generation before the final `DROP SCHEMA`; both the Linux
Live Postgres and Windows InstalledCurrent jobs passed the fixture and its new
stop-before-drop test; and the PostgreSQL service log has no `leases` activity
and no database error from fixture completion through shutdown. Station
attributes the 30 earlier `isolated wait recovery fault` entries to deliberate
fault-injection tests that ran before the fixture. Station also found the
earlier D161, L161, and PostgreSQL lifecycle source unchanged by blob.
`4cd34938` therefore has bounded pre-merge consumer credit from both product
authorities for this exact head only; any head movement invalidates it. This is
not the formal post-merge consumer-integration gate, which remains planned and
must attest the exact merged revision. App reported that the Local Daemon design
inspection at `4cd34938` had started; its result follows. The Application Client
design inspector-profile disposition was also pending. I161-1 remains
unresolved, and E161-1 remains withheld.

The Local Daemon design inspection of `4cd34938` returned APPROVED on
2026-10-22. Its fresh read-only inspector covered the whole nine-path range from
`7505568c` to `4cd34938`, not only the final fixture change, and the steward
adjudicated the result. The verdict resolves D161-1, D161-2a, D161-2b, D161-3,
and OS-PR161-0486BEE-1 in source, leaves no new material Local Daemon design
finding, and accepts the current proof-cleanup and isolation source design
independently of the historical incident. It preserves I161-1 as an external
operator-impact and restoration disposition and keeps E161-1 withheld; it grants
no overall isolation certification and no restoration, replay, cleanup, merge,
release, or post-merge gate authority. It also preserves the earlier causal
qualification: source approvals and passing CI do not establish the cause of the
earlier Windows heap corruption. Provenance: the steward and a separate fresh
inspector each ran as gpt-6-astra with high reasoning and the long-context tier;
these are distinct records. The verdict does not certify the separately required
Application Client inspector profile. `4cd34938` therefore has Local Daemon
design credit for this exact head only; any head movement invalidates it. The
steward's last provider read after writing the verdict returned mergeability
unknown, and the verdict carries no clean-merge claim forward; a later read for
this record showed the PR mergeable and clean at the same head, but merge
readiness still needs its own fresh guards. At that point the fresh Application
Client design inspection and the campaign's inspector-profile disposition were
pending, so `4cd34938` had no Application Client design credit and no merge or
closure credit. I161-1 remains unresolved, E161-1 remains withheld, and the
formal post-merge consumer gate remains planned.

On 2026-10-22 the campaign resolved the Application Client inspector-profile
disposition by enforcing an existing operator instruction, not by granting a
waiver. On 2026-09-24 the operator directed that all new sessions use GPT-6
Astra with high reasoning and 1.2M context, and a 2026-10-08 campaign directive
applies that instruction to all new sessions and delegates as gpt-6-astra with
high reasoning and the long-context tier, with no silent downgrade. That
standing instruction governs new inspectors over the Claude Opus 4.8 profile
this task previously named, and this record reconciles the task text; the
substantive requirement for a fresh, independent, read-only design inspection is
unchanged. The campaign's directive declines to credit the `7505568c`
inspection, which ran gpt-5.6-sol without a recorded context tier, and assumes
no equivalence between models. It assigns App's existing Application Client
design steward to run one fresh, independent gpt-6-astra, high-reasoning,
long-context inspection of the complete exact-head delta at `4cd34938`,
recording the actual profile, head, tree, comparison base, and inspector
identity. The operator asked for 1.2M context, but the last verified catalog
limit was 1,050,000, which does not prove an actual execution budget; the
inspection must record supported and observed context separately and report a
provisioning blocker rather than substitute a profile. The inspection could
proceed without waiting for this record, and `4cd34938` had no Application
Client design credit until its verdict returned; that verdict follows. The
directive granted no merge, closure, release, or consumer-launch authority and
required a freshly guarded campaign merge floor; a later user correction
withdrew that floor as an added delivery gate.

The fresh Application Client design inspection of `4cd34938` returned APPROVED
on 2026-10-22. Its fresh inspector ran gpt-6-astra with high reasoning and the
long-context tier, as retained runtime events record, and no numeric context
capacity is claimed. The steward session ran gpt-5.6-sol with high reasoning and
no recorded context tier; the earlier `7505568c` inspection receives no credit.
The read-only inspection covered the complete product range from merge base
`e4f585a7` to `4cd34938` (39 files, +14,761/-397) against tracker issue #152,
the accepted Application Client and Local Daemon designs, this task, ADR 0053,
and the accepted source-only 0.3.0, raw-absent v0.2.1 updater, and Windows
causal launch-witness decisions. It found no blocking design finding: the public
API and error taxonomy, the InstalledCurrent trusted-root model, the Windows
causal launch witness, updater compatibility, the D161 and L161 repairs, the
PostgreSQL lifecycle and final cleanup ordering, the ten-family SQLite and
PostgreSQL battery, and the Watcher and Operator Station consumer boundary all
conform to accepted authority. It does not certify final discovery-ledger or
field-report completion, which remain with their canonical owners. It keeps
I161-1 open and E161-1 withheld under the operator's preservation and read-only
review directive; it judges that they do not block this source-design acceptance
because the accepted contract makes no intra-user isolation claim, and it grants
no isolation acceptance, incident closure, or heap-corruption causal claim.
`4cd34938` therefore had Application Client design credit for that exact head
only; the later head movement invalidated it. Source approval, required CI, both
pre-merge consumer attestations, and both design inspections had passed at
`4cd34938`. The pre-merge obligations were not complete: E161-1 remains an
outstanding validation obligation, not an accepted gap, and I161-1 impact
assessment and isolation acceptance remain open. The verdict grants no merge,
release, closure, or post-merge consumer gate credit, and the formal post-merge
consumer gate remains planned.

On 2026-10-22 the user corrected the campaign operating model, as relayed by the
campaign session and App and recorded in the campaign directive
`campaign-implementer-ownership-directive-20261022.json`. The retained
implementer of PR #161 (`c0100c31`), supported by App, owns seeing the work
through completion: resolving outstanding evidence, including E161-1, through
permitted engineering; obtaining the required independent review, CI, consumer,
and design results; and completing the normal guarded delivery and merge flow.
The campaign and workstream orchestrators unblock, inform, and coordinate design
and dependencies, and escalate only a concrete material decision that cannot be
resolved within accepted scope. The campaign's unanswered E161-1 options prompt
and its added independent merge-floor certification are withdrawn; neither was
answered or waived, and no campaign audit or readiness certificate replaces
them. The correction infers no merge authorization and no E161-1 waiver: E161-1
remains an outstanding validation obligation, and the normal guarded merge
authority applies only once the accepted requirements are met. Existing gates,
exact-head rules, accepted design and dependencies, independent reviews, and the
I161-1 preservation and read-only review directive are unchanged. The correction
grants no access-control restoration or reset, workstation proof replay,
cleanup, release, tag, new worker, or new approval queue.

On 2026-10-22 the implementer pushed successor head `5d21a63d` (tree `e435edf7`)
to resolve E161-1. It is two commits over `4cd34938` (intermediate `669cb86e`)
and changes four paths (+588): a Windows CI job, the proof script
`tests/released_v021_upgrade_proof.ps1`, a release-candidate test, and
Application Client documentation. On an ephemeral CI runner, the proof downloads
the official v0.2.1 Windows release, verifies its pinned digest and build
identity, and runs the released upgrade in a fresh disposable root directly
under the volume root. It changes the access control list of that root only,
never its parent, and runs nothing on the operator's workstation. The
implementer reports static validation and fresh gpt-6-astra, high-reasoning,
long-context approvals with no findings: one for the exact delta from `4cd34938`
and one for the complete Application Client design at `5d21a63d`. An
intermediate review found that a script parameter collided with the read-only
PowerShell `$PID` variable; the final head renamed it, and the complete delta
was reviewed again. Both reviews state their limits: the hosted proof must pass
at this exact head before E161-1 receives runtime credit; the proof establishes
compatibility of a fresh layout written by the released installer, not drain of
a live v0.2.1 daemon; and neither review grants CI, consumer, merge, release, or
I161-1 credit. The implementer requested Local's source and design reviews and
fresh Watcher and Operator Station attestations. Required CI run 39805183080
completed with conclusion failure at 2026-10-22T15:39:42Z. Fifteen of its 17
jobs passed, including Live Postgres parity and Windows InstalledCurrent
Postgres process proofs, and two required Windows jobs failed. The new job
`Released v0.2.1 upgrade proof (Windows)` failed in its proof step after
checkout and the exact-head build succeeded; the script stopped with
`PSMODULEPATH failed with exit code 1`, and its cleanup of the exact root and
its evidence upload succeeded. `Build and test (windows-latest)` failed one
test,
`station_intent::tests::a_cooperatively_truncated_phase_still_returns_its_partial_page`,
which panicked with "a cooperative stop must be returned to the caller, not
abandoned"; `src\station_intent.rs` is unchanged from `4cd34938`. No cause is
established beyond that output, and E161-1 receives no runtime credit. Local's
source review returned CHANGES REQUIRED with two proof-script findings. L161-7:
the command timeout bounds only direct process exit, not output-pipe completion
or the wait after termination, and a timed-out command's output is discarded
before it is saved. L161-8: cleanup is authorized before the script has
exclusively created its root, so failing to create an already existing path can
still authorize its recursive deletion. Local's design revalidation also
returned CHANGES REQUIRED with P161-1, the same cleanup-before-ownership gap,
and P161-2: a process lookup that fails without returning a process object is
treated as confirmed exit. Watcher's pre-merge attestation returned CHANGES
REQUIRED: its source consumability approval stands because the Watcher-facing
files are byte-identical to `4cd34938`, but runtime qualification is withheld
because required CI failed. Operator Station approved source consumability and
returned runtime CHANGES REQUIRED for the two failed jobs; OS-PR161-0486BEE-1
remains closed. Neither consumer grants runtime consumer credit. Neither Local
review alleges an actual collision, deletion, or hang, and both confirm that
production source and the accepted product design are unchanged from `4cd34938`.
Local also qualifies the proof's scope. It installs the candidate into a fresh
root with `--skip-drain` and then attaches through the public client, so it does
not prove drain of a running v0.2.1 daemon or migration of an existing store. CI
evidence, caches, and build output sit outside the disposable root, environment
redirection is not registry virtualization, and the disposable hosted runner is
the outer isolation boundary. Local treats CI run 39805183080 as diagnostic only
for readiness. The implementer owns the repairs within the existing delivery
flow, and no new approval layer is added. Because the head moved, no `4cd34938`
evidence carries over, and `5d21a63d` has no gate credit. The I161-1 workstation
preservation and freeze are unchanged, and no release, tag, or fleet action is
authorized.

On 2026-10-22 the implementer pushed successor head `631aff22` (tree `ffa38ba6`)
to resolve E161-1. It was three commits over `5d21a63d`. Relative to `4cd34938`
it changed five paths (+476/-13): the CI workflow, Application Client
documentation, the `InstalledCurrent` process test, the release contract test,
and shared test isolation support. Production source was unchanged. The head
deleted the custom PowerShell proof script and implemented the official v0.2.1
compatibility proof as an ignored Windows test in the existing, reviewed Rust
`InstalledCurrent` harness. CI verifies and downloads the official v0.2.1
release, runs that test against the exact head, writes command logs and the
proof report directly to files, and uploads the evidence. The proof creates
owner-private Windows roots exclusively, so creation fails on an existing path.
It changes no parent access control list and never deletes a proof root
recursively; the ephemeral runner's teardown owns the roots. Shutdown reuses the
existing exact-generation daemon stop. The implementer reported compile-only
validation on Rust 1.99 and 1.98 (tests built without running, and Clippy), a
clean YAML lint, and fresh gpt-6-astra, high-reasoning, long-context approvals
with no findings: one for the exact delta from `5d21a63d` and one for the
complete Application Client design at `631aff22`. Both were source-only and
granted no CI, E161-1 runtime, merge, release, consumer, post-merge, or I161-1
credit. Local's design inspection approved the head, finding P161-1 and P161-2
structurally closed and the product design unchanged. Local's source review
returned CHANGES REQUIRED. It found L161-7, L161-8, P161-1, and P161-2
structurally closed in source, and it opened priority-2 finding L161-9: the new
empty-installation proof can fail before the released writer publishes a current
binary, and its destructor then panics a second time while resolving that
missing binary, aborting the test process and masking the original failure.
Local established this from source, not from a hosted run, and its design
approval did not override the source finding. Both Local reviews kept the
proof's scope: the official v0.2.1 writer installs the candidate into a fresh
root with `--from` and `--skip-drain`, and the candidate then attaches through
the public client. It does not prove a running predecessor daemon, an existing
store, network paths, or every platform. Required CI run 39809448340 at
`631aff22` had no failed job and no terminal result when the head moved; Local
treats it as diagnostic only. No `4cd34938` or `5d21a63d` evidence carried over,
and `631aff22` received no gate credit.

The implementer then pushed head `dacf3d18` (tree `5c2123ad`), one commit over
`631aff22` that changes two test paths (+16/-1), to fix L161-9. Best-effort
teardown now resolves the current binary fallibly. When the selection is absent
or invalid, it returns false and runs no command, so the original failure
surfaces and the root is retained; it adds no fallback to an ambient binary or a
raw process ID. A new Windows regression test drops an empty installation. The
implementer reports compile-only validation and fresh gpt-6-astra exact-delta
and design approvals; no artifact path or hash for those approvals has been
supplied. Local's source review approved the head with no findings. L161-9 is
resolved in source, the earlier proof findings remain closed, and production
source is unchanged. Local qualifies that the regression covers refusal for a
missing selection and an ordinary drop, not a drop during an existing panic or
every invalid-selection case, and that source approval does not show the hosted
proof running. Local's design inspection also approved the head. It accepts the
L161-9 closure: best-effort stop returns false before acting when the current
selection is missing, empty, unreadable, or invalid; the destructor adds no
second panic; the root is retained; and the explicit stop requirements are
unchanged. It records the same limit on the regression and is static design
approval only. The implementer requested fresh Watcher and Operator Station
attestations. Required CI run 39810406366 at this head completed with failure at
16:15:00Z; 16 of 17 jobs passed. The required official v0.2.1 Rust-harness proof
job 119934799074 (`Released v0.2.1 upgrade proof (Windows)`) succeeded, as did
the live PostgreSQL parity job 119934798990, the Windows `InstalledCurrent`
PostgreSQL process proof job 119934799083, and the Ubuntu build-and-test job
119934799183. Required job 119934799106, `Build and test (windows-latest)`,
failed in its Test step: in `tests/daemon_process_sqlite.rs`,
`real_process_copilot_fallback_cold_start` panicked at line 24 because the send
to the cold-start fallback exited with code -1073740940 (0xC0000374, which
Windows names STATUS_HEAP_CORRUPTION), and that test binary reported 47 passed
and 1 failed. The App observer retained the failed job log. No root cause or
relevance to this head is inferred, no rerun is recorded, and diagnosis belongs
to the implementer. The run grants no gate or readiness credit, and the
successful proof job alone does not accept E161-1. Watcher's pre-merge
attestation approved source consumability at `dacf3d18` and returned changes
required for runtime qualification because of that failed required job. It is
not the post-merge consumer gate. Operator Station's attestation likewise
approved source consumability and returned changes required for runtime,
withholding runtime consumer credit under blocker OS-PR161-DACF3D1-R1 for the
same failed Windows test. It records that the public consumer fixtures and the
incomplete-drop regression passed and that OS-PR161-0486BEE-1 remains closed.
Neither consumer infers a cause or merge readiness. No `631aff22` or earlier
evidence carries over. At `dacf3d18`, Local's source review and design
inspection had approved; the Application Client design approval is reported by
the implementer without a supplied artifact; Watcher and Operator Station
withheld runtime credit; and exact-head CI failed in a required Windows test
job. E161-1 and the I161-1 impact and isolation questions remain open, the
I161-1 workstation preservation and freeze are unchanged, and no merge, release,
tag, or fleet action is authorized.

The implementer then pushed head `52abb30e` (tree `e29d6b40`), nine commits over
`dacf3d18` that change five paths (+359/-7): the CI workflow,
`src/station_intent.rs`, a new Windows crash-evidence capture script under
`tests/`, the SQLite daemon process test, and the release contract test.
Relative to `4cd34938` the successor now changes eight paths (+851/-21), and
`src/station_intent.rs` is its first production source change. That change makes
slot acquisition and worker completion for a cancellable filesystem phase share
one absolute deadline-plus-grace boundary, instead of giving each its own grace
window; the implementer reports that it fixes a recurring race in cooperative
grace. The SQLite process test now treats any Windows child exit status above
255 as abnormal and fails, and a unit test maps -1073740940 to 0xC0000374. CI
adds Windows Error Reporting crash dumps, full debug symbols, symbol snapshots
before each process-test profile runs, gates that fail on collected or delayed
crash dumps, crash-evidence upload, and a separate Windows heap diagnostics job
with a 45-minute timeout that applies full PageHeap to the `telex.exe` image and
runs the SQLite daemon process test target up to three times, stopping at the
first failure. The implementer reports that it found no defensible source cause
for the earlier 0xC0000374 exit at `dacf3d18`. It also reports compile-only
validation on Rust 1.99 and 1.98, a clean YAML lint, and it requested Local's
source and design reviews and fresh Watcher and Operator Station attestations.
It then supplied its fresh gpt-6-astra receipts (high reasoning, long context):
an exact-delta source review and a design revalidation, both APPROVED with zero
findings. Both grant static source or design credit only and establish no hosted
CI success, heap-corruption cause or fix, merge readiness, or I161-1 closure.
Watcher's attestation approved source consumability: all Watcher-facing public,
fixture, backend, daemon, and contract files are unchanged; it treats the
product change as an internal Station scheduling correction outside Watcher's
public client seam; and it finds no change to the Watcher contract or accepted
consumer semantics. Runtime qualification awaits all 18 jobs of exact-head CI,
and the attestation grants no merge or post-merge credit. Local's source review
approved the `dacf3d18` to `52abb30e` delta with no findings. It accepts the
single deadline-plus-grace boundary, the abnormal Windows exit handling, and the
crash-dump, symbol-snapshot, PageHeap, dump-gate, and artifact-upload sources.
It notes that the existing Windows build-and-test job checks out the provider's
merge commit rather than the exact head and uploads its artifacts only on
failure, that it did not independently verify that saved executables and symbol
files match, and that it infers no heap cause or fix and no CI, E161, incident,
runtime, or merge result. Local's design inspection, run with gpt-6-astra at
high reasoning and long context, approved the static delta with no findings: the
grace repair preserves the accepted scheduling and cancellation boundary, and
the Windows status checks and hosted crash-evidence workflow strengthen failure
visibility without adding daemon authority or weakening the `InstalledCurrent`
and cleanup contracts. It records that the Station change is production code;
that PageHeap covers only the `telex.exe` image; that a ten-second wait before
each dump check does not prove Windows Error Reporting has finished writing;
that the existing build-and-test job uploads artifacts only on failure and its
artifact name does not prove it built the exact head; that no independent check
matched the saved symbol files to the executables; and that the 0xC0000374 cause
and fix remain unproven. It grants static design credit only, with no runtime,
E161, or merge credit. Operator Station's attestation approved source
consumability and the Station scheduling design with no blocking finding: the
shared boundary preserves mandatory cooperative partial-page progress without
granting contention a second grace period, and the remaining changes are
fail-closed Windows test and CI diagnostics. It marks its `dacf3d18` runtime
blocker OS-PR161-DACF3D1-R1 as superseded rather than closed, and it requires
terminal run 39819692324 with all 18 jobs successful before runtime credit.
Required CI run 39819692324 started at this head with 18 jobs, including the
official v0.2.1 proof job 119967372167, the live PostgreSQL parity job
119967372326, the Windows `InstalledCurrent` PostgreSQL process proof job
119967372106, and the Windows heap diagnostics job 119967372231. The run
completed successfully at 2026-10-22T17:33:08Z with all 18 jobs passing while
the PR stayed open and non-draft at this exact head, and GitHub then reported
the PR as mergeable with a clean merge state. The official v0.2.1 proof artifact
(ID 12443976768, digest
`sha256:8bb7557f0525c7b5e9fd657b890c78b6d07675f4fd1be81f6de4eeaa97044af7`) holds
a passing report (`report.json`, 3,012 bytes, SHA-256
`bfecda7380007cf617c00227c429cd150006787c27e75635eecb18248ab864be`). In it, the
candidate built from this head was installed with SHA-256
`da80f28e13d73f9fd54f2925ed4acd0aa937aa2a742f8725d49d20d62ebb7a0c`; the released
writer came from the official v0.2.1 Windows asset (SHA-256
`f5ffdaaa252d21edeb10d086a34bdcc67327b96db541e422ea725458d33090d0`); the old
writer's manifest had no admission field and kept its hash, modification time,
and file identity; and the proof made one probe before spawning, observed parent
and child admission and public attach, and saw the daemon exit. The heap
diagnostics artifact (ID 12444406553, digest
`sha256:2a9888132e1354fee55e4bc4a3cffb618e369a3e9e0e85d871f03a7da8cfe0db`) holds
three complete runs of the SQLite daemon process test target, each passing 49 of
49 tests (log SHA-256
`06f4eeba69ecefcc6b36ef3eb0163ff308f3fc5185a17551281bf9cc0b075f8f`,
`6823343a6fec42bd74ab8f33de7539d54f658a093b3fc63926c834998d21e0c6`, and
`e0eda4a712b2b223bda4d991345f5827a9d5f5df517dbeb4b0a73000710461b8`), and no
crash dump. These results give this head CI credit only: they identify no cause
for the earlier 0xC0000374 exit, the passing proof does not by itself accept
E161-1, and runtime consumer credit still requires Watcher and Operator Station
to qualify the head. The implementer's completion report attributes the repeated
partial-page failures to slot admission spending the cooperative deadline before
grace applied, which the shared deadline-plus-grace boundary fixes without a
second grace window. It claims no heap-overwrite cause and no heap-corruption
fix, and it reports passing local formatting and diff checks, Rust 1.99 and 1.98
compile-only builds, warnings-denied Clippy on the affected code, a PowerShell
syntax check, and YAML lint, with no workstation actions. App's merge preflight
found that main is 27 commits ahead of the PR's recorded base `2c2b9fb5`, all of
them changes to six `.streamliner` bookkeeping files and none to product paths,
so the head needs no rebase for them; the PR has no review threads, and its
issue remains open. Watcher's runtime-qualified pre-merge attestation
(`watcher-pr161-52abb30-premerge-consumability-attestation.md` in session
e00d296e-ffca-4477-b1c8-c4d340bd30be, 10,121 bytes, SHA-256
`e938504e1baff83ea02a19c6e318dff128abbdc3f0e897a1865342c4ba623c18`) then
approved the head: Watcher can consume the Application Client through supported
public surfaces alone, and all 18 jobs passed. Operator Station's runtime
attestation
(`operator-station-pr161-premerge-consumability-attestation-52abb30e-runtime.json`
in session 002c5122-fc0d-403c-8f87-625d87385d44, 12,018 bytes, SHA-256
`f9cf152e10cc778bd41554cac5fcda4dc10aaf28822662312cee8a4e28fd8f07`) also
approved it and granted runtime consumer credit, citing the Windows
build-and-test job's passing partial-page regression, previously failing
cold-start fallback test, and Windows exit-status test. It keeps
OS-PR161-DACF3D1-R1 superseded rather than closed and OS-PR161-0486BEE-1 closed
at this head. Both are pre-merge attestations; neither is the post-merge
consumer integration gate or a merge-readiness finding. App then authorized a
normal guarded merge of exact head `52abb30e` after a final live check at
2026-10-22T17:35:51Z found the PR open, non-draft, and mergeable with a clean
state, all 18 checks passing, no review threads, and the issue open. The
authorization is bound to that head and its guards and covers only the normal PR
merge; it grants no release, tag, fleet, issue-closure, Station application, or
post-merge gate authority, and it does not record acceptance of E161-1 or an
I161-1 impact and isolation disposition. PR #161 then merged at
2026-10-22T17:36:38Z as merge commit `4d25fdff375dcfd55b4a7b4613a5b5ecb8241bdb`
(tree `c57cdbd7b4ece031a01e6a04d4d740fd21ae8b62`), whose parents are main
`2b1ceb4c` and the reviewed head `52abb30e`; outside `.streamliner`, its tree
matches the reviewed head. The implementer posted a field report (issue comment
6347164525) and a merge receipt (issue comment 6347180924), and the issue
remains open. Push CI run 39824226010 for the merge commit was in progress when
this record was made. App has asked Watcher and Operator Station for the formal
post-merge consumer integration gate attestations. Operator Station then
approved the formal post-merge consumer integration gate for its side on exact
merged main `4d25fdff`
(`operator-station-application-client-consumer-integration-gate-4d25fdff.json`
in session 002c5122-fc0d-403c-8f87-625d87385d44, 10,372 bytes, SHA-256
`18d7b486d87d521393537741143364a1336ff278aa6a37a6d8f03458ea373b64`): it marks
its side of the gate satisfied, finds that the merge rewrote no reviewed public
consumer path, and requires no private fallback. It conditions any advance of
`station-app` on canonical completion of `client-conformance`, completion of the
combined consumer integration gate with all required receipts, and
reconciliation of both Operator Station graph dependencies; `station-app`
remains planned and unlaunched. Watcher then passed the formal post-merge
consumer integration gate for its side on the same merged main
(`watcher-application-client-postmerge-gate-4d25fdf.md` in session
e00d296e-ffca-4477-b1c8-c4d340bd30be, 9,021 bytes, SHA-256
`3fc149b6f6cab0401bdecf661686efed9666b52a3413a2207fc5bcc1d771a961`). It verified
that the merge's second parent is the reviewed head and that every load-bearing
public consumer byte is identical in the merge tree, and it discharged the
Application Client hold on `watcher-runtime-core`. It authorizes no
implementation launch, because the Watcher runtime task also requires a reviewed
and reconciled runtime tracker, which does not yet exist. With both required
consumer receipts recorded, App directed that the consumer integration gate be
marked passed and `client-conformance` completed on main. This record does both
and completes the `supported-client` checkpoint, whose four nodes are now
complete. No release, tag, fleet, or launch action followed the merge. No
`dacf3d18` or earlier evidence carries over. At `52abb30e`, Local's source
review and design inspection, the implementer's source and design reviews, and
Watcher's and Operator Station's source and runtime attestations have approved
the head, its required CI passed, the PR merged as `4d25fdff` under App's normal
guarded merge authorization, and both consumers passed the post-merge consumer
integration gate. The implementer's field report states that the permanent
hosted Windows proof using the official v0.2.1 asset resolves E161-1; this
record attributes that resolution to the implementer and adds no independent
acceptance. The I161-1 impact and isolation questions remain open, the I161-1
workstation preservation and freeze are unchanged, issue #152 remains open, and
no release, tag, fleet, issue-closure, or launch authority is granted.

The workstation run of the released-v0.2.1 upgrade proof (E161-1) was not
accepted. At `d08e764c`, the official v0.2.1 executable staged the candidate
into a fresh fixture root and public attachment succeeded. The proof script,
however, also rewrote the access control list of the operator's real
`AppData\Local` directory in several runs, and no prior descriptor or backup
exists. That incident, I161-1, withholds isolation certification. On 2026-10-22
the operator chose to preserve the current host access control lists and
evidence and to have the existing owners continue a bounded read-only impact
review. That choice resolves only the initial operator response. It authorizes
no restoration, inferred default, inheritance change, recursive reset, other
access-control change, proof rerun, or cleanup. Impact assessment and isolation
acceptance remain open; the choice did not accept E161-1, and it does not
establish an absence of impact, close the incident, or grant merge credit. Local
runtime and proof replay remain frozen, so runtime validation relies on CI;
ordinary source and CI work may continue within that freeze. It does not change
the gate evidence for PR head `4cd34938`, which remains scoped to that exact
head. The implementer pursued E161-1 through a hosted CI proof instead: the
first attempt at `5d21a63d` failed; the Rust-harness replacement proof job
passed at `dacf3d18`, but that head's CI run failed in a required Windows test
job; and the required CI run at head `52abb30e` passed, after which PR #161
merged as `4d25fdff`. The implementer's field report states that the permanent
hosted Windows proof using the official v0.2.1 asset resolves E161-1; this
record attributes that resolution to the implementer and adds no independent
acceptance. The node is complete; issue #152 remains open, and this record does
not close it.

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
- Exact-head public-seam attestations and post-merge gate receipts from Watcher
  and Operator Station for the completed `consumer-integration-gate`.

## Closeout Observations

- Issue #12 publication revision 4 records the design checkpoint, supported Rust
  core, and first supported Rust binding without claiming conformance.
- W-05 taxonomy wording from issue #124 completed within client-core issue #129
  and merged through PR #132.
- The Rust-first `first-binding` completed through issue #149 and PR #151.
  Issue #152 `client-conformance` completed through PR #161, merged as
  `4d25fdff`; the consumer gate and `supported-client` checkpoint are complete,
  and issue #152 remains open.
- Polluted PR #123 and its dirty worktree are deferred with rationale for
  protocol forensics; cleanup requires explicit operator authorization.
- The clean #118 implementation worktree is also deferred for cleanup until the
  operator requests it.
- Keep API convenience, additional bindings, and consumer-specific ergonomics
  out of contract work unless they expose a missing semantic. Any requirement
  that cannot be accepted must name its blocked consumer and owner rather than
  being softened into ambiguous shared wording.
