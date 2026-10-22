# Minimal Watcher runtime core

- **Workstream:** `telex-watcher`
- **Node:** `watcher-runtime-core`
- **Type:** implementation
- **Status:** shaped; planned and unlaunched
- **Attention:** watch
- **Depends on:** completed `dumb-watcher-contract-gate`
- **External dependencies:** completed Application Client `client-conformance`;
  passed and reconciled Application Client `consumer-integration-gate`
- **Tracker:** not yet created
- **Parent workstream:** [lossyrob/telex#100](https://github.com/lossyrob/telex/issues/100)
- **Campaign:** [Addressable Attention #102](https://github.com/lossyrob/telex/issues/102)

## Outcome

Replace the experimental Watcher v1 runtime with one production v2 core that
runs trusted local observational commands outside agent sessions and sends one
fixed-route Telex message when a detector reports an event.

The complete node owns persistent v2 registration and lifecycle, exclusive
registry ownership, bounded scheduling and process containment, opaque state,
runtime-generated event identity, durable pending operations, exact-operation
receipt recovery, and machine-readable attempt and health evidence. It consumes
the supported public Rust Application Client only.

This node does not deliver the management CLI, broad detector examples,
packaging, service installation, provider actions, or later operational
hardening. Those remain separate accepted nodes or gates.

## Authority

- [`../../../../docs/design/watcher.md`](../../../../docs/design/watcher.md) -
  normative Watcher contract.
- [`../design/current-design.md`](../design/current-design.md) - integrated
  workstream authority and Application Client boundary.
- [`../graph.json`](../graph.json) - accepted dependency ordering.
- [ADR 0046](../../../../docs/design/DECISIONS.md#0046--watcher-runs-provider-neutral-trusted-local-detectors-with-receipt-gated-state)
  - provider-neutral trusted-local execution, fixed routing, and receipt-gated
  state.
- [ADR 0050](../../../../docs/design/DECISIONS.md#0050--watcher-v2-uses-minimal-command-registration-and-runtime-owned-event-identity)
  - minimal command registration and runtime-owned identity.
- The canonical
  [registration](../../../../docs/design/schemas/watcher-registration-v2.schema.json),
  [request](../../../../docs/design/schemas/watcher-detector-request-v2.schema.json),
  and
  [result](../../../../docs/design/schemas/watcher-detector-result-v2.schema.json)
  v2 schemas.
- Application Client issue
  [#152](https://github.com/lossyrob/telex/issues/152), its final merged
  conformance PR, the exact-head Watcher and Operator Station attestations, and
  the separately passed `consumer-integration-gate`.

The existing `telex-watcher` crate and
[`docs/generic-watcher-spike-report.md`](../../../../docs/generic-watcher-spike-report.md)
are implementation learning only. The current v1 registry, detector identity,
`CliTelexAdapter`, CLI subprocess parsing, and private send-once behavior are
not production authority.

## Bundle rule

Keep all runtime-core behavior in one tracker and one delivery PR. Do not split
registry, scheduler, protocol, process containment, Application Client
integration, recovery, health, or backend proof merely for reviewability.

If implementation discovers a real prerequisite, preserve the same branch and
PR, record the dependency and exact resume condition, and continue after it
closes. Node-owned unfinished work is in progress, not a reason to claim a
partial runtime.

## Launch prerequisites

Do not create the implementation worker until all of these are true:

1. Application Client conformance is merged.
2. Watcher and Operator Station attest the same exact reviewed and green
   conformance source head that is merged.
3. `consumer-integration-gate` is passed and reconciled on `main`.
4. The runtime tracker and this task specification are reviewed and reconciled.
5. Launch uses a genuine new `gpt-6-astra` context with high reasoning and long
   context. Do not reuse an implementation, review, initializer, or Station
   product context.

Issue #144, Operator Station UI work, broad packaging, and optional examples
are not prerequisites.

## Deliverables

### V2 registry and ownership

- A versioned registry that persists:
  - registration and revision history needed by active evidence;
  - lifecycle, eligibility, health, activation, and success timestamps;
  - opaque committed state;
  - attempts and bounded diagnostics;
  - pending operations and their complete frozen values;
  - last allocated and committed event-sequence high-water marks;
  - committed event, receipt, payload, store, and recurrence evidence;
  - removed-ID tombstones and unresolved removed evidence;
  - runtime sessions, restart reconciliation, and health state; and
  - retained row/byte counts and positive warning thresholds.
- Permanent watch IDs. Removed IDs are tombstoned and never reused.
- Explicit isolation of unsupported v1 rows. V1 registrations never execute
  silently as v2.
- One process-lifetime exclusive owner for each canonical physical registry
  identity. Ownership is PID/start-time reuse-safe and acquired before sender
  attachment, scheduling, reconciliation, or mutation.
- A second runtime fails nonzero with `registry-already-owned` and does not
  mutate registry or sender state.

### V2 registration and detector protocol

- Concrete models conforming to the canonical v2 registration, request, and
  result schemas.
- Strict outer objects, I-JSON and RFC 8785 canonicalization, UTF-8 byte limits,
  calendar-valid UTC timestamps, command NUL rejection, and absolute
  working-directory validation.
- Direct argv execution without implied shell interpolation.
- Request values are exactly the strict canonical schema: attempt ID/time;
  watch ID, activation time, last success, and opaque parameters; and committed
  opaque state. Registration policy remains runtime-side and is not injected as
  extra request fields or hidden parameters.
- Correct `idle`, `event`, `terminal`, and `degraded` transitions, including
  omitted-state preservation and explicit JSON `null`.
- Detector output cannot set event identity, operation identity, backend,
  sender, target, attention, disposition, cadence, timeout, or an action.

### Scheduling and process containment

- Bounded global concurrency and single-flight execution per watch.
- Product-bounded cadence and timeout, deterministic per-watch jitter, bounded
  stdout/stderr draining, exponential failure backoff, and due-tick
  coalescing.
- One due execution after restart, not replay of every missed interval.
- Process-tree containment with kill-on-runtime-exit behavior on Windows and
  Unix.
- Startup reconciliation that marks prior runtimes and unfinished attempts
  interrupted without allocating an event, sending, or committing state.
- No affected watch becomes eligible until prior containment is proven ended.
  Unprovable containment pauses and blocks it with
  `orphan-containment-unproven`.
- All destructive process and daemon tests use isolated `TELEX_HOME`,
  `TELEX_DB`, `TELEX_INSTALL_ROOT`, and absolute branch binaries.

### Event identity and durable pending operations

- A strictly increasing, never-reused `eventSequence` within each permanent
  watch ID.
- Runtime-generated event ID bound one-to-one to
  `(watchId, eventSequence)`.
- Restart-stable Application Client operation ID for the exact send.
- One atomic stage-before-send transaction containing complete identities,
  captured registration revision, backend and logical-store binding, fixed
  route and policy, prior and proposed state, normalized envelope, canonical
  hashes, pending status, and separate reconciliation and send-retry budgets.
- Complete staged values survive restart. Hashes alone are insufficient.
- Retry never changes sequence, event ID, operation ID, store, route, policy,
  payload, or proposed state.
- Every event after a successful commit is a new recurrence with a new
  sequence, even when rendered content is identical. Recurrence evidence is
  visible but never silently suppresses delivery.

### Public Application Client integration

- Use only the supported public Rust Application Client and its accepted
  installed-current bootstrap.
- Stable sender responsibility with a fresh runtime identity, process-bound
  liveness, explicit sender selection, send-only capability, and bounded
  multi-address attach/reconcile/detach with compensation.
- Explicit backend profile and opaque logical-store identity carried through
  registration, membership, staging, receipts, evidence, and health.
- Fixed sender and target for one watch ID. No hidden rerouting or force
  takeover.
- Typed separation of durable acceptance, occupancy, push, recipient
  consumption, and workflow disposition. Only durable acceptance permits state
  commit.
- Startup and periodic sender reconciliation, plus reconciliation after typed
  membership loss. Partial or colliding membership keeps the runtime
  non-ready.
- Send-only addresses never create false inbound attendance. Actionable inbound
  backlog is not consumed; it makes runtime health degraded and
  `productionReady = false`.
- No CLI parsing, raw daemon IPC, spike helper, subprocess courier, or
  Watcher-private client.

### Receipt recovery and exact transitions

Implement the complete pending-operation state machine:

- Proven durable acceptance, or matching `previously-completed` or `duplicate`
  evidence, atomically commits proposed state, receipt evidence, sequence, and
  attempt result.
- A matching authoritative `NotRecorded` or definite transient pre-acceptance
  rejection retains the exact staged identity and permits only bounded
  exact-same-operation retry.
- `RetentionBoundaryCrossed` and unavailable history remain indeterminate and
  never authorize retry.
- Permanent pre-acceptance rejection pauses and blocks with
  `send-permanently-rejected`.
- Exhausted transient retry pauses and blocks with `send-retry-exhausted`.
- Indeterminate outcome makes the watch
  `active/reconciliation-pending/degraded`; no detector attempt or replacement
  send may start.
- Exhausted reconciliation pauses and blocks with
  `receipt-outcome-unresolved`.
- Payload, operation, event, or canonical identity conflict pauses and blocks
  with `operation-identity-conflict`.
- Evidence from another logical store pauses and blocks with
  `logical-store-mismatch`.
- Late acceptance commits the original transition.
- Late authoritative `NotRecorded` or transient rejection records proof,
  leaves the watch paused/blocked, and changes the blocked reason to
  `send-retry-exhausted` before any later finite exact-send retry grant.
- Late permanent rejection records proof and changes the blocked reason to
  `send-permanently-rejected`; no retry grant is available.
- Removal stops execution but retains the pending operation, identity, payload,
  tombstone, and later closure path.
- The runtime never guesses acceptance, blindly resends, forces local commit,
  discards unresolved evidence while leaving the watch live, or reruns the
  detector while its prior event is unresolved.

### Machine-readable evidence

- A v2 normalized metadata model with reserved Watcher provenance and detector
  metadata nested beneath it.
- A versioned v2 runtime/watch health schema implemented with the concrete
  model. The experimental health-v1 schema remains frozen history.
- Runtime health includes identity, heartbeat, readiness, registry revision,
  sender/store/membership state, reconciliation state, inbound backlog,
  retention, and typed diagnostics.
- Watch health includes lifecycle, eligibility, blocked reason, revision,
  sequence high-water marks, pending operation and budgets, attempts,
  suppressed due ticks, recurrence, sender readiness, retention, and typed
  diagnostics.
- Runtime heartbeat advances independently of detector execution.
- Attempt and event evidence is queryable by the later CLI without exposing
  credentials, raw private daemon frames, or arbitrary detector state in Telex
  messages.

### Typed local management boundary

- A local typed management interface owns register, list/show, pause, resume,
  update/state replacement, reconcile-now, finite reconciliation grant, finite
  send-retry grant, close-not-accepted, remove, health, attempt, event, pending
  operation, and removed-evidence operations.
- All mutations serialize through the exclusive registry owner and share the
  same transactions, lifecycle checks, pending-operation state machine, audit
  records, and typed refusal reasons as runtime execution.
- Mutation results include the resulting registration revision, lifecycle,
  eligibility, health, and typed refusal or recovery evidence.
- The interface is local-only and designed for the later CLI. It is not a
  public remote administration protocol, daemon wire API, or second semantic
  implementation.
- The runtime core is incomplete if command handlers would need to recreate
  registry transactions, grants, closure semantics, audits, or refusal logic.

## First practical validation input

The first practical target is replacing a session-owned GitHub PR polling loop.
It is a field-validation input, not a mandatory provider framework and not issue
#144.

Use one trusted-local detector command with a fixed Telex route and explicit
credential allowlist. Its documented coverage may include:

- external substantive reviews and issue comments after excluding the PR
  author, the operator login, and configured bots;
- changes requested;
- failed checks;
- blocked, dirty, behind, or unstable merge state;
- approved, clean, non-failing readiness;
- closed or merged terminal state; and
- an optional deterministic initial snapshot.

The detector owns those policies and its opaque cursor. It performs no provider
mutation. If inline review-thread comments are not queried, the field report
must state that exclusion and must not claim coverage of all review activity.
Adding an explicit review-thread query is allowed detector policy, not a
Watcher-core contract change.

## Out of scope

- The `watcher-cli-ux` management experience.
- Broad GitHub, Azure DevOps, HTTP, file, or command example packaging.
- Provider actions, merge, comment, approval, workflow automation, or agent
  launch.
- Remote executable registration or administration.
- Mandatory manifests, script hashes, pinning, kind allowlists, provider
  preflight, downtime declarations, fixtures, or a prescribed test framework.
- V1 migration tooling or automatic state/event-ID translation.
- Production installer, service supervisor, release publication, upgrade UX,
  multi-host failover, webhook ingestion, sandbox claims, broad retention
  compaction, or operational hardening.
- Operator Station UI or issue #153 bootstrap behavior.
- Streamliner state, workflow records, or orchestration artifacts in the
  product diff.

## Success criteria

- One v2 registry supports many watches and sender responsibilities with one
  exclusive owner and no silent v1 execution.
- All canonical schema examples and negative cases pass against the concrete
  v2 models.
- Scheduling, output bounds, timeout, backoff, jitter, restart, and
  process-tree containment are deterministic and tested on supported platforms.
- Event identity, staging, recovery, receipt-gated commit, recurrence, removal,
  and tombstone invariants survive restart and injected local commit failure.
- The public Application Client is the only Telex seam. SQLite and credentialed
  Postgres prove equivalent typed acceptance, membership, recovery,
  `NotRecorded`, exact retry, retention-boundary, mismatch, and compensation
  behavior.
- No detector runs while its prior event is unresolved.
- Health and evidence make every non-ready or blocked state explainable without
  parsing logs or CLI stderr.
- A bounded GitHub PR detector can exercise the runtime without adding provider
  semantics or actions to the runtime.
- Every material discovery has a durable disposition; no accepted node-owned
  work remains deferred at merge readiness.

## Validation, review, and reporting

- Run formatting, warnings-denied Clippy, Watcher unit/integration tests,
  canonical schema validation, SQLite and credentialed-Postgres runtime
  matrices, and the repository feature matrix.
- Inject accepted-send/local-commit failure, restart, late acceptance,
  authoritative `NotRecorded`, transient/permanent rejection, duplicate,
  payload mismatch, store mismatch, retention boundary, membership loss,
  collision, partial attachment, runtime death, detector timeout, output
  overflow, malformed/noncanonical result, and orphan-containment uncertainty.
- Test the GitHub PR detector with committed fixtures first. Any live proof uses
  an owned or explicitly authorized PR and observes only.
- Never use the default campaign coordination daemon or installed state for
  destructive evidence.
- Establish one full implementation-review baseline on the first stable
  complete head, run required CI, and obtain a fresh exact-head design
  inspection before merge readiness.
- Final review, required CI, and design inspection must cover the same exact
  final head. Any head movement invalidates affected evidence and requires the
  corresponding delta or full rerun under campaign policy.
- The field report must distinguish product guarantees, detector policy,
  experimental-v1 learning, and remaining CLI/hardening work.

## Invalidation conditions

Stop and report `decision-needed` if implementation requires:

- a new or weakened public Application Client semantic;
- CLI parsing, raw daemon IPC, a private client, or an implicit fallback;
- changing fixed route, runtime-owned identity, durable-acceptance commit, or
  unresolved-receipt safety direction;
- provider semantics or provider mutation in runtime code;
- remote executable registration, sandbox, multi-host, packaging, or service
  authority;
- automatic v1 execution or migration;
- a split without an independently useful boundary and explicit join
  conditions;
- destructive testing against shared user or campaign coordination state; or
- launch before conformance merge, both exact-head attestations, and the
  reconciled consumer gate.
