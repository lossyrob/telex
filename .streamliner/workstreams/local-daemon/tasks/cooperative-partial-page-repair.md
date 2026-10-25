# Repair cooperative station-intent partial-page completion

- **Workstream:** `local-daemon`
- **Node:** `cooperative-partial-page-repair`
- **Type:** implementation
- **Status:** blocked on accepted #187 delivery; main integrated at 130be3;
  current-head review/design/CI and terminal acceptance remain pending
- **Attention:** focus
- **Depends on:** `macos-persistent-lock-repair`,
  [#187](https://github.com/lossyrob/telex/issues/187)
- **Blocks:** `telex-watcher/watcher-cli-ux`, issue #179 / PR #180
- **Tracker:** [#181](https://github.com/lossyrob/telex/issues/181)
- **PR:** [#185](https://github.com/lossyrob/telex/pull/185), OPEN/draft;
  head `130be3b257e8208f521f9e4e91bc6795b991e023`,
  tree `fd53e11c478ae8e4b7ca19855734735803feac79`
- **Owner coordination:** Local `aeae4fa9-c5fc-4da9-b60e-bcf5b1a98f57`;
  implementation owner `9d4fee6c-63a6-4aa7-b384-a4df1cbcbb4c`
- **Parent workstream:** [#32](https://github.com/lossyrob/telex/issues/32)
- **Campaign:**
  [Addressable Attention #102](https://github.com/lossyrob/telex/issues/102)

## Outcome

Restore the accepted cooperative partial-page completion/progress behavior
through a discriminating diagnosis and minimal complete correction. Deliver
one separate repair PR that closes issue 181; no unrelated marker repair.

## Authority

Campaign `e06813c4-1db2-4d77-b8d3-3addc6a1824c` authorized this bounded
repair on 2026-10-23T16:44:24-04:00. Local submitted the exact directive and
task scope in `issue181-active-repair-promotion-20261023.json`.

> Authorized: one separate bounded Local root-repair owner for
> local-daemon-windows-cooperative-partial-page-overrun, using the scope you
> proposed. Diagnose the completion/permit/worker boundary first. Preserve
> single-slot behavior and the absolute D+grace and mandatory partial-page
> semantics. Make the minimal correction, then validate at the exact head on
> hosted CI. No third unchanged-head retry, skip, waiver, or unsupported
> timeout increase. Apply the discovered-work rule: the reconciler promotes
> the candidate to an active Local node with a GitHub issue that the repair PR
> closes, and records it as an upstream blocker of watcher-cli-ux (#179).
> PR #180 stays as is: no absorption, no Watcher-side change. Launch with
> gpt-6.1-sol, high reasoning, default context, under the existing workstation
> freeze. The marker cases stay separate candidates. When the repair PR is
> ready, it comes to the Campaign gate like any other. Tell the Watcher
> coordinator (fe2e883c) that #180 waits on this repair.

This is explicit implementation/launch and work-geometry direction, not a
merge authorization or hardening/closure decision. No product design or
contract change is authorized by this task.

## Inputs

- Canonical issue 181 body and Campaign's quoted authorization in this
  packet.
- Candidate local-daemon-windows-cooperative-partial-page-overrun and its
  retained attempt-1/2 evidence in the Local ledger.
- Associated PR head
  8826fad763f12ed6cde30a3f976a87e8807de775; both retained CI logs record
  provider test merge da52c4d687b517ec10aa3d673bc50e9863225156.
- Attempt 1 run 40008577194/job 120602732487, full log SHA256
  21aa0585d47364173ded4bd4b0742f480e7c3ddb3388e0802bb07a476a1dc8db:
  562 passed, 3 failed, 4 ignored.
- Attempt 2 same run/job 120608948530, full log SHA256
  4e6ec7a29337c0a27c2ddd8796fe212b8de2c6103177f1c78f16e9f4e38fdf62:
  partial-page assertion src/station_intent.rs:3865:14 recurred; 564 passed,
  1 failed, 4 ignored. The two marker tests passed; their first failures
  remain separate candidates.
- Existing test/helper/source and accepted Local Daemon design. Earlier
  52abb30 source review is historical context, not proof of this runtime
  cause.

The retained checkout identity is a provider test merge, not direct PR-head
execution. The exact logs and their custody history remain in
[`../discovered-work.json`](../discovered-work.json); no source-equality,
binary-rebuild or causal claim follows from the image identity.

## Diagnosis

Identify which completion boundary is responsible: permit acquisition,
worker scheduling/execution/result, cooperative deadline handling, or
another evidenced mechanism. Record a bounded discriminating account and
distinguish current observations from hypotheses. Do not assume runner load
or a previous cause merely from test names.

## Boundaries

### In scope

- The minimal source or test synchronization correction that satisfies the
  original partial-page outcome.
- Directly related regression coverage and documentation needed to
  demonstrate and explain the correction.
- Exact-head hosted Windows validation of the affected root-library test
  and relevant required suites, with truthful source/test-merge provenance.
- One implementation owner, one isolated branch, one node PR, direct
  review-response fixes and final Campaign gate handoff.

### Out of scope

- The idle-drain repush and live-CC marker candidates, unless separately
  dispositioned based on new evidence.
- Watcher implementation changes or incorporation into PR 180.
- Issue 153 transactional authority, broader station-intent redesign or new
  persistence mechanisms.
- Third unchanged-head retries, disabled tests, check waivers, unsupported
  timeout increases or degraded mandatory partial-page guarantees.
- Hardening/closure acceptance, release/tag, fleet adoption, merge without
  Campaign authorization or historical incident clearance.

## Invariants

- At most one cancellable filesystem phase owns the process-local permit.
- Admission and completion share the existing absolute D+grace budget; no
  sequential fresh grace windows.
- Original cooperative deadline, cancellation/generation fencing and
  truthful truncation remain.
- The mandatory partial-page/progress contract is preserved, not replaced
  by accepting Overran as success.
- No false claim that async timeout interrupts an entered blocking syscall
  or erases late completion.

## Success criteria

- A precise evidence-backed explanation distinguishes the selected
  corrective mechanism from plausible alternatives.
- A discriminating regression checks the actual corrected requirement, not
  merely compilation or retry success.
- Affected behavior and required suites pass on the exact repair head in
  isolated hosted Windows CI, with other relevant platform/profile coverage
  preserved.
- Complete implementation review, exact-head design inspection and Campaign
  repair-PR gate are satisfied; issue 181 closes with the authorized repair
  PR.
- Watcher dependency remains pending until actual accepted repair delivery;
  PR 180's own current-head requirements are not waived.

## Workstation freeze

No local runtime/test/proof execution, daemon/child process control,
PageHeap/registry or ACL/inheritance mutation, fixture-root actions,
restoration or cleanup. Read-only analysis, ordinary isolated
source/worktree edits, commit/push and permitted compile-only checks are
distinct from runtime validation. Runtime proof must use the authorized
hosted disposable runner. Do not touch installed user/campaign Telex state.

## Launch

Canonical promotion landed at
`3d517e17fba88559fbd5e69557ceb6264300aeff`. On 2026-10-23 Local completed
Streamliner v2 preparation and delivered the real kickoff to App owner
`9d4fee6c-63a6-4aa7-b384-a4df1cbcbb4c` in checkout directory
`lossyrob-expert-doodle`.
The launch-time branch was `lossyrob-expert-doodle`, initially at that main
commit. A later reconciler App inspection reports branch
`lossyrob-cooperative-partial-page-repair` and the same owner/checkout.

The requested runtime profile is `gpt-6.1-sol`, high reasoning, default
context tier and interactive mode; this records the request, not independent
verification of effective runtime settings. The canonical v2 binding is
`prompts\v2\launches\cooperative-partial-page-repair-20261023.json`
in the Telex prompt pack.
The reconciler read its `launched`/`kickoffDelivered` receipt and matched the
12,819-byte kickoff at SHA-256
`dfec5f6b8ff3e147096bb7e98dbb5ea52113fab3973363953663235ab5aba4b8`.
The binding snapshot is 3,560 bytes at SHA-256
`5af3fa466a2104fdf534f1ba09cad1e3e641e44047f7d60c4b17c867bd1bc0d8`.
Local's receipt records completed preparation checks and accepted immediate
kickoff delivery. The reconciler independently observed the same App owner
and checkout with busy activity; delivery acceptance/activity do not prove
implementation or CI completion.

This v2 launch corrects and supersedes the earlier preparation-pending and
session-online/write-authorized identity-exchange requirements in this task.
Local reports no API/SDK/v1 handshake; none is required for this v2 launch.
No duplicate writer, prepared-worktree copying or hand-written preparation
bypass is authorized. The workstation freeze remains in force.

## Draft repair association

On 2026-10-23 Local requested association of OPEN/draft
[PR #185](https://github.com/lossyrob/telex/pull/185) at head
`7b11b92630d0b2cf63423a77fc19232aff368825`, base `main`, branch
`lossyrob-cooperative-partial-page-repair`. Its body says `Closes #181`;
canonical owner remains `9d4fee6c-63a6-4aa7-b384-a4df1cbcbb4c`.
The reconciler independently verified this PR identity and closing text.

The owner reports absorbing DI-185-01 as a same-origin fixture correction
in [comment 6375067883](
https://github.com/lossyrob/telex/pull/185#issuecomment-6375067883).
Local and the reconciler read the owner-authored comment; that receipt is
not source review or independent verification of the reported correction.
At head-refresh intake, fresh review, design inspection and hosted
validation were unresolved. The later design-only receipt below qualifies
that design snapshot only; the draft is not Campaign-ready.

The steward's later report
`pr185-local-design-inspection-7b11b926-20261023.md` approves the bounded
DI-185-01 source resolution at exact head
`7b11b92630d0b2cf63423a77fc19232aff368825`, tree
`5582a2ca68d9c67a1dbfe99edd35842abb239d5f`, with zero open design findings
within the inspected scope. Local and the reconciler read and hash-matched
the 7,852-byte report at SHA-256
`5c9eb56e372fefdc36e3d9a359d7df5b56d809031ce30d9201ddc71162b60c1d`.
It supplements the immutable `57fdc975` changes-required inspection rather
than rewriting that history. This is design-only source resolution, not
independent App source-review completion, hosted runtime proof, historical
cause, accepted repair delivery or Campaign gate/merge acceptance. The
reconciler read the report, not product source or runtime evidence.

At the original draft head
`57fdc9752d3217f9ab9f2038bf90345ec48bae96`, Local reported queued/running
CI and one requested wait
through the existing coordinator; registration confirmation was still
pending. This is a historical snapshot, superseded for registration only
by the later receipt below.

Local subsequently reported receiving the existing coordinator's
wait-registered receipt for `wait-pr185-ci40025809314-57fdc97`, exact head
`57fdc9752d3217f9ab9f2038bf90345ec48bae96`, run `40025809314`.
The coordinator reported an explicit 21-job inventory including
cooperative-boundary, LivePG and Windows InstalledCurrent guards, with the
sole five-minute native schedule armed and verified, read-only. Local
relayed the wait ID to the owner. The reconciler did not independently
inspect scheduler state or CI results.

That registration is historical coverage of `57fdc975`, not registration
or validation of the current head. No old-head evidence, review, design
or CI acceptance carries over.

At head-refresh intake, Local reported independently reading metadata for
run `40026777909`:
`pull_request`, attempt 1, `head_sha`
`7b11b92630d0b2cf63423a77fc19232aff368825`. Local was requesting the
existing observer's rebind to this head/run; no registration receipt had
then arrived. That pending statement is the historical head-refresh
snapshot, qualified only for registration by the later receipt below.

Local later received coordinator `2a4bc4c8`'s successor registration for
`wait-pr185-ci40026777909-7b11b92` at exact head
`7b11b92630d0b2cf63423a77fc19232aff368825`, run `40026777909`.
The coordinator reports the full 21-job inventory including
cooperative-boundary, LivePG and Windows InstalledCurrent guards, with the
existing verified five-minute native schedule as sole observer. Local
relayed the successor ID to the owner. The old `57fdc975` /
`40025809314` wait was superseded without further query or log retrieval.
These are attributed coordinator/Local receipts, not the reconciler's
independent scheduler, inventory or CI-result inspection.

The reconciler did not query the new run or its results. Existing-observer
registration is not a new poller, CI success, runtime acceptance or
Campaign readiness. Independent App review, new-head hosted CI, accepted
delivery and the Campaign gate remain separate and are not credited here.

Draft association does not accept the proposed repair, establish runtime
success, conclude a historical root cause, or clear the Watcher dependency,
marker candidates, issue #153 or any existing gate. The original outcome,
evidence histories, workstation freeze and Campaign merge gate remain.

## Upstream dependency - 2026-10-24

Campaign authorized the separate `macos-persistent-lock-repair`, tracked by
[#187](https://github.com/lossyrob/telex/issues/187), upstream of this task.
Issue #181 is blocked on its accepted delivery. Existing owner
`9d4fee6c-63a6-4aa7-b384-a4df1cbcbb4c` retains OPEN/draft PR #185 unchanged
at `7b11b92630d0b2cf63423a77fc19232aff368825`; its branch, scope and closing
issue remain unchanged. The new root repair has its own owner and PR.

The launch/draft-association review/wait snapshots above are historical.
The ledger separately retains attempt-1 terminal evidence, the guarded
attempt-2 submission/registration and its recurrent failure. The one retry
allowance is consumed and exhausted. Those receipts preserve attribution,
distinct captures, unknown cause/actual test elapsed and required-CI failure;
no source-review, design, runtime or Campaign acceptance is added here.
No further retry, skip or threshold workaround is authorized.

Resume only after accepted upstream delivery and its field report permit
reconsidering this prerequisite. Neither this task nor Watcher #179 / PR #180
is automatically complete. The entire earlier outcome, authority, evidence,
launch, design and wait history remains; the workstation freeze, marker,
issue #153 and hardening/closure gates are unchanged. PR #185 is not edited
or absorbed by the new repair.

## Authorized upstream integration - 2026-10-25

Campaign source `toolu_01PMvueoUjg24dodu9bGRnrR`, retained in Local's
`pr189-merge-and-review-floor-intake-20261025.json`, directs this existing
owner to merge main into PR #185 and run CI again. This explicitly
supersedes the earlier unchanged-PR hold and resume-only wording for that
integration and validation only; it does not remove the #187 dependency or
accept its terminal delivery. PR #189 merged from reviewed head
`d0b1010594aff7c27f7629ceb43ddad3c79118be` as
`d03492ea425f001df530f4405808d3a817558291` at 2026-10-25T16:55:54Z;
issue #187 closed as completed at 16:55:56Z. Local and the reconciler
verified those provider facts separately from acceptance. The existing #187
owner still owes post-merge main CI and a field report; neither is verified
or accepted by this intake.

Local's PR #185 OPEN/draft `7b11b926` read is an intake-time snapshot.
The direction is not evidence of a new head, completed main integration,
source review, design, CI success or readiness. Revalidation of an
integrated head is not another authorized unchanged-head retry, skip,
waiver or threshold workaround. Old-head evidence remains historical;
current-head requirements are not waived. This node remains blocked on
accepted upstream delivery and its Watcher dependency remains pending.
Owner, scope, graph dependency, workstation freeze, marker candidates,
#153 and hardening/closure gates are unchanged. No second owner, observer,
runtime action or product edit by the reconciler is authorized.

The later prospective Local source-review floor also applies: the source
reviewer itself must publish an exact-current-head verdict receipt on this
PR with necessary scope/limits. A finalized `no-comments-to-post` review
still requires that receipt; owner attestation, a private artifact or
coordinator relay alone does not satisfy it. Verify that receipt before
future floor-complete readiness; a missing receipt is now a floor miss and
an older-head receipt stays historical. Include this additive rule in new
owner/reviewer kickoffs and delta-review requests, preserving the required
`gpt-6.1-sol` / high / default profile for new sessions. Do not rewrite
frozen kickoffs or prior review/readiness evidence.

Campaign's accepted PR #189 publication-gap debt is recorded separately in
the upstream task and brief, not cleared or retroactively satisfied here.
No retrospective receipt, new review, posting for a reviewer, empty formal
approval, merge reversal or issue reopening is requested. Reviewer
publication alone establishes no CI, merge authority, accepted delivery,
Watcher readiness or gate completion.

## Integrated draft-head receipt - 2026-10-25

The earlier `7b11b926` and integration-pending statements are historical
snapshots. PR #185 is OPEN/draft at
`130be3b257e8208f521f9e4e91bc6795b991e023`,
tree `fd53e11c478ae8e4b7ca19855734735803feac79`, same owner and branch
`lossyrob-cooperative-partial-page-repair`, base `main`, `Closes #181`.
Local and the reconciler verified PR identity and immutable commit
metadata: ordered parents are
`7b11b92630d0b2cf63423a77fc19232aff368825` and actual PR #189 merge
`d03492ea425f001df530f4405808d3a817558291`. This verifies the authorized
main integration, not the complete merge diff, source correctness or
runtime behavior. The reconciler read no product contents; neither
executed validation for this qualification.

Local's full `pr185-integrated-head-qualification-20261025.json` is
4,830 bytes, SHA-256
`9b9954f20a44c3d2fea5be94e94ed0c98a69ee9d6d1ce04f8c0580158fa38354`.
The owner
[receipt](https://github.com/lossyrob/telex/pull/185#issuecomment-6400532965)
is by `lossyrob`, created/updated 2026-10-25T17:02:17Z. Local and the
reconciler read it fully. Its retained body is 1,447 UTF-8 bytes, SHA-256
`202c4022ec5217ca87056e2a8308a09386870f1fbb97a47230457eb78f2179ed`.

The owner reports a clean main merge with no additional lock workaround,
retry, skip or threshold change. Both three-OS cooperative partial-page
and upstream station-intent-lock matrices, ten real partial-page
repetitions, owner-private filesystem coverage and existing provider-merge
suites remain. The owner reports later Campaign routing defers
consolidation assessment to the sentinel until after #185 lands and the
uncommitted consolidation draft was withdrawn before publication. These
are attributed source/routing claims, not the reconciler's product/YAML
inspection or authority for consolidation, matrix removal or another retry.
Formatting, SQLite library unit-test compilation without execution,
strict production-library Clippy and whitespace checks passed according to
the owner, not independent execution here. This does not validate the
broader optional test-Clippy command or rewrite its separate evidence.

Local's one bounded provider read confirmed
[run 40214432369](https://github.com/lossyrob/telex/actions/runs/40214432369),
`pull_request`, attempt 1 at exact head `130be3`; Local queried no result
or log. The reconciler did not query the run, CI results or scheduler.
The owner reports asking the existing sole coordinator to register it;
registration receipt is pending in this snapshot. No new observer or
registration completion is credited. New-head source/design requests are
owner-reported and outstanding, not completed verdicts. The reviewer must
publish its own exact-current-head PR receipt under the prospective floor,
including for no findings; no one posts on the reviewer's behalf here.

The owner's accepted-upstream/blocker-resolved wording does not establish
#187 post-merge main CI, field report or accepted terminal delivery. This
packet supplies neither that evidence nor dependency-removal authority.
Campaign's integration/revalidation permission is observed without
clearing those separate obligations. #187 remains in progress and this
node blocked with its sole #187 dependency intact. Old-head review/design
and failed-CI evidence cannot clear `130be3`; no current-head source/design/
CI success, operator readiness or Watcher #179 / PR #180 acceptance follows.
All ledger records, owners, gates, #153, hardening/closure and workstation
freeze remain unchanged. No local runtime, new owner/reviewer/observer,
retry, consolidation, provider mutation or receipt-debt clearance follows.

## Workflow

Use the full canonical App implementer role and paw-lite planning. The
Campaign directive and workstation freeze override older model and
local-runtime defaults. Independent review and hosted CI begin from
complete pushed scope. The existing wait coordinator owns recurring
observation; worker must not create a poller. Source-only approval, design,
runtime, merge and hardening remain separate.

## Ownership and handoff

Local aeae4fa9-c5fc-4da9-b60e-bcf5b1a98f57 coordinates launched owner
9d4fee6c-63a6-4aa7-b384-a4df1cbcbb4c under the v2 receipt above. Existing
Watcher c90bafbb preserves its branch/evidence; current Watcher coordinator
fe2e883c-debc-4573-a855-2cf1e0f89407 has been told PR 180 waits.

The separate repair PR must explicitly close
[#181](https://github.com/lossyrob/telex/issues/181). Return its complete
exact-head evidence and review/design results to Campaign. Reconcile
completion and release the pending Watcher dependency only from actual
accepted terminal delivery; this task grants no immediate merge or
hardening/closure acceptance.
