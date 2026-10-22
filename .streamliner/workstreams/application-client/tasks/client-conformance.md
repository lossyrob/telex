# Application Client conformance and consumer consumability

- **Workstream:** `application-client`
- **Node:** `client-conformance`
- **Type:** implementation
- **Status:** in progress; PR #161 is open at head `4cd34938`, which is intended
  to address Operator Station finding OS-PR161-0486BEE-1 at `0486beeb`; Local's
  source review and the retained delta review approved it, its required CI
  passed, the Watcher and Operator Station pre-merge consumer attestations and
  the Local Daemon design inspection approved it, a fresh gpt-6-astra
  Application Client design inspection is pending, and proof-isolation incident
  I161-1 is unresolved while its user-directed read-only impact review continues
- **Attention:** focus
- **Depends on:** completed `client-core`, completed `first-binding`
- **Tracker:** [lossyrob/telex#152](https://github.com/lossyrob/telex/issues/152)
- **Parent workstream:** [lossyrob/telex#117](https://github.com/lossyrob/telex/issues/117)
- **Campaign:** [Addressable Attention #102](https://github.com/lossyrob/telex/issues/102)

## Outcome

Deliver one executable conformance bundle for the supported Rust Application
Client. The same public `telex::application_client` cases must pass against
isolated SQLite and credentialed Postgres. Public-only Watcher-shaped send-only
and Operator Station-shaped bidirectional fixtures must prove that both
consumers can use the supported seam without CLI parsing, raw private daemon
IPC, spike helpers, consumer DTO promotion, or a product-private client. Runtime
fixtures must use the explicitly trusted installed-current Telex daemon policy;
compile-only evidence is insufficient.

This is one node, one tracker, and one delivery PR. It completes
`client-conformance` and supplies evidence for the later
`consumer-integration-gate`. It does not implement either product, pass that
gate by itself, publish a release, or establish production readiness.

## Authority

- [Issue #152](https://github.com/lossyrob/telex/issues/152) - tracker and
  complete node outcome; verified ASCII body SHA-256
  `D37C020B141801A106B7089EC61748C9810AEC6049EFE58C80227C576FA66AD0`.
- [Issue #12](https://github.com/lossyrob/telex/issues/12) - sole semantic owner,
  publication revision 4.
- [`../design/current-design.md`](../design/current-design.md) - canonical
  integrated design and conformance boundary.
- [`../../../../docs/design/application-client.md`](../../../../docs/design/application-client.md)
  - normative API-neutral contract, AC-C01 through AC-C20.
- [`../../../../docs/application-client-core.md`](../../../../docs/application-client-core.md)
  - supported Rust binding and the ten required conformance families.
- [`../discovered-work.json`](../discovered-work.json) - accepted
  `application-client-daemon-bootstrap-gap-v1` disposition and its cross-reference
  to Local Daemon trust item `application-client-daemon-bootstrap-trust-v1`.
- [`../../local-daemon/design/current-design.md`](../../local-daemon/design/current-design.md)
  - Local Daemon ownership of install authority, selector admission, process
  authentication, and upgrade/rollback publication.
- [`../../../../docs/notes/application-client/requirements-crosswalk.md`](../../../../docs/notes/application-client/requirements-crosswalk.md)
  - non-normative Watcher W-01 through W-15 and Operator AC-01 through AC-15
  traceability.
- [`../graph.json`](../graph.json) and
  [`../../../shaping/roadmap.md`](../../../shaping/roadmap.md) - accepted node,
  gate, and campaign ordering.

## Bundle rule

Keep every node-owned proof in issue #152 and one PR. Do not split by backend,
test family, fixture, migration, cleanup, or reviewability. A split requires an
independently useful confidence, release, provider, or cross-repository boundary
with explicit join conditions and campaign approval.

If a real prerequisite appears, preserve the same branch and PR, model the
owning `dependsOn` or `externalDependsOn` entry with the exact resume condition,
and continue after it closes. Node-owned unfinished work is in progress, not
blocked. If no modeled dependency owns a real blocker, report `blocked` with
`graphGap: true`; do not downscope the result or present an incomplete PR as
complete.

## Required conformance matrix

Run the same public Rust semantic cases against isolated SQLite and credentialed
Postgres:

1. Fresh runtime identity with stable application responsibility and logical
   store identity across reconnect and path or profile presentation changes.
2. Strict and bounded recovery; restart membership loss; deliberate detach;
   predicate death; owner demotion; collision evidence; and raw preservation of
   unknown future loss reasons.
3. Atomic-or-compensable multi-address attach, reconcile, and detach, including
   new membership -> `Detach`, changed existing membership ->
   `Reattach(previous_spec)`, idempotent refresh -> no destructive compensation,
   cancellation evidence, and crash continuation.
4. Send-only false-attendance prevention and bidirectional receive with exact
   delivery-row identity and bound acknowledgement.
5. Independent acceptance, occupancy, push, recipient-consumption, and
   workflow-disposition evidence, including acknowledgement-after-durable-ingest
   restart recovery.
6. Retry-stable operation replay; fingerprint, payload, store, and duplicate
   evidence; authoritative exact-tuple `NotRecorded`; retention-boundary
   invalidation; accepted-send indeterminate windows; prepared recovery handles;
   and post-restart reconciliation.
7. Unresolved, recent, and thread filtering before bounds, plus store-scoped
   source resolution and fail-closed source ambiguity.
8. Monotonic delta ordering, gap detection, resync, and no-regression backfill.
9. Compound prerequisite ordering, partial and indeterminate outcomes, recovery
   handles, terminal-step fencing, and crash continuation.
10. Schema v2-to-v3 migration, newer-schema refusal, bounded cleanup, retention
    generations, principal provenance, and exclusion of raw paths, credentials,
    backend rows, daemon frames, and private storage details from public
    evidence.

Reuse established backend, schema, Postgres-service, and test-isolation helpers.
Prefer plain repository tests and fixtures over a new conformance framework or
evidence schema unless a concrete missing failure requires one.

## Installed-current daemon bootstrap

Absorb `application-client-daemon-bootstrap-gap-v1` in this PR. Production
Watcher and Operator Station fixtures use this additive public surface:

```rust
#[non_exhaustive]
pub enum ApplicationDaemonBootstrap {
    InstalledCurrent { trusted_root: PathBuf },
    ExactExecutable { executable: PathBuf },
}

impl ApplicationClient {
    pub async fn connect_with_daemon(
        config: ApplicationClientConfig,
        daemon: ApplicationDaemonBootstrap,
    ) -> Result<Self, ApplicationClientError>;
}
```

`InstalledCurrent` is the production path. The additive constructor preserves
existing `ApplicationClientConfig` struct literals.
`ApplicationClient::connect(config)` and `ExactExecutable` retain exact-current
or exact-target development and test support only. Neither is an implicit
fallback from failed installed-current resolution. Exact-target support applies
the same canonical process-image, platform file-identity, and
untrusted-writability checks; it remains pinned, has no installed manifest
authority, and does not follow upgrade or rollback. Installed-current
configuration rejects a relative root and captures one immutable canonical
absolute root before any connection or reconnect.

For every installed-current connect-or-spawn cycle:

1. Obtain the Local Daemon-owned shared/read lease on the OS-backed selector
   coordination lock beneath the explicitly trusted absolute install root.
2. Canonicalize and validate the trusted root and its authority chain. Reject a
   root, selector, manifest, version directory, or executable controlled by an
   untrusted principal, or redirected through an unsafe symlink/reparse point.
   Require current-OS-user ownership and deny write, delete, or ownership
   control to another principal. Owner writability remains permitted for
   same-user upgrade.
3. Read one `current` tag and its manifest. Derive
   `<root>/versions/<tag>/telex[.exe]`; do not accept an arbitrary manifest path.
   Require the canonical version directory and executable to remain beneath the
   canonical root.
4. Validate exact tag, package version, build identity, schema range, protocol
   compatibility, and required security and Application Client capabilities.
   Require the manifest to bind the selected tag and executable.
5. Freeze one internal resolved target containing the canonical executable,
   platform file identity, selected tag, manifest identity and load-bearing
   fields, build identity, and compatibility facts. Use that same value for
   spawn and pre-`Hello` server authentication.
6. Hold shared admission through reuse or spawn, reuse-safe OS peer identity
   checks, compatibility negotiation, and successful `HelloAck`. Before binding
   its serving endpoint or publishing capability or readiness, the spawned
   daemon independently acquires shared selector admission; validates the
   captured selection token, a fresh installed-current resolution, selected
   manifest/build metadata, and its own process image; and holds its lease
   through publication. The parent retains its separate lease through
   authenticated `HelloAck`.

If either process dies, the remaining or next admission still prevents
stale-child publication. A token, image, or lock mismatch exits without
serving.

Upgrade and rollback hold the same selector lock exclusively across validation,
matching-daemon drain, predecessor exit, atomic `previous`/`current` switch, and
selector publication. Shared and exclusive admission prevents
resolve-old -> drain -> switch -> spawn-old. Lock acquisition or token
validation is bounded and fails closed where equivalent Windows and Linux
ownership/locking semantics cannot be proved. The implementation must not
accept both `current` and `previous`, resurrect a stale target, or clean up a
stale spawn after publishing authority.

The selector lock is one owner-restricted persistent file under the trusted
root. Unix uses a local-filesystem advisory shared/exclusive lock with
process-crash release; Windows uses the equivalent `LockFileEx` range lock. The
lock file is never replaced or deleted as part of selection. Lock order is
selector admission before daemon singleton/spawn admission; a drain served
under exclusive admission does not reacquire the selector lock. While holding
shared admission, the client re-reads and compares the complete selection
immediately before spawn. Unsupported ownership, filesystem, or lock semantics
fail closed.

A prestarted daemon is reusable only when its authenticated process image
matches the frozen current target's UID or SID, canonical path, and platform
file identity before `Hello`. `HelloAck` then proves the selected build
identity, auth policy, protocol, and capabilities. Linux uses an open
process-image descriptor plus device/inode identity. Windows captures canonical
final path and volume/file identity from handles and holds the executable handle
with compatible sharing through process creation. A missing, incompatible,
replaced, or foreign target produces
`ApplicationClientError::DaemonBootstrap(DaemonBootstrapFailure)` with one of
these typed reasons: `InvalidTrustedRoot`, `UnsafeInstallAuthority`,
`MissingCurrent`, `InvalidManifest`, `IncompatibleManifest`,
`SelectionUnstable`, `MissingExecutable`, `ExecutableIdentityMismatch`, or
`ForeignDaemon`. Durable public evidence does not expose raw authority paths.
The client never drains, kills, trusts, or starts beside a foreign daemon.
Manifest, canonical-path, and platform-file-identity checks do not claim
executable-content integrity, signature, publisher or package provenance,
protection from malicious same-user administration, or intra-user isolation.

Local Daemon owns the install layout, manifest/build contract, selector lock
and token, daemon process admission, OS peer checks, readiness publication, and
upgrade/rollback coordination. Application Client owns the public policy type,
config-compatible constructor, typed failure projection, and use of the
supported admission flow. Preserve the caller-owned Tokio runtime and explicit
membership reconciliation after daemon restart.

## Consumer-shaped evidence

The PR must include thin public-only fixtures or probes for both capability
families:

- **Watcher send-only:** defaults-disabled backend selection, stable
  responsibility/runtime/store identity, durable acceptance separated from
  occupancy/push/consumption/disposition, exact-operation recovery,
  retention-boundary behavior, SQLite/Postgres parity, and no false inbound
  attendance. The external fixture resolves and runs through
  `InstalledCurrent { trusted_root }`.
- **Operator Station bidirectional:** multi-address lifecycle and compensation,
  receive/acknowledgement after durable ingest, unresolved/history/thread
  recovery, metadata-bearing ordinary reply, exact-recipient disposition, source
  resolution, compound `Reply & Handle` ordering, health, delta/gap/resync,
  deliberate detach, cleanup, and provenance. The external fixture resolves and
  runs through `InstalledCurrent { trusted_root }`.

These fixtures use only the shared public client. They do not promote consumer
DTOs or implement detectors, scheduling, presentation, notifications,
mediation, installation, usability, or product workflow policy.

Any missing implementation of an already-accepted shared semantic required by
either merged consumer contract is repaired in this PR. A newly required public
semantic triggers `decision-needed`. Product-only behavior is routed to its
owning downstream node.

## Consumer attestations and gate evidence

After implementation review and required CI succeed on one exact head, but
before merge readiness:

- Watcher authority independently checks the exact bundle against its merged
  requirement set and confirms that no CLI, raw IPC, or Watcher-private client
  is needed.
- Operator Station authority independently checks the same exact bundle against
  its direct-attendance contract and confirms that no CLI, raw IPC, spike
  helper, product DTO promotion, or Station-private client is needed.

Both attestations name the same reviewed and green head. Head movement
invalidates them. After conformance merges, the attestations become evidence
for `consumer-integration-gate`; they do not pass the gate or authorize
consumer launch from this PR. Product runtime, UI, usability, packaging, and
operational evidence remain downstream.

## Work conservation and discovered work

Every material discovery from shaping, implementation, review, CI, runtime
proof, consumer attestation, design inspection, or the field report receives one
durable disposition in
[`../discovered-work.json`](../discovered-work.json).

- Required current-PR work is absorbed into this PR.
- A real prerequisite is modeled, and the same PR resumes after it closes.
- Product-only work is routed to the owning existing node or external tracker.
- Rejected work records why its cost exceeds concrete incremental value.

No material item may remain `untriaged` at merge readiness. Accepted same-PR
items must be delivered; downstream items need an owner and connected terminal
path.

## Out of scope

- Watcher detector/runtime/CLI implementation or Operator Station UI,
  notification, mediation, installation, and usability behavior.
- Passing `consumer-integration-gate`, launching either consumer, or completing
  `supported-client` from this PR.
- TypeScript/napi-rs, a separate client crate, C ABI, public socket or sidecar
  protocol, or consumer-specific DTO contract.
- Release publication, signing, distribution acquisition, general installer
  UX, broad packaging, and operational hardening beyond the installed selector,
  manifest, executable identity, and upgrade/rollback coordination required to
  prove runtime consumability here.
- CLI parsing, raw private daemon IPC, subprocess courier behavior, or any
  product-private fallback.
- `PATH` discovery, implicit default-root trust, embedded daemon `serve`,
  consumer-as-daemon execution, application-specific daemon or sidecar, public
  connector injection, direct backend transport, capability-file-only trust,
  accepting `current` plus `previous`, or foreign/exact-target fallback.
- Streamliner mutation, gate operation, or issue #12 publication updates from
  the implementation branch.

## Success criteria

- One backend-neutral public conformance suite executes every required family
  against isolated SQLite and credentialed Postgres with equivalent typed
  outcomes.
- Credentialed Postgres is mandatory in authoritative CI; a missing required
  environment fails instead of becoming success-shaped skip evidence.
- Public-only send-only and bidirectional consumer fixtures compile under
  documented `default-features = false` profiles and run through
  `InstalledCurrent { trusted_root }` on Windows and Linux against isolated
  SQLite and credentialed Postgres. They exercise representative lifecycle,
  messaging, recovery, history, and resync paths.
- Installed-current tests prove trusted-root and authority-chain checks,
  manifest/build/version compatibility, immutable tag behavior, shared/exclusive
  selector admission, persistent-lock lifetime and ordering, matching-only
  prestarted reuse, Linux descriptor/device/inode identity, Windows
  handle/volume/file identity, missing and foreign target failure,
  symlink/reparse rejection, concurrent spawn, selector-client death during
  spawn, child admission failure, upgrade, rollback, selector contention,
  daemon crash/restart, and no stale-version resurrection.
- The suite proves exact identities, typed failures,
  cancellation/indeterminate evidence, compensation, retry/reconciliation,
  migration, cleanup, and provenance without making backend-private assertions
  part of the public contract.
- No product-private seam, hidden runtime, sidecar, new ABI/process boundary,
  consumer DTO authority, or unrelated framework is introduced.
- Watcher and Operator Station provide exact-head consumer-authority
  attestations after review and CI.
- Every modeled blocker is closed and every material discovery is durably
  dispositioned.
- The final exact head has contiguous implementation review coverage, no
  unresolved actionable thread, successful required CI, both consumer
  attestations, and a passing fresh design inspection.
- The PR closes issue #152, contains only product deliverables, and contains no
  `.paw/**`, `.streamliner/**`, workflow transcripts, or scratch artifacts.

## Validation, review, and reporting

- Run formatting, warnings-denied Clippy, the complete Application Client
  SQLite suite, the same credentialed-Postgres conformance battery, schema
  migration/newer-schema tests, external public-consumer fixture profiles, and
  the repository feature matrix.
- Run the complete installed-current fixture matrix on Windows and Linux for
  SQLite and credentialed Postgres. Cover exact matching and mismatching
  prestarted images, selector movement under shared/exclusive admission,
  manifest and executable replacement attempts, protocol/capability skew,
  successful upgrade/rollback, failed drain/switch, parent death after spawn,
  child admission loss, and daemon-side rejection before
  endpoint/cap/readiness publication.
- Keep destructive or persistent tests isolated with unique temporary SQLite
  paths and per-test Postgres schemas. Do not operate against installed or user
  coordination state.
- Start review and CI concurrently only when the complete node bundle is
  present. Establish one full PAW baseline for the first stable,
  feature-complete head. Later clean deltas produce internal approval without
  another GitHub comment.
- After implementation review and required CI pass on one exact head, obtain
  the two consumer attestations and a fresh read-only design inspection. Use a
  new gpt-6-astra, high-reasoning, long-context inspector child, as the
  operator's standing instruction for all new sessions and delegates requires,
  with no silent substitution; that instruction supersedes the Claude Opus 4.8
  profile this task previously named. Any head movement invalidates same-head
  review, CI, attestation, and inspection evidence as applicable.
- Use the campaign wait coordinator only for immutable external preparation,
  CI, mergeability, or provider-run conditions. Child review, inspection, and
  design results use direct App messages.
- Before completion, post an issue #152 field report covering outcome, exact PR
  head, decisions, validation, review and CI coverage, consumer attestations,
  design inspection, discovery dispositions, deferred work, downstream impact,
  and clean-worktree state.

## Product design promotion

The same PR must promote this accepted split into product authority:

- add reserved ADR 0053 in `docs/design/DECISIONS.md`;
- update `docs/design/application-client.md` with explicit installed-current
  selection and typed Application Client responsibility;
- update `docs/design/daemon.md` with trusted-root/manifest/executable authority,
  shared/exclusive selector admission, captured-token validation, pre-`Hello`
  process authentication, and upgrade/rollback publication;
- update `docs/application-client-core.md` with the exact public Rust types,
  migration guidance, feature/source-pin behavior, and runtime examples.

The paired direct-main Tier B packet reconciles the Application Client and Local
Daemon `current-design.md` files before the implementation worker resumes. The
implementation branch does not edit `.streamliner/**`. The sole reconciler later
records completion evidence after the product design and implementation land.

## Source compatibility transition

Published v0.2.1 (source `212b76a4c586101bdc2a53264e2a4c3e2326671a`) contains
the Rust binding with an exhaustive `ApplicationClientError`. Adding
`DaemonBootstrap(DaemonBootstrapFailure)` and marking the enum
`#[non_exhaustive]` is a breaking source change. On 2026-10-08 the operator
authorized a source-only transition to 0.3.0 in this PR (campaign decision
`client-conformance-semver-decision-20261008.json`, SHA-256
`ec6fadc0a99628bd6bcedee912bf2c15ed7d20768d6c17bd2c72117913d0a8fc`).

The same PR:

- moves the root package `Cargo.toml` version and its `Cargo.lock` entry from
  0.2.1 to 0.3.0;
- updates every plugin, marketplace, bootstrap, proof-default, and version
  fixture surface that existing lockstep metadata and release-contract tests
  couple to that version, without weakening or decoupling those tests;
- keeps independent workspace and dependency versions, minimum supported
  versions, and historical or generic fixtures unchanged unless genuinely
  coupled;
- corrects public documentation to name v0.2.1 as the released baseline and
  0.3.0 as an unpublished source version; and
- documents source-consumer migration for typed bootstrap errors and
  non-exhaustive matching.

Decoupling versions or weakening those tests needs a separate policy
decision. The transition does not change daemon protocol or schema versions,
add a node, or split the bundle. It grants no tag, Release workflow dispatch,
GitHub, crates.io, or plugin publication, and does not alter v0.2.0 or v0.2.1.

## Dependency and promotion

`client-core` and `first-binding` are complete. Issue #152 alone does not make
this node ready: this task, the discovery ledger, and cross-workstream
dependency reconciliation must land on `main` first.

After that authority lands, the Application Client orchestrator resumes the
frozen issue #152 implementation worker on its existing branch and PR. Do not
launch a replacement worker or split the node. Consumer work remains blocked
until conformance merges, both exact-head attestations are accepted, and
`consumer-integration-gate` is separately passed and reconciled.

On 2026-10-08, under the operator-approved post-release direction recorded in
campaign directive `campaign-postrelease-execution-directive-20261008.json`
(SHA-256 `a4d98af7d891efe7164839c79b4431d07acfc1ceef606c54e46b82ab7eb5613d`),
the Application Client orchestrator resumed the same writer on the existing
`feature/client-conformance` branch at preserved clean head
`5fdd95c2c32cde08a6c936171b208bbf14f6677c`. No PR existed. The writer is
merging current main `2c084873719080b812e0d0c4bf92125afa751499` by ordinary
merge before full validation, exact-head review, and publication of the sole
delivery PR. Scope, invalidation conditions, and the accepted trusted
`InstalledCurrent` policy are unchanged.

On 2026-10-22 (UTC) the writer published the sole delivery PR, PR #161, at
exact head `9cc23e67a99440ee51e7b60fcf8d68f6b84edef3` (tree
`b678e0cc4eca2b598e5a7cae0fe02488523532b4`). Two retained independent reviews
of the full diff from merge base `e4f585a7` reported no actionable findings at
that head. Local's trust and spawn review of the same head required changes
(report SHA-256
`3ef421c8bac6028bec601204b23e1c0660da3bfaa91264e5aacdbdcfc72b6ab5`), and
required CI run 39716184110 completed with failure there.

Repairs moved PR #161 through `d08e764c`, `5efe5bf9`, and `20fe13b4` to exact
head `8bd3c330393409a79e88338d24e1174b1c7407b6` (tree
`4d06daec9200db8de5196d85d5eb47a7d8f1a96a`). Local findings L161-1 through
L161-5 covered probe pipe draining under the selector lock, stale capability
evidence, Linux process-group signaling after reap, deliberate-detach
precedence, and a cancellation-unsafe PostgreSQL migration lock. Local's
bounded review approved `8bd3c330` with no findings (report SHA-256
`1e535b95db8a77856d1a129374d4092fbde0633264ca8b342a225462a0f61b0f`), and the
retained `gpt-6-astra` review of `5efe5bf9..8bd3c330` approved with no
actionable findings (artifact SHA-256
`e290edffee2cf0d5d17200944c0b7dc6ac8ced24e613e6aab3ec22bfdda8fe81`). Earlier
retained approvals of `5efe5bf9` and `20fe13b4` were superseded by later
Local findings. These approvals cover source only; Local ran no tests. CI runs
39723759906 at `d08e764c` and 39726078689 at `5efe5bf9` failed and are
diagnostic.

CI run 39730569101 at `8bd3c330` failed in 5 of 16 jobs: Build and test on
Ubuntu and Windows, Live Postgres parity, Windows `InstalledCurrent` Postgres
process proofs, and Windows Copilot fallback end-to-end (log SHA-256
`633fddc8ea0331db27b1de251a965971989f93089dc03090d779e4bc27d0a2b6`). The
writer then submitted source changes intended to address them in exact head
`2b8ade1d23214fcaffef68f6d2b0143977b9a7a5` (tree
`db4a8ce7a8a8988b4ec276366932f113be6ef8fa`), a five-path test and fixture
change of +93/-66 lines that moves Windows test roots directly below the
volume root and leaves production authority validation unchanged. Local's
bounded review approved `2b8ade1d` with no findings (report SHA-256
`f8c1bd9b01296f76395d51f5c4b4565713f26a5b5892d8fe3139b56e6cafd286`). The
retained review sequence of five `gpt-6-astra` invocations, run with high
reasoning and long context, found defects that the writer corrected, then
concluded with an approval of `8bd3c330..2b8ade1d` (artifact SHA-256
`d8c96183e0c2429065b3f7a61fed81d9051127e55a65a291f3a82317bb0da917`). These
approvals cover source only; Local ran no tests. The writer reports that
full-workspace all-target Clippy is blocked by warnings in unchanged targets;
it is not green.

Required CI run 39734925308 at `2b8ade1d` completed with failure at
06:50:27Z. Three required Windows jobs failed: `InstalledCurrent` Postgres
process proofs (job 119678269082), Copilot fallback end-to-end (job
119678269115), and Build and test (job 119678269126). The other 13 jobs
passed, including Live Postgres parity (job 119678269183). Each failure
reports unsafe install authority. The consumer fixtures' restart-sender and
watcher attaches were rejected, the station-intent upgrade test failed at
pre-upgrade install-root validation, and `daemon_bootstrap` unit tests failed
(log SHA-256
`a370e7fe276901d004e717991571306291915268373264ea3c4472140c0d70f0`). The
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
Local's bounded review approved `ec8b88f7` with no findings (report SHA-256
`58a282d5bb264211b7bd26cd820a4efe09a55deb3bf93e0d252b3ac12098b482`), and two
retained `gpt-6-astra` reviewers, run with high reasoning and long context,
approved `2b8ade1d..ec8b88f7` with no actionable findings (artifact SHA-256
`4d6b7dd73f8c5d307f43dbf86a6d0286cb7de71f8b795bb73f6b7c315baf3753`; product
merge base `e4f585a7`). The earlier source resolutions of L161-1 through L161-5
are conserved. These are source approvals only. CI run 39737943130 was started
at `ec8b88f7`; its result is not recorded here.

Required CI run 39737943130 at `ec8b88f7` completed with failure at 07:19:29Z.
Its only failed job was Build and test on Windows (job 119687972692). The
`application_client_conformance` test binary started four tests: one was ignored
and two reported ok. The process then exited with `STATUS_HEAP_CORRUPTION`
(0xc0000374) before printing a result summary (log SHA-256
`2f5719ade578737febd4304ebb0924a2ca28d9e5453b87bf1326ae5ff58502a4`). The other
15 jobs passed, including the required Live Postgres parity (job 119687972631)
and Windows `InstalledCurrent` Postgres process proof (job 119687972934) jobs.
Those two results are diagnostic evidence only: the run failed, and the head has
no CI or readiness credit. The `ec8b88f7` source approvals stand as source
review only.

The writer then submitted head `7505568cc57a10300f94a2f1e8ab5c88af5cea3b` (tree
`f532f9863a41d53c1dfef1cc8bc8596c994a7e6b`), a one-path change to
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

Local's bounded review approved `7505568c` with no findings (report SHA-256
`c2dbb3c570dfd636a699c4b4c4edf8cd5519fe841275c49a4e95ea5177155af6`). Two
retained `gpt-6-astra` reviewers, run with high reasoning and long context,
approved `ec8b88f7..7505568c` with no actionable findings (artifact SHA-256
`ccefe15201e1a9f84275473e7c66a499c28171567f33489310b62f684c7a284f`; product
merge base `e4f585a7`). That artifact is the updated version that records
Local's correction of the history above; Local hash-checked its earlier version.
Neither review shows that the earlier SID lifecycle caused the heap corruption,
that `7505568c` fixes it, or that it reduces runtime risk; only CI can establish
the runtime outcome. The earlier source resolutions of L161-1 through L161-5 are
conserved. These are source approvals only. CI run 39740654750 was started at
`7505568c`; its result is not recorded here.

Required CI run 39740654750 at `7505568c` completed with success at 07:45:05Z.
All 16 jobs passed, including the required Live Postgres parity (job
119696796390) and Windows `InstalledCurrent` Postgres process proof (job
119696796291) jobs. PR #161 was then open, not a draft, and cleanly mergeable at
that exact head against base `2c2b9fb5`. The CI gate is met for `7505568c` only;
any later head needs its own source review and passing required CI. Passing CI
does not resolve incident I161-1 or certify proof isolation. App reports that
consumer attestations have started; none is recorded as passed, and the formal
consumer gate stays planned. Design inspection, campaign authorization, and
merge remain outstanding.

Both pre-merge consumer attestations approved exact head `7505568c` (tree
`f532f986`) on 2026-10-22, and neither found a private fallback or an unaccepted
dependency. The Watcher attestation (session `e00d296e`, artifact SHA-256
`8c6a96dfa18a98f77f76777081d36949dd71847e37d19e8823338e5a8ae53667`) found that a
public-only consumer, built with default features disabled, connects through the
`InstalledCurrent` trusted root and that attach, restart, durability, recovery,
send-only, health, and backend-parity semantics align. The Operator Station
attestation (session `002c5122`, artifact SHA-256
`ea9a9a137d717747e958b866a5b2bb5ec0f4987c19b36837bb81cf16606bbb41`) found that
multi-address attach, readiness, compensation, exact-recipient ingest before
acknowledgment, restart reconciliation, reply, disposition, source, health, and
backend-parity semantics align through public surfaces. Both attesters ran
`gpt-5.6-sol` with high reasoning; their context tier was not recorded. These
attestations meet this task's attestation requirement for `7505568c` only, and
any head movement invalidates them. After merge they become evidence for
`consumer-integration-gate`, which stays planned; they do not pass that gate or
authorize consumer launch.

The Application Client design inspection (session `074125a5`, artifact SHA-256
`31c02030ad752a56c0e6a557a7a1fbbab23ad1178c105ffa031729be4b02f27a`) approved
`7505568c` against merge base `e4f585a7` on 2026-10-22. It ran `gpt-5.6-sol`
with high reasoning, and its context tier was not recorded. The task then
specified a Claude Opus 4.8, high-reasoning, long-context inspector, so whether
this inspection satisfied the fresh design inspection requirement needed a
campaign disposition. The campaign later declined to credit it, as recorded
below. The inspection judged that incident I161-1 does not block its design
verdict, and it states that the incident remains unresolved, with isolation
acceptance, causality, and closure withheld.

The Local Daemon design inspection (steward session `af271672`, artifact SHA-256
`537cdf0b379c11b4c023544ee50f81f0d6b7e59d7f22a3441601c32f9fba8752`) required
changes at `7505568c` on 2026-10-22. It found four reachable design-boundary
defects by static analysis; none was reproduced at runtime. The steward and a
separate fresh inspector agent (`a618a9e4`) each ran `gpt-6-astra` with high
reasoning and long context.

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
through 13 commits (immediate parent `576ecebb`) that change eight paths
(+562/-120) and are intended to address D161-1, D161-2a, D161-2b, and D161-3.
The retained gpt-6-astra delta review (artifact `REVIEW-PR161-F02F0EE-DELTA.md`,
6,897 bytes, SHA-256
`dead034889886b40746bed0aa8322bab7fdee7b0d9c03f8bd94f3bba5475316c`; reviewer
agents `20687f1b` and `fe04d9d4`, each with high reasoning and long context)
approved the full range from `7505568c` with no remaining findings. It reports
that drain now authenticates the locked root's selected predecessor, conformance
setup drops only its own run's schema, fixture teardown no longer kills by
numeric PID and keeps roots when process exit is uncertain, and non-Linux Unix
bootstrap refuses before connecting or spawning. Its validation was compile-only
because local runtime is frozen under I161-1. This is a source approval only; it
does not resolve the design findings, which need fresh design inspections.
Required CI run 39755917809 started at `f02f0ee9`, including the required Live
Postgres parity (job 119747762473) and Windows `InstalledCurrent` Postgres
process proof (job 119747762498) jobs, and Local's source review is assigned;
neither result is recorded here. All `7505568c` CI, consumer, and design
evidence is historical and gives the successor no credit. The successor has no
gate credit yet: before merge it needs Local source approval, passing required
CI, both consumer attestations, both design inspections, a disposition of
I161-1, and campaign authorization.

Local's source review approved `f02f0ee9` against baseline `7505568c` with no
findings (report SHA-256
`03b1f374e36776153db614fccca007c327d7ee37f58a64fe051cf17df6feecf9`). It accepted
the source corrections for all four design findings. For D161-1, upgrade and
rollback now drain only after resolving the target root's selected predecessor
under the exclusive admission they already hold, and a versioned CLI launch uses
a selection token. For D161-2a, the cross-run PostgreSQL sweep is removed. For
D161-2b, fixture teardown stops the daemon gracefully through its own
executable, checks the exact process generation, and keeps roots when exit is
uncertain; the review does not cover explicit crash-induction helpers that still
signal by PID. For D161-3, unsupported non-Linux Unix bootstrap and the
defensive peer check fail closed, while legacy connections without an expected
file identity are unchanged. The earlier L161-1 through L161-5 repairs, the SID
authority, PostgreSQL transaction-scoped migration locking, and the public API
are conserved. Local's review was read-only: it ran no compilation, tests, or
runtime proof, and it did not independently audit the writer's compile-only
validation. This is a source approval only. It gives no runtime, consumer,
design, merge, or closure credit, and it does not accept the design findings as
resolved. Required CI run 39755917809 is still in progress; its result is not
recorded here. Incident I161-1 and the withholding of E161-1 are unchanged.

Required CI run 39755917809 at `f02f0ee9` completed with failure at
2026-10-22T09:48:54Z. Two of its 16 jobs failed. In the required Live Postgres
parity job (119747762473), the PostgreSQL leg of the Application Client
conformance battery panicked during the strict-recovery case: while restarting
the daemon, the daemon for `appconf-postgres` did not stop within the deadline.
The SQLite leg of the same battery passed. The non-required macOS Copilot
fallback E2E job (119747762373) failed to compile a test under `-D warnings`
because the method `set_daemon_executable` is never used. The other 14 jobs
succeeded, including the required Windows InstalledCurrent Postgres process
proofs (119747762498). The failure log is retained by the Application Client
session (18,759 bytes, 103 lines, SHA-256
`40b64b0ce92ed450979cc6ed142b294bf0cd1318955bd035c0992c78e2cd8ade`). Because a
required job failed, `f02f0ee9` receives no CI credit, and the passing jobs give
no readiness credit. The Local and retained delta-review approvals remain source
approvals only, and Local's acceptance of the D161 source corrections is
unchanged; neither is design acceptance. A further successor head is assigned to
address these failures. Any head movement invalidates all `f02f0ee9` evidence.
Incident I161-1 and the withholding of E161-1 are unchanged.

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
client. The retained delta review approved the change
(`REVIEW-PR161-689883E-DELTA.md`, 4,380 bytes, SHA-256
`4f61a7ce5b80c5fd9a28204788264cf5c37ec72989dc170b52a13b33098864e7`). Its
reviewers, agents `99ae208c` and `2488698f`, are recorded as gpt-6-astra with
high reasoning and long context. Its validation was compile-only under the
I161-1 freeze: formatting, all-features no-run compilation, and targeted Clippy
on Rust 1.99 and 1.98. The review identifies the timeout cause from source only
and states that CI must confirm it. Required CI run 39762898070 started at the
exact head, and Local's source review is assigned; neither has a result. The
head has no gate credit yet. Evidence from `f02f0ee9`, including its source
approvals and failed CI, is historical and does not carry forward. Incident
I161-1 and the withholding of E161-1 are unchanged.

Local's source review of `689883e2` against baseline `f02f0ee9` required changes
(report SHA-256
`9cfd8d18b95b7d7e120daecb5f344da36ddf8d03a7b37e00cf67193e565d9232`). Finding
L161-6 (P2) concerns the new auxiliary PostgreSQL repair task. The task waits
for the release-client lock before its five-second timeout starts, and it waits
for the publication locks after that timeout ends. Lease release holds the same
release-client lock while its query runs. A PostgreSQL row-lock wait or stalled
I/O can therefore keep the repair task, its shared state, and its single-flight
flag alive indefinitely, which prevents any later repair from being scheduled.
Healthy foreground calls still do not wait for the repair. Local established
this wait path from source and did not reproduce it. The requested change is one
absolute deadline over the whole repair attempt, or nonblocking inspection and
publication that safely defers when busy. The single-flight flag must be
released on completion, timeout, and cancellation, and any unpublished
replacement connection must be cleaned up. Foreground calls must stay
nonblocking, and drain must stay free of credential discovery. Local found no
other source issue: it accepted the macOS cfg change in source and raised no
further finding on the dedicated release client, credential handling, driver
cancellation, or migration locking. Local reports the earlier D161 and L161-1
through L161-5 source repairs conserved and the public client source unchanged.
It also states that no runtime evidence shows that client-lock contention caused
the earlier Live Postgres failure or that this head fixes it. The retained delta
review's approval is historical and does not override Local's finding. Local's
review was read-only and did not run the writer's compile-only validation.
Required CI run 39762898070 at `689883e2` later completed with failure at
2026-10-22T10:35:37Z. Two of its 16 jobs failed: the required Live Postgres
parity job (119771171825) and the Windows build-and-test job (119771171806). The
run is superseded by head movement and is diagnostic only. `689883e2` received
no source, CI, readiness, consumer, design, merge, or closure credit, and the
move to `0490f5e5` invalidates all `689883e2` current-head evidence.

Successor head `0490f5e5936484868d03d19f70daf7b9e2ef4df3` (tree
`61cdd28b44d71df1d02333f5f6eefbc09ffe207c`) is one commit directly on `689883e2`
and became the PR #161 head. The product merge base remained `e4f585a7`. It
changes only `src/backend/postgres.rs`, +97/-15, and was intended to address
L161-6. One five-second budget covers the whole auxiliary repair attempt, a drop
guard was intended to release the single-flight flag on every exit, and an
unpublished replacement is dropped on timeout. Foreground calls stay
nonblocking, and drain stays free of credential discovery. The head added
deterministic unit tests, compiled but not run under the I161-1 freeze. The
retained delta review approved it (`REVIEW-PR161-0490F5E-DELTA.md`, 3,893 bytes,
SHA-256 `d9b1a1c3eddebb9989690e3c55bd20ddeb562eafe9e1d02531f8148303e50ad4`). Its
reviewers, agents `2887b2b4` and `0d21e13e`, are recorded as gpt-6-astra with
high reasoning and long context, and its validation was compile-only. Local's
source review then required changes again (report SHA-256
`1687ded371daf22ad744716264458c238983b02f26beeab76712054af76eea26`). It accepted
the whole-attempt timeout, cleanup once the repair task has started, and the new
tests for those paths. It found the foreground, credential, fencing, and
migration code otherwise unchanged. A narrow part of L161-6 remained: the
single-flight flag was set when the repair was scheduled, but the guard that
clears it was created only when the spawned task first ran. If the queued task
was dropped before then, for example during runtime shutdown, the flag stayed
set and no later repair could be scheduled. Local required the guard to be
created synchronously when the flag is acquired and moved into the task, plus a
test that drops the unpolled task and confirms that the flag clears and can be
acquired again. This finding is static and was not reproduced. Local reported
D161, L161-1 through L161-5, the SID authority, the public API, and migration
locking conserved. The retained delta review's approval does not override
Local's finding. Local's review was read-only and did not run the writer's
validation. Required CI run 39763861998 at `0490f5e5` was superseded by head
movement while still in progress, after its required Live Postgres parity job
had already failed. It is diagnostic only, and no terminal result is recorded
here. `0490f5e5` received no source, CI, readiness, consumer, design, merge, or
closure credit, and the move to `745eaf28` invalidates all `0490f5e5`
current-head evidence.

Successor head `745eaf28bb9d0abe70a79f8023dc7a98fa66d5da` (tree
`6e61c22b1116b68965554da1bdafe5c36372b016`) is one commit directly on `0490f5e5`
and is now the PR #161 head. The product merge base remains `e4f585a7`. It
changes only `src/backend/postgres.rs`, +42/-13, and is intended to close the
remaining part of L161-6. The repair guard is now created synchronously as soon
as the single-flight flag is acquired and is moved into the scheduled task, so
dropping the task before it first runs clears the flag. A new unit test drops
the unpolled repair future and checks that the flag clears and can be acquired
again. The retained delta review approved it (`REVIEW-PR161-745EAF2-DELTA.md`,
3,118 bytes, SHA-256
`cb21d30ac179c8fa6783bb94e7f577e89b2690444cdd1e6f35f1dcf3cc27f5e7`). Its
reviewers, agents `9e3f3795` and `1cea26a6`, are recorded as gpt-6-astra with
high reasoning and long context, and its validation was compile-only. Local's
source review approved `745eaf28` against baseline `0490f5e5` with no findings
(report SHA-256
`fe9b60d4e03aac6b51967b86a68d329a75e1a5269f6508afbdded6b8ff8f1d8c`). Local
accepted L161-6 as resolved in source, covering both the whole-attempt deadline
and guard ownership before the first run. The repair body, connection setup,
foreground path, reconnect, and backend implementation are otherwise unchanged.
Local reports D161, L161-1 through L161-5, the SID authority, the public API,
and transaction-scoped migration locking conserved. Local inspected the new test
but did not run it; the test covers the repair future directly, and Local
verified the outer task from source. Local's review was read-only and did not
run the writer's validation. Local states that the approval does not show that
the Live Postgres timeout is fixed or that CI will pass. This is a source
approval only. It gives no CI, runtime, consumer, design, merge, or closure
credit. Required CI run 39764752556 then ran at the exact head; its result
follows. Evidence from `0490f5e5` does not carry forward. Incident I161-1 and
the withholding of E161-1 are unchanged.

Required CI run 39764752556 at `745eaf28` completed with failure at
2026-10-22T10:57:59Z. Two of its 16 jobs failed, and the other 14 succeeded. In
the required Live Postgres parity job (119777319781), the PostgreSQL leg of the
Application Client conformance battery failed: the daemon for `appconf-postgres`
did not stop within the test deadline while the strict-recovery check restarted
it, the same symptom seen at `f02f0ee9`. In the required Windows
InstalledCurrent Postgres process-proof job (119777319882), the concurrent
first-use test failed because a concurrent connect returned Unavailable after a
backend operation failed (diagnostic `442fe890f96b80f8`). The retained failure
log is `pr161-ci39764752556-745-failed.log` (51,728 bytes, 236 lines, SHA-256
`2714cb2a2617f309ad37714cb72bd577ccc8cdf3e4c8fb21f768228148702d3b`). The log
shows these symptoms but not their cause. Because required jobs failed,
`745eaf28` receives no CI or readiness credit, and the Live Postgres timeout is
not shown to be fixed. The Local and retained delta-review approvals of
`745eaf28` remain source-only. A further successor was assigned; its head needs
its own source review and passing required CI. Incident I161-1 and the
withholding of E161-1 are unchanged.

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
proactively. The retained delta review approved it
(`REVIEW-PR161-0486BEE-DELTA.md`, 4,643 bytes, SHA-256
`c6f27ac396c54d312ca9ad4fe566b4973c08e3d098e68a7a2c714ed99ca54db3`). Its
reviewers, agents `fd257b51` and `fb340fb6`, are recorded as gpt-6-astra with
high reasoning and long context. Its validation was formatting, compilation, and
lint only under the I161-1 freeze, with no runtime or database test. The review
links the two failures to these causes from source and the CI log, and it states
that CI must confirm both repairs. Local's source review then approved
`0486beeb` against baseline `745eaf28` with no findings (report SHA-256
`fc373fdc7b99e45d638590d9734217b46985c3d47ce9d49603dece1b1ebbda12`). It accepted
the source as follows. Only the daemon's profile backends open the release
connection eagerly; public construction and ordinary activity use one
connection, although an explicit lease release can still open the release
connection lazily. The connection driver is abort-owned through setup and
publication. Each lease-release call has one absolute five-second deadline, with
fencing, invalidation, and repair preserved; the deadline applies per call, not
across a whole multi-member drain. Each scenario stops its daemon before its
exact PostgreSQL schema is discarded, and the test profile is published once
under a lock. Local recorded three qualifications. Dropping a connection is
local cleanup and does not guarantee synchronous remote cancellation. The
absence of a daemon capability file, which the teardown uses to skip the stop,
does not prove that no daemon endpoint exists. Local did not verify the review's
claim that these changes caused or fix the CI failures. Local reports L161-6
still resolved in source and D161 and earlier source authority conserved.
Local's review was read-only and did not run the writer's validation. This is a
source approval only. It gives no CI, runtime, consumer, design, merge, or
closure credit. Required CI run 39770808804 then ran at the exact head; its
result follows. The move from `745eaf28` invalidates all `745eaf28` current-head
evidence, and earlier approvals and CI results are historical. Incident I161-1
and the withholding of E161-1 are unchanged.

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
verdicts. Watcher APPROVED (session e00d296e-ffca-4477-b1c8-c4d340bd30be;
gpt-5.6-sol, high reasoning, context tier unavailable; artifact
`watcher-pr161-0486bee-premerge-consumability-attestation.md`, 15,537 bytes,
SHA-256 `07880162da63e9c4cd3814c2ce141028d861eee3c4fb024c6d4dd38c807e6e22`). It
found that Watcher can consume the client through supported public surfaces
only, and that the D161 and L161 repairs and the PostgreSQL lifecycle changes
add no Watcher-private seam and no new compatibility or design dependency.
Operator Station returned CHANGES REQUIRED (session
002c5122-fc0d-403c-8f87-625d87385d44; gpt-5.6-sol, high reasoning, context tier
unavailable; artifact
`operator-station-pr161-premerge-consumability-attestation-0486beeb.json`,
13,035 bytes, SHA-256
`c96ed005240d26972c99255bbeaf88835a9605c6932c1d43ddd377565a83559e`). Station
found the public API and its fixture behavior compatible and found no private
seam or new design dependency. Its blocker, OS-PR161-0486BEE-1, is a
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
gpt-6-astra exact-delta review (artifact `REVIEW-PR161-4CD3493-DELTA.md`, 1,739
bytes, SHA-256
`0a06e1e3e47b747f87b7be2a98df014cb997791f89d91417774437b6ac3757d3`; high
reasoning, long-context tier) approved it with no findings, using static
inspection only. Local's exact-head source review (report
`pr161-local-delta-4cd3493-20261022.json` in session
aeae4fa9-c5fc-4da9-b60e-bcf5b1a98f57, 5,078 bytes, SHA-256
`b870fe063084a5224df82979954393d16fe0ec543d293afb1efa89cd46e0ef59`) also
approved it with no findings and accepted OS-PR161-0486BEE-1 as resolved in
source. Local qualified that approval: the later destructor stop remains best
effort and is not a numeric-PID kill or root cleanup; the regression check is
textual and protects only the current straight-line function, not arbitrary
future control flow; Local inspected the check but did not run it; and source
review does not establish runtime daemon termination. Local found all other
tracked files identical to `0486beeb`, so the earlier D161, L161, SID-authority,
public-API, and PostgreSQL lifecycle source authority is conserved. Both
approvals are source-only. Required CI run 39775306925 then ran at the exact
head; its result follows. The head movement invalidated all `0486beeb`
current-head evidence, including its CI credit and Watcher's approval, so
`4cd34938` started with no CI, consumer, design, merge, or closure credit.
I161-1 remains unresolved, E161-1 remains withheld, and the formal post-merge
consumer gate remains planned.

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
(session e00d296e-ffca-4477-b1c8-c4d340bd30be; gpt-5.6-sol, high reasoning,
context tier unavailable; artifact
`watcher-pr161-4cd3493-premerge-consumability-attestation.md`, 13,555 bytes,
SHA-256 `db84520d1af042d21967f70d5c8ea0b9a6527e749874e8e89c2259fa77e6b6e5`)
found that the cleanup-order delta establishes verified daemon termination
before the run-owned schema is dropped, without changing the public Application
Client, the public Watcher fixture, the daemon, or the PostgreSQL backend, and
that no private fallback or unaccepted Watcher compatibility or design
dependency is required. Operator Station (session
002c5122-fc0d-403c-8f87-625d87385d44; gpt-5.6-sol, high reasoning, context tier
unavailable; artifact
`operator-station-pr161-premerge-consumability-attestation-4cd34938-runtime.json`,
12,995 bytes, SHA-256
`75cda963a6ad78b8984e9617c312cd984c7f0bc91efb3663433ae634555d52de`) found that
it can consume this exact revision through supported public surfaces only, with
no private fallback, and closed OS-PR161-0486BEE-1 in source and runtime.
Station's evidence: the fixture stops the exact daemon generation before the
final `DROP SCHEMA`; both the Linux Live Postgres and Windows InstalledCurrent
jobs passed the fixture and its new stop-before-drop test; and the PostgreSQL
service log has no `leases` activity and no database error from fixture
completion through shutdown. Station attributes the 30 earlier `isolated wait
recovery fault` entries to deliberate fault-injection tests that ran before the
fixture. Station also found the earlier D161, L161, and PostgreSQL lifecycle
source unchanged by blob. `4cd34938` therefore has bounded pre-merge consumer
credit from both product authorities for this exact head only; any head movement
invalidates it. This is not the formal post-merge consumer-integration gate,
which remains planned and must attest the exact merged revision. App reported
that the Local Daemon design inspection at `4cd34938` had started; its result
follows. The Application Client design inspector-profile disposition was also
pending. I161-1 remains unresolved, and E161-1 remains withheld.

The Local Daemon design inspection of `4cd34938` returned APPROVED on 2026-10-22
(correlation `local-daemon-pr161-design-4cd3493-20261022`; artifact
`pr161-local-design-inspection-4cd3493-20261022.md` in session
af271672-5d33-418c-bb9c-0f4d4570c7d7, 28,991 bytes, SHA-256
`3f71c77310f7cb64d5f19ed18c687fdc085c8342b0e570b6d1e7d07f44a47c39`). Its fresh
read-only inspector covered the whole nine-path range from `7505568c` to
`4cd34938`, not only the final fixture change, and the steward adjudicated the
result. The verdict resolves D161-1, D161-2a, D161-2b, D161-3, and
OS-PR161-0486BEE-1 in source, leaves no new material Local Daemon design
finding, and accepts the current proof-cleanup and isolation source design
independently of the historical incident. It preserves I161-1 as an external
operator-impact and restoration disposition and keeps E161-1 withheld; it grants
no overall isolation certification and no restoration, replay, cleanup, merge,
release, or post-merge gate authority. It also preserves the earlier causal
qualification: source approvals and passing CI do not establish the cause of the
earlier Windows heap corruption. Provenance: the steward (session
af271672-5d33-418c-bb9c-0f4d4570c7d7) and a separate fresh inspector (agent
1792262a-dca4-4b30-8b11-7496e7e06744) each ran as gpt-6-astra with high
reasoning and the long-context tier; these are distinct records. The verdict
does not certify the separately required Application Client inspector profile.
`4cd34938` therefore has Local Daemon design credit for this exact head only;
any head movement invalidates it. The steward's last provider read after writing
the verdict returned mergeability unknown, and the verdict carries no
clean-merge claim forward; a later read for this record showed the PR mergeable
and clean at the same head, but merge readiness still needs its own fresh
guards. At that point the fresh Application Client design inspection and the
campaign's inspector-profile disposition were pending, so `4cd34938` had no
Application Client design credit and no merge or closure credit. I161-1 remains
unresolved, E161-1 remains withheld, and the formal post-merge consumer gate
remains planned.

On 2026-10-22 the campaign resolved the Application Client inspector-profile
disposition by enforcing an existing operator instruction, not by granting a
waiver. On 2026-09-24 the operator directed that all new sessions use GPT-6
Astra with high reasoning and 1.2M context (campaign session
e06813c4-1db2-4d77-b8d3-3addc6a1824c, turn 692, 2026-09-24T21:10:20.739Z), and a
2026-10-08 campaign directive
(`campaign-postrelease-execution-directive-20261008.json`, SHA-256
`a4d98af7d891efe7164839c79b4431d07acfc1ceef606c54e46b82ab7eb5613d`) applies that
instruction to all new sessions and delegates as gpt-6-astra with high reasoning
and the long-context tier, with no silent downgrade. That standing instruction
governs new inspectors over the Claude Opus 4.8 profile this task previously
named, and this record reconciles the task text; the substantive requirement for
a fresh, independent, read-only design inspection is unchanged. The campaign's
directive (`pr161-app-design-profile-directive-4cd34938-20261022.json` in
campaign session e06813c4-1db2-4d77-b8d3-3addc6a1824c, 4,695 bytes, SHA-256
`ca7557e0b6d5cc78d1a2b818085c3eb57ad6a002c1cf9561d81c1f8a983a5fa0`) declines to
credit the `7505568c` inspection, which ran gpt-5.6-sol without a recorded
context tier, and assumes no equivalence between models. It assigns App's
existing Application Client design steward to run one fresh, independent
gpt-6-astra, high-reasoning, long-context inspection of the complete exact-head
delta at `4cd34938`, recording the actual profile, head, tree, comparison base,
and inspector identity. The operator asked for 1.2M context, but the last
verified catalog limit was 1,050,000, which does not prove an actual execution
budget; the inspection must record supported and observed context separately and
report a provisioning blocker rather than substitute a profile. The inspection
may proceed without waiting for this record. `4cd34938` has no Application
Client design credit until that verdict returns. The directive grants no merge,
closure, release, or consumer-launch authority, and the fresh design verdicts
and a freshly guarded campaign merge floor remain required.

The released-v0.2.1 upgrade proof (E161-1) is not accepted. Official v0.2.1
staging of the `d08e764c` candidate and public attachment succeeded, but the
proof script rewrote the access control list of the operator's real
`AppData\Local` directory in several runs, with no prior descriptor or backup
(incident I161-1, evidence SHA-256
`0d50f4885a75f369ef218d18e907e0a01baeeb84171069af91ee34f1cec26e24`). Isolation
certification is withheld. On 2026-10-22 the operator chose to preserve the
current host access control lists and evidence and to have the existing owners
continue a bounded read-only impact review, as relayed by the campaign session
(e06813c4-1db2-4d77-b8d3-3addc6a1824c) and App
(eaa99eef-33a6-40ff-8b8d-3168e6f2a805), and recorded in the campaign directive
`i161-1-operator-preservation-directive-20261022.json` (2,561 bytes, SHA-256
`e3f051483ab895f1804cfa6572a3045d679aecf9a200e857eb3ebcf8afdc30e6`). That choice
resolves only the initial operator response. It authorizes no restoration,
inferred default, inheritance change, recursive reset, other access-control
change, proof rerun, or cleanup. Impact assessment and isolation acceptance
remain open and E161-1 remains withheld; the choice does not establish an
absence of impact, close the incident, or grant merge credit. Local runtime and
proof replay remain frozen, so runtime validation relies on CI; ordinary source
and CI work may continue within that freeze. It does not change the gate
evidence for PR head `4cd34938`, which remains scoped to that exact head. A
fresh gpt-6-astra Application Client design inspection, merge authorization, and
merge remain outstanding. Scope, invalidation conditions, and the accepted
trusted `InstalledCurrent` policy are unchanged.

## Invalidation conditions

Stop and report `decision-needed` if conformance requires a new public semantic,
weakened accepted distinction, new language/ABI/process boundary,
consumer-specific shared policy, private fallback, hidden runtime or sidecar,
unmodeled dependency, a platform without provable equivalent selector
admission/trust semantics, manifest/build validation that cannot fail closed,
material validation gap, or split without a real independent boundary.
