# Repair bounded persistent-lock acquisition on macOS

- **Workstream:** `local-daemon`
- **Node:** `macos-persistent-lock-repair`
- **Type:** implementation
- **Status:** in progress; one v2 owner authorized, preparation and launch
  pending canonical landing and Local verification
- **Attention:** focus
- **Depends on:** none
- **Blocks:** `local-daemon/cooperative-partial-page-repair`, issue #181 /
  draft PR #185; transitively Watcher #179 / PR #180 through its existing
  dependency
- **Tracker:** [#187](https://github.com/lossyrob/telex/issues/187)
- **Owner coordination:** Local `aeae4fa9-c5fc-4da9-b60e-bcf5b1a98f57`;
  one new worker authorized, concrete identity pending
- **Parent workstream:** [#32](https://github.com/lossyrob/telex/issues/32)
- **Campaign:**
  [Addressable Attention #102](https://github.com/lossyrob/telex/issues/102)

## Outcome

Diagnose the OS-lock acquisition and timing-test boundary, then make the
minimal evidence-backed source or test-precondition correction. Preserve
persistent owner-private locks, non-theft/release behavior and bounded
acquisition semantics. Deliver one separate repair PR that closes issue
#187 and returns to the Campaign gate.

## Authority

Campaign `e06813c4-1db2-4d77-b8d3-3addc6a1824c` authorized this bounded
repair on 2026-10-24T17:25:59.555-04:00. Local supplied the directive and
complete scope in `issue187-active-repair-promotion-20261024.json`.

> Authorized: one separate bounded Local repair for
> local-daemon-macos-persistent-lock-acquisition-time-bound, using the scope
> you proposed. Diagnose the OS-lock acquisition and timing-test boundary
> first. Make the minimal evidence-backed source or test-precondition
> correction, and preserve persistent owner-private locks, non-theft/release
> behavior and bounded acquisition semantics. Then validate on hosted macOS
> and the other relevant required jobs at the exact head. No further retry,
> skip or threshold workaround. Apply the discovered-work rule: promote it
> through the reconciler to an issue and node that its PR closes, upstream
> of #181. PR #185 stays draft and unchanged. Launch the owner with the v2
> adapter: gpt-6.1-sol, high reasoning, default context, interactive, under
> the unchanged freeze. Its PR comes to the Campaign gate.

This authorizes one separate repair issue/node/owner and the direct
upstream dependency of #181. It grants no merge, runtime-isolation,
hardening/closure or issue #153 acceptance. Earlier no-promotion and
conditional-retry snapshots retain their historical meaning; they do not
override this later directive or authorize another retry.

## Inputs and evidence limits

- Issue #187 and Campaign's directive above.
- Existing discovery
  `local-daemon-macos-persistent-lock-acquisition-time-bound` in
  [`../discovered-work.json`](../discovered-work.json), including all
  retained source, custody, terminal, retry and recurrence history.
- Run `40026777909`, direct source head
  `7b11b92630d0b2cf63423a77fc19232aff368825`: attempt 1 job
  `120664829096`, 58 passed / 1 failed, module duration 14.16 seconds;
  authorized attempt 2 job `120677596239`, 58 passed / 1 failed, module
  duration 4.61 seconds.
- Original full log: 68,750 bytes, SHA-256
  `f306e9dc28502d6b040171dd6a2338cd5f1b0e88da97e32e3956674396eaa1ed`.
  Retry full log: 54,733 bytes, SHA-256
  `a9b994da7f3fab26df4c44f36681d517dae89beb548536b9dc99d60146fae221`.
  Both identify direct checkout and explicit verification of `7b11b926`,
  not a provider test merge. Distinct formatted captures remain separate.
- Owner evidence [comment 6375263838](
  https://github.com/lossyrob/telex/pull/185#issuecomment-6375263838) and
  [comment 6375688185](
  https://github.com/lossyrob/telex/pull/185#issuecomment-6375688185).
  Local read these and relevant retained-log excerpts; Local did not
  independently reproduce the failure.
- Failure:
  `station_intent::tests::persistent_os_lock_is_bounded_and_never_removed`.
  The failing assertion is `src/station_intent.rs:4979` at `7b11b926`.
  It checks less than 1.54 seconds using the 20ms retry / 25-attempt
  constants. Actual acquisition elapsed is not printed; 14.16 and 4.61
  seconds are module durations. The preceding contention-error assertion
  passed; later persistence/release/reacquisition assertions were not
  reached. Five relevant named passes do not credit skipped repetitions.
- The full named test body is unchanged between PR #185 base `36155b1` and
  `7b11b926`. Lock implementation/constants unchangedness is owner-reported.
  Repetition establishes the assertion recurred, not scheduler-delay
  causality, lock theft or a specific product defect.
- Normative `docs/design/daemon.md`, `docs/design/DECISIONS.md` and Local
  `current-design.md`. Main includes the separate sentinel PR #184 fixture
  build reuse, not PR #185; do not copy or absorb that unmerged repair.

Owner-reported source review and the retained design-only approval do not
waive required CI. The single named-job unchanged-head retry allowance is
consumed and exhausted. Neither failure is superseded as successful;
required CI is not accepted green, and no skipped repetitions are credited.
The full job/alias inventory and scheduler lifecycle remain attributed to
the coordinator; Local's bounded identity/excerpt checks are distinct.

## Diagnosis

Diagnose the OS-lock acquisition and timing-test preconditions before
selecting a fix. Record a bounded discriminating account that separates
observations from hypotheses. Explain why the minimal correction satisfies
the existing lock contract rather than merely making a timing assertion
pass. Do not infer a cause from module duration or prior source approval.

## Boundaries

### In scope

- Bounded source/evidence diagnosis of OS-lock acquisition and timing-test
  preconditions before selecting a correction.
- Minimal evidence-backed product source or test-precondition correction,
  directly related regression coverage and documentation.
- Hosted macOS and other relevant required validation at the exact final
  head, preserving truthful source/test-merge checkout provenance.
- One implementation owner in one isolated App worktree, one separate PR
  closing #187, independent source review/design inspection and Campaign
  gate handoff.

### Out of scope

- Further unchanged-head retry, skip, waiver or threshold workaround for
  the recorded failures; no unsupported timeout/grace/tolerance increase.
- Editing PR #185 or absorbing its separate partial-page fix. Its existing
  owner preserves the draft unchanged.
- Watcher changes, new recording-marker or optional lint scope, Windows
  delayed-writer repair, issue #153 transactional authority or broad
  lock/persistence redesign.
- Release/tag, runtime-isolation clearance, hardening/workstream closure or
  merge authority.

## Invariants

- Persistent owner-private lock files and live-holder exclusion/non-theft
  remain.
- Release/reacquisition behavior and bounded acquisition semantics remain;
  no unsupported grace/tolerance or threshold workaround.
- Partial-page single-slot admission, one absolute D+grace,
  cancellation/fencing and mandatory-page contracts remain.
- Meaningful bounded-failure, persistence and release assertions remain;
  do not skip checks or accept overrun as successful validation.
- Do not claim an async timeout can interrupt an already-entered blocking
  OS/filesystem call.

## Success criteria

- A precise evidence-backed diagnosis explains why the minimal correction
  satisfies the existing lock contract rather than merely passing a timing
  assertion.
- Meaningful regression coverage retains contention, bounded failure,
  persistence and release/reacquisition behavior with valid preconditions.
- Hosted macOS and relevant required jobs pass at the exact final head with
  truthful source/test-merge identities. Historical passing subsets or a
  retry do not substitute.
- Independent source review and exact-head design inspection resolve
  outcome-blocking findings; the separate repair PR explicitly
  `Closes #187` and goes to the Campaign gate.
- Accepted delivery and its field report permit the existing #181 owner
  to resume. Neither #181 nor its downstream Watcher dependency is
  automatically complete.

## Workstation freeze

Local read-only diagnosis, ordinary isolated worktree edits/commits/pushes,
preparation artifacts and compile-only checks are permitted. No local
tests/runtime/proofs, daemon or test-child control, ACL/inheritance,
registry/PageHeap/WER, proof fixture-root creation/restoration/cleanup or
installed/user/campaign state actions. Runtime validation belongs to
authorized hosted disposable CI. This task does not grant runtime-isolation
clearance.

## Launch - pending Local verification

One owner is authorized but not yet created. After canonical
issue/node/task landing and Local verification, Local runs the v2 App
launch adapter:

1. Create one held interactive worktree owner with `gpt-6.1-sol` / high
   reasoning / default context and no `notify_on_idle`; bind it immediately.
2. Run packet 05 through one general-purpose Task in that exact checkout,
   using the full v2 owner guide plus Local domain notes.
3. Verify installed context, handoff and hash, deliver the real kickoff
   once, and persist the launched binding.

These are requested profiles and future actions, not effective runtime
verification or a launch receipt. No API/SDK preparation, v1
session-online/write-authorized handshake, sibling worktree, duplicate
owner or hand-written preparation bypass. The reconciler does not launch
the owner.

## Workflow

Use paw-lite for planning, implementation and testing under the v2 owner
guide, subject to the freeze. New initializer/owner/reviewer profiles are
`gpt-6.1-sol` / high / default; the existing steward remains unchanged.
The owner stays interactive and ordinary messages omit mode.

Source review and required exact-head design inspection may proceed in
parallel with hosted CI at the same final head. Use the existing wait
coordinator for registered external CI waits; no duplicate observer or
polling of App review/design completion. The freeze and no-further-retry
directive override older generic guide defaults.

## Ownership and handoff

Local `aeae4fa9-c5fc-4da9-b60e-bcf5b1a98f57` coordinates one new owner.
Steward `af271672-5d33-418c-bb9c-0f4d4570c7d7` handles design; reconciler
`b737a932-c687-4b95-b273-8335a59f6e89` alone writes canonical geometry.
Existing #181 owner `9d4fee6c-63a6-4aa7-b384-a4df1cbcbb4c` keeps PR #185
draft and unchanged. Campaign `e06813c4-1db2-4d77-b8d3-3addc6a1824c` owns
the final gate.

Return the complete separate PR, explicit `Closes #187`, diagnosis,
regression coverage, exact-head hosted results and independent
review/design receipts to Campaign. Only accepted terminal delivery and a
field report permit reconsidering the #181 prerequisite. No direct Watcher
edge is added: its existing dependency reaches #187 transitively through
#181. Source approval, retry, launch or issue closure alone does not
establish accepted delivery, merge or any hardening/closure gate.
