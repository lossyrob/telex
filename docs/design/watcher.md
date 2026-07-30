# Telex Watcher - Production Contract

## Status

**Normative design specification.** This document defines the production
contract for Telex Watcher, a separately supervised application that runs
trusted local detector commands outside agent sessions and emits normalized
Telex messages.

The implemented vertical spike and its evidence remain documented in
[Generic Watcher Spike Report](../generic-watcher-spike-report.md). The original
production boundary is recorded in
[ADR 0046](DECISIONS.md#0046--watcher-runs-provider-neutral-trusted-local-detectors-with-receipt-gated-state).
The minimal v2 authoring, registration, identity, and compatibility direction is
recorded in
[ADR 0050](DECISIONS.md#0050--watcher-v2-uses-minimal-command-registration-and-runtime-owned-event-identity).

Mechanism-level Telex membership, liveness, lease, delivery, receipt, and
disposition semantics remain governed by [daemon.md](daemon.md). Telex core
treats Watcher event metadata as opaque. The namespacing guidance in
[EXTENSIONS.md](proposals/EXTENSIONS.md) is compatible with this contract but
remains a proposal and is not an interpretation dependency for Watcher.

The **Telex Watcher application** is unrelated to the daemon message-recipient
category named `watchers`. This document does not extend or reinterpret that
recipient role.

## Product boundary

Watcher has one purpose:

```text
trusted local detector
        |
        | versioned request/result
        v
Telex Watcher
        |
        | fixed-route event
        v
durable Telex send
```

Watcher:

- runs as a persistent per-user process outside agent sessions;
- executes trusted local observational commands on a bounded schedule;
- persists registration, opaque state, attempts, pending sends, event evidence,
  recurrence diagnostics, and health;
- owns backend, sender, target, attention, disposition, cadence, timeout,
  environment, process bounds, and retry policy;
- generates stable event identity and commits event-producing state only after
  durable Telex acceptance;
- performs one reaction only: send a Telex message; and
- uses the shared Telex Application Client contract owned through issue #12.

Watcher does not:

- interpret GitHub, Azure DevOps, HTTP, file, or provider semantics;
- decide provider cursor, replay, downtime, or recurrence policy;
- execute a configurable post-detection action;
- merge, approve, mutate, launch, or orchestrate downstream work;
- accept remote registration or replacement of executable code;
- claim to sandbox or attest the bytes of same-user detector commands;
- require manifests, script pinning, kind allowlists, provider preflight,
  provider fixtures, or a template framework;
- provide hosted webhook ingestion or multi-host failover; or
- define a Watcher-specific public Telex client API.

Provider behavior belongs in trusted user-owned detector code. Consequential
action belongs to the recipient woken by Telex.

## Ordinary authoring flow

The ordinary workflow is deliberately short:

1. Write or copy a local observational script.
2. Optionally run it with a sample v2 request.
3. Register its command, cadence, timeout, backend, and fixed route.
4. Leave Watcher running outside the agent session.
5. Inspect Watcher status and attempt diagnostics if it fails.

Manifests, hashes, pinning, provider preflight, downtime declarations, fixtures,
and deeper conformance tests are project choices. They are not prerequisites for
ordinary registration.

## Terms

| Term | Meaning |
|---|---|
| **Runtime** | One Watcher process incarnation. Each process has a fresh, never-reused runtime ID. |
| **Watch** | A durable local registration containing a detector command and generic Watcher policy. |
| **Detector** | A trusted local command that observes a source and returns one structured result. |
| **Attempt** | One bounded detector execution against one committed prior state. |
| **Pending operation** | A durably staged event transition that has not yet been proven accepted or rejected. |
| **Event sequence** | A strictly increasing, never-reused integer allocated within one permanent watch ID. |
| **Event ID** | A runtime-generated globally unique alias bound one-to-one to `(watchId, eventSequence)`. |
| **Operation ID** | The retry-safe Application Client identity for one exact Telex send operation. |
| **Normalized event** | The fixed-route Telex message plus Watcher provenance. |
| **Committed event evidence** | Durable receipt and state-transition evidence for an accepted event identity. |
| **Recurrence hash** | A canonical hash of fixed route/policy plus detector event content, excluding runtime identity and receipt fields. |
| **Lifecycle** | The durable watch state: `active`, `paused`, `terminal`, or `removed`. |
| **Eligibility** | Whether a watch may start a detector attempt: `eligible`, `reconciliation-pending`, or `inactive`. |
| **Health** | The current operational condition of a runtime or watch, separate from lifecycle and eligibility. |

## Runtime architecture

Watcher is an application, not part of the local exchange:

```text
local management CLI
        |
        v
Watcher registry/state <----> Watcher runtime ----> Telex Application Client
                                  |
                                  +----> bounded detector child processes
```

The registry is local administrative state. Registration and executable
mutation are local-only. Telex messages cannot add, update, or replace detector
commands.

The first production runtime supports one process per registry. A runtime may
serve many watches and many stable sender addresses. Multi-host ownership or
failover for one registry/watch is deferred.

Before sender attachment, scheduling, reconciliation, or registry mutation, the
runtime must acquire exclusive ownership keyed by the canonical physical
registry identity. Ownership is process-lifetime and PID/start-time reuse-safe.
A second runtime fails startup nonzero without touching sender membership or
registry state and exposes a supervisor-visible `registry-already-owned`
diagnostic. This application lock is separate from the Telex daemon's store
ownership and lease epoch.

## Watch registration v2

The canonical machine-readable input shape is
[watcher-registration-v2.schema.json](schemas/watcher-registration-v2.schema.json).

Required registration input is limited to command identity, generic execution
bounds, backend selection, and fixed routing:

| Field | Required | Contract / default |
|---|---:|---|
| `schemaVersion` | yes | `2`. |
| `id` | yes | Stable watch ID. A removed ID is tombstoned and never reusable. |
| `command` | yes | Non-empty local argv. No shell interpolation is implied. |
| `intervalSeconds` | yes | Requested cadence, subject to product minimum/maximum bounds and jitter. |
| `timeoutSeconds` | yes | Detector execution timeout, subject to product bounds. |
| `backendProfile` | yes | Explicit configured Telex backend/profile name. Credentials are not copied into registration. |
| `sender` | yes | Stable Telex sender responsibility. |
| `target` | yes | Fixed Telex target. |
| `attention` | no | Defaults to `next-checkpoint`. |
| `requiresDisposition` | no | Defaults to `true`. |
| `workingDirectory` | no | Registration command's current absolute directory is captured, validated, and persisted when omitted. |
| `environmentAllowlist` | no | Inherited environment variable names. Defaults to `[]`. |
| `parameters` | no | Opaque detector parameters. Defaults to `{}`. |
| `initialState` | no | Initial opaque committed state. Defaults to `null`. |

Registration validates schema version, watch ID, argv, timing bounds, addresses,
backend/profile name, working directory, environment variable names, and opaque
JSON size/canonicalizability before persistence.

Every successful update increments `registrationRevision`. Updates may change
command, execution policy, backend, or route, but cannot replace an already
staged pending operation. That operation retains the captured registration
revision, route, attention, disposition, and payload until it is committed,
proven not accepted, or retained unresolved by removal.

### Command and working-directory semantics

Watcher executes the argv directly and never inserts a shell. The persisted
working directory is inspectable through management status.

An absolute `argv[0]` names the executable directly. A non-absolute value is
resolved on each attempt through the documented controlled launch environment.
Users who require stable command resolution choose an absolute path or a
trusted wrapper. Watcher records the attempted argv and working directory for
diagnostics but does not claim script-byte provenance.

A missing working directory, unresolved executable, or non-executable command is
a typed process failure. It does not advance state or allocate an event. Normal
bounded failure backoff applies.

### Core versus optional project hardening

These concerns are absent from v2 registration and runtime semantics:

- manifests or template descriptors;
- script digests, pinning, or follow-path modes;
- event-kind allowlists or allowed prefixes;
- provider preflight;
- provider cursor/downtime declarations;
- provider fixtures or provider conformance suites; and
- provider/credential budgets.

Projects may implement them in detector code, command wrappers, examples, tests,
or later separately versioned optional tooling. "Optional" does not mean an
optional v2 registration field.

Watcher retains only provider-neutral invariants: strict versioned envelopes,
canonicalizable opaque JSON, fixed content and process bounds, single-flight
execution, generic scheduling/backoff, fixed routing, receipt-gated commit,
runtime identity evidence, diagnostics, and recurrence metrics.

## Lifecycle, eligibility, and health

Lifecycle remains intentionally small:

| Lifecycle | Meaning | Legal transitions |
|---|---|---|
| `active` | Watch is enabled. It schedules only when eligibility is `eligible`. | `paused`, `terminal`, `removed` |
| `paused` | Detector attempts are disabled. May be operator-selected or health-blocked. | `active` through explicit resume or successful late reconciliation, `removed` |
| `terminal` | Detector completed the watch. No further detector scheduling. | `removed` |
| `removed` | Administratively removed but identity, tombstone, and unresolved evidence are retained. | none |

Eligibility is a separate scheduling axis:

| Eligibility | Meaning |
|---|---|
| `eligible` | An active watch may start a detector attempt when due. |
| `reconciliation-pending` | A pending operation bars detector execution while its original send outcome is reconciled. |
| `inactive` | Lifecycle does not permit detector attempts. |

Health is `ready`, `degraded`, `blocked`, or `inactive`.

Legal combinations include:

| Lifecycle | Eligibility | Health | Meaning |
|---|---|---|---|
| `active` | `eligible` | `ready` | Eligible and operating normally. |
| `active` | `eligible` | `degraded` | Eligible, but a bounded attempt or sender condition failed. |
| `active` | `reconciliation-pending` | `degraded` | Bounded automatic reconciliation is in progress; no detector attempt may start. |
| `paused` | `inactive` | `inactive` | Explicit operator pause. |
| `paused` | `inactive` | `blocked` | Non-transient condition requires operator action. |
| `terminal` | `inactive` | `inactive` | Detector completed. |
| `removed` | `inactive` | `inactive` | Administratively removed; evidence retained. |

`blockedReason` is non-null only for `paused`/`blocked`. V2 reasons are:

- `receipt-outcome-unresolved`;
- `unsupported-registration-version`; and
- `orphan-containment-unproven`.

Address occupancy never controls lifecycle. A target or sender becoming
unoccupied does not silently cancel or expire a watch.

## Detector protocol v2

Watcher invokes a detector with a single JSON request on stdin. The normative
shape is
[watcher-detector-request-v2.schema.json](schemas/watcher-detector-request-v2.schema.json).

```json
{
  "schemaVersion": 2,
  "attempt": {
    "id": "attempt-uuid",
    "now": "2026-07-30T18:00:00Z"
  },
  "watch": {
    "id": "github-pr-133",
    "activatedAt": "2026-07-30T17:45:00Z",
    "lastSuccessAt": null,
    "parameters": {
      "repo": "lossyrob/telex",
      "pullRequest": 133
    }
  },
  "state": null
}
```

`attempt.now`, `watch.activatedAt`, `watch.lastSuccessAt`, parameters, and
committed opaque state give detector code enough generic facts to implement its
own provider cursor, replay, recurrence, and gap policy. A detector may persist
additional timing facts in opaque state. Watcher does not interpret them.

The detector exits zero and writes exactly one JSON result to stdout. The
normative shape is
[watcher-detector-result-v2.schema.json](schemas/watcher-detector-result-v2.schema.json).

```json
{
  "schemaVersion": 2,
  "outcome": "event",
  "nextState": {
    "lastReviewId": 8421
  },
  "event": {
    "kind": "github.pull-request.review",
    "subject": "External review received on PR #133",
    "body": "A reviewer requested changes.",
    "metadata": {
      "reviewer": "example",
      "reviewState": "CHANGES_REQUESTED"
    }
  }
}
```

The detector does not author event identity. `event` has no `id`,
`eventSequence`, or `operationId`. It also cannot set backend, sender, target,
attention, disposition, cadence, timeout, or an action.

The outer request/result objects are strict. Unknown fields are rejected.
Detector `parameters`, `state`, `nextState`, and `event.metadata` remain
arbitrary JSON.

All opaque values must be valid I-JSON and RFC 8785 canonicalizable. The parser
rejects duplicate object member names, invalid Unicode, non-finite numbers, and
numbers outside the canonicalizer's supported interoperable range.
Noncanonicalizable registration input is rejected. Noncanonicalizable detector
output is `diagnosticCategory = canonicalization-failed`: no send or state
advancement, and normal bounded failure backoff applies.

The v2 runtime accepts v2 registrations and detector results only. An unsupported
version never advances state or sends. Version selection is explicit; later
major versions require an explicit compatibility or migration decision.

### Outcomes

| Outcome | Event | State behavior | Lifecycle behavior |
|---|---|---|---|
| `idle` | forbidden | Valid `nextState` commits immediately | remains active |
| `event` | required | `nextState`, or unchanged prior state when omitted, commits only after durable Telex acceptance | remains active |
| `terminal` | optional | Event-producing state is receipt-gated; eventless state commits directly when supplied | becomes terminal after commit |
| `degraded` | forbidden | Must not contain or advance `nextState` | remains active with failure/backoff |

An `idle` state advance asserts that the detector successfully evaluated the
source and intentionally classified all observations through the new cursor as
non-actionable. A detector must not advance past work it did not evaluate.

A bare `terminal` result with neither `event` nor `nextState` is valid. It leaves
prior state unchanged, records the attempt, and transitions the watch to
terminal.

For `event` and event-producing `terminal`, omitted `nextState` means unchanged
committed prior state. Omission never creates an undefined state.

Process exit status is separate from detector outcome. Nonzero exit is an
execution failure, not `degraded`.

### Protocol limits

V2 preserves the established bounded protocol limits:

| Value | Limit |
|---|---:|
| Detector stdout | 256 KiB |
| Detector stderr | 64 KiB |
| Opaque parameters/state/`nextState` | 256 KiB each when serialized |
| Event kind | 256 UTF-8 bytes |
| Event subject | 512 UTF-8 bytes |
| Event body | 128 KiB |
| Detector metadata | 64 KiB serialized |
| Complete normalized Watcher metadata | 80 KiB serialized |

JSON Schema string lengths do not substitute for these UTF-8 byte limits. The
runtime enforces byte caps before send or commit. Oversize or truncated output is
a failure and is never sent as success.

Event kind is detector-owned opaque application vocabulary. It must be non-empty,
contain no disallowed control characters, and remain within the byte cap.
Watcher does not require the v1 lowercase dot-separated pattern or an allowlist.
Telex core continues to carry kind without interpreting it.

## Runtime versus detector responsibility

| Concern | Owner |
|---|---|
| Provider query and credentials used by that query | Detector/project |
| Provider cursor, replay, ignored observations, and downtime-gap policy | Detector opaque state |
| Event kind, subject, body, metadata, and recurrence intent | Detector |
| Command, working directory, environment names, cadence, timeout | Registration/Watcher |
| Backend, sender, target, attention, disposition | Registration/Watcher |
| Process timeout, output caps, single-flight, concurrency, jitter, backoff | Watcher |
| Event sequence, event ID, operation ID, normalized envelope | Watcher |
| Durable send, receipt reconciliation, receipt-gated state commit | Watcher + Application Client |
| Consequential downstream action | Telex recipient |

## Event identity and receipt-gated state

### Identity invariants

1. A watch ID is a permanent registry namespace allocation. Removed IDs are
   tombstoned and never reused.
2. `eventSequence` starts at 1, increases strictly within a watch ID, and is
   never reused.
3. Sequence allocation is monotonic but not guaranteed dense. A permanently
   rejected or unresolved removed operation may leave a documented gap.
4. `eventId` is generated by Watcher when the sequence is staged and is bound
   one-to-one to `(watchId, eventSequence)`.
5. `operationId` identifies the exact Application Client send operation. It is
   stable across reconciliation and same-operation retry but is not the semantic
   event identity.
6. Retry never changes sequence, event ID, operation ID, route, policy, or
   payload.
7. A later detector event after a successful commit is a recurrence and receives
   the next sequence even when its content matches a prior event.

Recipients may deduplicate by `eventId` or by the canonical
`(watchId, eventSequence)` pair.

### Staging transaction

Before the first send, Watcher atomically persists:

- watch ID, event sequence, event ID, and operation ID;
- attempt ID and captured registration revision;
- fixed backend, sender, target, attention, and disposition;
- committed prior state and proposed next state;
- the complete normalized Telex envelope;
- canonical hashes of prior state, proposed state, and normalized envelope; and
- pending status and reconciliation budget.

The complete values, not only their hashes, must survive restart. This lets
Watcher commit the exact proposed transition after a reconciled acceptance or
retry the exact same operation after a proven rejection without rerunning the
detector.

### Send and reconciliation transitions

| Evidence | Required transition |
|---|---|
| No event or staging failed | No sequence/send/state change. A later attempt may execute normally. |
| Definite pre-acceptance rejection or failure | Keep the staged identity and retry the exact same operation under bounded policy. |
| Durable acceptance proven | Atomically commit proposed state, committed sequence, receipt/evidence, and attempt result. |
| Accepted send but local commit failed | Keep the pending operation; reconcile the original operation and commit when acceptance is proven again. |
| Indeterminate result | Set eligibility to `reconciliation-pending`; do not run the detector or author a replacement send. |
| Bounded reconciliation budget exhausted | Pause/block with `receipt-outcome-unresolved`. |
| Late acceptance after blocking | Commit; return an event watch to active/ready or finish a terminal watch. |
| Late rejection after blocking | Return the same staged operation to retryable active/degraded under an explicit finite retry budget. |
| Proven permanent pre-acceptance rejection | Close the allocation as `not-accepted`, retain its sequence tombstone, and permit later detector execution at the next sequence after operator correction. |
| Removal while unresolved | Stop execution; retain pending operation and unresolved tombstone for later evidence closure without resurrecting the watch. |

Automatic or operator-requested retry is legal only through a retry-safe
Application Client primitive using the exact persisted operation ID, event ID,
sequence, sender, target, payload, and retry budget. If the supported client
cannot guarantee identity-checkable same-operation retry, recovery is query-only
reconciliation and the watch remains blocked.

The contract forbids:

- assuming accepted or rejected;
- forced local commit;
- forced discard while keeping the watch live;
- blind resend;
- rerunning the detector while its prior event outcome is unresolved; and
- a timeout that globally chooses an acceptance direction.

This preserves the safe failure direction: a duplicate remains possible in the
accepted-send/local-commit uncertainty window, but consume-before-send loss is
not silently introduced.

### Recurrence and externally inspectable duplicates

After successful commit, every later detector `event` is a new occurrence and
receives a new sequence and event ID. Detector state decides whether a persistent
condition should return `event` again or become `idle`.

Watcher computes `recurrenceHash` over:

- backend/profile, sender, target, attention, and disposition; and
- detector kind, subject, body, and metadata.

The hash excludes event sequence, event ID, operation ID, attempt/runtime IDs,
receipts, and timestamps. Health and retained evidence expose:

- `lastRecurrenceHash`;
- `consecutiveIdenticalRecurrences`;
- `firstIdenticalAt`;
- `lastIdenticalAt`; and
- `lastIdenticalEventSequence`.

Watcher does not suppress identical events, apply content-based backoff, or
automatically pause at a universal threshold. Identical content does not prove
provider intent. Cadence is the generic rate bound. Operators may inspect,
pause, remove, change cadence, or correct detector state. Optional operational
tooling may add project-owned thresholds and routing, but must not implicitly
notify the event target.

Every normalized message carries stable Watcher identity, and committed evidence
binds it to Telex message/receipt identity. Duplicate behavior is therefore
externally inspectable even when the accepted-send/local-commit window produces
more than one Telex message for one Watcher event identity.

## Trust, credentials, and environment

Detector commands are arbitrary trusted same-user code. Environment clearing,
timeouts, output bounds, and process containment reduce accidents; they do not
create a sandbox or authorization boundary.

Registration is local-only. A Telex message cannot register, update, or replace
an executable.

Detector processes start from a documented minimal platform launch baseline plus
values for explicitly allowlisted inherited variable names. Registration stores
variable names, never values. Values are read at each attempt. Credentials never
appear in detector request JSON.

One runtime process may serve many watches. Environment allowlists select names
from that runtime's environment; they do not create separate secret-value
domains. Mutually untrusted watches or watches requiring different values for the
same name must run under separate supervised runtime environments/registries.

An operator may use a named credential or hardening wrapper as part of the
registered command. Watcher does not interpret that wrapper or claim provenance
for the executable bytes it selects.

Stdout/stderr and diagnostic storage are bounded. Values inherited through
token/PAT/key/secret-like allowlist names are redacted from retained stderr on a
best-effort basis. Arbitrary local code and provider error bodies can still
expose sensitive context, so exported diagnostics require review/redaction.

## Scheduling and execution

The runtime provides:

- bounded global detector concurrency;
- single-flight execution per watch;
- configured cadence within product bounds;
- deterministic per-watch jitter;
- bounded stdout/stderr draining;
- bounded execution timeout;
- process-tree termination on timeout and graceful shutdown;
- bounded exponential failure backoff; and
- one due execution after restart, never replay of every missed interval.

Provider/credential-wide rate budgets are detector/project policy. The runtime's
generic concurrency, cadence, jitter, timeout, and failure backoff remain the
common floor.

Catch-up and downtime correctness belong to detector policy. The request exposes
activation time, last successful evaluation time, current time, parameters, and
committed opaque state. A detector that requires durable replay or a maximum gap
must encode and enforce that policy itself. Watcher does not silently claim that
a window-only detector is safe after downtime.

## Process containment and restart

The detector process tree must not outlive abrupt Watcher death.
Platform-specific implementation may use different primitives, but the behavior
is fixed:

1. Detector descendants are placed in containment with kill-on-runtime-exit
   semantics.
2. Runtime startup marks prior `running` runtimes `interrupted`.
3. Unfinished detector attempts close as `runtime-interrupted` with no event
   allocation, state commit, or send.
4. Pending operations survive restart and reconcile before that watch becomes
   eligible.
5. An affected watch remains ineligible until the new runtime proves prior
   containment ended.
6. If containment proof cannot be made, the watch is paused with
   `blockedReason = orphan-containment-unproven`.

Runtime and hardening nodes own Windows/Unix mechanisms and destructive evidence.
The default Telex coordination daemon is never used for destructive proof.

## Failure and recovery

| Condition | State/send effect | Lifecycle/eligibility/health | Recovery |
|---|---|---|---|
| `idle` | commit valid `nextState`; no send | active/eligible/ready | none |
| accepted `event` | atomic event/state/attempt commit | active/eligible/ready | none |
| accepted terminal event | atomic commit | terminal/inactive/inactive | remove when desired |
| eventless terminal | direct state/attempt commit | terminal/inactive/inactive | remove when desired |
| `degraded` | no state/send | active/eligible/degraded + backoff | later successful attempt |
| nonzero exit, malformed/oversize/noncanonical result, timeout | no event allocation/state/send | active/eligible/degraded + backoff | later successful attempt or operator correction |
| missing working directory/executable | no event allocation/state/send | active/eligible/degraded + backoff | registration or local path correction |
| send failure proven before acceptance | retain exact pending operation | active/reconciliation-pending/degraded | same-operation retry |
| indeterminate receipt or accepted-send/local-commit failure | no new detector execution | active/reconciliation-pending/degraded | result/receipt reconciliation |
| reconciliation budget exhausted | no new detector execution | paused/inactive/blocked | reconcile now, finite same-operation retry budget, or remove |
| unsupported v1 registration | no execution | paused/inactive/blocked | explicit v2 re-registration/migration |
| unproven orphan containment | no execution | paused/inactive/blocked | operator proof/cleanup + resume |
| sender partial/unready | no affected watch execution/send | runtime degraded | reconcile/compensate |
| registry already owned | no sender attach, scheduling, or registry mutation | runtime startup fails | stop owning runtime or select another registry |
| actionable inbound backlog on a send-only sender | no inbound consume/ack | runtime degraded, `productionReady = false` | repair routing through owning Telex workflow |

Repeated identical recurrences remain visible through the recurrence metrics.
They do not silently alter lifecycle or health.

## Runtime identity and sender membership

Each Watcher process creates a fresh, never-reused runtime ID. One runtime
application session spans all sender addresses needed by the registry. Stable
sender responsibilities survive process replacement; runtime identity does not.

Before a watch can send, the runtime:

- attaches the sender with required PID/start-time liveness;
- verifies session, address, predicate, owner, and readiness;
- reconciles senders at startup, registry revision, periodically, and after
  typed membership loss;
- remains non-ready on partial attachment;
- uses caller-bounded reconcile-and-send only when it preserves the exact staged
  operation identity;
- never force-takes an address; and
- detaches every known sender on graceful shutdown.

Collision, partial result, compensation, retry, and detach outcomes are
observable through health.

### Send-only application stations

Watcher sender addresses are dedicated send-only responsibilities. They must not
be advertised as reply-capable targets.

The shared Application Client must represent send-only membership so it does not
count as inbound application attendance. A send addressed to a send-only Watcher
sender receives the address policy's unoccupied or rejected result, not a false
application-delivered result.

Watcher never drains, acknowledges, drops, or dead-letters inbound traffic. If
an interim integration exposes inbound actionable backlog, health reports an
operational error and production readiness is false.

## Normalized Telex event v2

The emitted Telex message uses:

- `from`: registration sender;
- `to`: registration target;
- `kind`: bounded detector kind;
- `attention`: registration policy;
- `requiresDisposition`: registration policy;
- `subject` and `body`: bounded detector values; and
- `metadata`: normalized Watcher metadata.

The v2 metadata projection is:

```json
{
  "schemaVersion": 2,
  "watcher": {
    "watchId": "github-pr-133",
    "eventSequence": 7,
    "eventId": "watcher-event-uuid",
    "operationId": "application-operation-uuid",
    "attemptId": "attempt-uuid",
    "runtimeId": "runtime-uuid",
    "logicalStoreId": "store:opaque-stable-id",
    "registrationRevision": 3,
    "detectorSchemaVersion": 2
  },
  "detector": {
    "reviewer": "example",
    "reviewState": "CHANGES_REQUESTED"
  }
}
```

The top-level keys `schemaVersion`, `watcher`, and `detector` are reserved and
constructed by Watcher. Arbitrary detector metadata is nested under `detector`,
so it cannot collide with Watcher provenance.

Recipients can inspect watch identity, event ordering, retry identity, attempt,
runtime, store, registration, and detector protocol version. V2 does not
guarantee which script bytes ran. Projects that need script provenance record it
in their wrapper, detector metadata, or separate audit system.

Prior/next state hashes, normalized-envelope hash, recurrence hash, registration
policy snapshot, and typed receipt remain in the local evidence ledger. Detector
state is never exposed in the message.

## Durable acceptance and receipts

The Application Client must return typed results that distinguish:

- durable message acceptance;
- rejection before acceptance;
- partial or indeterminate outcome;
- previously completed or duplicate operation;
- target occupancy at acceptance time;
- push attempt/acceptance;
- recipient transport consumption/acknowledgment; and
- workflow disposition.

Watcher commits event state on durable acceptance only. Occupancy and push are
diagnostic facts. Recipient consumption and disposition happen later and are not
part of the detector transaction.

The spike's specific `delivered` and `queued-unoccupied` strings are evidence,
not the shared client API.

## Health and diagnostics

The management surface must expose JSON projections for runtime and watch status.
The existing
[watcher-health-v1.schema.json](schemas/watcher-health-v1.schema.json) is frozen
for experimental v1 and is not the v2 health contract.

V2 runtime status includes:

- runtime ID, PID, start/heartbeat times, status, and `productionReady`;
- registry revision and restart reconciliation state;
- sender address/store identity, readiness, lease epoch, membership loss,
  pending unconsumed count, and inbound actionable count;
- retention rows/bytes and warning thresholds; and
- typed runtime diagnostic categories.

V2 per-watch status includes:

- watch ID, registration revision, detector schema version, activation time;
- lifecycle, eligibility, health, and blocked reason;
- last allocated and committed event sequence;
- pending operation identity, state, reconciliation attempts/deadline, and last
  evidence time when present;
- last attempt, success, event, and next-attempt times;
- consecutive failures and diagnostic category;
- recurrence hash/count/timestamps/last sequence;
- sender readiness; and
- retained rows/bytes and warning state.

Per-watch `diagnosticCategory` includes:

- `detector-degraded`;
- `process-failure`;
- `malformed-output`;
- `canonicalization-failed`;
- `output-limit`;
- `timeout`;
- `send-failure`;
- `unknown-receipt`;
- `receipt-reconciliation-pending`; and
- `runtime-interrupted`.

An active watch with `eligibility = reconciliation-pending` is not scheduler
eligible even though lifecycle remains active. Exhausted reconciliation changes
it to paused/blocked. Recurrence counters are observable facts and do not by
themselves change health.

Runtime heartbeat updates independently of detector execution. A local service
supervisor or operator CLI is the first consumer. Automatic remote health
notification is deferred and must later be thresholded, coalesced, and
explicitly routed; the event target is never implicitly spammed.

Runtime implementation must add machine-readable v2 metadata/health conformance
before promotion. This node freezes only the three issue-named v2 schemas;
metadata and health projection schemas belong to runtime implementation once
its concrete model exists.

## Retention and provenance

For each permanent watch ID, retain:

- the removed-ID tombstone;
- last allocated and committed sequence high-water marks;
- committed event identity/receipt spine;
- unresolved pending operation identity and payload;
- registration revision/policy snapshot for each staged or committed event; and
- enough recurrence evidence to support the health projection.

Historical event identity and receipt bindings are never rewritten in place.
Future compaction may remove bulky attempt and diagnostic payloads only through
a versioned contract that preserves non-reuse, sequence high-water, unresolved
operations, receipt binding, and recipient dedupe evidence.

The runtime health surface exposes retained rows/bytes and positive warning
thresholds. Backup, capacity defaults, and safe compaction remain operational
hardening work.

## Experimental v1 compatibility

The following schemas remain frozen historical artifacts:

- [watcher-detector-request-v1.schema.json](schemas/watcher-detector-request-v1.schema.json);
- [watcher-detector-result-v1.schema.json](schemas/watcher-detector-result-v1.schema.json);
- [watcher-event-metadata-v1.schema.json](schemas/watcher-event-metadata-v1.schema.json);
- [watcher-health-v1.schema.json](schemas/watcher-health-v1.schema.json).

A v2 runtime does not silently execute persisted v1 registrations or accept v1
detector results. Mixed registries remain usable: unsupported v1 registrations
are visibly paused/blocked with `unsupported-registration-version`, while valid
v2 watches continue.

Baseline migration is explicit:

1. Choose a new v2 watch ID.
2. Update the detector or place a compatibility wrapper around it.
3. Review and choose opaque state: import a reviewed v1 state value, supply a
   new baseline, or intentionally start from `null`.
4. Register under v2 and preserve the old watch tombstone.

No automatic event-ID, cursor, or script-provenance translation is implied. An
optional importer or v1 detector adapter may be built later. It must discard v1
detector-authored event identity in favor of runtime-owned v2 identity and must
not constrain the v2 core surface.

## Application Client requirements

Watcher consumes the shared API-neutral Application Client contract. It does not
select a package, language binding, or daemon wire representation.

The client must support:

1. stable sender responsibility with fresh runtime identity;
2. process-bound liveness;
3. atomic or explicitly partial multi-address lifecycle;
4. caller-selected strict or bounded membership recovery;
5. typed membership loss and collision;
6. explicit sender selection;
7. send-only station capability;
8. typed separation of durable acceptance, occupancy, push, consumption, and
   disposition;
9. stable opaque logical-store identity on status and receipts;
10. restart-stable application operation identity;
11. accepted, rejected, partial, indeterminate, previously-completed, and
    duplicate operation outcomes;
12. operation-result and receipt lookup after restart;
13. identity-checkable retry of the exact same operation, preserving sender,
    target, payload, event identity, operation identity, and retry budget; and
14. health, reconciliation, compensation, and cleanup primitives without CLI
    parsing or raw daemon IPC.

If the supported client cannot provide same-operation retry at that strength,
Watcher uses query-only reconciliation and remains blocked. It never falls back
to CLI subprocess parsing, raw daemon IPC, spike-private helpers, or a
Watcher-private client.

The non-normative Application Client requirements crosswalk may need a later
owner update for the strengthened reconciliation requirement. ADR 0050 records
that follow-up; this node does not edit the crosswalk.

## Schema and documentation conformance

The canonical v2 schema set produced by this contract is:

- [watcher-registration-v2.schema.json](schemas/watcher-registration-v2.schema.json);
- [watcher-detector-request-v2.schema.json](schemas/watcher-detector-request-v2.schema.json);
- [watcher-detector-result-v2.schema.json](schemas/watcher-detector-result-v2.schema.json).

Contract validation must:

- validate all three against JSON Schema draft 2020-12;
- accept the normative examples in this document;
- reject detector-authored event identity, route, attention, disposition,
  cadence, timeout, and action fields;
- reject invalid outcome/event/state combinations;
- verify v1 schemas remain unchanged and are linked only as v1 compatibility
  artifacts; and
- verify design links and ADR anchors resolve.

Runtime implementation must add conformance between its concrete v2 registration,
request, result, normalized metadata, and health models and the corresponding
normative contract. Provider-specific fixtures and conformance suites remain
project-owned, not prerequisites for Watcher registration.

## Downstream implementation checklist

`watcher-runtime-core` is ready to detail only when it can answer:

- Where are registrations, lifecycle, eligibility, opaque state, attempts,
  pending operations, sequence high-water marks, committed receipt evidence,
  recurrence diagnostics, runtime records, and health persisted?
- Which canonical registry identity and PID/start-time-safe lock enforce one
  mutation owner?
- How are complete staged payloads and hashes committed before send and recovered
  after restart?
- Which Application Client primitives prove accepted/rejected/indeterminate/
  previously-completed outcomes and same-operation retry?
- How are late acceptance, late rejection, permanent rejection, removal, and
  unresolved tombstones represented?
- How are backend/profile selection and opaque logical-store identity threaded
  through registration, membership, receipts, evidence, and health?
- How are v2 metadata and health projections made machine-readable and checked
  alongside the three canonical schemas?
- Which platform mechanisms prove detector-tree death after abrupt runtime exit?
- How does service supervision consume health and detect staleness?
- How are unsupported v1 rows isolated without blocking valid v2 watches?
- How are retention warnings configured while preserving the identity/receipt
  spine?
- How is the accepted shared client consumed without a private fallback?

`minimal-example-pack` remains optional teaching material. Examples may document
provider cursor/replay, optional hardening, credentials, and tests, but agents
may ignore them and author a detector directly.

## Open-question dispositions

| Question | Disposition |
|---|---|
| Minimal registration | Command, cadence, timeout, backend, sender, and target are required; other generic fields have explicit defaults. |
| Event identity | Runtime-owned permanent watch namespace plus monotonic sequence and stable event ID. |
| Unknown receipt | Reconcile the exact staged operation; never guess acceptance. Exhaustion blocks visibly. |
| Recurrence | Every post-commit detector event is a new occurrence. Watcher observes identical recurrence but does not infer intent or suppress it. |
| Script provenance | Not a v2 core claim. Projects may add wrappers, hashes, or metadata. |
| Kind policy | Detector-owned opaque vocabulary subject only to generic string and byte bounds. |
| Downtime/cursor policy | Detector-owned through request timing facts and opaque state. |
| Templates/manifests/preflight/tests | Optional project/tooling choices, absent from v2 registration/runtime semantics. |
| V1 migration | Explicit new watch ID, detector update/wrapper, and reviewed state decision. Optional adapter deferred. |
| Shared Application Client | Required; no private fallback. |

## Test isolation

All daemon lifecycle, strict-send, crash, upgrade, handoff, branch-binary, and
real wake proofs use an isolated test plane:

- unique absolute `TELEX_HOME`;
- dedicated `TELEX_DB`;
- unique `TELEX_INSTALL_ROOT`; and
- absolute worktree branch binary.

The default local daemon and installed launcher are campaign coordination
infrastructure and are never destructive test targets.
