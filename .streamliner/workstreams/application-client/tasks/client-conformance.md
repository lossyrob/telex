# Application Client conformance and consumer consumability

- **Workstream:** `application-client`
- **Node:** `client-conformance`
- **Type:** implementation
- **Status:** ready; implementation worker frozen before bootstrap-dependent mutation
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
- After implementation review and required CI pass on one exact head, obtain the
  two consumer attestations and a fresh read-only design inspection. Use a new
  Claude Opus 4.8, high-reasoning, long-context inspector child. Any head movement
  invalidates same-head review, CI, attestation, and inspection evidence as
  applicable.
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

## Dependency and promotion

`client-core` and `first-binding` are complete. Issue #152 alone does not make
this node ready: this task, the discovery ledger, and cross-workstream
dependency reconciliation must land on `main` first.

After that authority lands, the Application Client orchestrator resumes the
frozen issue #152 implementation worker on its existing branch and PR. Do not
launch a replacement worker or split the node. Consumer work remains blocked
until conformance merges, both exact-head attestations are accepted, and
`consumer-integration-gate` is separately passed and reconciled.

## Invalidation conditions

Stop and report `decision-needed` if conformance requires a new public semantic,
weakened accepted distinction, new language/ABI/process boundary,
consumer-specific shared policy, private fallback, hidden runtime or sidecar,
unmodeled dependency, a platform without provable equivalent selector
admission/trust semantics, manifest/build validation that cannot fail closed,
material validation gap, or split without a real independent boundary.
