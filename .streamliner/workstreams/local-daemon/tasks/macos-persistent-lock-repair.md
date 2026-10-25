# Repair bounded persistent-lock acquisition on macOS

- **Workstream:** `local-daemon`
- **Node:** `macos-persistent-lock-repair`
- **Type:** implementation
- **Status:** in progress; OPEN/non-draft PR ready for operator;
  exact-head source/design/hosted evidence aligned; delivery/gates pending
- **Attention:** focus
- **Depends on:** none
- **Blocks:** `local-daemon/cooperative-partial-page-repair`, issue #181 /
  draft PR #185; transitively Watcher #179 / PR #180 through its existing
  dependency
- **Tracker:** [#187](https://github.com/lossyrob/telex/issues/187)
- **PR:** [#189](https://github.com/lossyrob/telex/pull/189), OPEN/non-draft;
  head `d0b1010594aff7c27f7629ceb43ddad3c79118be`
- **Owner coordination:** Local `aeae4fa9-c5fc-4da9-b60e-bcf5b1a98f57`;
  implementation owner `5d9b3155-0327-40d1-936d-7a3652ea90c3`
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

## Launch requirements - promotion-time snapshot

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

### Actual v2 launch receipt - 2026-10-24

The pending/not-created preparation and launch statements above are the
promotion-time snapshot, qualified only for launch by this later receipt.
Local reports packet 05 preparation verified and one real kickoff delivered
to owner `5d9b3155-0327-40d1-936d-7a3652ea90c3` from canonical promotion
`970be041537d11a4e29e5c62b8aaf884646120b6`. Initial checkout and branch were
`lossyrob-stunning-telegram`; the same checkout now uses
`lossyrob-macos-persistent-lock-repair`. Local's later branch observation
records the owner's pre-edit rename and the same initial HEAD. The handoff,
kickoff and Task receipt retain their initial branch and hashes; this is
not a new checkout or second owner. The reconciler's receipt-time App read
observed the same owner/checkout/current branch with busy activity.

The launched v2 binding is
`prompts\v2\launches\macos-persistent-lock-repair-20261024.json`
in the Telex prompt pack. Its retained snapshot is 4,956 bytes, SHA-256
`3a446633b479d7219ea775c723835174bf9c8ec241509e2a4fcba11752a57a12`.
It records `launched` and `kickoffDelivered`, delivery observed by
2026-10-24T22:04:09.2658667Z, and the later branch observation at
2026-10-24T22:05:04.7267257Z. Local reports immediate App delivery by verified
file reference, no mode override, and one delivery. These transport and
preparation claims remain attributed; busy activity is not implementation
or runtime proof.

The reconciler matched the 13,070-byte kickoff at SHA-256
`d55d9a1b43448d4a4e10a7921bd7cf277e7e818c4be722e9bd40c060d2712f09`
and its byte equality to the original handoff. The 14,820-byte handoff
snapshot has SHA-256
`122aa6bd35745065c6fb98bb60229a13baf4e60b5d05d8f18dc683d0354247f8`.
No delivery count or effective model/context capacity was independently
replayed or proved.

The binding records a parent context correction removing an unrequested
implementation-subagent ban introduced during preparation. The single App
owner and independent review roles remain; scope, freeze and kickoff are
unchanged. The reconciler matched the current 12,439-byte package context
to the installed Streamliner context, both at SHA-256
`af941df33f9f34d941f250005c5a8a5d97e6602eaa995e7442a58913fa88e1a1`.
The original Task preparation receipt is a distinct 4,117-byte snapshot at
SHA-256
`14871baa7c94883670fd4c4ddc39b1d61eedcd98716a7557e4861e6fc9911f3f`.
Its `prepared-not-delivered` state, initial branch, binding fingerprint and
old 12,518-byte context fingerprint are historical. That context SHA-256 is
`1d3897a7db5358645b7e52fa1735811e75e8c0fde1b3f3a3ca1456e02d60156e`.
It is not the current context fingerprint or evidence of a repeated Task.
The parent correction, rather than a rewritten Task receipt, records the
current context. The reason for the correction remains parent-reported;
the reconciler verified current bytes/equality, not a reconstructed
pre-correction context or every preparation check.

The requested initializer and owner profiles remain `gpt-6.1-sol` / high /
default, with the owner interactive. They are requests, not effective
runtime-capacity proof. No product implementation, local runtime/test,
hosted CI result, accepted repair delivery, readiness, merge or gate
acceptance is claimed by the launch receipt. Issue #181 remains blocked,
PR #185 stays draft at `7b11b926` unchanged, and the workstation freeze and
all existing gates remain.
The reconciler did not prepare, launch or replace an owner.

### Draft repair and absorbed fixture state - 2026-10-24

PR #189 is OPEN/draft at `d0b1010594aff7c27f7629ceb43ddad3c79118be`,
tree `b8bd08f36a9a2a377358ba5cc7c1468fc5e8cdf8`, branch
`lossyrob-macos-persistent-lock-repair`, base `main`, body `Closes #187`.
Provider metadata snapshot: 3,075 bytes, SHA-256
`c6d90ba18274c8acff452bb664a74faf4db0ea24f2735cd0ef3a0eba75c228d3`.
The owner reports rebase onto `ef975014`; the product merge-base is that
commit. Canonical `917a0564` adds only the separate owner-launch receipt.

Local verified new run `40138318787` metadata matches this head,
`pull_request` event and attempt 1, without querying results. Same-role
reviewer/steward delta coverage and sole-wait replacement were requested;
Local's registration receipt is still pending. Owner-reported PAW-Review
recovery and requested `gpt-6.1-sol` / high / default are not a verdict or
effective-capacity proof. Local resolves reviewer alias `81c3b103` to
`d258aeb7-1af0-4587-bdcc-76f43f03b7d6`; no App review polling or new role
launch is performed by the reconciler.

The owner's initial `b03a2c2` / run `40137413218` invalid-workflow report is
historical and supplies no test credit. At corrected old head `add79d51`,
run `40137584510` / macOS job `121002280897` directly checked out that source
and printed its provider event SHA separately. The retained 70,647-byte log
has SHA-256
`ab8f97976bb8b5bb4952fae5610c5bb81c41d53839564434dd9ab3166a48426b`.
Selected excerpts show station-intent 58/0 in 5.05s and measured lock
contention 483.77725ms, then platform_fs 12/1 in 0.69s/exit 101. Those are
old-head partial results, not aggregate CI or current-head validation.
The reconciler matched bytes/hash and read named excerpts, not the whole
log; no provider log refetch or local runtime action occurred.

The distinct failure is
`concurrent_owner_private_directory_creation_is_idempotent` comparing a
canonical returned child with an unresolved temporary-parent spelling.
Local approved absorbing only existing-parent canonicalization before the
still-absent `scope` join, preserving eight creators, barrier, joins,
assertions and all production canonical/privacy behavior. The ledger entry
`local-daemon-macos-concurrent-directory-canonical-parent-fixture` records
the disposition, not a new upstream. Its source packet is 5,240 bytes at
SHA-256
`e5af2da25ea367e3b497c1cd7605d1b67a0434183ae1e3db93aa9903338afa69`.
Original `implementationConfirmed=false` remains historical; later Local
inspection confirms the correction in `d0b1010` source only. This does not
diagnose the original lock failure or absorb unrelated lint work.

The retained steward's bounded delta design receipt APPROVES exact
`d0b1010594aff7c27f7629ceb43ddad3c79118be`, zero new design findings:
`pr189-local-design-inspection-d0b1010-20261024.md`, 8,771 bytes, SHA-256
`95ee6a7e920e240d7e7d6290d147e1c096885fe40bd54a7975f968748e66c88e`.
Local and the reconciler read/hash-verified it fully. It preserves the
eight-creator race, separates upstream PR #188 changes from the fixture
delta, and verifies the prior three repair files byte-identical to
`add79d51`. The earlier 14,575-byte design receipt at SHA-256
`f71e15d7581398d4a069afc17fcfa0e5515c095f6fd4c657f6943a418d4dc5f1`
remains historical, not new-head runtime clearance. The delta receipt's
canonical-main `ef975014` is its inspection-time snapshot. Design-only
coverage is exact-head and invalidated by head movement.

No current-head App source review, hosted execution, CI-green, delivery,
readiness, historical-cause conclusion, runtime-isolation clearance,
merge or Campaign gate is credited by this record. No timeout/tolerance,
retry or skip workaround follows. The #187 scope, #181 block, PR #185,
Watcher dependency chain, workstation freeze, #153 and every unrelated gate
remain unchanged.

### Operator-ready receipt - 2026-10-24

The preceding draft, source-only, review/CI/wait-pending statements are
historical snapshots. This later bounded receipt qualifies them without
rewriting their evidence. Local independently verified OPEN/non-draft,
CLEAN, zero review threads and sole `Closes #187` at the same exact
`d0b1010594aff7c27f7629ceb43ddad3c79118be`,
tree `b8bd08f36a9a2a377358ba5cc7c1468fc5e8cdf8`. The same owner, branch and
base `main` remain. The provider's returned base SHA `ef975014` is not
silently equated with canonical `9b5a89e2`; the product comparison base
remains `ef975014`, while subsequent canonical advances are artifacts.

Local's full qualification is
`pr189-operator-ready-qualification-20261024.json`, 12,200 bytes, SHA-256
`7f76d4eff30a59ab1ec0d249243ef7c95a4b6103fbdb57a3c70771c403b4fb28`.
The later owner
[receipt](https://github.com/lossyrob/telex/pull/189#issuecomment-6390571479)
is by `lossyrob`, created/updated 2026-10-24T22:54:22Z. Local and the
reconciler read that comment fully; its use of the finalized source verdict
qualifies the review artifact's earlier owner-delivery-pending snapshot.

App reviewer `d258aeb7-1af0-4587-bdcc-76f43f03b7d6` finalized the exact-head
source verdict against `ef975014`: zero demonstrated #187 blockers, zero
ready comments and one retained, reasonedly skipped advisory C1. Skipping
that optional adjacent-boundary suggestion skipped no test or check.
IMP-01 is resolved in both folded YAML command scalars. The scope covers
all four candidate files and final YAML/fixture deltas, excluding inherited
PR #188 paths and PR #185. The external disposition is
`no-comments-to-post`, not a submitted GitHub approval; an empty provider
review-decision field is not approval. Retained `ReviewComments.md` is
39,919 bytes, SHA-256
`7c9b665ccc2d3f34c7ed3e449b102b1a51659ceb758df0972d44a75fcb4da3b6`.
Local and the reconciler matched its bytes/hash and read selected identity,
final-verdict/C1/disposition sections, not its full history or product code.
The exact-head design receipt above remains APPROVED, zero new findings.

Local's one bounded readiness read verified all 21 reported check entries
COMPLETED/SUCCESS and separately matched run `40138318787`, attempt 1,
`pull_request`, exact head and terminal success:
[run 40138318787](https://github.com/lossyrob/telex/actions/runs/40138318787).
This is not a recurring observer or branch-protection/ruleset audit. The
reconciler did not independently query CI results. Retained CI-only receipt
`pr189-final-ci-receipt.txt`, 1,338 bytes, SHA-256
`d15ce38ad131b102fa94c17950a45df4740a4551763da39e62b521fb75e628f7`,
was read fully. Its draft/source-review-pending/not-ready statements are
historical; its CI facts remain unchanged, not rewritten.

Retained macOS job `121004496256` directly verified `d0b1010`; separately
printed provider event SHA is
`578da1a18610d4d593d00674d85f675d7a9955a1`.
`pr189-d0b1010-macos-121004496256.txt` is 53,641 bytes, SHA-256
`1387dee909b5ae501fa98dddb4a9bfb745520b33235b2cd2c2244f88f0efe12b`.
Local and the reconciler matched bytes/hash and selected verifier, named
passes and complete-module summaries only, not the full log or a refetch.
Station-intent passed 58/0 in 2.73s; platform_fs passed 13/0 in 1.18s.
Measured contention was 514.511584ms, below the unchanged 1,540ms assertion,
not a hard 480ms synchronous-return guarantee. The fixture pass retains the
inspected eight-creator/barrier/assertion structure; the log does not trace
each creator. Other direct-matrix/provider-merge provenance remains the
owner/coordinator report, not independently inspected logs here.

The packet's owner-reported observer/request-pending wording is historical
for receipt custody. Later Local receipt `call_wn8etHPLWc0mly1Bc8Qh90ln`
relays coordinator `2a4bc4c8` source `call_PMjPjqPXbawyeLK9XHhnf4aE`:
`wait-pr189-ci40138318787-d0b1010` was done at 2026-10-24T22:39:01Z with
21 inventoried successes under exact-head/tree/base guards. Matrix jobs
`121004496153`/`121004496256`/`121004496284` used direct head; Live Postgres
`121004496197` and InstalledCurrent `121004496247` used provider merge.
The separate owner-requested lifecycle-only `wait-pr189-merge-d0b1010`
remains active, so the existing sole schedule stays armed. This is
coordinator-attributed registry/scheduler state, not an independent Local
or reconciler scheduler audit, a new observer or merge permission.
The immutable 12,200-byte packet is unchanged; later source capture
`r189-observer-later.txt` is 942 bytes, SHA-256
`823126e0ff79b3cf0266ed2b8286da63e868a02ab53b52fcb14058835e7e6773`.

This qualifies operator/Campaign consideration only at this head. Any head
movement invalidates readiness pending new evidence. Merge remains the
operator's decision; actual merged identity, main CI, field report and
accepted delivery remain pending. Node #187 stays in progress, #181 blocked
solely on it, and PR #185 draft/unchanged. No ledger, owner, dependency,
Watcher, #153, hardening/closure, freeze or gate state changes. Neither CI,
source/design approval, ready status nor issue closure alone releases a
downstream obligation. No retry, local proof, source re-review or provider
mutation was performed for this reconciliation.

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

Local `aeae4fa9-c5fc-4da9-b60e-bcf5b1a98f57` coordinates launched owner
`5d9b3155-0327-40d1-936d-7a3652ea90c3` under the v2 receipt above.
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
