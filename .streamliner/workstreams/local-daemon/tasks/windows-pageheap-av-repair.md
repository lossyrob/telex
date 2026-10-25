# Required Windows PageHeap access-violation repair

- **Workstream:** `local-daemon`
- **Node:** `windows-pageheap-av-repair`
- **Type:** implementation
- **Status/attention:** in progress / focus; one authorized owner pending
  canonical preparation and launch
- **Tracker:** [#166](https://github.com/lossyrob/telex/issues/166),
  existing open issue; no duplicate issue or discovered-work record
- **Dependencies:** none; no automatic #193 or Watcher edge
- **Coordinator:** Local `aeae4fa9-c5fc-4da9-b60e-bcf5b1a98f57`

## Outcome

Diagnose and repair native access violations in required hosted Windows
process diagnostics. Obtain a native fault stack or a bounded reproducible
diagnosis, fix the confirmed memory/lifecycle defect with the smallest
complete correction and regression coverage, then validate the full
diagnostic target and relevant feature/platform profiles at the final
reviewed head. A green repetition is not a causal fix.

This is one independent Local investigation, parallel with #193 / PR #194.
It does not absorb that repair or change its owner, scope, status or gates.
Keep the distinct lifecycle observations below separate. A shared native
exception class, test target or command family does not establish one
fault location or cause. If diagnosis demonstrates independent defects,
report exact evidence to Local and Campaign for separate item dispositions.
Do not silently combine them or discard a retained case.

## Campaign authority

Campaign source `toolu_01RiD7o4ZxQLma9dbVQPkPSx`, relayed in the complete
Local promotion packet:

> Campaign: PR #194 (#193 fix) is blocked under strict CI by a Windows heap
> diagnostics access violation (0xc0000005) during busy-station successor
> handoff/status: run 40224540059, job 121263011458, at f8b42d20. This is at
> least the second heap-diagnostics AV, after the prior-main PageHeap
> teardown AV that the sentinel reported in #182 (issuecomment-6401222833).
> A recurring AV in a required job now blocks every PR. Please file it as
> its own item, if you haven't already, and staff an owner now, in parallel
> with #194. No retry, flake exception or waiver. If you judge the two AVs
> to be distinct defects, say so and give each its own item.

Local reuses existing OPEN #166 and the existing
`local-daemon-windows-pageheap-access-violation` record. Local reports no
existing implementation owner after its issue/session inventory. That
inventory is Local-attributed, not a new reconciler inventory or launch.
The earlier candidate/unassigned/no-owner authority remains historical.
One v2 owner may launch after Local verifies the actual canonical landing
receipt; no additional staffing approval is required.

## Retained lifecycle observations

**Original status-hint observation.** At
`40a5c916d839dc81432738f122e03a00fd31628e`, run `39854080691`, job
`120087601259`, the status-hint process case observed `0xc0000005`.
The full target passed 49/49 twice, then 48/49 on its third repetition.
Existing issue #166 reports retained artifact `12458867179` and no dump
in the original owner's downloaded archive. This is historical source
reporting, not a new reconciler archive inspection or native diagnosis.
The precise fault location and cause remain unknown.

**Concurrent first use.** The
[existing #166 comment](
https://github.com/lossyrob/telex/issues/166#issuecomment-6359016085)
also retains the PR #173 concurrent-first-use attachment AV, run
`39910906688`, job `120273438217`. Keep it as a separate lifecycle
observation. Its historical one-retry statement gives no current retry
permission.

**Prior-main destructor stop/drain child.** At
`f603875bddd19a6f76f08a408ba3fbd98d3073bb`, run `40217995470`, job
`121243212833`, the parity test's destructor invoked
`daemon stop --drain`; that child reported `0xc0000005` after printing
`draining=true`. The
[sentinel report](
https://github.com/lossyrob/telex/issues/182#issuecomment-6401222833)
identifies harness line 518, stop helper line 201, `Drop` line 265 and
closing test scope line 1410. This identifies the observation boundary,
not a parity failure, successful drain or long-lived-daemon fault site.
The second full repetition failed 48/1 after one 49/49 pass.
Retained log `main-40217995470-job-121243212833.log` is 107,711 bytes,
SHA-256:
`f182848a1ca3c3bb197717f8d19e1cb145bf9f90dbc92992ff5a15599d5a632b`.

**PR #194 busy-station status child.** At
`f8b42d20acd3ae358a71a576d00c76b3f571ac6c`, run `40224540059`,
attempt 1, job `121263011458`, the busy-station successor test's invoked
status child exits `0xc0000005`. Exact source identifies
`--json --address <addr> status`; those argv are not printed in the log.
Observation boundaries are harness line 518, `run_with_session` line 156,
`wait_until_push_restored` line 4596 and caller line 5701.

The **THIRD** full repetition failed 48/1 after **TWO** 49/49 passes.
This explicitly corrects the owner's earlier tail-only second-repetition
report; retain that old report as superseded history. Full result sites
are lines 567, 623 and 707. Local reports matching the complete retained
68,029-byte log and exact helper/caller source boundary. Its SHA-256 is:
`ab83e9544bd72cdf257c70fb079dfe3798272bbe87e61904e3de64a9eece9954`.
The exact source blob is
`b1f708c5e8948884c49207fe4aba4e051dc7f23a`.

The job reports artifact `12610344684`, 49,500,264 compressed bytes,
ZIP SHA-256:
`abcf936b532fd62b06053fad3d86a72bcf33a633f1ee5a811bb7bcf9f89ceba0`.
Neither Local nor the #193 owner downloaded/unpacked that archive or
inspected its dumps. Its contents and native fault site are unverified.
The Rust assertion stack is not the native child fault stack. Passing
station-boundary jobs do not waive this required failure or establish a
#193 cause.

The reconciler read the complete Local promotion packet/domain notes,
existing public issue/comment and public sentinel report. Raw-log
hash/source-excerpt checks remain attributed to Local. No new log fetch,
CI inventory, artifact/dump inspection, source replay or native diagnosis
is claimed by canonical promotion.

## Diagnosis and correction

Preserve original failing logs, binary and symbol provenance, and any
dumps actually produced. Diagnose on authorized disposable hosted Windows
infrastructure with exact checkout binaries and isolated absolute
`TELEX_HOME`, `TELEX_DB` and `TELEX_INSTALL_ROOT`. Determine which child
failed and distinguish common versus independent causes from evidence.
Do not execute downloaded binaries on the workstation.

Do not replace diagnosis with timing guesses, successful repetition,
ignored abnormal exits or best-effort success fallbacks. Native exception
status stays a hard failure even if the child emitted valid JSON.

No retry/rerun, flake exception, waiver, skipped case, disabled diagnostic
job or weakened assertion is authorized. Intentional hosted diagnostic
instrumentation/correction must be explicit and bounded; no unchanged-head
reruns or unbounded loops to obtain green. Old issue/guide retry language
does not apply.

Preserve owner-private locks, non-theft, generation fencing, late-guard
disposal and the production 480ms station-intent budget. Ordinary exit 1
with SQLite `LockFileEx` os33, `ForeignDaemon`, seed-budget admission,
restoration counters, deadlines and `Overran` are distinct families.
Do not infer #154, #192, #180, #193 or a shared cause from test names or
exception status. Accepted #181/#187 delivery and retained debts stay
unchanged.

## Hosted validation and exact-head PR gates

Retain the complete process-diagnostics target and relevant feature and
platform profiles. Targeted success alone is insufficient. Final full
required Windows heap diagnostics, default workspace validation and
affected profiles must pass at the exact reviewed head, with failing
captures retained. The separate PR #194 remains under strict CI; this
promotion supplies no check waiver or readiness.

Use an independent App source reviewer. That reviewer itself publishes
the exact-head public PR verdict/scope/limits receipt, including zero
findings. Source review, steward inspection and hosted CI may proceed in
parallel, but all final evidence binds one final head. Steward inspection
gates the PR, not this authorized launch. One separate PR closes #166 and
returns to the Campaign merge gate.

## Workstation freeze

Read-only source/evidence work, ordinary isolated source edits,
commits/pushes and compile-only checks are allowed. No local tests,
runtime/proofs, daemon/test-child control, PageHeap/WER/debugger setup,
ACL/inheritance/registry changes, real-root actions, installed-state
changes or fixture cleanup. The earlier isolated Watcher-proof exception
does not apply to this repair.

## Single v2 launch and coordination

Local prepares one held App worktree owner after verifying the actual
canonical receipt. No `base_branch`, idle notification, sibling worktree,
API/SDK or v1 handshake; `coordinate_with_creator=false`.
Bind it immediately, run packet 05's general-purpose Task in that exact
checkout/existing branch, verify the full guide plus these domain notes
and current overrides, then deliver the real kickoff once by file/hash.
Use `paw-lite` and the full v2 owner guide; keep `.paw` uncommitted.
The reconciler prepares canonical artifacts and does not launch an owner.

New owner/preparer/source-reviewer profile is
`gpt-6.1-sol / high / default`; owner mode is interactive. Ordinary
messages omit mode. The existing steward profile is unchanged.
Use the available `PAW-Review` custom agent, not an older prefixed name.
Requested profiles and authorization do not prove preparation, launch,
effective capacity, implementation or validation.

Local coordinator: `aeae4fa9-c5fc-4da9-b60e-bcf5b1a98f57`.
Steward: `af271672-5d33-418c-bb9c-0f4d4570c7d7`.
Reconciler: `b737a932-c687-4b95-b273-8335a59f6e89`.
External-CI wait coordinator: `2a4bc4c8-1211-49d4-ba68-9d05d5d7530d`.
Campaign: `e06813c4-1db2-4d77-b8d3-3addc6a1824c`.
Keep PR #194's observer separate and do not duplicate polling.
Reviewer/steward results use direct messages.

No new Watcher edge, #153/closure/hardening completion, product runtime
action, admission/debt/release clearance or workstation-freeze waiver
follows from this artifact promotion.

## 2026-10-25 - Actual owner launch and future merge notification

This is an additive current-owner qualification. The original promotion's
pending preparation/owner statements above remain the historical snapshot,
not a claim that no owner has now launched.

Local reports one actual v2 owner:
`d7a17cd2-08c2-4760-b1a6-1209c2d0f80c`, checkout and launch branch
`lossyrob-special-meme`, context package
`ctx-20261025193837-7445b42c`. Its requested profile is
`gpt-6.1-sol / high / default / interactive`, no idle notification.
Local reports the kickoff delivered once and that same identity, checkout
and branch observed busy at `2026-10-25T19:44:19.2231752Z`.
Those are Local-attributed observations, not a reconciler App/provider
query or independent verification of effective model/capacity.

Local's separate branch packet received at
`2026-10-25T19:47:02.524Z` reports independent App confirmation of that same
owner/checkout, busy on current branch `lossyrob-windows-heap-av-repair`.
The original preparation/delivery branch remains `lossyrob-special-meme`.
This is not another owner, checkout or preparation. The packet's later
archive-inventory and dependency-candidate reports are excluded from this
launch-only successor; they are not diagnosis or source/design approval.

The original immutable launch-binding snapshot is 4,407 bytes, SHA-256:
`9c36cb79f92ae6defc70faed72a5455c872cd9aeec2dc1a7fdf165e66be030a3`.
Kickoff is 15,666 bytes, SHA-256:
`54b679962dc4fd1f01a7e98340185d76fe630108735a291295675784b7a55086`.
The reconciler read/captured the full binding, kickoff and handoff and
verified exact kickoff/handoff byte equality. Full guide/current overrides,
installed context equality, only two untracked `.paw` preparation files and
unchanged source/index checks remain Local-attributed; no replay is claimed.

Preparation is pinned to actual promotion
`f0053e3b94349b91dace2ca691d03fb281815c76`. The later observed
`df37d5683683662db3ede85b86d7b66e9f4064cb` changed only three Watcher
artifacts; Local reports the selected #166 node/task/policy exact.
This is not source integration, new preparation or review/CI carry.

### Event-driven follow-through, not completed delivery

Campaign authority `toolu_01KB1nMvqiqDxQiC7MQkUWPP`, relayed in the
original Local binding and complete delivered kickoff, requires the #166
owner to report its actual repair merge immediately to Local rather than
waiting for post-merge validation or the field report. Local verifies the
actual merge and then wakes the existing #193 owner,
`519693a5-0607-4aba-a968-345f3b1cd1b5`, to integrate main, run fresh CI
and request the PR #194 gate. The #166 owner continues its own post-merge
obligations separately.

This future merge/wake has not occurred in these sources. It does not
change #193/#194 ownership, scope, dependencies, status or gates; a future
fresh gate is not readiness now. There is no second launch/preparation,
product/source/diagnostic/CI replay, shared native-cause conclusion,
rerun/waiver, debt/HOLD/release or workstation-freeze clearance.
All original distinct AV observations and hosted-only exact-head review,
public reviewer receipt, diagnostic coverage and non-theft/fencing/480ms
requirements remain controlling.
