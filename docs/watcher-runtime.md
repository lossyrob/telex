# Watcher v2 runtime and local administration

Watcher runs trusted local observational commands and sends their events to a
fixed Telex route. The detector owns provider policy and an opaque cursor.
Watcher owns scheduling, execution bounds, identities, and the transaction
between that cursor and durable Telex acceptance.

The Rust crate exposes `Runtime`, `RuntimeConfig`, and
`management::LocalManagement`. Start the ordinary per-user runtime process:

```text
telex-watcher --registry <absolute-registry-path> --install-root <trusted-install-root> run
```

Omitting `run` preserves the original runtime-host invocation. In another
process, use the same `--registry` with the management commands below. Only
runtime startup requires `--install-root`; management never connects a sender.
Service installation, supervision, and automatic upgrades remain separate work.
The experimental v1 CLI adapter is retired; v1 example detectors and schemas
remain historical evidence and are not accepted by this runtime.

This runtime delivery requires process containment on Windows and Linux.
macOS and other unclaimed hosts reject detector execution. macOS remains a
required Watcher product capability tracked in
[#165](https://github.com/lossyrob/telex/issues/165); this delivery does not claim
macOS containment.

## Runtime failure diagnostics

If the host exits 1 with `runtime-failed`, inspect its stderr for the failing
stage and cause chain. This works even when startup fails before a runtime
record exists. Capture both streams when supervising the host: stdout retains
the ordinary human or JSON response; stderr carries the redacted diagnostic.
Its 4096-byte UTF-8 limit includes the final newline, and `[truncated]` marks an
omitted tail. Environment values, credential-like text and connection strings
are omitted; review diagnostics before exporting them. See the
[exit-code reference](guide/src/reference/exit-codes.md#watcher-runtime-host).

## Detached user process

On Windows, a normal PowerShell launcher can start the ordinary runtime and
exit without supervising it:

```powershell
$binary = 'C:\path\to\telex-watcher.exe'
$registry = 'C:\private-watch\registry.sqlite'
$installRoot = 'C:\trusted-telex-install'
$runtime = Start-Process -FilePath $binary -PassThru -ArgumentList `
    "--registry `"$registry`" --install-root `"$installRoot`" run" `
    -RedirectStandardOutput 'C:\private-watch\runtime.stdout.log' `
    -RedirectStandardError 'C:\private-watch\runtime.stderr.log'
& $binary --registry $registry --json status
```

Create the private parent directory first and use an absolute trusted binary.
An initial endpoint race can report `runtime-not-running`; do not register
until a later status result reports the expected runtime ID and readiness.
Keep the returned PID and registry identity with the run record. Stop the
exact owner through `& $binary --registry $registry --json shutdown`; wait for
that PID to exit, then confirm status reports `runtime-not-running`.

This is not an installer, service, or restart supervisor. A field test must
also use unique absolute `TELEX_HOME`, `TELEX_DB`, and `TELEX_INSTALL_ROOT`
values, an isolated trusted installation, and absolute branch binaries. Do
not stop an existing operator daemon or alter real directory permissions.

## Local ownership and administration

`Runtime::open` acquires a process-lifetime OS lock on the physical registry file
before initializing the registry or connecting a sender. Path aliases and hard
links contend for the same lock. A second owner receives
`registry-already-owned`; it cannot mutate registrations or membership. Kernel
lock release does not depend on a reusable PID file.

The returned `LocalManagement` handle sends bounded in-process requests to that
owner. The host exposes those same typed operations over a versioned same-user
local endpoint, created after ownership acquisition. Unix uses an owner-only
Unix domain socket in a user-private directory. Windows uses a named pipe with
an owner-only current-token-user DACL and rejects remote clients. Physical file
identity makes canonical path aliases and hard links reach the same owner.

The CLI never opens SQLite, acquires the registry lock, or falls back to offline
mutation. A missing registry, absent owner, or stale endpoint returns
`runtime-not-running`. Shutdown removes or invalidates the endpoint before
releasing the registry lock. Hosts embedding the library must retain the opaque
`Runtime::retain_registry_ownership()` guard until endpoint teardown completes.

The Unix socket is `~/.telex-watcher/<physical-identity-sha256>`, with a
validated owner-only `0700` directory and `0600` socket. Existing directory
permissions are validated, never repaired. Windows uses
`\\.\pipe\telex-watcher-v1-<user-sid-sha256>-<physical-identity-sha256>`.
Identity lookup is metadata-only: Unix device/inode or Windows volume/file ID
queried with `FILE_READ_ATTRIBUTES`, never registry data or an ownership lock.
Both peers are checked as the same OS user. Windows also protects the first
pipe instance against an already-bound endpoint.

Frames are a four-byte big-endian length and one JSON object. The wire version
is 1, with one request per connection. Requests contain `version`, the existing
serde-tagged management `command`, and optional presentation-only `filters`.
Filters use the CLI selector names in camelCase with boolean values; omitted
selectors default to false. They are accepted only for `show` and compatible
`list` commands: removed/unsupported selectors cannot broaden the typed list
command's inclusion flags. Unknown fields, nonboolean selectors, duplicate
members, and selectors on mutations are rejected before admission.

Responses contain `version` and `result`, an externally tagged `Ok`/`Err`.
`Ok` contains the versioned safe output envelope described below, including any
typed management refusal; `Err` contains a transport `code` and safe `message`.
The owner executes the unchanged `Command` through `LocalManagement`, then
applies the shared allowlist and accepted filters to its unchanged in-process
`Response` before serialization and the response cap. Raw state, parameter
values and argument values do not cross the response wire. Human and JSON
output consume that same projected envelope. For example, a paused-only list
with no matches exchanges these objects:

```json
{"version":1,"command":{"type":"list","value":{"includeRemoved":false,"includeUnsupported":false}},"filters":{"paused":true}}
{"version":1,"result":{"Ok":{"schemaVersion":1,"command":"list","ok":true,"result":{"type":"watches","value":[]}}}}
```

Registration is an exact byte vector so the owner's strict parser, not the
transport, decides registration admission. Explicit-null state replacement
stays distinct from omission. Wire, output-envelope, and v2
detector/registration schema versions are independent.

Requests and responses are capped at 16 MiB; at most 16 clients are admitted
concurrently. Connect, read, write, owner-execution, and client-response
deadlines are respectively 2, 5, 5, 30, and 41 seconds. Oversize and slow
requests fail before admission. Accepted deep state and excluded raw values
cannot exhaust a response that fits after projection and filtering. A genuinely
oversize projected result reports `response-too-large`: inspect a fixed ID
with `show`/`status`, use list filters, or reduce the history `--limit`.
Matching results are never silently truncated. Inspect before retrying a
mutation. Once writing a command may have admitted it, missing or invalid
replies report `outcome-unknown`, never a safe-to-resend assertion.

`Command` covers registration, filtered list/show, pause/resume, update
with optional state replacement, reconciliation, finite query/send grants,
permanent-rejection closure, removal, and health/attempt/event/pending inspection.
This is local same-user administration, not remote administration. A host must
not expose it through Telex messages or accept remote executable registrations.

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
Reconciliation audit rows retain the same prior/resulting revisions and
old/new committed-state hashes returned to the caller, including whether the
state changed.

## Commands and output

All commands accept `--registry PATH` or `TELEX_WATCHER_REGISTRY` and `--json`.
Human output and JSON use the same safe typed projection. Management commands
require a running owner; `run` (or no command) starts that owner.

| Command | Purpose |
| --- | --- |
| `register FILE` | Validate one v2 registration and capture the current absolute directory when omitted. |
| `list [FILTERS]`, `show ID [FILTERS]` | Inspect effective policy, revision, lifecycle, sequence high-water marks, pending identity, recurrence, and attempt summary. |
| `pause ID`, `resume ID` | Pause execution or explicitly resume a corrected watch. Pending operations refuse both. |
| `update ID FILE [--state-file FILE]` | Replace mutable registration policy; preserve committed state unless a state file is supplied. |
| `reconcile-now ID` | Query the exact pending operation, including on a removed tombstone. |
| `grant-reconciliation ID --attempts N --seconds N` | Grant finite query-only recovery for `receipt-outcome-unresolved`. |
| `grant-send-retry ID --attempts N --seconds N` | Grant finite exact-send retry only after the required authoritative proof. |
| `close-not-accepted ID` | Close a definitively permanent pre-acceptance rejection without advancing state. |
| `remove ID` | Stop future execution and retain the ID, tombstone, and unresolved evidence. |
| `status [ID]`, `health [ID]` | Inspect runtime readiness/heartbeat/staleness or per-watch health. |
| `attempts ID [--limit N]`, `events ID [--limit N]` | Inspect retained attempts and operation/receipt evidence. |
| `pending ID` | Inspect the exact staged operation and its independent query/send budgets. |
| `receipt-evidence OPERATION_ID [--limit N]` | Inspect typed receipt observations. |
| `removed-evidence ID [--limit N]` | Inspect retained operation evidence for a removed watch. |
| `shutdown` (alias `stop`) | Gracefully stop the owner and detach its known senders. |

Filters are `--active`, `--paused`, `--terminal`, `--removed`,
`--unsupported-v1`, and `--unresolved-tombstones`. Selectors form a union within
the explicitly included record kinds. Removed and unsupported kinds are
independently opt-in: combine `--removed --unsupported-v1` or use `--all` to
include removed v1 records.
`--include-removed` and `--include-unsupported` include those records without
restricting the other lifecycle states; `--all` includes everything. A fixed-ID
`show ID` with no filters discovers removed and unsupported records. An
unresolved tombstone is removed with a retained pending operation.

Grant attempts are 1 through 100 and grant duration is 1 through 86400 seconds.
History limits are 1 through 1000, default 20. Runtime concurrency is 1 through
16, default 4. Grants do not replace operation identity or implicitly resume
through a receipt, store, payload, or containment fence.

The update file is a complete v2 registration, not a partial patch. Immutable
ID, backend, sender, and target changes are refused by the owner. Omitted
`--state-file` preserves committed state even if `initialState` appears in the
registration file. A state file containing the JSON document `null` explicitly
commits null; duplicate members, invalid interoperable numbers/Unicode, and
oversize state are rejected.

For example, with an already-running owner:

```powershell
$watcher = 'C:\trusted-build\telex-watcher.exe'
$registry = 'C:\watcher-data\registry.sqlite'
& $watcher --registry $registry register .\registration.json
& $watcher --registry $registry --json show github-pr
& $watcher --registry $registry pause github-pr
Set-Content -Encoding ascii -NoNewline .\new-state.json 'null'
& $watcher --registry $registry update github-pr .\registration.json --state-file .\new-state.json
& $watcher --registry $registry resume github-pr
& $watcher --registry $registry status github-pr
& $watcher --registry $registry remove github-pr
& $watcher --registry $registry --json list --removed
& $watcher --registry $registry removed-evidence github-pr
```

JSON envelopes use `schemaVersion: 1`, `command`, `ok`, and, on a typed response,
`result: { "type": ..., "value": ... }`. A refusal/error adds `error.code` and
`error.message` and exits nonzero. For example, status without an owner returns:

```json
{"schemaVersion":1,"command":"status","ok":false,"error":{"code":"runtime-not-running","message":"no owning runtime is listening; start telex-watcher run first"}}
```

Mutation results contain the safe watch projection, typed refusal/recovery
evidence, and `audit` with action and prior/resulting registration revision.
For an admitted unsupported-v1 removal, `result.value.audit` is exactly this
shape (the retained revision may differ):

```json
{"action":"remove","priorRevision":1,"resultingRevision":1,"stateChanged":false,"stateHashStatus":"not-applicable","stateHashReason":"unsupported-registration-version"}
```

Neither hash key is present. The registry's existing audit payload additionally
retains its `revision` field; prior/resulting revision metadata is additive.

Default output omits opaque state and parameters, arbitrary command argument
values, detector diagnostic text, event bodies/metadata, and credential-bearing
client details. Effective policy still shows the executable, argument count,
directory, cadence, timeout, attention/disposition, inherited environment
**names**, backend, and fixed sender/target. Receipt identity, axes, hashes,
budgets, and typed diagnostic categories remain inspectable. A timeout or lost
reply is not proof that an admitted mutation failed; inspect the watch and
pending/evidence commands before retrying.

## Persistent event transaction

The registry uses separate v2 watch, revision, attempt, operation, audit, and
runtime tables. Existing v1 watch IDs are isolated as
`unsupported-registration-version`; their state and historical tables remain
intact. A new v2 registration requires a new ID.

An admitted `remove` of an unsupported-v1 row retains its opaque state and
permanent identity/evidence without applying v2 state hashing. Its mutation
audit includes `stateChanged: false`, `stateHashStatus: "not-applicable"`, and
`stateHashReason: "unsupported-registration-version"`; `oldStateHash` and
`newStateHash` are absent, not null. No legacy digest or migration is implied.
Other unsupported-v1 mutations remain refused. V2 state hashing and removal
semantics are unchanged.

An event first becomes one atomic staged operation containing:

- permanent watch ID, monotonic sequence, event ID, and operation ID;
- captured registration, route, backend profile, and logical-store identity;
- the complete public-client send request and recovery handle;
- prior and proposed opaque state, canonical hashes, and recurrence hash;
- independent finite receipt-query and send budgets.

Watcher writes detector requests to stdin as ordinary valid JSON. The wire
bytes need not be RFC 8785/JCS-canonical: canonical formatting can turn an
admitted floating-point token into an out-of-range integral token. Strict
request validation, I-JSON/canonicalizability checks, schema and byte limits,
and the admitted numeric model still apply. RFC 8785 canonical bytes remain
authoritative for hashes and durable evidence; they are not the detector
request transport encoding.

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

Readiness requires membership reconciliation for the current registry revision
and verified bindings for every required sender. Health exposes
`reconciledRegistryRevision` and each profile's `requiredSenders`; startup and a
new registration remain non-ready until their membership is checked. An empty
sender snapshot is not evidence that a newly registered watch is ready.

The watch retains its original logical store and the last observed store
separately. A mismatched observation cannot demote a terminal watch or erase a
containment or pending-receipt blocker. After the original store is verified, a
non-pending store-mismatch block becomes eligible for explicit resume; it does
not resume automatically. Every remaining fence must still be cleared. If a
profile was corrected, restart the runtime to select its corrected client.

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

The same SQLite and credentialed-Postgres runtime matrices also invoke the
actual CLI against the live owner's local endpoint. They cover immutable
updates, pending pause/update/resume refusals, finite query/send grants,
retention uncertainty, permanent-rejection closure, removed reconciliation,
tombstone reuse, unsupported-v1 removal, and exact safe JSON audit output.
`tests/cli_v2.rs` covers parsing, ordinary process-host lifecycle, explicit-null
replacement, filtering, safe projections, and typed failures.

Transport tests cover same-user success, physical path aliases/hard links,
stale/missing endpoints, first-instance collision, malformed/oversize/slow
clients, concurrency, dropped-caller admission, bounded replies, and cleanup.
Windows tests inspect the protected DACL and verify it grants only the exact
current token-user SID. For an additional alternate-user local check, use an
already-existing different OS account to run the same absolute binary with
the owner's absolute `--registry` and `--json status`; it must refuse access
and must not expose registration or health data. Do not create users, grant
access, or alter existing directory permissions for this check. The automated
suite does not claim an actual second-user logon when no such account is
available.

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
