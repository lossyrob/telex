# Telex Watcher Current Design

This document is the canonical integrated design for the Telex Watcher
workstream. Project-level documents under `docs/design/` remain authoritative
for their contracts; this document records how those contracts, accepted
decisions, workstream boundaries, and downstream gates fit together.

## Authority

The current design is grounded in:

- the merged [Watcher production contract](../../../../docs/design/watcher.md);
- [ADR 0046](../../../../docs/design/DECISIONS.md#0046--watcher-runs-provider-neutral-trusted-local-detectors-with-receipt-gated-state),
  retaining the provider-neutral, trusted-local, fixed-route, receipt-gated
  architecture;
- [ADR 0050](../../../../docs/design/DECISIONS.md#0050--watcher-v2-uses-minimal-command-registration-and-runtime-owned-event-identity),
  superseding ADR 0046's mandatory authoring ceremony and detector-authored
  identity;
- the [Application Client contract](../../../../docs/design/application-client.md),
  which owns the supported Telex integration seam;
- the [campaign roadmap](../../../shaping/roadmap.md), [workstream brief](../brief.md),
  [graph](../graph.json), and [reconciliation note](../reconciliation-note.md).

The minimal v2 contract was promoted through
[issue #133](https://github.com/lossyrob/telex/issues/133) and
[PR #135](https://github.com/lossyrob/telex/pull/135), merged as
`b91e8301899351c0411d6e2e9ac5290af8a3cb4c`.

If this summary conflicts with a normative project design document, the project
design document wins and this file must be reconciled.

## Intended outcome

Telex Watcher is a separately supervised, per-user, headless application that
runs trusted local observational commands outside agent sessions and emits
durable Telex messages:

```text
external condition
      -> trusted agent-authored detector
      -> Telex Watcher
      -> fixed-route durable Telex send
      -> responsible agent or human-attended station
```

An agent can write or copy a small detector, optionally exercise it, register
its command plus generic execution and routing policy, and leave it running
without retaining a session-owned waiter or polling task.

## Responsibility boundary

Watcher owns:

- persistent local registration, lifecycle, opaque state, attempts, pending
  operations, recurrence evidence, diagnostics, and health;
- generic cadence, timeout, concurrency, retry, backoff, process containment,
  output, and retention bounds;
- immutable backend profile, sender, and target for one permanent watch ID;
- stable runtime identity and send-only sender membership;
- runtime allocation of event sequence, event ID, and exact send operation ID;
- durable staging, receipt reconciliation, and atomic receipt-gated state
  commit; and
- one reaction only: sending a Telex message.

The detector owns provider and repository semantics, event kind and content,
cursor and replay behavior, recurrence intent, and provider-specific gap or
downtime policy. On macOS the detector also owns conformance to the
non-detaching execution contract: its root and every local descendant must
remain in the Watcher-assigned attempt process group. Watcher owns assignment
and cleanup of that group. Windows and Linux retain their stronger
detached-descendant containment guarantees. Telex owns durable transport,
receipt axes, address semantics, and disposition. The recipient owns all
consequential action after wakeup.

Watcher does not become a workflow engine, mutate providers, launch agents,
host human UX, accept remote executable registration, claim to sandbox
same-user code, provide hosted webhook ingestion, or define a private Telex
client API.

## Minimal v2 authoring contract

Ordinary registration requires only:

- a permanent watch ID;
- an argv command;
- cadence and timeout;
- an explicit backend profile;
- a stable sender; and
- a fixed target.

Attention, disposition, working directory, inherited environment names,
parameters, and initial opaque state have defaults or optional generic
semantics. Watcher executes argv directly without implied shell interpolation.
Registration and executable mutation remain local-only.

Manifests, script digests or pinning, event-kind allowlists, provider preflight,
cursor or downtime declarations, fixtures, conformance suites, provider
budgets, and template frameworks are optional project choices. They may live in
detector code, wrappers, examples, tests, or separately versioned tooling, but
they are not fields or prerequisites of the v2 runtime contract.

## Identity and receipt transaction

A watch ID is a permanent registry namespace. Removed IDs are tombstoned and
never reused. Within that namespace, Watcher allocates a strictly increasing,
never-reused event sequence, a runtime-generated event ID bound one-to-one to
that sequence, and a restart-stable Application Client operation ID for the
exact send.

Before sending, Watcher durably stages the complete operation: identities,
registration revision, logical-store binding, fixed route and policy, prior and
proposed state, normalized payload, canonical hashes, and bounded recovery
budgets. Retry never changes the staged identity, route, policy, payload, or
state transition.

Event-producing state advances only after typed durable Telex acceptance. It
does not advance from occupancy, push, transport consumption, workflow
disposition, timeout, or an assumed outcome. An indeterminate send blocks new
detector execution while Watcher reconciles the exact staged operation. Watcher
never blindly resends, forces a local commit, or discards unresolved evidence
while keeping the watch active.

After a successful commit, a later detector event is a new recurrence with a
new sequence and event ID even when its content is identical. Watcher exposes
recurrence and duplicate evidence but does not infer provider intent or suppress
events by content.

## Version and migration boundary

The canonical v2 machine contracts are:

- [registration v2](../../../../docs/design/schemas/watcher-registration-v2.schema.json);
- [detector request v2](../../../../docs/design/schemas/watcher-detector-request-v2.schema.json);
- [detector result v2](../../../../docs/design/schemas/watcher-detector-result-v2.schema.json).

The v1 schemas and spike implementation remain frozen historical evidence. A v2
runtime does not silently execute a v1 registration or accept a v1 result.
Migration requires a new v2 watch ID, a detector update or wrapper, an explicit
opaque-state decision, and preservation of the old tombstone. No event-ID,
cursor, or script-provenance translation is implied.

## Application Client seam

Production Watcher consumes the shared Application Client and has no private
fallback. Application Client issue #12 publication revision 3 and product merge
`4ecbe84e99e00ab0cea3bcf3619d539c222746af` close the former AC-C14 semantic
gap. The supported core now returns authoritative exact-store,
exact-operation `NotRecorded`, permits only identity-checkable retry of the
exact same operation after that result, and returns
`RetentionBoundaryCrossed` when absence can no longer be proven. A retention
boundary never authorizes retry.

The supported public Rust binding and `client-conformance` proof across SQLite
and credentialed Postgres are merged. Watcher's formal post-merge
`consumer-integration-gate` passed against exact merged main `4d25fdff` and is
reconciled complete on `main`. This completion does not weaken recovery: when
acceptance is uncertain, Watcher remains reconciliation-first and query-only;
only authoritative exact-operation `NotRecorded` permits bounded
exact-same-operation retry, and `RetentionBoundaryCrossed` never authorizes
retry. Production Watcher has no CLI-output, raw daemon IPC, or private-client
fallback.

## Acceptance and dependency boundaries

The operator accepted `dumb-watcher-contract-gate` as contract-usability
acceptance: the minimal v2 design supports the intended short authoring path
without mandatory framework ceremony. That acceptance completes the
`minimal-contract-accepted` checkpoint. It does not prove a production runtime,
an operational five-minute workflow, or any later gate.

`watcher-runtime-core` is in progress. On 2026-10-22 the operator approved its
launch through the Streamliner workflow v2 adapter. It runs under issue #162 and
`.streamliner/workstreams/telex-watcher/tasks/watcher-runtime-core.md`, its
reviewed and reconciled tracker and task specification. Both Application Client
dependencies are satisfied. For this node only, the operator scoped process-tree
containment proof to Windows and Linux. macOS remains a required Watcher product
platform, but #162 must fail closed there before detector launch and must not
claim macOS support. The connected `watcher-macos-containment` contract item is
in progress under issue #165 and
`.streamliner/workstreams/telex-watcher/tasks/watcher-macos-containment.md`. The
connected `watcher-macos-containment-runtime` implementation item is in progress
under issue #172 and `tasks/watcher-macos-containment-runtime.md` in this
workstream. On 2026-10-23, under the operator's one-item, one-PR rule, the
design steward approved splitting the former #165 scope: #165 owns only the
normative macOS contract delivered by PR #168 in `docs/design/watcher.md` and
ADR 0054, and #172 owns macOS runtime implementation and proof. The inherited
native ARM64 and x64 Phase 1 investigation rejected recursive kqueue tracking
and ordinary process groups as solutions to the former arbitrary
detached-descendant contract. On 2026-10-22 the operator selected a revised
macOS contract for trusted non-detaching detectors. On macOS the detector root
and every local descendant must remain in the Watcher-assigned attempt process
group; work that escapes that group is outside the cleanup guarantee. This is a
trusted behavioral precondition, not a sandbox claim, and it does not weaken the
existing Windows or Linux guarantees. #162 remains fail-closed on macOS and must
not claim macOS support. The `watcher-runtime-core` merge sequencing condition
for Phase 2 was satisfied when PR #164 merged as
`d0237dbb0b50dafd5518d403a5ff8d81cd5ee25c`; issue #162 does not need to close
before #172 implementation. On 2026-10-23 the design steward approved the
candidate Phase 2 proof shape, subject to corrections, and kept product
implementation on hold. Product implementation remains held until PR #168 merges
the clarified ADR 0054 and normative Watcher text, the Phase 2 plan incorporates
that disposition, pinned XNU source reasoning and native ARM64/x64 evidence
establish the fork/group-empty proof, and the owner returns the selected
algorithm and evidence for implementation authorization. At exact proof head
810bd24fe7ca3415a4b32c19b762ed32a68d952b, native ARM64 and x86_64 run
39904383638 rejected the one-shot SIGKILL completion candidate: a
barrier-controlled in-group descendant survived the accepted group SIGKILL and
answered two post-seal challenges one second apart. Both post-reap observations
reported presence, and ESRCH appeared only after private teardown. The result
preserves the pinned-source old-group closure argument and fail-closed receipt
rule but proves that one accepted group SIGKILL is not a completion condition.
Product implementation remains held. Any successor must retain the destructive
anchor and nonzero-signal authority until a separately justified completion
transition, then seal, exactly reap, and observe under the existing proof rules;
if no ordinary-user completion condition can be proved, return decision-needed.
After those gates are accepted, implementation may proceed under #172 without
waiting for #162's runtime repair, field report, or closure. The fail-closed
macOS behavior from #162 remains authoritative until the complete #172
implementation lands. An ordinary per-attempt private guard remains within
Watcher runtime authority only when it is unprivileged, ephemeral, shipped with
the runtime, controlled through inherited private handles, exposes no reusable
service endpoint, and exits after identity-matched cleanup evidence. Any
privileged, entitled, installed, persistent, shared, launchd-managed, brokered,
or VM-backed architecture returns to the operator. Campaign runs the merge gates
for both macOS items and merges at the exact cleared SHA. This host-scope
decision and follow-up launch do not clear #162's accepted store-binding
lifecycle/recovery blocker or premature `productionReady` blocker. At exact head
`40a5c916d839dc81432738f122e03a00fd31628e`, independent source review
`5620648207` and design inspection accepted the B2 store-binding
lifecycle/recovery repair and B3 `productionReady` reconciliation fence at
predecessor `d17100bfce6b17ccd03dff01fc66e3d8455cd812`. Independent final-delta
review `5620856808` then verified that the successor changed only two
documentation files and that all product source, tests, schemas, dependencies,
and workflow remained byte-identical; exact-head design inspection accepted
those documentation corrections at `40a5c916`. All 18 required CI jobs passed at
that exact head in run `39854080691`, attempt 2, after one retry of two shared
Windows failures tracked separately in #166 and #167. The operator merged
PR #164 at exact product head `40a5c916d839dc81432738f122e03a00fd31628e` on
2026-10-23 as main `d0237dbb0b50dafd5518d403a5ff8d81cd5ee25c`; issue #162
remains open. Under the steward's post-merge disposition, #162 owned a bounded
factual documentation correction linking the shipped v2 health and
event-metadata schemas, with no new tracker or ADR. PR #170 merged that
documentation correction at exact head
`6760bb2235584e744047ec3953395a4e804dba2b` on 2026-10-23 as main
`2513ccdac136a148c5603fd4a0e08962fb9e0929`. The field report must be posted as a
new comment on issue #162, and that comment's permalink becomes the durable
disposition link. Additional post-merge public-management validation found that
removal of an isolated unsupported v1 row can fail when its unchanged opaque
state is not v2-canonicalizable, because the audit path applies v2 state hashing
to every admitted mutation. The steward approved a bounded runtime repair
under #162: unsupported-row removal must preserve the opaque state, permanent
identity, tombstone, and evidence; skip v2 hashing through an explicit
schema-version branch; and audit that state hashes are not applicable. Strict v2
canonicalization remains unchanged. PR #170 remains independently valid. The
repair, exact-head review and CI, post-merge validation, #162 field report, and
final reconciliation remain outstanding. The `watcher-runtime-core` merge gate
runs in shadow mode, so the operator merges. `minimal-example-pack` completed
under issue #144 in PR #163. Campaign ran the first live merge-gate pilot and
cleared exact head `fdb4f52fa8e582cc97492787f0841e0b15af893f`, which the owner
merged as `5ad7eb04c1357044315e433bd12f34347b6a4cf6`. None of these launches
grants release, tag, publish, fleet, CLI, packaging, I161-1, or Operator Station
authority. The downstream `five-minute-custom-watch-gate` still owns operational
proof that an agent can create, register, leave running, and diagnose a custom
watch in five minutes.

`watcher-macos-containment-runtime` does not enter the dependency list of the
Windows-path `five-minute-custom-watch-gate`. It replaces
`watcher-macos-containment` in the product-level `usable-watcher` and
`closure-ready` checkpoints and as a dependency of `closure-gate`; the contract
item remains its prerequisite. The operator-selected contract makes macOS
support achievable without claiming arbitrary detached-descendant containment,
but fail-closed behavior remains the only accepted macOS runtime state
until #172 supplies proven implementation.

## Superseded assumptions

ADR 0050 supersedes ADR 0046 only for mandatory script provenance and pinning,
allowed-kind policy, registration-owned downtime declarations, required
template framework or conformance, and detector-authored event identity. ADR
0046 remains binding for provider-neutral trusted-local execution, local-only
registration, fixed routing, structured results, generic bounds and
diagnostics, no workflow actions, durable acceptance before state commit,
visible at-least-once duplicates, and the shared Application Client boundary.

The operator's 2026-10-22 decision in issue #165 directs the workstream to
supersede the prior cross-platform assumption that every supported Watcher host
must contain arbitrary detached descendants. The selected direction preserves
that guarantee on Windows and Linux and uses the narrower assigned-process-group
contract for trusted non-detaching detectors on macOS. The owner's pending
product-design change, PR #168, will record the decision as ADR 0054 in
`docs/design/DECISIONS.md` and update `docs/design/watcher.md`. Until that
change merges, the normative Watcher design still states the prior
requirement, #162 remains fail-closed on macOS, and no macOS support claim is
accepted.

Issue #127 and unmerged PR #131 remain optional-example and implementation
learning, not current product authority.

Issue #144 and PR #163 closed the minimal-example extraction question. The
selected scope is five standalone v2 examples for GitHub PRs, Azure DevOps
builds, HTTP/JSON, local files, and commands, with offline schema and provider
checks. They retain PR #131's useful boolean-typing, command-failure, and
offline transport-testing lessons without adopting its mandatory template,
provenance, kind-policy, preflight, downtime, or shared-framework machinery.
Optional provider and credential hardening remains downstream project policy,
not unresolved Watcher core scope.

## Unresolved design questions

- Whether an explicit v1 compatibility adapter is worth maintaining instead of
  documented re-registration.
