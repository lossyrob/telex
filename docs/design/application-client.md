# Application Client Contract

## Status and authority

This document is the normative, API-neutral semantic contract for a Telex
Application Client used by long-lived applications such as
[Telex Watcher](watcher.md) and [Operator Station](operator-station.md).
Issue [#12](https://github.com/lossyrob/telex/issues/12) owns the shared
contract, issue [#118](https://github.com/lossyrob/telex/issues/118) converges
the two consumer requirement sets, and
[ADR 0049](DECISIONS.md#0049--one-api-neutral-application-client-contract-governs-explicit-station-capabilities-and-forbids-private-fallbacks)
records the load-bearing boundary. Issue
[#152](https://github.com/lossyrob/telex/issues/152) owns Application Client
conformance, and
[ADR 0053](DECISIONS.md#0053--application-client-selects-the-daemon-through-an-explicitly-trusted-installed-current-bootstrap)
records the accepted daemon-selection policy the AC-C21 requirement promotes.

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
- MUST NOT expose receive, acknowledgment, unresolved/history query, or inbound
  backlog health evidence;
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

### AC-C21: Daemon selection is an explicit, admission-gated policy

Production consumers MUST select the shared Telex daemon through an
explicitly trusted absolute install root. The configuration MUST:

- reject a relative or empty trusted root;
- capture one immutable canonical absolute root before any connection or
  reconnect;
- treat the installed selector's `current` tag as the sole selection
  authority; `previous` is bookkeeping for rollback and is never
  independently acceptable; and
- never search `PATH`, embed a daemon `serve` entry in the consumer, open
  raw daemon IPC as a public seam, talk directly to a backend, or fall
  back to a foreign executable.

A subordinate pinned-target policy MAY be exposed for development and
tests only. It applies the same canonical process-image, platform file
identity, and untrusted-writability checks; has no installed manifest
authority; and does not follow upgrade or rollback. A source-compatible
legacy or dev connect that binds peer identity to the calling executable
MAY remain available, but it MUST NOT be an automatic fallback from a
failed installed-current resolution.

For each installed-current connect-or-spawn cycle the client MUST resolve
and validate one immutable target that includes:

- selected tag and versioned executable;
- selected manifest bound to that tag and executable, with validated
  build identity, package version, supported schema range, protocol
  version, and required security and Application Client capabilities;
- canonical containment of the selected version directory and executable
  beneath the canonical trusted root;
- current-OS-user ownership of the root and the authority chain, denying
  write, delete, or ownership control to any other principal on
  equivalent Unix permission and Windows DACL semantics (owner
  writability remains permitted for same-user upgrade administration);
- refusal of any selector, manifest, version directory, or executable
  that redirects through a symlink or reparse point;
- one canonical selected target used for both spawn and pre-connection
  peer authentication; and
- platform file and process identity evidence (an open process-image
  descriptor plus device and inode identity on Linux; canonical final
  path plus volume and file identity from held executable handles on
  Windows, with compatible sharing preserved through process creation).

These fields are compatibility and selection metadata. The contract MUST
NOT claim an executable-content digest or hash, an executable-content
migration or missing-digest rule, a signature, publisher or package
provenance, protection from malicious same-user administration, or
intra-user isolation. Any bundle-integrity SHA-256 recorded next to this
contract is publication-time evidence for the design sources; it is not
executable identity and never serves as bootstrap trust.

Selector movement MUST be serialized by one persistent OS-backed
shared/exclusive coordination lock beneath the trusted install root.
Unix uses a local-filesystem advisory shared/exclusive lock with
process-crash release; Windows uses the equivalent owner-restricted
range lock. The lock file MUST NOT be replaced or deleted as part of
selection. Filesystems that cannot prove equivalent ownership and
shared/exclusive lock semantics are unsupported and MUST fail closed.

Lock behavior MUST distinguish:

- **Parent shared admission.** The parent connect-or-spawn holds the
  shared lease from before reading `current` through resolution,
  prestarted or spawned peer authentication, and successful readiness
  acknowledgment.
- **Child independent shared validation.** A spawned daemon
  independently acquires a shared admission lease before binding its
  serving endpoint or publishing capability or readiness, validates the
  captured non-secret selection token against a fresh installed-current
  resolution and its own process image, and holds that lease through
  endpoint, capability, and readiness publication before releasing.
- **Upgrade and rollback exclusive.** Upgrade and rollback hold the
  exclusive lease across candidate validation, matching-daemon drain,
  predecessor exit, atomic `previous`/`current` switch, and selector
  publication. The drain operates inside that exclusive context and
  MUST NOT reacquire the shared lease.
- **Lock order.** Selector admission MUST precede daemon singleton or
  spawn admission.
- **Bounded fail-closed movement.** Selector movement and admission
  contention retries observed only for `current` MUST be bounded and
  fail closed on exhaustion as a typed unstable-selection outcome.
- **Matching-only prestarted reuse.** A prestarted daemon is reusable
  only when reuse-safe process identity, canonical process-image path,
  and platform file identity match the frozen target. A foreign peer
  MUST be refused before the client sends any store or session metadata
  or its version and capability handshake.

Bootstrap failures MUST be typed and MUST NOT expose the raw authority
path or manifest binding as durable public evidence. The taxonomy MUST
distinguish at least: invalid trusted root, unsafe install authority,
missing current selector, invalid manifest, incompatible manifest,
unstable selection, missing executable, executable identity mismatch,
and foreign daemon. Additional typed variants MAY be added; the
contract MUST NOT collapse a known reason into an untyped bucket.

The client MUST NOT drain, kill, trust, or start beside a foreign
daemon. It MUST NOT expose raw daemon IPC frames as a supported
application contract or promote a private admission or drain seam as
public policy. The Local Daemon owns the install layout, manifest and
build contract, selector lock and selection token, daemon process
admission, OS peer checks, readiness publication, and upgrade and
rollback coordination; the Application Client owns the public policy,
typed failure projection, and use of the supported admission flow.

**Application binding note.** The supported Rust binding realizes this
policy as `ApplicationDaemonBootstrap::InstalledCurrent { trusted_root
}`, the subordinate `ExactExecutable { executable }`, and the additive
`ApplicationClient::connect_with_daemon(config, daemon)` constructor,
with `ApplicationClientError::DaemonBootstrap(DaemonBootstrapFailure)`
carrying the typed failure reasons. These names are Rust implementation
guidance for this binding. Other bindings MAY choose different names;
they MUST preserve the semantic contract stated above and MUST NOT
expose raw daemon IPC as a supported application API.

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
  shared semantic is missing;
- searching `PATH`, embedding a daemon `serve` entry in the consumer, or
  starting beside a foreign executable in place of the AC-C21
  installed-current selection.

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
   - exercise both send-only and bidirectional capabilities;
   - exercise AC-C21 installed-current selection, including trusted-root and
     authority-chain refusal, manifest/build/version compatibility, immutable
     tag behavior, shared and exclusive selector admission, matching-only
     prestarted reuse, platform file and process identity mismatch,
     symlink/reparse refusal, concurrent spawn, selector-client death during
     spawn, child admission failure, upgrade, rollback, selector contention,
     daemon crash and restart, and no stale-version resurrection.
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
