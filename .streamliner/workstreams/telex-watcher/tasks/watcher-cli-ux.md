# Watcher CLI and diagnostics UX

- **Workstream:** `telex-watcher`
- **Node:** `watcher-cli-ux`
- **Type:** implementation
- **Status:** in progress; existing owner session
  `c90bafbb-6e49-4ced-a684-e15cfadfc69e` retained
- **Attention:** focus
- **Depends on:** completed `watcher-runtime-core`
- **External dependency:** satisfied Local
  `local-daemon/cooperative-partial-page-repair`, tracker
  [#181](https://github.com/lossyrob/telex/issues/181)
- **PR:** [#180](https://github.com/lossyrob/telex/pull/180), retained in place
- **Tracker:** [lossyrob/telex#179](https://github.com/lossyrob/telex/issues/179)
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
4. Launch uses a genuine new context with the current v2 launch adapter
   profile (`gpt-6.1-sol`, high reasoning, default context tier, per the
   2026-10-23 operator directive). Do not reuse the runtime
   implementation/reviewer, an initializer, or a Station product context.

Issue #144, broad packaging, runtime hardening, and Operator Station issue #153
are not prerequisites.

Prerequisite 1 is met: `watcher-runtime-core` completed on 2026-10-23, so the
original runtime-core graph dependency is satisfied. The Watcher coordinator
opened tracker issue #179 and launched owner session
`c90bafbb-6e49-4ced-a684-e15cfadfc69e` on 2026-10-23 from main
`6dc79e40dfacba8c1e71ac46ef1be376d43a50ad`. The launch used the
`gpt-6.1-sol` / high reasoning / default context tier profile. The node launched
in progress with watch attention. The launch binding record is
`watcher-cli-ux-20261023.json` under
`streamliner/telex/prompts/v2/launches/` in the planning repository, with
kickoff SHA-256 `71f5b924`.

### Pending Local repair dependency

On 2026-10-23 Campaign authorized one separate Local repair,
`cooperative-partial-page-repair`, tracked by
[#181](https://github.com/lossyrob/telex/issues/181), and made it an upstream
blocker of this node. Delivery is blocked with watch attention; PR #180 and
its existing owner remain in place. The root repair is not absorbed into
this PR and changes no Watcher implementation scope or product semantics.

The pending external dependency clears only after the Local repair is
merged with exact-head hosted validation and Campaign acceptance, then
reconciled from actual terminal evidence. Existing review, design and
field-proof evidence keeps its exact-head coverage and limitations; this
dependency neither discards it nor upgrades it to current-head acceptance.
PR #180's own current-head review, design, required CI and live owned-PR
proof remain required. No check waiver, third unchanged-head retry,
hardening acceptance or immediate merge is authorized.

### Accepted Local delivery and resumed focus - 2026-10-25

Campaign expressly accepts #181 delivery and releases only PR #180's
Local prerequisite after actual #185 merge
`65f5846d50bc4c07b3eaf9ffed0800f4cb19a17b`,
merged-head main CI `40217242404` with 24/24 and the full
[field report](
https://github.com/lossyrob/telex/issues/181#issuecomment-6401070057).
Owning Local independently accepts the same bounded outcome after its
complete provider CI/Docs and retained Windows/macOS checks; terminal
qualification is 13,796 bytes, SHA-256
`efa69b137efb54b6de605cd72165eb9d407374bdf9df3e17f99dc675813f0f2d`.
The report's original submission language is historical, not acceptance
inferred from posting, issue closure or merge alone.

The owning Watcher coordinator separately changes #179 from blocked/watch
to in-progress/focus. This attention decision comes from Watcher, not
from Local's original preserve-watch snapshot. Local's later 2,185-byte
scope supplement, SHA-256
`dc36573c964aac911ddecdc8f27b987b8c6c025fa9ecf5ec50b66bdeb092aada`,
qualifies only that attention instruction without rewriting the packet.
The existing Local external-dependency edge becomes satisfied and stays
in place with the same identity/target/label/URL. Completed runtime-core,
owner `c90bafbb-6e49-4ced-a684-e15cfadfc69e`, tracker and PR #180 remain.

This clears one upstream prerequisite. It does not make PR #180 ready,
merged or accepted; its owner retains fresh exact-head source/design/CI,
field-proof and Campaign obligations after integration. The transition's
reported old head is not new-head validation. The old blocked/pending and
watch-attention text is retained as historical. Marker candidates remain
separate. Local real-time 200ms coverage/unknown-cause/new-item-not-retry,
sentinel/Clippy and publication debts remain; no new implementation,
runtime/CI/retry/observer, release, #153, hardening/closure or freeze
authority follows. The owner does not wait for canonical publication.

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

### Local management transport

The runtime's management handle is in-process, and the runtime holds exclusive
registry ownership, so a separate CLI process cannot open the registry while
the runtime runs. This node adds a local management transport between the CLI
and the owning runtime process:

- The owning runtime exposes one local endpoint per registry, created only
  after it acquires registry ownership and removed or invalidated when
  ownership ends. The CLI is a thin client; it never opens or locks the
  registry while a runtime owns it.
- The endpoint executes the existing typed management `Command` through
  `LocalManagement`. Versioned JSON carries a typed, allowlisted safe
  projection of the owner's unchanged `Response`, rather than requiring the
  raw in-process response to cross the wire. The owning process applies that
  projection and the accepted read filters before response serialization and
  the frame byte cap. A bounded, versioned request envelope may carry
  presentation-only selectors for those existing read filters; they add no
  management operation or mutation authority. The projection preserves all
  required public results, refusal/recovery semantics, revisions, lifecycle,
  eligibility, health, and audit/evidence fields. Human and machine-readable
  output derive from the same projected result. It adds no operation,
  mutation, or bypass that `LocalManagement` lacks, and every mutation still
  runs through the registry owner's transactions.
- Access is limited to the same local OS user as the runtime: a Unix domain
  socket in a user-private directory with owner-only permissions, or a Windows
  named pipe with an owner-only DACL that rejects remote clients. The endpoint
  is never reachable over the network or through Telex messages, and it never
  accepts remote executable registration.
- The endpoint location derives from the canonical registry identity, so path
  aliases reach the same runtime. A stale endpoint whose owner has gone yields
  a typed `runtime-not-running` result, never a hang or a silent fallback.
- With no owning runtime, mutation commands fail with typed
  `runtime-not-running` unless the implementation provides an explicit offline
  mode that acquires registry ownership itself and runs the same in-process
  handle; any such mode must refuse while another owner exists and must not
  connect a sender.
- Bounded request size, per-request timeout, and concurrent-client limits
  apply. A dropped client does not roll back an admitted mutation, matching
  `LocalManagement`.

Document the endpoint, its authorization boundary, and the not-running
behavior in `docs/watcher-runtime.md` and `docs/design/watcher.md`, and record
the decision as a new ADR in `docs/design/DECISIONS.md`. This is local
same-user administration, not a remote administration protocol, a service, or
an installer.

The steward added this subsection on 2026-10-23 when approving the tracker
body; evidence: on main, `telex-watcher/src/management.rs` exposes only an
in-process handle, `ownership.rs` and `registry_v2.rs` hold the registry
under an exclusive OS lock, and `docs/watcher-runtime.md` says the handle is
not a remote administration protocol.

The steward clarified the response boundary on 2026-10-23 after PR #180 review
found that raw response framing could reject accepted nested state and large
raw records before safe projection or filtering; evidence:
https://github.com/lossyrob/telex/pull/180#issuecomment-6372224177. This permits
endpoint-side typed projection without changing runtime management semantics.

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

### Runtime-core documentation clarifications

The one item PR must make these shipped v2 behaviors explicit in the user-facing
Watcher documentation and machine-readable management guidance. This is
clarification only; it must not change runtime, schema, numeric admission,
canonicalization, hashing, evidence, removal, or migration semantics.

- **Detector request transport:** Watcher writes the v2 detector request to
  detector stdin as ordinary valid JSON. The transport bytes are not required to
  be RFC 8785/JCS-canonical. Existing strict request validation, I-JSON and
  canonicalizability requirements, schema and byte limits, and admitted numeric
  model remain unchanged. RFC 8785 canonical JSON remains authoritative for
  hashes and durable evidence and must not be substituted for detector-request
  transport.
- **Unsupported-v1 removal audit:** After the existing rules admit `Remove` for
  an unsupported-v1 record, Watcher retains the opaque state and permanent
  identity/evidence without applying v2 state hashing. The removal audit reports
  `stateChanged: false`, `stateHashStatus: "not-applicable"`, and
  `stateHashReason: "unsupported-registration-version"`, and omits
  `oldStateHash` and `newStateHash` entirely. It must not emit null hash keys, a
  legacy digest, or fallback hash semantics. Other unsupported-v1 mutations
  remain refused, and strict v2 hashing and removal audit behavior remain
  unchanged.

Evidence: issue #162 comments `6358391843` and `6358767711`, implemented by
PR #173 and merged as main `55cb6b116607bceccf3ce0b72550e2d849ca5426`.

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
5. Start the runtime as an ordinary detached user process on a host with
   claimed containment (Windows or Linux; macOS fails closed until
   `watcher-macos-containment-runtime` lands), and leave it operating after
   the originating agent session ends. No installer, service registration,
   or autostart is used; document the exact start and stop commands.
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
- Detector authors and operators can distinguish request transport from
  canonical hash/evidence encoding, and unsupported-v1 removal output exposes
  the exact non-applicable audit semantics without implying migration, state
  change, or relaxed v2 validation.

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
- Test the local management transport: same-user success, refusal of
  another local user where the platform allows it in CI or a documented
  local check, stale-endpoint `runtime-not-running`, alias paths reaching the
  same owner, oversize and slow-client bounds, and that no command opens the
  registry while a runtime owns it.
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
- Validate the documentation and CLI JSON examples against the shipped runtime:
  distinguish ordinary detector-request wire bytes from RFC 8785 hash/evidence
  bytes, and snapshot the exact unsupported-v1 `Remove` audit shape including
  omission of both hash keys.

## Invalidation conditions

Stop and report `decision-needed` if implementation requires:

- changing the accepted runtime lifecycle, pending-operation, identity,
  receipt, or tombstone semantics;
- a provider-specific command or provider action in the Watcher CLI;
- remote executable mutation or administration;
- a management transport reachable beyond the same local OS user, carried
  over Telex messages, or exposing operations beyond the typed management
  `Command` set;
- a private Telex client, raw daemon IPC, CLI-output parsing, or fallback;
- automatic v1 migration;
- packaging or service authority to make the management operations function;
- claiming issue #144, the five-minute gate, or broad runtime hardening complete;
- a split without an independently useful boundary and explicit join
  conditions; or
- launch before the completed and reconciled `watcher-runtime-core`.

## Accepted terminal delivery - 2026-10-25

Campaign accepted bounded #179 delivery after PR #180 merged as
`0d5347a6e1b8a1716b524c537c4886d3f4d49b65`, exact-main CI passed 21/21
and the owner published the
[final field report](
https://github.com/lossyrob/telex/issues/179#issuecomment-6401968111).
Owning Watcher separately directs completed status with watch attention.
Owner `c90bafbb-6e49-4ced-a684-e15cfadfc69e` is finished and retained as
historical owner; no replacement or new launch follows. Earlier
pending, blocked, in-progress and readiness statements above are snapshots,
not the current terminal disposition. Their policy and evidence limits
remain in force.

### Distinct readiness, proof, gate and terminal facts

**Owner readiness, not merge authority.** The
[ready packet](
https://github.com/lossyrob/telex/pull/180#issuecomment-6401567213)
reports source `05d48500d47418e5cffddfcc619d7d6117af784a`,
[required run 40220250174](
https://github.com/lossyrob/telex/actions/runs/40220250174) successful
21/21, fresh reviewer-owned source/R17 and design receipts. The full
Windows job executed provider merge
`f909b296dcc1a2c11edaf65eb983e173ef023f75`; this is not direct
source-head execution. Dedicated boundary jobs separately verified the
direct source head. Readiness requested Campaign's gate, not self-merge.

**Proof applicability, not a rerun.** The isolated proof actually ran at
`2a04cd1a5ec48578efeefe739f8c9ce91691575f`. Campaign's
[explicit applicability decision](
https://github.com/lossyrob/telex/pull/180#issuecomment-6401339475)
carries it to `05d48500` on product-tree equivalence across exactly seven
CI, CONTRIBUTING and Local-ledger paths. The proof stays an execution at
its original head; old reviews or CI are not silently carried forward.
No new local or hosted field proof was directed. Receipt-time push,
recipient consumption and workflow disposition remain distinct axes.

**Campaign gate and operator approval.** The
[public gate](
https://github.com/lossyrob/telex/pull/180#issuecomment-6401765994)
cleared exact source `05d48500` as **merge + debt (medium-high)**. The
security/ACL hard stop went to the operator. Campaign reports the actual
ask_user choice "Merge it (Recommended)" at about 18:53Z, under section 2
of the v2 merge gate; planning ledger commit `dd857a7` records the gate.
The public gate, operator decision and technical readiness are separate
facts. Operator approval does not establish missing cross-user evidence.

**Actual merge.** PR #180 merged on 2026-10-25 as
`0d5347a6e1b8a1716b524c537c4886d3f4d49b65`; Campaign reports use of
`--match-head-commit 05d48500`. Merge and issue closure alone are not
delivery acceptance.

**Merged-main CI.** The owner reports
[run 40223537702](
https://github.com/lossyrob/telex/actions/runs/40223537702), attempt 1 at
that exact merge, completed 21/21 successfully, observed at
`2026-10-25T19:12:23.746Z`. Its full Windows job checked out exact main,
not a PR provider merge. This is the field report's hosted observation,
not a new reconciler log inspection, retry or runtime execution.

**Field report and accepted delivery.** The report submitted acceptance
separately and retained the gate's debts. Campaign subsequently accepted
terminal delivery directly at `0d5347a6`; the owner reports Campaign's
acceptance at 19:13:53Z. Owning Watcher then separately completed #179.
No owned proof process or CI observer remains, according to the owner.
Design impact is `updated-docs`: the product/design references and ADR
0055 already landed through PR #180. This artifact section changes none
of those documents.

### Accepted debt retained without upgrading evidence

The field report's four debt paragraphs are retained below, with line
wrapping only:

1. Different-OS-user refusal has **not been exercised at runtime**.
   Access-rule assertions, same-user behavior and manual-check guidance
   do not substitute for an actual cross-user connection/refusal test.
2. The PR's recorded consequence is `normal`; Campaign says it arguably
   should be `high`. The security/ACL stop therefore went to the operator,
   who approved the merge. This report does not silently relabel that
   history.
3. Reviews since `8826fad7` have been delta-only and from the same model
   family as the owner. Exact-head reviewer-owned receipts exist; they do
   not supply cross-family or fresh full-diff assurance.
4. The reviewer's earlier workstation `ApplicationClient::connect` setup
   observation remains unconfirmed and excluded from validation. Prior
   owner local test shells also lacked imposed outer TELEX overrides.
   Those execution disclosures are retained, not the validation basis or
   a diagnosis. If the connection observation recurs in hosted evidence,
   Local owns attribution.

The successful main job also passed the #193 seed candidate, but does not
establish the cause or resolution of Local's earlier refusal. Separate
Local #193 remains Local-owned; no retry, skipped assertion, lock-policy
change, flake exception or shared-component repair is inferred.

### Readiness consequence and unchanged closure

Both dependencies of `five-minute-custom-watch-gate` are now completed.
Its status stays planned and attention stays focus: ready is derived,
not a new status value. It is pending with the operator/Campaign, with
no Watcher owner or launch. Completing #179 does not pass that usability
gate or complete the product-level usable checkpoint.

`closure-gate` still depends on `runtime-hardening` and
`watcher-macos-containment-runtime` (#172). The #172 HOLD, all other
nodes/checkpoints/edges, the Watcher ledger and every Local record remain
unchanged. No packaging, service installation, hardening/admission,
release, debt clearance or workstation-freeze waiver follows.
