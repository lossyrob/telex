# Application Client Current Design

## Status and authority

This file is the canonical integrated design for the `application-client`
workstream. It summarizes accepted repository authority; it does not replace the
normative documents.

Authority is ordered as follows:

1. [`docs/design/application-client.md`](../../../../docs/design/application-client.md)
   is the sole normative, API-neutral semantic contract. Issue #12 remains its
   semantic owner.
2. [ADR 0049](../../../../docs/design/DECISIONS.md#0049--one-api-neutral-application-client-contract-governs-explicit-station-capabilities-and-forbids-private-fallbacks)
   defines the shared-client boundary, explicit capabilities, and prohibition on
   product-private fallbacks.
3. [`docs/design/daemon.md`](../../../../docs/design/daemon.md) governs daemon,
   membership, delivery, acknowledgement, and backend behavior beneath the
   client.
4. [`docs/application-client-core.md`](../../../../docs/application-client-core.md)
   describes the supported Rust core and the later conformance obligations.
5. The
   [requirements crosswalk](../../../../docs/notes/application-client/requirements-crosswalk.md)
   records non-normative traceability for Watcher W-01 through W-15 and Operator
   AC-01 through AC-15.
6. The [workstream brief](../brief.md) and
   [campaign roadmap](../../../shaping/roadmap.md) define workstream scope and
   campaign dependencies without overriding product design.

If this summary conflicts with a normative project design document, the project
design document wins and this file must be reconciled.

## Intended outcome

Long-lived non-agent applications use one supported Telex client for lifecycle,
capability, messaging, delivery, acknowledgement, receipt, history, recovery,
health, and backend semantics. Watcher and Operator Station consume the same
semantic boundary without parsing CLI output, depending on private daemon IPC,
or creating product-specific client forks.

The merged Rust core is `telex::application_client`. The first supported binding
is the Rust library surface at that path in the root `telex` crate. This decision
promotes the existing semantic boundary for Rust consumers; it does not create
an external language, ABI, or process boundary.

## Responsibility boundary

The Application Client owns generic application integration semantics:

- stable responsibility and fresh runtime identity;
- attach, reconcile, bounded recovery, deliberate detach, and compensation;
- send-only and bidirectional capability;
- explicit sender selection;
- exact recipient delivery and acknowledgement;
- separate receipt and workflow evidence axes;
- retry-stable operations, history, source resolution, health, and ordered
  resynchronization; and
- backend-neutral behavior over supported SQLite and Postgres stores.

Watcher owns detector execution, scheduling, provider semantics, event-state
transactions, and runtime policy. Operator Station owns presentation, human
interaction, and local workflow policy. Telex core owns durable transport,
membership, delivery rows, leases, backend authority, installed-version
selection, daemon process admission, and upgrade/rollback coordination.

The client does not interpret product metadata, choose human-routing policy,
become a workflow engine, expose database or daemon-wire details as its public
contract, or provide a private fallback when a shared semantic is missing.

## Identity, lifecycle, and capability

`ApplicationResponsibility` is stable configuration. Each client process uses a
fresh, never-reused `RuntimeId`. Schema v3 persists an opaque `LogicalStoreId`
that survives restart and presentation changes without exposing a path,
connection string, credential, or store key.

Membership explicitly declares `send-only` or `bidirectional` capability.
Send-only membership can author messages but cannot create inbound attendance
or expose receive, acknowledgement, inbound-history, or backlog semantics.
Bidirectional membership receives only through exact delivery identity.

Multi-address attach, reconcile, and detach either succeed for the requested set
or return typed per-address completion, failure, collision, and compensation.
Compensation reverses only work performed by that call. Durable deliberate-detach
intent is scoped to responsibility and address; bounded repair cannot resurrect
it. A later explicit attach clears the intent only after membership commits.

Membership loss preserves known reasons as distinct typed evidence and retains
the raw token for unknown future reasons. Strict recovery returns typed loss.
Bounded recovery stays within the caller's budget and preserves capability,
sender, liveness, ownership, and deliberate-detach constraints.

## Delivery, acknowledgement, and receipt evidence

A bidirectional receive result binds store, message, recipient, delivery row,
role, and acknowledgement capability to one exact delivery. The application
acknowledges only after restart-safe ingest. Acknowledging one recipient never
consumes another recipient's delivery.

Durable acceptance, occupancy at acceptance, push attempt, exact-recipient
consumption, quarantine, and workflow disposition remain separate evidence.
No axis implies another. Send and reply return an acceptance-time snapshot;
receipt refresh obtains later consumption and disposition evidence without
inventing push evidence.

An oversized historical primary delivery that cannot fit the supported receive
frame becomes a typed, durable quarantine outcome. Structural daemon provenance
makes quarantine sticky and auditable, prevents supported application APIs from
minting it, and allows later deliveries to make progress. Quarantine does not
fabricate application consumption or replace the latest ordinary workflow
disposition.

## Retry-safe operations and recovery

Applications prepare retryable send or reply operations before the first
attempt and persist the complete `RecoveryHandle`. The handle binds operation,
responsibility, opaque store, canonical payload, and the operation-evidence
retention generation.

Prior completion may be adopted only when operation, responsibility, store, and
comparable canonical payload evidence match. Pre-acceptance rejection carries
typed transient or permanent retryability. An unproved acceptance boundary
remains indeterminate.

`NotRecorded` is authoritative only when one consistent backend snapshot proves
that the exact operation, result, message mapping, and receipt evidence are
absent and that cleanup has not crossed the handle's retention generation. A
crossed or missing generation returns `RetentionBoundaryCrossed` and never
authorizes retry. Store, payload, or responsibility mismatch fails closed.

When durable message acceptance succeeds but result persistence fails, send and
reply return typed indeterminate evidence with the staged handle. Reconciliation
projects recorded state into typed accepted, rejected, partial, indeterminate,
duplicate, or pending outcomes rather than requiring consumers to parse backend
state strings or opaque result prose.

## Backend, health, and ordered state

SQLite and Postgres implement the same backend-neutral semantic model. Schema v3
stores operation records, deliberate-detach intent, compound steps, state
versions, and ordered deltas through additive, idempotent migration. A client
refuses a newer unsupported schema.

Operation reconciliation reads its evidence from one SQLite transaction or
Postgres repeatable-read snapshot. Concurrent Postgres first attempts use
conflict-safe replay behavior instead of leaking uniqueness failures.

Backend access does not prove principal identity. SQLite operating-system
identity and the configured Postgres user are `unverified` evidence unless a
future backend supplies stronger authenticated provenance.

Health keeps sender readiness, receive readiness, membership loss, collision,
reconciliation, compensation, deliberate detach, backlog, acknowledgement
pending, degraded, and unattended states separate. Ordered deltas use persisted
monotonic versions; gaps require typed resynchronization, and resync cannot
regress state. Compound steps preserve caller-declared prerequisites and prevent
a dependent terminal disposition from completing before its prerequisite is
durable.

## First-binding promotion boundary

The first supported binding is Rust-first in the root `telex` crate and preserves
the import path `telex::application_client`. It reuses the merged core rather
than adding a translation layer or second package boundary.

Supported application consumers disable package defaults and select one of these
profiles:

- SQLite: `default-features = false`, feature `sqlite`;
- Postgres: `default-features = false`, feature `postgres`;
- Postgres with Entra: `default-features = false`, feature `entra`, which
  includes `postgres`; or
- dual backend: `default-features = false`, features `sqlite` and `postgres`,
  optionally adding `entra`.

The `self-update` feature is not part of any application-consumer profile.
Consumers that choose a single backend do not inherit the other backend or CLI
release behavior.

The root crate version governs Rust source compatibility. Public types and
behavior that carry the accepted Application Client semantics are the supported
surface. These include stable responsibility and runtime/store identities,
capability, exact-delivery acknowledgement, recovery handles, typed errors, and
the message, delivery, receipt, disposition, operation, compound-step, health,
history, and delta records required by the normative contract. Backend records,
daemon frames, CLI types, private helpers, and consumer DTOs are not promoted.
Breaking changes to the supported surface require an explicit compatible-version
transition and migration guidance; exact versioning and deprecation mechanics
remain implementation choices. The Rust surface does not promise a stable C ABI,
JSON wire protocol, or cross-language serialization contract.

## Installed-current daemon bootstrap

Production consumers select the shared Telex daemon through an explicitly
trusted absolute install root:

```rust
#[non_exhaustive]
pub enum ApplicationDaemonBootstrap {
    InstalledCurrent { trusted_root: PathBuf },
    ExactExecutable { executable: PathBuf },
}

#[non_exhaustive]
pub enum DaemonBootstrapFailure {
    InvalidTrustedRoot,
    UnsafeInstallAuthority,
    MissingCurrent,
    InvalidManifest,
    IncompatibleManifest,
    SelectionUnstable,
    MissingExecutable,
    ExecutableIdentityMismatch,
    ForeignDaemon,
}

impl ApplicationClient {
    pub async fn connect_with_daemon(
        config: ApplicationClientConfig,
        daemon: ApplicationDaemonBootstrap,
    ) -> Result<Self, ApplicationClientError>;
}
```

`InstalledCurrent` is the supported production policy for Watcher and Operator
Station. The additive constructor preserves existing `ApplicationClientConfig`
struct literals. Existing `ApplicationClient::connect(config)` retains its
current-executable behavior for source compatibility, but that path and
`ExactExecutable` are subordinate development and test support rather than an
automatic production fallback. Configuration rejects a relative trusted root
and captures one immutable canonical absolute root before any connection or
reconnect, so a working-directory change cannot reinterpret it.

For each installed-current connect-or-spawn cycle, the client obtains a shared
OS-backed selector-admission lease on a persistent lock file under the trusted
root before reading `current`. Unix uses a local-filesystem advisory lock with
process-crash release. Windows uses an owner-restricted shared/exclusive
`LockFileEx` range lock. The lock file is never replaced or deleted as part of
selection. A filesystem that cannot prove equivalent ownership and lock
semantics is unsupported.

While holding shared admission, the client validates the root, selector,
selected manifest, version directory, and executable; resolves one canonical
target containing the selected tag, manifest identity and load-bearing fields,
canonical executable path, and platform file identity; and uses that immutable
target for both spawn and pre-`Hello` peer authentication. It re-reads and
compares the complete selection immediately before spawn. The parent lease
remains held through a successful `HelloAck`. Upgrade and rollback hold the same
lock exclusively across daemon drain, predecessor exit, validated selector
switch, and selector publication. Lock order is selector admission before daemon
singleton/spawn admission, and a drain served inside exclusive admission does
not reacquire the selector lock.

Before binding its serving endpoint or publishing capability or readiness, a
spawned daemon independently acquires a shared selector-admission lease,
validates the captured selection token, a fresh installed-current resolution,
the selected manifest/build metadata, and its own process image, and holds its
lease through publication. The parent retains its separate lease through
authenticated `HelloAck`. If either process dies, the remaining or next
admission still prevents stale-child publication. A token, image, or lock
mismatch exits without serving. This prevents an old selection from spawning
after a switch; neither `current` plus `previous` acceptance nor post-spawn
stale-daemon cleanup is allowed.

Installed-current resolution derives
`<trusted-root>/versions/<current-tag>/telex[.exe]`; it does not trust an
arbitrary manifest path or search `PATH`. The canonical version directory and
executable must remain beneath the canonical trusted root. Selector, manifest,
version-directory, and executable aliases that can redirect through symlinks or
reparse points fail closed. The root and authority chain must be owned by the
current OS user and must not grant write, delete, or ownership control to
another principal. Owner writability remains necessary for same-user upgrade
administration.

The selected manifest must bind its tag and executable and supply validated
build identity, package version, supported schema range, protocol version, and
required capabilities. These fields are compatibility and selection metadata.
They do not provide executable-content integrity, signature, publisher or
package provenance, protection from malicious same-user administration, or
intra-user isolation.

A prestarted daemon is reusable only when OS peer checks prove the same user,
reuse-safe PID/start time, UID or SID, canonical process-image path, and
platform file identity before metadata leaves the client.
`HelloAck` then proves the selected build identity, auth policy, protocol, and
capabilities. Linux uses an open process-image descriptor plus device/inode
identity. Windows captures canonical final path and volume/file identity from
handles and holds the executable handle with compatible sharing through process
creation. A different image is a foreign daemon; the client does not trust,
drain, kill, or start beside it.

Selector admission and authority publication belong to Local Daemon. The
Application Client owns the public bootstrap policy, additive constructor,
typed failures, and use of the supported admission flow. Bootstrap failures are
reported as `ApplicationClientError::DaemonBootstrap(DaemonBootstrapFailure)`.
The typed reasons are `InvalidTrustedRoot`, `UnsafeInstallAuthority`,
`MissingCurrent`, `InvalidManifest`, `IncompatibleManifest`,
`SelectionUnstable`, `MissingExecutable`, `ExecutableIdentityMismatch`, and
`ForeignDaemon`. They do not expose raw authority paths as durable application
evidence. Internal selector movement or admission contention retries are
bounded; exhaustion fails closed as `SelectionUnstable`.

`--skip-drain` remains a CLI break-glass action, not supported installed-current
evidence. A failed drain, switch, token validation, or readiness publication
preserves the old selection or leaves consumers visibly unavailable; it never
authorizes a stale or foreign daemon. SQLite uses release followed by
next-call spawn. Postgres preserves its accepted fencing and ordered-handoff
semantics. Both backends use the same installed-current trust boundary.
`ExactExecutable` applies the same canonical process-image, platform file
identity, and untrusted-writability checks, remains pinned, and does not follow
upgrade or rollback. It has no installed manifest authority and does not claim
one.

The binding executes in a caller-provided Tokio runtime. It does not create a
hidden runtime, application-specific daemon, or sidecar. Selecting and
auto-spawning the shared installed Telex daemon through this explicit policy
does not transfer daemon lifecycle ownership to the consumer. Cancellation is
not evidence that a durable operation failed or was absent: callers persist a
prepared `RecoveryHandle` before the first retryable attempt and reconcile an
uncertain result. Cancelling receive work does not acknowledge a delivery.
Lifecycle cancellation must retain typed partial and compensation evidence.

napi-rs/TypeScript, a separate client crate, C ABI, public socket or sidecar
protocol, and consumer-specific DTOs remain deferred. The historical TypeScript
`Station` sketch remains evidence, not authority. These deferrals do not permit
CLI parsing, raw private daemon IPC, or a product-private client fallback.

This promotion does not relax the semantic contract or complete conformance.
`client-conformance` must still exercise the same accepted cases against the
supported Rust surface and both backends before either consumer integration may
claim support.

## Conformance boundary

The merged core establishes an implementation, not completed conformance. Before
`client-conformance`, the workstream cannot claim:

- full SQLite/Postgres parity across the required semantic matrix;
- complete proof for restart and membership loss, deliberate detach,
  multi-address compensation, send-only false-attendance prevention, exact
  delivery acknowledgement, or acknowledgement-after-ingest recovery;
- complete proof for receipt separation, operation replay, payload or store
  mismatch, authoritative absence, retention-boundary handling, bounded history,
  source resolution, delta gaps, no-regression resync, compound recovery, schema
  migration, cleanup, or principal provenance;
- a supported external binding, consumer integration, or removal of every
  temporary consumer seam; or
- the `supported-client` checkpoint, production readiness, packaging and upgrade
  readiness, or operational hardening.

The conformance work must exercise the same semantic cases against SQLite and
credentialed Postgres. Consumer integration remains gated until both product
workstreams validate the supported client without a private fallback.

## Superseded assumptions

- A holder or resident waiter is not the general Application Client model.
- Client-owned heartbeat, generic auto-registration, and path-derived store
  identity do not define application lifecycle or authority.
- Occupancy, push acceptance, transport consumption, and workflow disposition
  are not interchangeable receipts.
- The historical TypeScript API sketch, CLI parsing, raw daemon IPC, subprocess
  couriers, spike helpers, and product-private clients are not supported public
  seams.
- Operator-specific mediation, route-back, notification, or mode vocabulary is
  not part of the shared client contract.

## Remaining questions and confidence

- **High confidence:** the shared semantic boundary, Rust core, identity model,
  explicit capability split, exact-delivery acknowledgement, typed operation
  recovery, quarantine evidence, backend-neutral contracts, Rust-first binding,
  and installed-current daemon bootstrap boundary are accepted authority.
- **Unresolved implementation details:** feature aliases beyond the accepted
  profiles, broader compatibility and deprecation mechanics, test layout, and
  consumer host projection.
- **Deferred design choices:** TypeScript/napi-rs, a separate crate, C ABI,
  public socket or sidecar protocols, consumer DTOs, and later language order.
- **Not yet proven:** the complete cross-backend conformance matrix, consumer
  integration, packaging, upgrade behavior, and operational hardening.
