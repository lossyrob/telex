# Supported Rust Application Client

The root `telex` crate exposes `telex::application_client` as the first supported
application binding. It implements the API-neutral contract in
[`docs/design/application-client.md`](design/application-client.md) without
making applications parse CLI output or depend on daemon frames, database paths,
connection strings, or product-private helpers.

The public module defines its own `ApplicationCapability` and error taxonomy;
daemon IPC enums are not part of the supported API. Message, delivery,
disposition, operation, compound-step, and delta records returned by the module
are intentional stable domain records. Their JSON payload fields are opaque
versioned evidence and must not be interpreted as backend table layouts.

## Dependency profiles

Application consumers disable the root package defaults and select backend
features explicitly:

| Profile | Cargo features |
| --- | --- |
| SQLite | `sqlite` |
| Postgres | `postgres` |
| Postgres with Entra | `entra` (includes `postgres`) |
| SQLite and Postgres | `sqlite,postgres` |
| SQLite and Postgres with Entra | `sqlite,entra` |

For example:

```toml
[dependencies]
telex = { git = "https://github.com/lossyrob/telex", rev = "<full-commit-sha>", default-features = false, features = ["sqlite"] }
```

None of these profiles enables `self-update`. A single-backend profile does not
enable the other backend.

No published Telex release contains this binding yet. Source consumers must
replace the placeholder with the full commit ID of a reviewed revision. The
compatibility promise applies to that pinned source revision and subsequent
documented version transitions; an unpinned Git dependency follows a moving
branch and is outside the promise.

## Compatibility boundary

The root `telex` crate version governs Rust source compatibility. The supported
surface is the contract-bearing types and behavior exposed by
`telex::application_client`: client and maintenance handles; responsibility,
runtime, store, and operation identities; capability and recovery policy;
lifecycle and compensation results; send, reply, recovery, and reconciliation;
exact delivery and acknowledgement; receipt evidence; typed errors; health,
provenance, history, and ordered deltas; compound operations; and bounded
maintenance. Contract-bearing `telex::model` types already used by these public
signatures share that source-compatibility commitment.

Breaking changes require an appropriate crate-version transition and migration
guidance. Telex deprecates before removal when a compatible transition is
possible. Serde support lets applications persist typed evidence; it does not
promise stable JSON, a C ABI, a cross-language serialization format, or a public
daemon protocol. Backend rows not already used by the binding, daemon frames,
CLI types, private helpers, and product DTOs are not supported binding surfaces.

## Runtime and cancellation

The caller creates and configures the Tokio runtime. Application Client futures
run in that runtime, SQLite blocking operations use its blocking pool, and
Postgres connection drivers use tasks on that runtime. The binding creates no
Tokio runtime, application-specific daemon, or sidecar.

Cancellation stops the caller's observation of a future; it does not prove that
Telex or the selected backend committed nothing:

- For multi-address attach, reconcile, or detach that may be canceled, create a
  stateful operation with `begin_attach`, `begin_reconcile_many`, or
  `begin_detach_many`. Drive `run` or `advance` in the caller's cancellation
  selection. After cancellation, `cancelled_outcome` preserves completed
  per-address results and compensation, identifies one in-flight address that
  may have committed, and lists addresses not attempted. Reconcile the uncertain
  address before retrying. An in-flight canceled reconciliation remains
  `InProgress` in lifecycle health (and `recovering` for bounded repair) until a
  later reconciliation establishes a terminal result. The direct `attach`,
  `reconcile_many`, and
  `detach_many` methods are run-to-completion conveniences.
- Canceling `receive` does not acknowledge a delivery. A returned
  `ReceivedDelivery` remains unacknowledged until the caller durably ingests it
  and calls `acknowledge` or records a terminal exact-recipient disposition.
- Before polling `send` or `reply`, persist the `RecoveryHandle` returned by
  `prepare_send` or `prepare_reply`. Reconcile a canceled or transport-uncertain
  attempt with `reconcile_operation`; cancellation never authorizes a blind
  resend.
- A canceled acknowledgement, disposition, cleanup, or recovery call may have
  committed. Use its exact identity and authoritative refresh or reconciliation
  rather than inferring absence from cancellation.

The binding does not retain a detached lifecycle executor. The caller owns the
operation object and the Tokio task that drives it.

## Core model

- `ApplicationResponsibility` is stable configuration. `RuntimeId` is generated
  from OS randomness for each `ApplicationClient` instance and is never reused.
- `LogicalStoreId` is an opaque identity generated once and persisted in schema
  v3. First open, restart, symlink/path spelling, and profile presentation changes
  therefore resolve to the same store identity. Public results never expose the
  internal SQLite path, Postgres target, credential, or store key.
- Every membership declares `send-only` or `bidirectional`. Send-only membership
  can author messages but is excluded from inbound occupancy, receive, ack,
  history queries, and inbound backlog health evidence.
- Strict recovery reports typed membership loss. Bounded recovery retries only
  within the caller's budget and preserves capability, sender, and liveness.
- `attach`, `reconcile_many`, and `detach_many` return one aggregate result for
  an address set. A partial result identifies each completed and failed address
  and supplies typed compensation only for work performed by that call. A new
  membership receives `Detach`; a changed pre-existing membership receives
  `Reattach(previous_spec)`; an idempotent refresh receives no destructive
  compensation.
- Deliberate application detach is keyed by stable
  `ApplicationResponsibility` and address. The durable record retains the
  runtime ID and capability as audit evidence, so a replacement process cannot
  reverse the detach through bounded repair. A later explicit attach clears the
  intent only after daemon membership commits successfully.
- Receive results carry message ID, recipient, delivery-row ID, role, store ID,
  and an `AckHandle` bound to that exact delivery. Ack remains an explicit action
  after durable application ingest.
- Oversized historical primary deliveries return
  `ApplicationClientError::DeliveryQuarantined` with structured recipient,
  message, serialized-byte, and frame-limit evidence. `may_continue` is
  currently always `true`; callers should preserve the field and continue
  receiving.
- `ReplyRequest.metadata` is opaque application data. Telex fingerprints and
  transports the bytes unchanged through reply creation, persistence, thread
  reads, and receive projection; interpretation and extension-field semantics
  remain application or binding responsibilities.
- Durable acceptance, occupancy, push, exact-recipient consumption, and workflow
  disposition remain separate `ReceiptAxes`. The axes returned by send/reply are
  an acceptance-time snapshot. `refresh_receipt_axes` refreshes exact delivery
  consumption and workflow disposition; push-attempt evidence is currently
  reported as unavailable rather than implied current.
- `EvidenceState::Quarantined` is sticky recipient-consumption evidence.
  `DispositionRow.origin == "daemon-quarantine"` is the authoritative
  structural provenance; principal and note text are descriptive only and
  cannot mint this origin through supported disposition APIs.
- Application operations use caller-supplied `OperationId` values. Reuse with the
  same payload reconciles the prior result; reuse with different input is a typed
  `OperationMismatch`.
- Pre-acceptance rejection exposes `RejectionRetryability::Transient` or
  `Permanent`; retry policy never depends on parsing error prose.
- `SendResult` and `OperationMismatch` carry canonical `PayloadIdentity`
  evidence. Prior completion is adopted only for the same operation ID and a
  comparable matching payload digest.
- `RecoveryHandle` stages operation ID, responsibility, and opaque logical-store
  identity plus the durable operation-evidence retention generation. Call
  `prepare_send` or `prepare_reply` with the complete request and persist the
  returned handle before the first attempt. The client derives the same
  canonical payload identity that `send` or `reply` uses.
  Reconciliation with a handle from another store fails with
  `StoreBindingMismatch`; store rebinding requires an explicit new operation.
- `reconcile_operation` returns `OperationReconciliation::NotRecorded` only when
  one backend snapshot proves the operation, result, message mapping, and
  receipt evidence absent for the exact tuple and confirms that no cleanup
  crossed the handle's retention generation. A crossed or missing generation
  returns `RetentionBoundaryCrossed`, which never authorizes retry.
- Recorded reconciliation projects persistence into typed `Accepted`,
  `Rejected`, `Partial`, `Indeterminate`, `Duplicate`, or `Pending` outcomes.
  Consumers do not parse backend state strings or result/recovery JSON.
- If Telex returns `Sent` but durable result persistence fails, `send` and
  `reply` return `ApplicationClientError::Indeterminate` with the staged
  recovery handle. They never report that accepted-send/local-result window as
  ordinary unavailability. The same rule applies when reconciliation discovers
  durable message acceptance but cannot persist the promoted accepted result.
- Snapshot/delta reads use a persisted monotonic store version. A gap returns
  `ResyncRequired`; timestamps are not ordering fences.
- Compound steps are declared durably with prerequisite edges. A terminal
  disposition step cannot complete before its declared prerequisite is durable.

## Backend and schema behavior

SQLite and Postgres implement the same backend trait operations. Schema version
3 adds application operation records, stable-responsibility detach intents,
compound-step records, state versions, and ordered deltas. Migration is additive
and idempotent. A client refuses a store whose schema is newer than the library
supports.

Operation reconciliation reads the operation row, message mapping, and
responsibility-scoped retention generation in one SQLite transaction or
Postgres repeatable-read snapshot. Concurrent Postgres first attempts use
conflict-safe insertion, so one begins and an identical contender receives
`Replay` rather than a uniqueness error. Postgres locks a pre-existing replay
row in the same statement that returns it, so cleanup cannot remove the row
between conflict detection and replay classification.

Opening schema v3 repairs a missing disposition `origin` column without
inferring quarantine origin for historical rows from principal or note prose.

SQLite principal strings are `unverified`; local OS identity is not authenticated
backend evidence. Postgres exposes the configured connection user as
`unverified`; authenticated transport access alone is not identity proof.

## Operating guidance

- Keep operation IDs in restart-safe application state before authoring a
  retryable operation.
- Ack only after the application has durably ingested enough state to resume.
- Treat `Unknown { raw_reason }` as typed forward-compatible evidence. Do not
  reinterpret it as deliberate detach, collision, or safe automatic repair.
- Use application `cleanup` with explicit age and row-count bounds. It deletes
  only terminal records owned by that responsibility and preserves in-flight
  work, messages, deliveries, dispositions, other apps, and the store-global
  delta journal. Deleting terminal operation evidence advances a durable,
  responsibility-scoped retention generation so older absence checks fail
  closed. Store administrators use `ApplicationStoreMaintenance` with an
  explicit version floor to prune global deltas.
- A capability mismatch or missing delivery-row identity is a fail-closed version
  skew signal. Upgrade the daemon/client pair; do not fabricate an ack identity.
- Existing daemon `StationHealth` remains available for compatibility. New
  applications should use `ApplicationHealth`, which keeps sender readiness,
  receive readiness, backlog, ack-pending, recovering, degraded, and unattended
  evidence separate. Its typed lifecycle evidence includes membership loss,
  collision, reconciliation, pending compensation, and durable deliberate
  detach. Known restart, predicate-death, owner-demotion, and collision reasons
  remain distinct. Daemon status carries terminal predicate-loss evidence, and
  unknown future `needs_attach_reason` values retain the exact raw wire token.
- `reconcile_operation` checks the durable operation-to-message mapping written
  in the same transaction as message acceptance. After a crash in the
  accepted-send/local-result window, it promotes a matching pending operation to
  `accepted` before the application authors a replacement. If no record or
  mapping exists and the persisted retention generation still matches, it
  returns typed authoritative `NotRecorded` evidence.

## Required later conformance coverage

The `client-conformance` node must run the same semantic cases against SQLite and
credentialed Postgres:

1. fresh runtime identity and stable responsibility/store identity;
2. strict and bounded recovery, restart loss, deliberate detach, predicate death,
   owner demotion, collision evidence, and unknown future loss reasons;
3. atomic-or-compensable multi-address attach/reconcile/detach;
4. send-only false-attendance prevention and bidirectional exact delivery ack;
5. independent receipt/workflow axes and ack-after-ingest restart recovery;
6. operation replay, fingerprint mismatch, authoritative exact-tuple
   `NotRecorded`, retention-boundary invalidation, accepted-send indeterminate
   windows, and post-restart reconciliation;
7. unresolved/recent/thread filtering before bounds and store-scoped source
   resolution;
8. monotonic delta ordering, gap detection, resync, and no-regression backfill;
9. compound prerequisite ordering, partial/indeterminate outcomes, recovery
   handles, and crash continuation;
10. schema v2-to-v3 migration, newer-schema refusal, bounded cleanup, principal
    provenance, and raw path/credential exclusion.

This binding does not complete that matrix, authorize Watcher or Operator
Station integration, or establish packaging, upgrade, or production-readiness
claims.
