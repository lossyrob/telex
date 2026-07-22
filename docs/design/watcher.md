# Telex Watcher — Production Contract

## Status

**Normative design specification.** This document defines the production
contract for Telex Watcher, a separately supervised application that runs
trusted local detector commands outside agent sessions and emits normalized
Telex messages.

The implemented vertical spike and its evidence remain documented in
[Generic Watcher Spike Report](../generic-watcher-spike-report.md). This
contract promotes the successful semantics and explicitly excludes the spike's
private integration seams.

The load-bearing boundary is recorded in
[ADR 0046](DECISIONS.md#0046--watcher-runs-provider-neutral-trusted-local-detectors-with-receipt-gated-state).

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
        | normalized event, fixed registration policy
        v
durable Telex send
```

Watcher:

- runs as a persistent per-user process outside agent sessions;
- executes trusted local observational commands on a bounded schedule;
- persists registration, opaque state, attempts, event evidence, and health;
- owns sender, target, attention, disposition, cadence, timeout, environment,
  and script policy;
- performs one reaction only: send a Telex message; and
- uses the shared Telex Application Client contract owned through issue #12.

Watcher does not:

- interpret GitHub, Azure DevOps, HTTP, file, or provider semantics;
- execute a configurable post-detection action;
- merge, approve, mutate, launch, or orchestrate downstream work;
- accept remote registration or replacement of executable code;
- claim to sandbox same-user detector commands;
- provide hosted webhook ingestion or multi-host failover; or
- define a Watcher-specific public Telex client API.

Provider behavior belongs in editable detector scripts and templates.
Consequential action belongs to the recipient woken by Telex.

## Terms

| Term | Meaning |
|---|---|
| **Runtime** | One Watcher process incarnation. Each process has a fresh, never-reused runtime ID. |
| **Watch** | A durable local registration containing detector and Watcher policy. |
| **Detector** | A trusted local command that observes a source and returns one structured result. |
| **Attempt** | One bounded detector execution against one committed prior state. |
| **Detector event** | The detector-proposed ID, kind, subject, body, and metadata. |
| **Normalized event** | The fixed-route Telex message plus Watcher provenance. |
| **Sent-event ledger** | Durable evidence keyed by `(watchId, eventId)` that binds a committed transition to its message receipt. |
| **Lifecycle** | The durable watch state: `active`, `paused`, `terminal`, or `removed`. |
| **Health** | The current operational condition of a runtime or watch, separate from lifecycle. |

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

## Watch registration

A production registration contains the following policy.

| Field | Contract |
|---|---|
| `id` | Stable watch ID. Removed IDs are not reusable. |
| `detectorSchemaVersion` | `1` for the initial production runtime. |
| `command` | Non-empty local argv. No shell interpolation is implied. |
| `scriptPath` | Canonical absolute path whose bytes supply script provenance. |
| `workingDirectory` | Canonical absolute local directory. |
| `scriptMode` | `pinned` or `follow-path`. |
| `scriptDigest` | Required bare lowercase SHA-256 hex for `pinned`; absent for `follow-path`. |
| `backendProfile` | Explicit local Telex backend/profile selection; credentials are not copied into the registration. |
| `sender` | Stable Telex sender responsibility. Immutable for a watch ID. |
| `target` | Fixed Telex target. Immutable for a watch ID. |
| `attention` | Fixed Telex attention policy. |
| `requiresDisposition` | Fixed recipient-disposition policy. |
| `intervalSeconds` | Bounded cadence subject to runtime minimums and jitter. |
| `timeoutSeconds` | Bounded detector execution timeout. |
| `allowedEventKinds` | Optional exact namespaced kinds accepted from the detector. |
| `allowedEventKindPrefixes` | Optional namespaced prefixes accepted from the detector. |
| `environmentAllowlist` | Inherited environment variable names, never values. |
| `parameters` | Opaque JSON supplied to the detector. |
| `initialState` | Opaque JSON committed before the first attempt. |
| `maxSafeDowntimeSeconds` | Maximum gap the source can safely recover from, or `null` for durable replay. |

At least one exact kind or prefix must authorize every event-producing result.
Watcher validates the policy; detector output cannot introduce a new kind.

Every successful update increments a registration revision. Updates cannot
change `id`, `sender`, or `target`. A kind-policy change automatically pauses an
active watch. Explicit resume confirms the new policy. This prevents an active
watch from changing its downstream event vocabulary without an operator
checkpoint.

Watcher persists `activatedAt` when a registration first becomes active and
replaces it on every explicit resume. `registrationRevision` is per-watch;
`registryRevision` is a separate registry-wide change counter. They are
independent diagnostics and must not be numerically compared.

Registration validates paths, argv, timing bounds, addresses, variable names,
JSON sizes, script mode/digest, and kind policy before persistence.

## Lifecycle and health

Lifecycle is authoritative and intentionally small:

| Lifecycle | Meaning | Legal transitions |
|---|---|---|
| `active` | Eligible to schedule when runtime and sender are ready. | `paused`, `terminal`, `removed` |
| `paused` | Not scheduled. May be operator-selected or health-blocked. | `active` through explicit resume, `removed` |
| `terminal` | Detector completed the watch. No further scheduling. | `removed` |
| `removed` | Administratively removed but provenance retained. | none |

`blocked` is a health state, not a fifth lifecycle. A non-transient failure moves
the watch to `paused` with `health = blocked` and a typed reason. Recovery
requires a registration/script correction followed by explicit resume, or
removal.

Legal lifecycle/health combinations are:

| Lifecycle | Health | Meaning |
|---|---|---|
| `active` | `ready` | Eligible and operating normally. |
| `active` | `degraded` | Eligible, but the latest bounded attempt or sender condition failed. |
| `paused` | `inactive` | Explicit operator pause. |
| `paused` | `blocked` | Automatic non-transient safety/configuration pause with typed `blockedReason`. |
| `terminal` | `inactive` | Detector completed; no further scheduling. |
| `removed` | `inactive` | Administratively removed; provenance retained. |

Every other combination is invalid. `blockedReason` is non-null only for
`paused`/`blocked`.

A single-event watch is represented by an event-producing `terminal` result.
Watcher does not need a second single-shot lifecycle engine.

Address occupancy never controls lifecycle. A target or sender becoming
unoccupied does not silently cancel or expire a watch. Address-bound expiration
is deferred.

## Detector protocol v1

Watcher invokes a detector with a single JSON request on stdin. The normative
machine-readable shape is
[watcher-detector-request-v1.schema.json](schemas/watcher-detector-request-v1.schema.json).

```json
{
  "schemaVersion": 1,
  "attempt": {
    "id": "attempt-uuid",
    "now": "2026-07-22T05:00:00Z"
  },
  "watch": {
    "id": "github-pr-110",
    "parameters": {
      "repo": "lossyrob/telex",
      "pullRequest": 110
    }
  },
  "script": {
    "mode": "pinned",
    "sha256": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
  },
  "state": {
    "lastReviewId": 8372
  }
}
```

The bare `script.sha256` field preserves the implemented v1 wire contract.
Algorithm qualification is added only to Watcher-owned audit and message
records.

The detector exits zero and writes exactly one JSON result to stdout. The
normative shape is
[watcher-detector-result-v1.schema.json](schemas/watcher-detector-result-v1.schema.json).

```json
{
  "schemaVersion": 1,
  "outcome": "event",
  "nextState": {
    "lastReviewId": 8421
  },
  "event": {
    "id": "github:review:8421",
    "kind": "github.pull-request.review",
    "subject": "External review received on PR #110",
    "body": "A reviewer requested changes.",
    "metadata": {
      "reviewer": "example",
      "reviewState": "CHANGES_REQUESTED"
    }
  }
}
```

The outer request/result objects are strict. Unknown fields are rejected.
Detector `parameters`, `state`, `nextState`, and `event.metadata` remain
arbitrary JSON.

All opaque values must be valid I-JSON and RFC 8785 canonicalizable. The parser
rejects duplicate object member names, invalid Unicode, non-finite numbers, and
numbers outside the canonicalizer's supported interoperable range. Registration
rejects noncanonicalizable `parameters`/`initialState` before persistence.
Detector output that cannot be canonicalized is
`diagnosticCategory = canonicalization-failed`: no send or state advancement,
and normal bounded failure backoff applies.

Any field addition, removal, shape change, or semantic change requires a new
`schemaVersion`. V1 is not extended additively. The initial runtime accepts v1
only and rejects every other version without state advancement. Concurrent
version selection and migration require a later contract revision; v1 reserves
no negotiation fields.

### Outcomes

| Outcome | Event | State behavior | Lifecycle behavior |
|---|---|---|---|
| `idle` | forbidden | Valid `nextState` commits immediately | remains active |
| `event` | required | Commits only after durable Telex acceptance | remains active |
| `terminal` | optional | Event-producing state is receipt-gated; eventless state commits directly | becomes terminal after commit |
| `degraded` | forbidden | Must not contain or advance `nextState` | remains active with failure/backoff |

An `idle` state advance asserts that the detector successfully evaluated the
source and intentionally classified all observations through the new cursor as
non-actionable. This includes ignored observations. A detector must not advance
past work it did not evaluate.

A bare `terminal` result with neither `event` nor `nextState` is valid. It leaves
the prior state unchanged, records the attempt, and transitions the watch to
terminal.

Process exit status is separate from detector outcome. Nonzero exit is an
execution failure, not `degraded`.

### Protocol limits

| Value | Limit |
|---|---:|
| Detector stdout | 256 KiB |
| Detector stderr | 64 KiB |
| Opaque state / `nextState` | 256 KiB serialized |
| Event ID | 512 UTF-8 bytes |
| Event kind | 256 UTF-8 bytes |
| Event subject | 512 UTF-8 bytes |
| Event body | 128 KiB |
| Detector metadata | 64 KiB serialized |
| Complete normalized Watcher metadata | 80 KiB serialized |

JSON Schema string lengths do not substitute for these UTF-8 byte limits. The
runtime enforces the byte caps before send or commit. Oversize/truncated output
is a failure and is never sent as success.

## Event kind authority

Detector kinds are provider/application vocabulary, but Watcher attests that the
kind was allowed by registration policy.

An event kind must:

- use lowercase ASCII letters, digits, and hyphens in two or more dot-separated
  segments;
- match `allowedEventKinds` exactly or one configured
  `allowedEventKindPrefixes` entry; and
- remain within the protocol byte cap.

A kind has at least two dot-separated segments; additional depth remains
application vocabulary. An allowed prefix is normalized lowercase text ending
in `.` and matches the event kind by exact UTF-8 byte prefix. This makes
`github.pull-request.` authorize that namespace while preventing a short partial
token from authorizing an unrelated segment.

A mismatch is a non-transient policy failure. Watcher records the attempt,
pauses the watch with `health = blocked` and
`blockedReason = event-kind-not-allowed`, and requires update/resume.

Telex core continues to carry the kind without interpreting it.

## State, send, and deduplication transaction

For an event-producing result, the required order is:

```text
read committed prior state
-> execute detector
-> validate result, script provenance, and registration policy
-> normalize fixed-route Telex event
-> send through the Application Client
-> receive typed durable-acceptance receipt
-> atomically commit next state + sent-event evidence + attempt result
```

Watcher commits event-producing state only when Telex durably accepts the
message. Current occupancy, a push attempt, recipient consumption, and
disposition are separate facts and do not control this transaction.

If send fails, returns an unknown/malformed result, or cannot prove durable
acceptance, prior state remains current. The detector may report the same stable
event again.

If Telex accepted the message but Watcher crashes before local commit, a
duplicate is possible. This is the deliberate at-least-once failure direction:
prefer a visible duplicate over silent consume-before-send loss.

### Sent-event evidence

The durable key is `(watchId, eventId)`. An event ID must be stable and unique
within one watch ID for that watch ID's entire retained lifetime.

Committed evidence binds:

- watch and event ID;
- attempt ID and registration revision;
- prior-state and committed-next-state hashes;
- normalized-envelope hash;
- algorithm-qualified executed script digest;
- sender and target;
- opaque logical-store identity;
- Telex message ID and typed receipt; and
- commit timestamp.

State and normalized-envelope hashes use `sha256:<hex>` over RFC 8785 JSON
Canonicalization Scheme UTF-8 bytes. Historical hashes are never rewritten in
place. A future hash algorithm requires a new ledger/schema version.

If an event ID already has matching committed envelope evidence, the attempt is
a visible `stale-duplicate` no-op. If the evidence conflicts, the attempt is
`duplicate-event-conflict`; Watcher pauses the watch with
`health = blocked` and `blockedReason = event-id-conflict`.

Neither duplicate branch sends, advances detector state, marks terminal, or
overwrites committed evidence. Collision recovery requires a detector or
registration revision followed by explicit resume.

## Script provenance

### Pinned

`pinned` is the production default:

- registration requires a lowercase bare SHA-256 digest;
- Watcher reads and hashes the selected bytes immediately before execution;
- mismatch pauses the watch with `blockedReason = pinned-digest-mismatch`;
- no detector process starts on mismatch; and
- recovery requires explicit update/repin and resume.

### Follow path

`follow-path` is an explicit development mode:

- hash immediately before execution;
- execute those selected bytes;
- hash again before accepting output;
- a changed digest is `script-drift`, with no send or state commit; and
- repeated drift contributes to degraded health/backoff.

Atomic file replacement is recommended for detector edits.

Every attempt and emitted event records `sha256:<hex>` for the executed bytes.
SHA-256 is frozen for v1. Changing the algorithm requires a new audit/schema
version and explicit repinning; old evidence remains qualified and comparable.

## Trust, credentials, and environment

Detector commands are arbitrary trusted same-user code. Environment clearing,
timeouts, and process containment reduce accidents; they do not create a
sandbox or authorization boundary.

Registration is local-only. A Telex message cannot register, update, or replace
an executable.

Detector processes start from a cleared environment plus:

- a documented minimal platform launch baseline needed to locate the command and
  user/system/temp/locale directories; and
- values for explicitly allowlisted inherited variable names.

Registration stores variable names, never values. Values are read at each
attempt. Credentials never appear in detector request JSON.

One runtime process serves many watches, so allowlists select names from that
runtime's environment; they do not create separate secret-value domains.
Mutually untrusted watches or watches requiring different values for the same
name must run under separate supervised runtime environments/registries.

An operator may use a named credential wrapper as part of the registered command.
Watcher does not interpret the wrapper or provider.

Stdout/stderr and diagnostic storage are bounded. Values inherited through
token/PAT/key/secret-like allowlist names are redacted from retained stderr on a
best-effort basis. Arbitrary local code and provider error bodies can still
expose sensitive context, so public reports and exported diagnostics require
review/redaction.

## Scheduling and execution

The runtime provides:

- bounded global detector concurrency;
- single-flight execution per watch;
- configurable cadence within product bounds;
- deterministic per-watch jitter to avoid synchronized provider bursts;
- bounded stdout/stderr draining;
- bounded execution timeout;
- process-tree termination on timeout and graceful shutdown;
- bounded exponential failure backoff; and
- one due execution after restart, never replay of every missed interval.

Provider/credential-wide rate budgets are not runtime provider policy. They are
owned by detector templates and operational hardening. The runtime's generic
concurrency, cadence, jitter, and backoff remain the common floor.

### Catch-up and downtime gaps

Production detectors must be cursor-clean: one execution can query from committed
opaque state and classify every observation since that cursor.

Each registration has `maxSafeDowntimeSeconds`:

- `null` means the source provides durable replay from committed state;
- a positive value declares the longest safe recovery gap; and
- exceeding it pauses the watch with `blockedReason = downtime-gap`.

The elapsed-time reference is the latest successful evaluation, or persisted
`activatedAt` for a watch that has never succeeded. A watch registered while the
runtime is offline uses its registration activation time; an explicit resume
replaces that timestamp. Runtime
startup completes interrupted-attempt and containment reconciliation, then
evaluates the downtime limit **before** placing any overdue watch on the
scheduler. An unsafe gap therefore cannot run one catch-up attempt first.

The health record retains last successful evaluation, detected downtime, and the
declared limit. Watcher does not silently resume as healthy after an unsafe gap.
Operator correction and explicit resume are required.

Window-only detectors that cannot query missed observations must declare the
gap and are not eligible as production templates without provider replay.

## Process containment and restart

The detector process tree must not outlive abrupt Watcher death.
Platform-specific implementation may use different primitives, but the behavior
is fixed:

1. Detector descendants are placed in containment with kill-on-runtime-exit
   semantics.
2. Runtime startup marks prior `running` runtimes `interrupted`.
3. Unfinished attempts close as `runtime-interrupted` with no state, receipt, or
   ledger commit.
4. An affected watch remains ineligible until the new runtime proves prior
   containment ended.
5. If the proof cannot be made, the watch is paused with
   `blockedReason = orphan-containment-unproven`.

Runtime and hardening nodes own Windows/Unix mechanisms and destructive evidence.
The default Telex coordination daemon is never used for destructive proof.
There is no automatic or unaudited safety override: operator recovery requires
external cleanup/proof followed by explicit resume.

## Failure and recovery

| Condition | State/send effect | Lifecycle/health | Recovery |
|---|---|---|---|
| `idle` | commit valid `nextState`; no send | active/ready | none |
| accepted `event` | atomic event/state/attempt commit | active/ready | none |
| accepted terminal event | atomic commit | terminal/inactive | remove when desired |
| eventless terminal | direct state/attempt commit | terminal/inactive | remove when desired |
| `degraded` | no state/send | active/degraded + backoff | later successful attempt |
| nonzero exit, malformed/oversize/noncanonical result, timeout | no state/send | active/degraded + backoff | later successful attempt or operator correction |
| send failure or unknown receipt | no state/event commit | active/degraded + backoff | reconcile sender and retry later |
| pinned mismatch | detector not started | paused/blocked | repin/update + resume |
| follow-path drift | no state/send | active/degraded + backoff | stable file; repeated drift may lead operator to pause |
| event kind mismatch | no state/send | paused/blocked | policy/script update + resume |
| event ID collision | no state/send/terminal transition | paused/blocked | script/registration revision + resume |
| sender partial/unready | no affected watch execution/send | active/degraded runtime | reconcile/compensate |
| unsafe downtime gap | no execution | paused/blocked | source reconciliation/update + resume |
| unproven orphan containment | no execution | paused/blocked | operator proof/cleanup + resume |
| registry already owned | no sender attach, scheduling, or registry mutation | runtime startup fails | stop the owning runtime or select another registry |

Repeated degradation must be visible through the health surface before the
production runtime passes its usability gate. Automatic Telex
degradation/recovery notifications are deferred to operational hardening. A
future notification policy must be thresholded, coalesced, and explicitly routed
to an operator-health target; the event target is never implicitly spammed.

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
- uses a caller-bounded reconcile-and-send retry budget after typed membership
  loss; Watcher configures that budget to one retry in v1;
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

Watcher never drains, acknowledges, drops, or dead-letters inbound traffic. If an
interim integration exposes inbound actionable backlog, health reports an
operational error and the production gate fails. Production does not approximate
this semantic with the spike's sender occupancy.

## Normalized Telex event

The emitted Telex message uses:

- `from`: registration sender;
- `to`: registration target;
- `kind`: detector kind after registration-policy validation;
- `attention`: registration policy;
- `requiresDisposition`: registration policy;
- `subject` and `body`: bounded detector values; and
- `metadata`: the normalized Watcher metadata below.

The normative metadata shape is
[watcher-event-metadata-v1.schema.json](schemas/watcher-event-metadata-v1.schema.json).

```json
{
  "schemaVersion": 1,
  "watcher": {
    "watchId": "github-pr-110",
    "eventId": "github:review:8421",
    "attemptId": "attempt-uuid",
    "runtimeId": "runtime-uuid",
    "logicalStoreId": "store:opaque-stable-id",
    "registrationRevision": 3,
    "detectorSchemaVersion": 1,
    "script": {
      "mode": "pinned",
      "digest": "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
    }
  },
  "detector": {
    "reviewer": "example",
    "reviewState": "CHANGES_REQUESTED"
  }
}
```

The top-level keys `schemaVersion`, `watcher`, and `detector` are reserved and
constructed by Watcher. Arbitrary detector metadata is nested as the value of
`detector`, so it cannot collide with Watcher provenance.

Detector metadata is capped at 64 KiB serialized. Complete normalized metadata,
including Watcher overhead, is capped at 80 KiB serialized.

Prior/next state hashes and normalized-envelope hash remain in the local audit
ledger. The message carries the identifiers and script provenance recipients need
for deduplication and source inspection without exposing detector state.

Watcher preserves the v1 detector body and Unicode subject semantics after the
existing control-character validation; it does not perform display-oriented
rewriting. Telex recipients/renderers own safe escaping of bidi/control display
effects, link handling, and the no-execution boundary.

## Durable acceptance and receipts

The Application Client must return a typed receipt that distinguishes:

- durable message acceptance;
- target occupancy at acceptance time;
- push attempt/acceptance;
- recipient transport consumption/acknowledgment; and
- workflow disposition.

Watcher commits event state on durable acceptance only. Occupancy and push are
diagnostic facts. Recipient consumption and disposition happen later and are not
part of the detector transaction.

The spike's specific `delivered` and `queued-unoccupied` strings are evidence,
not the shared client API.

## Health surface

The initial management surface is:

```text
telex-watcher status --json
telex-watcher show <watch-id> --json
```

Both return projections conforming to
[watcher-health-v1.schema.json](schemas/watcher-health-v1.schema.json).

The health document includes:

- `schemaVersion`, `observedAt`, and declared `staleAfterSeconds`;
- runtime ID, PID, start/heartbeat times, status, aggregate sender readiness,
  per-sender diagnostics, runtime diagnostic categories, and registry revision;
- restart reconciliation status and interrupted-runtime/unfinished-attempt/
  containment-pending counts;
- per-watch logical-store identity, registration revision, activation time,
  lifecycle, and health;
- consecutive failures;
- last attempt, success, and event times;
- next attempt;
- blocked reason and diagnostic category;
- sender readiness;
- `maxSafeDowntimeSeconds`; and
- retained rows/bytes, warning thresholds, and warning state.

Runtime heartbeat updates independently of detector execution. A local service
supervisor or operator CLI is the first consumer. A stale heartbeat, non-ready
sender set, blocked watch, or repeated degradation must be visible without
reading raw registry tables or parsing logs.

Runtime status is `starting`, `reconciling`, `ready`, `degraded`, or `stopping`.
The reconciliation object is `not-required`, `running`, `complete`, or
`blocked`. Sender entries identify address, logical store, status, typed
membership-loss detail, lease epoch, reconciliation time, pending unconsumed
count, and inbound actionable count. `senderReady` remains the aggregate
all-required-senders-ready predicate.

Legal runtime/reconciliation combinations are:

| Runtime status | Reconciliation status | Meaning |
|---|---|---|
| `starting` | `not-required` | Runtime setup has not discovered prior interrupted work. |
| `reconciling` | `running` | Prior runtime/attempt/containment recovery is in progress. |
| `ready` | `complete` or `not-required` | Reconciliation no longer blocks scheduling; only `ready` is readiness. |
| `degraded` | `complete` or `blocked` | Runtime can report status but one or more operational conditions block full readiness. |
| `stopping` | any current value | Shutdown is authoritative; no new scheduling or reattachment begins. |

`reconciliation = blocked` means containment or interrupted-state recovery
requires operator action and therefore pairs with `runtime.status = degraded`.
A sender is `ready` after verified attachment, `degraded` during bounded
transient loss/reconciliation, and `blocked` after collision or uncompensated
partial attachment requiring operator action.

Sender `membershipLossReason` is `daemon-restart`, `predicate-death`,
`collision`, `deliberate-detach`, `needs-attach`, `owner-demoted`, or `null`.
Sender `diagnosticCategory` separately classifies `membership-loss`,
`collision`, `partial-attachment`, `inbound-backlog`, or `null`.

`blockedReason` is a closed v1 vocabulary:

- `event-kind-not-allowed`;
- `event-id-conflict`;
- `pinned-digest-mismatch`;
- `downtime-gap`; and
- `orphan-containment-unproven`.

Per-watch `diagnosticCategory`, per-sender `diagnosticCategory`, and runtime
`diagnosticCategories` are separate versioned scopes from the health schema.
Runtime/sender conditions are not fanned out to every watch that shares an
address. New categories require a health-schema revision rather than ad hoc
free-form strings.

Automatic remote health notification is deferred; supervisor-visible health is
not.

## Retention and provenance

Current watch state and the sent-event ledger are retained for the lifetime of a
watch ID. Removed watch IDs are not reusable.

Until operational hardening defines safe compaction and backup:

- provenance retention is intentionally unbounded;
- no destructive event-ledger GC is allowed;
- health exposes retained rows/bytes and configurable warning thresholds; and
- threshold state is visible to operators.

Runtime retention counts all persisted registry rows and estimated bytes.
Per-watch retention counts that watch's registration/state, attempts,
diagnostics, and event evidence. `warning` is true when
`rows >= warningRows` **or** `bytes >= warningBytes`; thresholds are positive.
Both row and byte thresholds are mandatory in v1 and cannot be disabled
independently.

Attempts and diagnostic-payload retention, backup, and safe compaction are owned
by operational hardening. That node must define a capacity model and default
numeric thresholds before closure.

## Application Client requirements

Watcher exports semantic requirements to issue #12. It does not select an API,
package, language binding, or daemon wire representation.

The shared Application Client must support:

1. **Stable responsibility, ephemeral runtime.** Stable sender addresses survive
   a fresh never-reused process/session identity.
2. **Process-bound liveness.** Membership can carry typed PID/start-time
   predicates.
3. **Multi-address lifecycle.** Attach/reconcile/detach is atomic or returns
   explicit partial results and compensation handles.
4. **Caller-selected recovery.** A caller can choose strict `NeedsAttach` or
   bounded automatic repair without losing liveness predicates.
5. **Typed membership loss.** Daemon restart, predicate death, collision, and
   deliberate detach remain distinguishable.
6. **Bounded reconcile-and-send.** One semantic operation can repair typed
   membership loss under a caller-selected bounded retry budget without parsing
   CLI stderr or exposing raw IPC. Watcher selects one retry in v1.
7. **Explicit sender selection.** Multi-address applications do not rely on
   ambiguous default-from inference.
8. **Observable collision.** Current owner/epoch and retry/reset guidance are
   visible; there is no hidden force takeover.
9. **Typed receipt separation.** Durable acceptance, occupancy, push,
   consumption, and disposition are distinct.
10. **Station capability.** Send-only membership does not advertise inbound
    attendance; bidirectional membership is explicit.
11. **Inbound application semantics.** A receiver has cursor/receive,
    acknowledgment, disposition, and reply semantics.
12. **Lifecycle status.** Runtime session, predicates, lease epoch, sender
    readiness, reconciliation, and detach outcomes are inspectable.
13. **Daemon restart recovery.** Applications receive typed loss or can
    explicitly reattach without a resident waiter.
14. **Dedup guidance.** Message/event identity and the accepted-send/local-commit
    duplicate window are documented.
15. **Backend/store selection and provenance.** Applications explicitly select a
    configured backend/profile and receive a stable, equality-comparable opaque
    logical-store identity on status, receipt, and receive records without raw
    paths, credentials, or connection strings.

Production Watcher runtime promotion is hard-gated on the campaign-owned
`application-client-ready` checkpoint. There is no private-seam fallback.

The following spike mechanisms are explicitly not the contract:

- CLI subprocess parsing for attach/status/detach/send;
- `TELEX_WATCHER_INTERNAL_SEND_ONCE_V1`;
- direct dependence on raw daemon IPC or current internal Rust library seams;
- sender occupancy as proof of application consumption; and
- provider-specific logic in the shared client.

## Detector template obligations

The detector-template library must:

- validate every fixture against the canonical request/result schemas;
- declare detector schema version;
- declare template version and source provenance/digest;
- document provider cursor/replay and `maxSafeDowntimeSeconds` guidance;
- demonstrate allowed event-kind policy;
- keep provider semantics in editable scripts; and
- treat copied/customized detectors as user-owned code.

GitHub, customized GitHub, Azure DevOps, HTTP/JSON, and local command/file
templates demonstrate the protocol; they do not become runtime providers.

## Schema and packaging conformance

The production runtime promotion gate must add CI checks that compare its
request/result/event-metadata/health models with the four canonical schemas.
Schema drift blocks promotion.

The template-library promotion gate validates every shipped fixture against the
same schemas.

Before production publishing, packaging acceptance records:

- the exact `cargo package --list -p telex-watcher` or release-equivalent
  invocation;
- the default feature set;
- the expected published artifact/bin list; and
- proof that only the product binary ships by default.

`fake_detector` and `fake_telex` must be behind a non-default `test-support`
feature or an equivalent test-only package.

## Open-question dispositions

| Question | Disposition | Owner / rationale / downstream impact |
|---|---|---|
| Exact detector envelope | Accepted as strict v1 plus canonical schemas. Arbitrary JSON remains only in the implemented opaque fields. | Watcher contract; runtime/template CI must prevent drift. |
| Eventless state advancement | `idle.nextState` commits after successful evaluation; advancing means observations were intentionally classified, including ignored observations. | Detector authors/templates; unseen or failed evaluation must not advance. |
| Pinned versus follow path | Pinned is production default; follow-path is explicit development mode with double hash and drift rejection. | Runtime implements; templates default pinned. |
| Credential exposure | Cleared environment plus minimal baseline and explicit name allowlist; wrappers are operator command policy. | Runtime/hardening; no values in registration/request. |
| Initial lifecycle | Active, paused, terminal, removed. Single-event is terminal; removal is cancellation. | Runtime. Address-bound expiration deferred. |
| Repeated degradation notification | Supervisor-visible versioned health is required. Automatic Telex notification is deferred and must later be thresholded/coalesced/explicitly routed. | Operational hardening; production usability requires local health first. |
| Shared Application Client | Requirements exported to #12; no API or private-seam fallback. | Campaign/#12 owns acceptance; runtime remains blocked on checkpoint. |
| Test helper packaging | Must be non-default test support or test-only package. | Runtime node, with mechanical package-list proof. |
| Address-bound expiration | Deferred; occupancy never silently expires a watch. | Runtime/hardening if a real lifecycle source appears. |
| Audit retention/compaction | Initial event provenance intentionally unbounded with health warnings; safe compaction/backup deferred. | Operational hardening; storage growth is explicit. |
| Provider-wide budgets | Runtime supplies generic concurrency/cadence/jitter/backoff only. | Templates/hardening own provider credential budgets. |
| Abrupt-death platform proof | Behavior frozen here; OS mechanisms and destructive proof deferred. | Runtime/hardening; uncertain containment blocks the watch. |
| Template compatibility | Templates declare schema/template version, provenance, and replay limits; copied scripts are user-owned. | Detector-template-library. |
| Exclusive registry ownership | One PID/start-time-safe process-lifetime owner is required; contention fails startup before Telex or registry mutation. | Runtime; prevents split-brain state/send commits. |
| Canonical JSON failures | Opaque JSON is restricted to the I-JSON/RFC 8785 canonicalizable subset; failure is visible and non-advancing. | Runtime/templates; deterministic evidence requires a portable pre-image. |
| Send-only inbound anomalies | Normal sends to send-only stations are unoccupied/rejected; any interim backlog is a sender-health error and production-gate failure, never silently consumed. | #12 owns station capability; runtime/hardening own diagnostics. |
| Hosted ingestion, sandboxing, signed catalogs, multi-host failover, remote admin, rich UI | Deferred/out of scope. | Separate future work; no pressure on this contract. |

## Downstream implementation checklist

`watcher-runtime` is ready to detail only when it can answer:

- Where are registration revisions, lifecycle, opaque state, attempts, event
  ledger, runtime records, and health persisted?
- Which canonical registry identity and PID/start-time-safe lock enforce one
  mutation owner, and what supervisor diagnostic is emitted on contention?
- How are backend/profile selection and opaque logical-store identity threaded
  through registration, membership, receipts, ledger evidence, and health?
- How are the four schemas represented and checked in CI?
- How are RFC 8785 hashes and algorithm-qualified evidence produced?
- Which platform mechanisms prove detector-tree death after abrupt runtime exit?
- How are sender partial results compensated and surfaced?
- How does the service supervisor consume the health schema and detect staleness?
- How are unsafe downtime gaps, event collisions, pinned mismatch, and kind
  mismatch paused and resumed?
- What package-list test excludes helper binaries?
- Where are retention growth thresholds configured and reported?
- How is the accepted #12 client consumed without private seam fallback?

`detector-template-library` is ready to detail only when it can answer:

- Which protocol and template version does each template declare?
- Which event kinds/prefixes must registration authorize?
- What cursor/replay behavior makes the detector safe after downtime?
- Which credentials are allowlisted and how are private values kept out of
  fixtures and docs?
- How are every request/result fixture and provider omission edge case tested?

If downstream owners are assigned before this contract merges, they should
provide a lightweight contract-consumable acknowledgment. Otherwise these
questions become explicit launch-acceptance gates for their nodes.

## Test isolation

All daemon lifecycle, strict-send, crash, upgrade, handoff, branch-binary, and
real wake proofs use an isolated test plane:

- unique absolute `TELEX_HOME`;
- dedicated `TELEX_DB`;
- unique `TELEX_INSTALL_ROOT`; and
- absolute worktree branch binary.

The default local daemon and installed launcher are campaign coordination
infrastructure and are never destructive test targets.
