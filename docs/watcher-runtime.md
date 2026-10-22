# Watcher v2 runtime core

Watcher runs trusted local observational commands and sends their events to a
fixed Telex route. The detector owns provider policy and an opaque cursor.
Watcher owns scheduling, execution bounds, identities, and the transaction
between that cursor and durable Telex acceptance.

The Rust crate exposes `Runtime`, `RuntimeConfig`, and
`management::LocalManagement`. The binary is a minimal runtime host:

```text
telex-watcher --registry <absolute-registry-path> --install-root <trusted-install-root>
```

Registration and administration use the in-process typed handle. The management
CLI, service installation, and automatic upgrades are separate work. The
experimental v1 CLI adapter is retired; v1 example detectors and schemas remain
historical evidence and are not accepted by this runtime.

## Local ownership and administration

`Runtime::open` acquires a process-lifetime OS lock on the physical registry file
before initializing the registry or connecting a sender. Path aliases and hard
links contend for the same lock. A second owner receives
`registry-already-owned`; it cannot mutate registrations or membership. Kernel
lock release does not depend on a reusable PID file.

The returned `LocalManagement` handle sends bounded in-process requests to that
owner. `Command` covers registration, filtered list/show, pause/resume, update
with optional state replacement, reconciliation, finite query/send grants,
permanent-rejection closure, removal, and health/attempt/event/pending inspection.
It is not a remote administration protocol. A host must not expose it through
Telex messages or accept remote executable registrations.

Every admitted mutation runs in the same SQLite transactions as scheduler and
recovery work. Refusals have a typed `Refusal`; results contain the current
registration revision, lifecycle, eligibility, and health. Updates cannot
change ID, backend profile, sender, or target. Pending operations refuse update,
pause, and resume. `None` state replacement preserves the cursor;
`Some(Value::Null)` replaces it with JSON null.

The registry records successful and refused management operations. An update
during an attempt invalidates that attempt's captured revision; its result
cannot commit or stage an event. Removal cancels execution, retains pending
operations, and permanently reserves the ID.

## Persistent event transaction

The registry uses separate v2 watch, revision, attempt, operation, audit, and
runtime tables. Existing v1 watch IDs are isolated as
`unsupported-registration-version`; their state and historical tables remain
intact. A new v2 registration requires a new ID.

An event first becomes one atomic staged operation containing:

- permanent watch ID, monotonic sequence, event ID, and operation ID;
- captured registration, route, backend profile, and logical-store identity;
- the complete public-client send request and recovery handle;
- prior and proposed opaque state, canonical hashes, and recurrence hash;
- independent finite receipt-query and send budgets.

The sequence is allocated only when staging commits. A send intent is persisted
before polling the client. Restart therefore queries an interrupted operation;
it never assumes that a staged event was unsent. Exact retry preserves the
request, operation ID, event ID, sequence, state, and client recovery handle.

Matching durable acceptance commits state, sequence, receipt evidence, and the
attempt together. Occupancy, push, consumption, and disposition remain separate
diagnostic axes. A local commit failure retains the pending operation for
receipt lookup after restart.

Authoritative `NotRecorded` and explicitly transient pre-acceptance rejection
permit bounded exact retry. Unknown history and `RetentionBoundaryCrossed`
permit queries only. Exhaustion, permanent rejection, identity conflict, and
store mismatch retain evidence and expose a typed blocked reason. A query-only
grant cannot authorize a send: late absence proof first changes the blocked
reason to `send-retry-exhausted`, after which a separate finite send grant is
required. Permanent rejection requires audited `CloseNotAccepted`; its sequence
is never reused.

Removed operations can still be reconciled. Closure updates the retained
tombstone and never resurrects the watch. Identical later events after a
successful commit receive new identities; recurrence metrics observe exact
rendered-content equality without suppressing events.

## Telex integration

Production uses the public Rust Application Client with explicit
`InstalledCurrent` authority. `ExactExecutable` is available through
`RuntimeConfig` for isolated development and tests. There is no PATH search,
CLI-output parser, raw daemon connection, or private fallback for Telex.

The registry persists one stable application responsibility. Each logical store
gets a fresh application runtime/session, shared by all of that store's sender
addresses and profile aliases. A separate Watcher process-incarnation ID binds
attempts and normalized message provenance. Store identity remains explicit in
membership, staged operations, receipts, and health.

Senders attach with `SendOnly`, process-bound liveness, explicit sender
selection, and bounded lifecycle calls. Partial attachment retains typed
cancellation and compensation evidence and keeps affected senders non-ready.
Startup, registry changes, periodic reconciliation, and membership loss drive
repair without force takeover. Graceful shutdown detaches known senders.

Watcher never receives, acknowledges, or dispositions inbound messages. An
actionable backlog on a sender address degrades runtime health and sets
`productionReady` to false.

## Execution and evidence

Global detector concurrency is configurable from 1 to 16. Each watch is
single-flight. Deterministic jitter, capped exponential failure backoff, and
coalesced due ticks bound execution; restart schedules at most one due attempt
instead of replaying missed intervals. Runtime heartbeat advances independently
of detector execution.

Detector argv executes directly with its captured absolute working directory.
Only the documented platform baseline and explicitly allowlisted inherited
environment variables are available. Environment values are not stored in
registration or detector requests. These are accident bounds for trusted
same-user code, not a sandbox.

Stdout is one v2 JSON result, capped at 256 KiB; stderr is capped at 64 KiB.
Opaque state and parameters are each capped at 256 KiB. UTF-8 byte caps and
recursive I-JSON validation supplement JSON Schema string constraints. Hashes
use RFC 8785 canonical JSON and `sha256:<lowercase-hex>`.

The versioned machine projections are
[event metadata v2](design/schemas/watcher-event-metadata-v2.schema.json) and
[runtime/watch health v2](design/schemas/watcher-health-v2.schema.json).
Detailed local evidence retains typed public-client results, not private daemon
frames. Opaque detector state never appears in normalized Telex metadata.

Completed identity/receipt evidence and unresolved payloads are retained.
Health exposes retained logical row/byte counts and positive warning
thresholds. Default warnings are 100,000 rows or 256 MiB; these are warnings,
not compaction or a capacity guarantee. Capacity policy and versioned safe
compaction remain operational hardening work.

## Validation boundary

`registry_v2::tests` injects acceptance, commit failure, restart, authoritative
absence, transient/permanent rejection, retention uncertainty, identity/store
conflict, late outcomes, grants, removal, and containment uncertainty.
`sqlite_watcher_runtime_matrix` and `postgres_watcher_runtime_matrix` exercise
the same transactions through the public client against isolated real stores.
`TELEX_PG_REQUIRE=1` makes missing PostgreSQL configuration a failure.

Before running a library-only runtime matrix, build the branch's Telex,
detector fixture, and guardian binaries with
`cargo build -p telex -p telex-watcher --all-features --bins`. `cargo test --lib`
does not build those executable targets itself.

The first practical detector is under
`telex-watcher/field-validation/github-pr`. It is an observe-only, fixture-first
GitHub PR detector, separate from the optional example pack. Its README states
the exact review/comment/check coverage and exclusions.

Daemon and destructive process proofs use unique absolute `TELEX_HOME`,
`TELEX_DB`, and `TELEX_INSTALL_ROOT`, plus absolute branch binaries. No proof
targets an installed user daemon or changes permissions on existing user
directories.
