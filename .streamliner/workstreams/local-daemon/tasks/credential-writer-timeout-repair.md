# Repair Windows delayed credential-writer timeout

## Current workstation run policy (2026-10-26)

At 12:44 ET the operator lifted the I161-1 blanket workstation freeze.
Local builds, tests and runtime runs are allowed within the task's authorized
scope. Use isolated `TELEX_*` roots and a temporary `LOCALAPPDATA`, except for
the separately authorized #201 dogfood against the real installation. That
exception permits real-installation use, not real-directory ACL or permission
changes. I161-1 preservation remains: no ACL restoration or reset.

Exact-head hosted CI remains the gate evidence. Local results support it; they
do not replace it or retroactively change earlier validation/debt dispositions.
Older freeze, compile-only and hosted-only records below remain historical,
including older sections labelled current; they do not renew a blanket local
run prohibition. Other scope, dependency, merge, incident and retained-debt
decisions are unchanged. This policy does not launch #201, satisfy its
dependencies or grant unrelated runtime authority.

Authority: Campaign `toolu_01JDdrjgTsK2e181xhhLdeYY`, relaying the operator:
"Also what is the 'freeze'? If it's blocking then remove it." On this operator's
workstation, the existing `gh-pub` profile now resolves to `lossyrob`; personal
GitHub writes can use it directly without a token override.


- **Workstream:** `local-daemon`
- **Node:** `credential-writer-timeout-repair`
- **Type:** implementation
- **Status:** in progress; one owner already launched
- **Attention:** focus
- **Depends on:** none; coordinate overlap without inferring an edge
- **Tracker:** [#167](https://github.com/lossyrob/telex/issues/167)
- **PR:** pending; one separate PR must use `Closes #167`
- **Owner:** `9a868a31-2c8d-429a-aee3-7e195076a4dd`
- **Owner coordination:** Local `aeae4fa9-c5fc-4da9-b60e-bcf5b1a98f57`
- **Parent workstream:** [#32](https://github.com/lossyrob/telex/issues/32)
- **Campaign:** [Addressable Attention #102](https://github.com/lossyrob/telex/issues/102)

## Current policy override

The 2026-10-26 same-head CI rerun and Sentinel decision authority below
supersedes only the original no-rerun clauses. #167 remains held/no merge.
The original launch snapshot and observations remain history, not proof
of current CI, wake state or accepted repair.

## Original outcome and launch authority

Diagnose and repair the Windows delayed inherited credential-writer
timeout in a separate closing PR. Distinguish lifecycle, outer test-budget
and hosted-scheduling possibilities without choosing a cause from the
timeout signature alone. Preserve delayed writer output after shell exit,
output/error semantics, effective negative assertions and owned-process
cleanup; validate the evidence-backed correction at the final head.

Campaign original direction `toolu_014vkRgy6ysjNiGD88kQeR6L`, confirmed
directly in `toolu_01Jw6FHd8GL21QG41ruVtEnr`, authorized one Opus 5.5/high
public-record-only owner: launch first, record after. Local
`call_d9NxdDsCntpTp9O4amIuuF1e` reports the already-completed launch.
This task records it; it neither starts another owner nor makes
implementation wait for bookkeeping.

The complete Local launch packet is 5,274 bytes, SHA-256
`ff74705a9428f38b9276eed956ba21cc50ecacbffa24232e26ea0ce887910ad9`.
Its original bytes remain in private custody; the canonical projection
replaces only the private checkout path by `lossyrob-curly-potato`.
The read-only App observation confirms the same session, Local
creator/coordinator, checkout, current branch
`lossyrob-fix-167-delayed-writer-timeout` and observed busy activity.
Local reports kickoff at creation and no private-context transfer.
The requested Opus 5.5/high/default/interactive profile does not prove
effective capacity. No exact starting working head, tree or clean-checkout
report was received in this frozen launch snapshot. Public-main
`be8b9910` was preflight only, not checkout verification; the observed
current branch is not a claim about the initial generated branch.

## Retained evidence and uncertainty

Every earlier candidate/unassigned field, original evidence and all four
successor occurrences remain exact history in the ledger. Promotion adds
no occurrence and does not truncate observations to match Campaign's
"four times" wording. That wording is Campaign's recorded count, not an
independent census, total recurrence count or failure-rate estimate.

The latest recorded direct-main observation is run `40261754314`,
attempt 1, Windows job `121375431957` at executed `72c2a093`, not the later
artifact base or a synthetic PR merge. Local's retained classification
places `credential_command_shell_exit_preserves_delayed_inherited_writer`
at `tests/credential_command.rs:516:6`, the first unwrap of the outer
five-second timeout around `pg_connect_config(&profile(&command))`.
The inner result unwrap at 517, password assertion at 518 and
empty-obligations assertion at 522 are not observed.

Target 10 passed / 1 failed / 2 ignored in 10.91 seconds is not command
latency. The configured 150ms delayed writer is not observed execution
or actual delay. Source/log/hash/excerpt verification remains attributed
to Local and Sentinel as originally recorded; the reconciler did not
replay it. Upload metadata for artifact `12625860097` establishes no
archive/dump/executable finding. No native status, command-output,
process-liveness, leak, corrupting operation, source/common cause or
controlled-binary claim follows. The original launch prohibited reruns. That prohibition is superseded
only for same-head CI reruns by the current policy below; all failures
and observations remain history. Keep #166 native failures,
#203 Watcher harness exits and accepted #204 ForeignDaemon repair separate.

## Scope and invariants

Reconstruct the plan from public issue/PR/hosted-CI evidence and committed
source. Do not read or transfer another session's private transcripts,
packets, `.paw`, checkout or diagnostic files. The reconciler's custody
records are not owner research inputs. Diagnose the actual boundary and
make the smallest complete correction supported by evidence.

Preserve delayed inherited-writer output after shell exit, output/error
semantics, meaningful negative assertions and owned-process cleanup.
Do not suppress timeout/error evidence, discard the delayed output or
skip semantic assertions to obtain green. Justify a confirmed test-budget
change against production behavior rather than treating target duration
as command latency.

Product/test/daemon runtime stays on authorized hosted infrastructure.
No local runtime, fixture/process/debugger action, cosmetic-head retry,
waived/skipped/weakened check, unbounded experiment, new observer or
scheduler follows. Same-head CI reruns follow the current policy below. No installed-state, registry/ACL, I161-1
or workstation-freeze change. Artifact review grants no additional
experiment or runtime permission beyond the owner's bounded authority.

## Coordination and gates

Coordinate overlapping credential, daemon, CI or dependency edits with
`d7a17cd2-08c2-4760-b1a6-1209c2d0f80c` before publishing. d7 remains
#166 / PR #196's sole writer; do not write that branch or absorb its
repair. Coordinate overlap without serializing unrelated analysis or
inventing a graph dependency. Use existing external wait coordinator
`2a4bc4c8-1211-49d4-ba68-9d05d5d7530d`, not a second scheduler.

Deliver one PR using `Closes #167`, with independent exact-final-head
source review and reviewer-origin public verdict/scope/limits receipt,
design inspection as applicable, strict required hosted CI and Campaign
gate. No autonomous merge. Launch, busy activity or one clean observation
does not grant source/runtime/CI/readiness or accepted repair credit.

This promotion adds one Local node/task only. All existing graph nodes,
edges, statuses, attention and design links remain exact; no Watcher
change or hardening-gate clearance is inferred. Both #193/#198 wakes
remain pending in their original structures and recipients. Only Local's
verification of the actual #166 product-repair merge triggers them;
staffing, artifact landing or readiness does not. No #166/#203/#204
acceptance, debt/HOLD, release or freeze clearance follows.

## 2026-10-26: Current same-head CI rerun and decision authority

Campaign `toolu_01BAMphx5UxVR6KEHAWxh5Ud`, relaying the operator's
09:25 ET directive, supersedes this launch snapshot's blanket no-rerun
prohibition for same-head CI reruns. Retain the failing run, job and test
on the existing tracker or discovered-work record first; then failed jobs
may rerun at the unchanged head. A green exact-head rerun satisfies CI.
Merging red is forbidden; repeated failure of the same test is a finding
to report. No cosmetic-head retry, waived/skipped/weakened check, local
runtime or product retry weakening is authorized.

Existing Sentinel `fc772c8f-2a38-40de-a868-69a5397dbbc5` owns the
cross-family Windows flake/environment investigation and approach decision
under the R15 amendment. #167 B/C and #175/LockFileEx-33 staffing are
deferred to its recommendation. #167 remains held/no merge; its existing
repair owner and obligations are not reassigned. No experiment result,
recommendation, acceptance, CI result or new staffing follows.

The launch snapshot's superseded wording is retained here as history:

> Preserve historical retry statements as history only; they authorize no
> retry now.

> Product/test/daemon runtime stays on authorized hosted infrastructure.
> No local runtime, fixture/process/debugger action, retry/rerun,
> cosmetic-head retry, waiver, flake exception, unbounded experiment, new
> observer or scheduler follows.

These quotations do not override the current same-head CI rerun policy.
All observations, owned-process cleanup, delayed output and semantic
assertions remain protected; a green rerun is not proof of a causal fix.
The earlier pending-wake language is launch history, not a new trigger or
permission to resend either already-admitted #193/#198 wake.
Campaign planning commit `74721a829f1dda93d1d2acc869a60fa2266c45f1` is a
private process-document change, not telex main or a product merge.
Recording grants no #166 debt/causality or #167/#203/#207 acceptance,
release, hardening or freeze credit and must not delay existing work.
