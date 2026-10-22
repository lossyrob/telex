# Watcher CLI and diagnostics UX

- **Workstream:** `telex-watcher`
- **Node:** `watcher-cli-ux`
- **Type:** implementation
- **Status:** shaped; planned and unlaunched
- **Attention:** watch
- **Depends on:** completed `watcher-runtime-core`
- **Tracker:** not yet created
- **Parent workstream:** [lossyrob/telex#100](https://github.com/lossyrob/telex/issues/100)
- **Campaign:** [Addressable Attention #102](https://github.com/lossyrob/telex/issues/102)

## Outcome

Provide a concise local management and diagnostics surface for the production
v2 Watcher runtime. An agent can register a trusted local command, inspect its
effective fixed policy, pause or resume it, make a safe update, recover a
pending send through explicit bounded operations, remove it without erasing
evidence, and diagnose runtime or watch failure without reading the registry or
parsing ad hoc logs.

This is a separate node from `watcher-runtime-core`. It consumes the completed
runtime registry, state machine, health model, and internal management API; it
does not reimplement them in command handlers.

The first practical usability target is replacing a session-owned GitHub PR
polling loop. That proof uses one detector-owned policy and does not launch the
optional issue #144 example pack or broaden Watcher into a provider framework.

## Authority

- [`../../../../docs/design/watcher.md`](../../../../docs/design/watcher.md) -
  management operations, lifecycle, refusal rules, health, and diagnostics.
- [`../design/current-design.md`](../design/current-design.md) - accepted
  authoring and product boundaries.
- [`../graph.json`](../graph.json) - `watcher-runtime-core` dependency and later
  five-minute gate.
- The completed [`watcher-runtime-core`](watcher-runtime-core.md) task and its
  merged exact behavior.
- The canonical v2 registration, detector, normalized metadata, and concrete
  runtime health schemas.

The experimental v1 command names may inform ergonomics, but they do not define
v2 behavior. The CLI must not preserve the current `CliTelexAdapter`, private
send-once behavior, v1 registry shape, or detector-authored event identity.

## Bundle rule

Keep the complete ordinary management path in one tracker and one PR. Do not
split commands, JSON output, pending-operation recovery, or diagnostics into
separate partial deliveries.

The node is not complete merely because basic add/list/run commands exist. It
must expose the accepted refusal, recovery, tombstone, health, and evidence
semantics needed to operate the runtime safely.

## Launch prerequisites

Do not create the CLI implementation worker until:

1. `watcher-runtime-core` is merged and reconciled as completed.
2. Its concrete local management API and v2 health/evidence schemas are
   available on `main`.
3. The CLI tracker and this task specification are reviewed and reconciled.
4. Launch uses a genuine new `gpt-6-astra` context with high reasoning and long
   context. Do not reuse the runtime implementation/reviewer, an initializer,
   or a Station product context.

Issue #144, broad packaging, runtime hardening, and Operator Station issue #153
are not prerequisites.

## Deliverables

### Command surface

Exact spelling may follow repository conventions, but the CLI must provide:

- register;
- list and show;
- pause and resume;
- update, including explicit committed-state replacement;
- reconcile now;
- grant a finite reconciliation budget;
- grant a finite exact-send retry budget;
- close a definitively not-accepted operation;
- remove;
- runtime and per-watch status/health;
- attempts and committed events; and
- pending-operation and removed-evidence inspection.

The runtime process entry point may remain separate from management commands.
Service installation and supervision are later work.

### Output and audit behavior

- Every command supports stable machine-readable JSON.
- Human output is concise and derived from the same typed result, not a second
  semantic path.
- Every mutation returns the resulting registration revision, lifecycle,
  eligibility, health, and typed refusal or recovery result.
- Mutation audit records identify the operation, prior and resulting revision,
  and state hashes where committed state changes.
- Show/list filters can explicitly include active, paused, terminal, removed,
  unsupported-v1, and unresolved-tombstone records.
- Status exposes runtime readiness, heartbeat/staleness, sender/store state,
  reconciliation, inbound backlog, retention warnings, and typed diagnostics.
- Watch inspection exposes sequence high-water marks, pending operation and
  budgets, recurrence, latest attempts, suppressed due ticks, sender readiness,
  and retained evidence.
- No command prints credentials, inherited secret values, raw daemon frames,
  private backend rows, or unrestricted detector state by default.

### Required operation semantics

#### Register

- Accept one v2 registration file or equivalent explicit local input.
- Apply canonical schema and runtime validation before persistence.
- Capture the current absolute working directory when omitted.
- Reject an existing or tombstoned ID.
- Never infer shell interpolation or accept remote executable registration.

#### Pause and resume

- Pause moves an active eligible watch to paused/inactive.
- Pause refuses while a pending operation exists; the operator must reconcile
  or remove instead.
- Resume replaces `activatedAt`, retains `lastSuccessAt`, and moves a corrected
  paused watch to active/eligible.
- Resume refuses unresolved pending evidence and uncorrected blocked reasons.

#### Update and state replacement

- Verify no pending operation exists in the same registry transaction that
  applies the update.
- Increment registration revision so update and event staging cannot race.
- Permit only mutable command, directory, cadence, timeout, attention,
  disposition, environment-name, parameter, and committed-state fields.
- Omitted state replacement preserves committed state. Explicit JSON `null`
  commits null.
- Reject immutable ID, backend, sender, or target changes, removed watches,
  unsupported v1 rows, and pending operations.
- Audit old and new canonical state hashes when state changes.

#### Reconciliation and finite grants

- `reconcile now` queries the exact staged operation for active, blocked, or
  removed watches and records typed evidence without changing identity.
- A finite reconciliation grant applies only to
  `receipt-outcome-unresolved`. It adds a bounded query budget and transitions
  to active/reconciliation-pending/degraded without authorizing a send.
- A finite send-retry grant applies only after matching authoritative
  `NotRecorded` or transient rejection has been recorded as
  `send-retry-exhausted`. It authorizes bounded retry of the exact same
  operation and refuses replacement identity, store, route, payload, or state.
- Neither grant requires a separate resume and neither replaces
  `activatedAt` or `lastSuccessAt`.
- `RetentionBoundaryCrossed`, unavailable history, mismatched payload, and
  mismatched store never authorize retry.

#### Close not accepted

- Apply only to a definitively permanent pre-acceptance rejection.
- Retain terminal rejection evidence and the allocated sequence tombstone,
  clear the pending operation without advancing detector state, and permit a
  later explicit resume at the next sequence after external correction.
- Refuse indeterminate, retained-boundary, mismatched, or potentially accepted
  outcomes.

#### Remove and evidence closure

- Remove stops future execution and retains the permanent ID, sequence
  high-water marks, pending operation, payload hashes, receipt bindings,
  tombstone, and evidence.
- Removal never asserts accepted or rejected and never makes the ID reusable.
- Later reconciliation may close accepted, rejected, or authoritative
  `NotRecorded` evidence on the removed tombstone without resurrecting it.

## First practical usability proof

After the CLI node is complete, use it to replace one session-owned GitHub PR
wait loop:

1. Prepare one trusted-local v2 detector for an owned or explicitly authorized
   PR.
2. Register its command, an accepted bounded cadence, timeout, backend profile,
   stable sender, and fixed target.
3. Use an explicit credential allowlist. Do not print or persist the token.
4. Establish a baseline without emitting a false actionable event.
5. Leave the runtime operating after the originating agent session ends.
6. Observe one authorized PR transition, receive the fixed-route Telex event,
   and verify committed receipt/state evidence.
7. Inspect status and attempts, exercise pause/resume or a safe update, and
   remove the watch while retaining its tombstone.
8. Confirm no session-owned Loop task or other poller remains.

Detector policy may cover external reviews/comments, requested changes, failed
checks, merge blockage, ready-to-merge, and terminal closure. It performs no
provider action. The proof must document exact coverage and exclusions. If
inline review-thread comments are not queried, it must not claim all review
activity.

This proof is the first practical target for the later
`five-minute-custom-watch-gate`; completing this node does not itself pass that
builder-owned gate.

## Out of scope

- Runtime registry, scheduler, process containment, sender lifecycle, or
  receipt state-machine reimplementation.
- Broad optional GitHub/Azure DevOps/HTTP/file/command examples from issue #144.
- Provider actions, workflow execution, agent launch, or remote administration.
- Mandatory manifests, pinning, digests, kind allowlists, provider preflight,
  downtime declarations, fixtures, or a prescribed detector framework.
- Automatic v1 import or compatibility behavior.
- Installer, service supervisor, release publication, updater integration,
  cross-host ownership, webhook ingestion, broad packaging, or hardening.
- A private Telex adapter, CLI parsing for Telex semantics, raw daemon IPC, or
  fallback when the public Application Client is unavailable.
- Streamliner state, workflow records, or orchestration artifacts in the
  product diff.

## Success criteria

- An agent can complete the ordinary flow: write or copy a command, optionally
  exercise it, register it, leave it running, and diagnose it later.
- Every required management operation has stable JSON, concise human output,
  audit evidence, and exact refusal tests.
- Pending-operation commands preserve identity and cannot convert uncertainty
  into assumed acceptance, rejection, or a replacement send.
- Removed and unsupported-v1 records remain discoverable through explicit
  filters.
- Runtime and watch health are sufficient to distinguish normal operation,
  detector degradation, sender unready, reconciliation pending, blocked
  receipt, collision, inbound backlog, containment uncertainty, and retention
  warning.
- The GitHub PR field proof runs without a session-owned waiter, provider
  mutation, private Telex seam, or issue #144 launch.
- Every material discovery has a durable disposition; no accepted node-owned
  CLI or diagnostics work remains deferred at merge readiness.

## Validation, review, and reporting

- Run formatting, warnings-denied Clippy, CLI parser tests, JSON snapshot or
  schema tests, runtime-backed command integration tests, and the repository
  feature matrix.
- Exercise every legal lifecycle transition and every required refusal,
  including pending-operation pause/update/resume rejection, immutable route
  change, tombstone reuse, unsupported v1 row, invalid grant, indeterminate
  close, and mismatched exact-operation retry.
- Run CLI integration against isolated SQLite and credentialed Postgres through
  the public Application Client-backed runtime.
- Use committed fixtures before any live GitHub proof. Live observation uses an
  owned or explicitly authorized PR and never mutates it.
- Establish one full implementation-review baseline on the first stable
  complete head, run required CI, and obtain a fresh exact-head design
  inspection before merge readiness.
- Final review, required CI, and design inspection must cover the same exact
  final head. Any head movement invalidates affected evidence and requires the
  corresponding delta or full rerun under campaign policy.
- The field report must distinguish CLI guarantees, detector policy, exact PR
  coverage/exclusions, and remaining five-minute, packaging, and hardening work.

## Invalidation conditions

Stop and report `decision-needed` if implementation requires:

- changing the accepted runtime lifecycle, pending-operation, identity,
  receipt, or tombstone semantics;
- a provider-specific command or provider action in the Watcher CLI;
- remote executable mutation or administration;
- a private Telex client, raw daemon IPC, CLI-output parsing, or fallback;
- automatic v1 migration;
- packaging or service authority to make the management operations function;
- claiming issue #144, the five-minute gate, or broad runtime hardening complete;
- a split without an independently useful boundary and explicit join
  conditions; or
- launch before the completed and reconciled `watcher-runtime-core`.
