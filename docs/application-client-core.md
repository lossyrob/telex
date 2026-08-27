# Supported Application Client core

The Rust library exposes `telex::application_client` as the supported semantic
boundary for long-lived applications. It implements the API-neutral contract in
[`docs/design/application-client.md`](design/application-client.md) without
making applications parse CLI output or depend on daemon frames, database paths,
connection strings, or product-private helpers.

This node ships the core, not a public language binding. Later bindings wrap
these types and preserve their identities, outcomes, and error distinctions.

The public module defines its own `ApplicationCapability` and error taxonomy;
daemon IPC enums are not part of the supported API. Message, delivery,
disposition, operation, compound-step, and delta records returned by the module
are intentional stable domain records. Their JSON payload fields are opaque
versioned evidence and must not be interpreted as backend table layouts.

## Core model

- `ApplicationResponsibility` is stable configuration. `RuntimeId` is generated
  from OS randomness for each `ApplicationClient` instance and is never reused.
- `LogicalStoreId` is an opaque identity generated once and persisted in schema
  v3. First open, restart, symlink/path spelling, and profile presentation changes
  therefore resolve to the same store identity. Public results never expose the
  internal SQLite path, Postgres target, credential, or store key.
- Every membership declares `send-only` or `bidirectional`. Send-only membership
  can author messages but is excluded from inbound occupancy, receive, and ack.
- Strict recovery reports typed membership loss. Bounded recovery retries only
  within the caller's budget and preserves capability, sender, and liveness.
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
  identity plus the durable operation-evidence retention generation. Create and
  persist the full handle with `operation_reference` before the first send.
  Reconciliation with a handle from another store fails with
  `StoreBindingMismatch`; store rebinding requires an explicit new operation.
- `reconcile_operation` returns `OperationReconciliation::NotRecorded` only when
  the exact store/responsibility/operation tuple is absent and no cleanup crossed
  the handle's retention generation. A crossed or missing generation returns
  `RetentionBoundaryCrossed`, which never authorizes retry.
- Snapshot/delta reads use a persisted monotonic store version. A gap returns
  `ResyncRequired`; timestamps are not ordering fences.
- Compound steps are declared durably with prerequisite edges. A terminal
  disposition step cannot complete before its declared prerequisite is durable.

## Backend and schema behavior

SQLite and Postgres implement the same backend trait operations. Schema version
3 adds application operation records, compound-step records, state versions, and
ordered deltas. Migration is additive and idempotent. A client refuses a store
whose schema is newer than the library supports.

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
  evidence separate.
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
