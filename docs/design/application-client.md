# Application Client Contract

## Status and authority

This document is the normative, API-neutral semantic contract for a Telex
Application Client used by long-lived applications such as
[Telex Watcher](watcher.md) and [Operator Station](operator-station.md).
Issue [#12](https://github.com/lossyrob/telex/issues/12) owns the shared
contract, issue [#118](https://github.com/lossyrob/telex/issues/118) converges
the two consumer requirement sets, and
[ADR 0049](DECISIONS.md#0049--one-api-neutral-application-client-contract-governs-explicit-station-capabilities-and-forbids-private-fallbacks)
records the load-bearing boundary.

The terms **MUST**, **MUST NOT**, **SHOULD**, **SHOULD NOT**, and **MAY** are
normative. This contract defines semantics, state, and evidence. It does not
choose a package name, language binding, FFI, public socket protocol, daemon
frame, or product-specific API shape.

Supporting, non-normative requirements traceability and provenance are in
[requirements-crosswalk.md](../notes/application-client/requirements-crosswalk.md).
That note does not define or modify this contract's semantics. The canonical
bundle identity and source provenance are in
[application-client.bundle.json](application-client.bundle.json).

## Scope

The Application Client is the supported semantic boundary between an
application and Telex membership, messaging, receipt, history, and recovery
operations. One contract serves send-only and bidirectional applications while
exposing only capabilities that the application actually holds.

The client owns:

- stable application responsibility and ephemeral runtime identity;
- attach, reconcile, recovery, and detach semantics;
- typed liveness, collision, membership-loss, and readiness evidence;
- explicit sender selection and capability-aware message operations;
- durable acceptance, exact recipient delivery, acknowledgment, and
  disposition state;
- retry-safe operation identity, source identity, bounded history, delta
  ordering, and resynchronization;
- backend-neutral behavior and authenticated-principal provenance where the
  selected backend supplies it.

The client does not own Watcher detector execution, scheduling, state
transactions, or provider policy. It does not own application-specific
presentation, human workflow selection, mediation judgment, notification
policy, routing vocabulary, UI, or campaign vocabulary.

## Terms

**Application responsibility**
: A stable logical responsibility and its configured address or addresses. It
  may survive process replacement.

**Runtime identity**
: A fresh, never-reused identity for one application process or session
  incarnation. A stable responsibility and a runtime identity are never
  interchangeable.

**Station capability**
: The declared capability held at an address: `send-only` or `bidirectional`.
  Capability is explicit and inspectable.

**Logical store identity**
: An opaque, stable, equality-comparable identity for the selected Telex store.
  It persists across application and daemon restart and exposes no raw path,
  credential, token, or connection string.

**Membership**
: The association of a logical store, application responsibility, runtime
  identity, address, capability, liveness predicates, and lease owner/epoch.

**Exact delivery identity**
: The tuple that identifies one recipient delivery: logical store identity,
  message ID, recipient address, and delivery-row identity. Delivery role is
  carried as context but does not replace recipient identity.

**Operation identity**
: A caller-supplied, retry-stable identity for an application-authored
  operation across retry, process replacement, and result reconciliation.

**Snapshot fence**
: A snapshot identity or monotonic per-axis version boundary that orders
  subsequent deltas and prevents resynchronization from regressing observed
  workflow state.

**Compensation handle**
: A typed reference to partial lifecycle work that a caller can inspect,
  complete, or reverse without guessing what succeeded.

## Normative semantic contract

### AC-C01: Application identity and explicit lifecycle

An application responsibility MUST remain distinct from every runtime identity
that occupies it. Each process or session incarnation MUST use a fresh,
never-reused runtime identity.

The lifecycle MUST distinguish:

1. first attach;
2. deliberate detach;
3. typed reattach or restart recovery;
4. reconciliation of uncertain or partial membership.

A deliberate detach MUST be durable application intent for recovery purposes.
Automatic repair MUST NOT resurrect a deliberately detached membership.

### AC-C02: Process-bound liveness predicates

Membership MUST support typed process liveness predicates that include process
ID and process start time where the host can supply them. Implementations MUST
evaluate the pair as one reuse-safe predicate and MUST NOT treat a recycled
process ID as proof that the original runtime remains alive.

The predicate and its last evaluation evidence MUST be inspectable through
health projection.

### AC-C03: Multi-address lifecycle is atomic or compensable

Attach, reconcile, and detach over multiple addresses MUST have one of two
outcomes:

- atomic success for the requested set; or
- an explicit per-address result containing completed work, failed work,
  collision evidence, and compensation handles.

Partial attachment MUST NOT be reported as application-ready. A failed
multi-address operation MUST preserve enough evidence to retry, compensate, or
restore the prior configuration without inferring state from logs.

### AC-C04: Caller-selected recovery and reconcile-and-send

The caller MUST select either:

- strict membership, where loss returns a typed `NeedsAttach`-class result; or
- bounded automatic repair, with an explicit retry budget.

Repair MUST preserve the original capability, liveness predicates, ownership
constraints, and sender selection. It MUST NOT silently convert strict
membership into generic registration.

The client MUST provide a typed reconcile-and-send operation or equivalent
compound primitive. It MUST return the final send result, typed membership
failure, or an indeterminate result with a recovery handle. No caller is
required to parse CLI output or raw daemon IPC.

Restart recovery MUST support explicit reattach or typed restart-loss
observation without requiring a resident per-application waiter or holder.

### AC-C05: Membership loss and collision are typed

Membership-loss projection MUST distinguish at least:

- `daemon-restart`;
- `predicate-death`;
- `collision`;
- `deliberate-detach`;
- `needs-attach`;
- `owner-demoted`;
- `unknown`.

The taxonomy is extensible. Implementations MUST preserve every known reason
without collapsing it into `unknown`, and MUST retain an explicit unknown or raw
forward-compatible representation when a newer reason is not recognized.

A collision result MUST expose the current owner identity and lease epoch when
available, plus bounded retry, reset, or wait guidance. The client MUST NOT hide
force takeover and MUST NOT silently replace another live application. If the
prior owner cannot be proved gone, the operation fails closed and remains
visibly blocked.

### AC-C06: Sender selection is explicit

Every application-authored send, reply, disposition, or compound message
operation MUST name the sender responsibility or an unambiguous sender handle.
When more than one sender is possible, omission MUST return a typed ambiguity
error rather than choosing one. When none is attached, the operation MUST
return typed membership loss rather than silently using no sender.

### AC-C07: Station capability is explicit

`send-only` and `bidirectional` are distinct station capabilities.

A send-only membership:

- MUST NOT advertise inbound application attendance;
- MUST NOT expose receive or acknowledgment operations;
- MUST NOT make a target address appear application-deliverable merely because
  a sender responsibility occupies it.

A send directed to an address that has only send-only membership MUST return
the address policy's unoccupied or rejected result, never a false
application-delivered result.

A bidirectional membership MAY receive only after it has established the
receive capability and exact delivery identity required by AC-C09.

### AC-C08: Receipt and workflow axes remain separate

The client MUST represent these facts as separate typed axes:

1. durable message acceptance;
2. target occupancy at acceptance;
3. push attempt and push acceptance;
4. exact recipient transport consumption or acknowledgment;
5. workflow disposition.

No axis implies another. In particular:

- occupancy is not durable acceptance;
- push is not recipient consumption;
- recipient consumption is not workflow disposition;
- workflow disposition is not evidence that an application durably ingested a
  delivery unless the exact recipient acknowledgment also exists.

Applications that commit local event state on send, including Watcher, MUST be
able to use durable acceptance as the commit gate while retaining the other
axes as independent diagnostics.

### AC-C09: Bidirectional receive binds acknowledgment to one delivery

A bidirectional receive result MUST contain:

- the complete message and opaque metadata;
- logical store identity and message ID;
- recipient address and exact delivery-row identity;
- delivery role;
- an acknowledgment capability bound to that exact recipient delivery;
- any snapshot or ordering evidence needed for restart-safe resynchronization.

Acknowledging one recipient MUST NOT consume another recipient's delivery of
the same message. A missing delivery row MUST return a typed no-op or mismatch
result and MUST NOT fabricate a consumed row.

Receive MAY be projected as a stream, callback, or poll by a binding, but those
shapes MUST preserve identical delivery and acknowledgment semantics.

Message acceptance MUST verify that the actual recipient-specific serialized
receive frame fits the supported transport limit before persisting the message
or any delivery row. The check MUST include JSON escaping and generated delivery
fields, MUST parse and deduplicate recipients once, and MUST enforce the
protocol recipient-count limit before creating destination address records.
If an older store already contains a row that cannot be represented without
changing its body, subject, or metadata, the daemon MUST preserve those stored
values, atomically record a terminal `rejected` disposition for the exact
recipient delivery with daemon provenance and a bounded diagnostic note,
consume that transport delivery, and return a typed receive-specific quarantine
outcome containing recipient, message ID, serialized bytes, frame limit, and
continue-receiving guidance. This outcome is post-acceptance and MUST NOT use
the pre-acceptance rejection taxonomy. Sender receipt refresh and state deltas
MUST expose a sticky structural daemon-quarantine origin that supported
application disposition APIs cannot mint, and MUST NOT report application
recipient consumption as accepted even after a later workflow disposition.
The latest ordinary workflow disposition remains a separate axis. This legacy quarantine is an explicit
progress exception: the unrepresentable delivery is not handed to the
application, but it cannot permanently block later receivable deliveries, and
the durable evidence remains auditable after restart. Notification-only CC
copies MUST be skipped with an operator diagnostic rather than fabricating an
obligation-bearing workflow disposition.

### AC-C10: Acknowledgment follows durable application ingest

An application MUST acknowledge a delivery only after it has stored enough
state to replay or resume the application action after restart. The client MUST
make acknowledgment an explicit action; it MUST NOT infer acknowledgment from
occupancy, rendering, any application-side side effect, or transport output.

The receive-health surface MUST keep at least these conditions distinguishable:

- acknowledgment pending;
- pending unconsumed delivery;
- inbound actionable backlog;
- attended but deaf;
- recovering;
- degraded;
- stopped or unattended;
- unknown.

Actionable backlog MUST degrade readiness even when membership remains
occupied. Occupancy alone is never the healthy state.

### AC-C11: At-least-once identity and no-regression resynchronization

Redelivery identity MUST be per recipient. Applications MUST be able to dedupe
the same exact delivery after process or daemon restart without conflating
another recipient or another logical store.

Initial snapshot and subsequent deltas MUST share a snapshot fence or monotonic
per-axis versions. Resynchronization and backfill MUST NOT regress message,
delivery, acknowledgment, disposition, health, or recovery state.

At-least-once duplication is preferable to consume-before-deliver loss. The
client MUST expose enough identity and evidence to make duplicates visible and
reconcilable.

### AC-C12: Unresolved obligations and bounded history are queryable

The client MUST support:

- all unresolved obligations for an exact recipient or application scope;
- bounded recent message and delivery history;
- bounded thread history.

Filtering semantics and limits MUST be explicit. A limit MUST NOT be applied
before a required recipient, unresolved-state, thread, or time filter in a way
that can silently omit matching records. Recovery MUST NOT require full-store
materialization.

### AC-C13: Message operations return typed, identity-checkable results

The semantic surface MUST include:

- send with explicit sender;
- metadata-bearing reply;
- read-thread;
- exact-recipient acknowledgment;
- exact-recipient disposition;
- source resolution;
- the compound operations in AC-C20.

Results MUST be typed and MUST carry the identities needed to check what store,
message, recipient, operation, and sender they describe. Reply MUST NOT
implicitly disposition another recipient's obligation.

### AC-C14: Application-authored operations are retry-safe

Application-authored operations MUST accept a stable operation identity.
Before an operation that can be retried, the application MUST be able to
persist that identity in restart-safe state.

The result model MUST distinguish:

- accepted with durable receipt;
- rejected before acceptance;
- partial;
- indeterminate within the accepted-send/local-commit duplicate window;
- previously completed or duplicate operation;
- authoritative `not-recorded`; and
- unavailable absence evidence after a retention boundary.

A rejection proved to occur before durable acceptance MUST carry typed
retryability: `transient`/retryable or `permanent`/non-retryable. Callers MUST
NOT infer retry safety from free-form error text. Transport or peer failures
whose acceptance boundary is not proved remain `indeterminate`.

Duplicate or previously completed evidence is authoritative only when it proves
the same stable operation identity and a comparable canonical payload identity
for the attempted operation. Mismatched or non-comparable payload evidence MUST
return a typed conflict and MUST NOT authorize replay success or adoption of the
prior result.

After restart, the client MUST support operation-result and receipt
reconciliation before the application authors a replacement. It MUST preserve
the original sender, recipient, payload identity, and retry budget during that
reconciliation.

Before the first send attempt, the application MUST persist the complete typed
operation reference supplied by the client. That reference MUST bind the exact
logical store, application responsibility, operation identity, comparable
payload identity, and current operation-evidence retention generation.

An authoritative `not-recorded` result MUST prove that no operation record,
operation-to-message mapping, result, or receipt exists for that exact tuple and
that the reference's retention generation still equals the store's current
generation for the application responsibility. Because accepted message
insertion and operation-to-message evidence are atomic, that proof also proves
that durable acceptance did not occur for the exact operation. Cleanup that
deletes any terminal operation evidence MUST advance the durable retention
generation. A missing legacy generation or a generation mismatch MUST return a
typed retention-boundary outcome, not `not-recorded`, and MUST NOT authorize
retry or replacement.

Pending or indeterminate reconciliation MUST use the operation identity together
with the opaque logical-store identity staged when the operation began. A store
binding mismatch remains blocked or indeterminate and MUST NOT authorize retry,
acceptance, or result adoption. Rebinding to another store requires an explicit
new or recovery operation.

### AC-C15: Source identity is store-scoped

Source identity is `(logical store identity, message ID)`. A same-number message
from another store MUST NOT be opened or treated as the source.

The same opaque logical-store identity also fences retry-stable operation
reconciliation under AC-C14; source or operation evidence from another store
cannot be silently rebound.

Authoritative `not-recorded` evidence is scoped further by application
responsibility and operation identity. Absence observed for another
responsibility, another store, or after the staged retention generation changed
is not evidence about the requested operation.

The logical store identity MUST NOT expose a raw path, credential, token, or
connection string.

Source resolution MUST distinguish:

- `authoritative`;
- `captured-only`;
- `mismatch`;
- `unavailable`.

A captured copy MAY support human inspection, but it MUST NOT be represented as
authoritative when the selected logical store cannot reproduce the source.

### AC-C16: Backend selection preserves one semantic contract

The client MUST explicitly select a configured backend or profile. SQLite and
credentialed Postgres MUST preserve the same identities, lifecycle,
capability, message, receipt, acknowledgment, disposition, retry, and recovery
semantics.

Backend-specific transport or authentication details MAY affect availability
and provenance but MUST NOT alter the semantic meaning of a result.

When the backend supplies an authenticated principal, the client MUST carry the
principal and provenance. A principal MUST be labeled `verified` only when the
client can cite authenticated evidence; otherwise it is `unverified` or
`unavailable`. Backend access alone is not cryptographic identity proof.

### AC-C17: Lifecycle and health projection is evidence-bearing

Health projection MUST cover:

- logical store identity for each address or membership projection;
- configured responsibility and station capability;
- runtime identity and liveness predicates;
- registration, lease owner, and lease epoch;
- per-address readiness;
- typed membership loss and collision;
- reconciliation and compensation state;
- deliberate detach and recovery outcomes;
- pending unconsumed, inbound actionable, and acknowledgment-pending work;
- receive and sender readiness.

Projection MUST retain evidence for each degraded state. It MUST NOT collapse
membership occupancy, application readiness, operator attendance, notification
submission, and human availability into one `online` value.

Every status or health record for an address or membership MUST carry the same
stable, opaque logical store identity used by receive and operation results.

### AC-C18: Discovery, retry, and cleanup are bounded and scoped

The client MUST provide local discovery, bounded retry or throttling, receipt
identity cross-checking, and application-scope cleanup without CLI parsing or
raw daemon IPC.

Cleanup MUST be evidence-preserving and MUST NOT delete Telex messages,
delivery rows, dispositions, another application's membership, or another
application's local scope. Ambiguous ownership fails closed.

### AC-C19: Delta events have explicit ordering and recovery

Message, delivery, acknowledgment, disposition, health, and recovery changes
SHOULD be available as delta-oriented events so applications do not serialize
the full feed on every mutation.

Every delta MUST carry a snapshot fence or monotonic per-axis ordering token and
the identity of the state it advances. A detected gap, restart, or version
mismatch MUST trigger typed resync or backfill. Applying a delta or resync MUST
NOT regress workflow state.

### AC-C20: Compound operations preserve ordering and recovery evidence

The client MUST provide general primitives for compound application workflows
that may include metadata-bearing message authorship, reply, and exact-recipient
workflow effects such as disposition. The client does not decide which steps a
product requires.

For a caller-declared sequence, the client MUST:

- preserve the required durable ordering;
- return per-step accepted, rejected, partial, or indeterminate outcomes;
- expose recovery handles for incomplete work;
- preserve the stable operation identity across retry;
- support a machine-readable outcome record before a caller performs a
  non-stale terminal closure that depends on that record.

When a caller declares that one authored operation must precede terminal
disposition, the disposition MUST NOT become terminal until the prerequisite
operation is durably accepted. Whether a workflow requires a reply, another
message, or disposition alone remains caller policy.

## Product boundary and prohibited fallback seams

The following remain outside the shared client:

- Watcher detector request/result schemas, scheduling, state transitions,
  allowed event kinds, provider templates, script policy, and runtime health;
- application-specific human-obligation selection, notification presentation,
  mediation/routing vocabulary, and UI;
- campaign-local kinds, addresses, and attention-routing policy.

The following MUST NOT become a supported production fallback:

- parsing Telex CLI stdout, stderr, or exit text as an application API;
- using raw daemon IPC or private Rust internals as a product contract;
- using sender occupancy or push attempt as proof of recipient consumption;
- using spike environment variables, helper binaries, namespaces, local UUID
  files, or store-path fingerprints as shared identity;
- implementing a Watcher-private or Operator-private client when a required
  shared semantic is missing.

If a required semantic is absent, the affected consumer remains blocked until
the shared client contract and implementation are extended.

## `application-client-ready`

`application-client-ready` means:

- this API-neutral contract is accepted;
- every Watcher W-01 through W-15 and Operator AC-01 through AC-15 requirement
  has an explicit disposition and strength-preserving mapping;
- the canonical design bundle and issue #12 publication have passed their
  required exact-byte approvals.

It does **not** mean:

- a supported client core exists;
- any language binding exists;
- conformance tests pass;
- Watcher or Operator Station uses the client;
- either consumer is production-ready.

The checkpoint scope is `design-only`. It unlocks detailed downstream
implementation and promotion work while preserving all implementation,
conformance, integration, and hardening gates.

## Downstream decomposition

The accepted contract decomposes into these ordered work areas:

1. **Supported client core**
   - implement AC-C01 through AC-C20 over supported daemon and backend
     capabilities;
   - define typed semantic models without exposing private IPC.
2. **First binding**
   - select the first language and public API shape;
   - prove that binding shape does not weaken the semantic contract.
3. **Conformance**
   - create backend-parity, restart, collision, receipt-axis, exact-delivery,
     retry-window, snapshot-fence, and compound-operation tests;
   - exercise both send-only and bidirectional capabilities.
4. **Watcher integration**
   - replace spike-private send/membership seams with the supported client;
   - preserve receipt-gated state and send-only readiness invariants.
5. **Operator Station integration**
   - consume the ordinary bidirectional client primitives without a
     Station-private seam;
   - preserve exact-delivery ack, opaque metadata-bearing reply, per-recipient
     disposition, unresolved/history recovery, all AC-C15 source-resolution
     states, and generic compound ordering when the caller declares a sequence.
6. **Operational hardening**
   - validate principal provenance, observability, bounded storage, performance,
     packaging, upgrade, and failure recovery in supported environments.

No consumer integration may bypass the supported core or conformance work with
a product-private client.
