# Repair Windows InstalledCurrent ForeignDaemon reattachment

- **Workstream:** `local-daemon`
- **Node:** `foreigndaemon-reattach-repair`
- **Type:** implementation
- **Status:** completed with retained gate debt; follow-through pending
- **Attention:** focus
- **Depends on:** none; coordinate overlap, do not infer a dependency
- **Tracker:** [#204](https://github.com/lossyrob/telex/issues/204)
- **PR:** [#205](https://github.com/lossyrob/telex/pull/205), merged at faf4f883; #204 closed completed
- **Owner:** `8afc363c-1af3-447d-a472-336f92d57998`
- **Owner coordination:** Local `aeae4fa9-c5fc-4da9-b60e-bcf5b1a98f57`
- **Parent workstream:** [#32](https://github.com/lossyrob/telex/issues/32)
- **Campaign:** [Addressable Attention #102](https://github.com/lossyrob/telex/issues/102)

## Outcome and authority

Establish the concrete failed precondition or code path behind Windows
InstalledCurrent crash/reattach `DaemonBootstrap(ForeignDaemon)`, make
the smallest complete evidence-backed correction, and preserve meaningful
crash/restart, delayed-publication, successor identity and single-spawn
coverage. Deliver one separate PR closing #204 through exact-head review,
hosted validation and the normal Campaign gate.

Campaign original direction `toolu_01An1S92Jpt43cjRLd4HDCXC`, confirmed
directly in `toolu_014EXnE5H5oXyQfrYA6reKfs`, authorized a tracker and
one public-record-only Opus 5.5/high owner: launch first, record afterward.
Owning Local reports issue creation and the already completed launch in
`call_Hm5HNZfojopq27h8yU4vvjeO`. This task records that launch; it is
not a new launch prerequisite, second owner or repair acceptance.

The launch binding is 5,309 bytes, SHA-256
`e996c5a138355120c4a81aeb916402bf9bc1ad20961dd119420352a659ff7f4a`.
Local reports the fresh main-based checkout started clean at
`7e1bfc3add904565a808952953be14a98f4f2180`, tree
`e4bc34bbce33627d95613112482d1c54597cb3fe`, on branch
`lossyrob-foreigndaemon-reattach-repair`. Read-only App identity confirms
the same session, creator and checkout; cleanliness, kickoff admission
and requested profile remain Local-attributed.

## Evidence and uncertainty

The existing ledger record retains both prior Windows observations:
run `40222860912` / job `121258004551` at `4deaf30c`, and
run `40225451733` / job `121265733770` at `5da770cf`.
Their full evidence and candidate/unassigned source statements are
historical, not rewritten or erased by this promotion. The older Ubuntu
timeout and #166 native-exception cases remain distinct.

The new public occurrence is PR #196, event head
`854855f145752957025ae0df38df6f7b36859176`, run `40250813915`,
attempt 1, job `121342086267`, Windows InstalledCurrent Postgres proofs.
The executed checkout is pull-merge
`1813c432a9a43178b17dc6909f2fe272367b5e8e`, tree
`6dd90524660f6cb171f68be6f78913ada3bcdf5c`, with ordered parents base
`db8eaa75b019ab143b527fb93005bab45e2ca96e` then that PR head.
This is not a direct event-head checkout.

`installed_current_crash_restart_and_reattach_recovers_membership`
fails at `tests/installed_current_process.rs:508:5` with
`ready: false` and `ic:crash:station` reporting
`Failed(DaemonBootstrap(ForeignDaemon))`, no compensation, validation
error or cancellation. Fixture: 6 passed / 0 failed / 1 ignored.
InstalledCurrent target: 29 passed / 1 failed / 2 ignored, 139.79 seconds.
Target duration is not stage latency.

Local's public log is 51,310 bytes, SHA-256
`c8ec9f6d1cb0e4a842f0dcc3cdc461227316c20821268a4b2df8e7386b08abe2`.
Issue #204 records checkout lines 82-112, failure 545-546, result 573,
and the executed source boundary 471-526. These log/Git/source checks
are Local-attributed; the reconciler read the complete issue and launch
binding, not raw CI logs, underlying source, archives or runtime.
The log-byte hash identifies completed-job text, not an artifact ZIP.

The public signature identifies no actual foreign/hostile process, image,
internal authentication/publication check, native exception, common cause
or causal PR source. Local reports no `0xc00*` status in this job.
After the test's initial successful attach and deliberate crash, the
bounded reconcile fails before successor-PID, marker and single-spawn
assertions. The configured 1,500ms delay and `BoundedRepair { retries: 4 }`
neither prove the intended window occurred nor authorize a CI retry.

## Scope and invariants

Use public issue/PR/hosted-CI evidence and committed source with a newly
reconstructed plan, not another session's transcript, private files,
`.paw` or checkout. Independently identify the failed precondition before
choosing a correction. Do not presume that all retained observations
have the same cause.

Preserve authenticated peer selection, owner-private authority, non-theft,
generation fencing, bounded recovery and refusal of genuinely foreign
peers. Do not turn `ForeignDaemon` into blanket success, skip assertions,
widen budgets or weaken ownership/authentication to obtain green.
Add meaningful regression coverage or cause-discriminating diagnostics.

Runtime diagnosis and tests are limited to authorized disposable hosted
Windows. Local work permits source/evidence reads, isolated edits,
ordinary commits/pushes and compile-only checks. No local tests, daemon/
test-child control, debugger, PageHeap/WER, registry/ACL/user-root or
installed-state changes, or fixture cleanup. No unchanged-head retry/
rerun, waiver, flake exception, disabled required check or new observer.
Explicit bounded instrumentation/correction at a new source head must
preserve failed evidence and does not permit rerunning unchanged heads.

## Coordination and gates

d7 (`d7a17cd2-08c2-4760-b1a6-1209c2d0f80c`) remains #166 / PR #196's
sole writer. The #204 owner uses its own branch and separate closing PR,
never writes the #196 branch, and coordinates overlapping daemon,
bootstrap, test or CI edits with d7 before publishing. Do not serialize
unrelated analysis behind #166 or imply a shared defect.

Exact-final-head Windows InstalledCurrent coverage and all relevant
required CI, reviewer-origin public source verdict, design review and
the normal Campaign merge gate remain. Launch or a clean repetition
alone is not a causal fix, readiness or accepted delivery. Any genuinely
independent out-of-scope work requires explicit owning-role disposition.

This promotion adds one Local node/task and changes no existing node,
dependency or attention. It creates no automatic Watcher edge/status
change. Both existing #166 actual-product-repair-merge wakes remain
pending: d7 reports, Local verifies and separately rings #193 owner
`519693a5-0607-4aba-a968-345f3b1cd1b5` and #198 owner
`a96926b1-3e4f-4578-a197-33de51d669c7`. This artifact landing is not
their trigger and grants no runtime, retry, repair, CI/readiness,
waiver, debt/HOLD, release or freeze clearance.

## 2026-10-26 - Accepted completion with four retained gate debts

Campaign explicitly accepts this bounded #204 repair as completed with
debt. This is terminal acceptance, not merely a merged PR or a repair
waiting for main CI. The same owner,
`8afc363c-1af3-447d-a472-336f92d57998`, retains follow-through.
Earlier implementation, unknown-cause and launch-only wording above
remains the exact historical snapshot, qualified by this acceptance.
It is not a current assertion that the accepted repair remains pending.

[PR #205](https://github.com/lossyrob/telex/pull/205) merged at
`2026-10-26T02:41:47Z` as
`faf4f883631c7aaec057dde4beca36daa29d29dd`, tree
`1b754b186622b384bc9adaf3a835be7c6faa4df7`. Its ordered parents are
main `7e91a2337b7768e767f75c80b2535b7b0cd2530d` and reviewed head
`e11c56ce2eb9005edd81e42f77c4a5a571128a98`, whose tree is
`1707ac6b24fd323f320ff551dfaaf3614b25e09f`. Issue #204 closed completed
at `2026-10-26T02:41:49Z`.

The [public Campaign gate](
https://github.com/lossyrob/telex/pull/205#issuecomment-6407030627)
is `MERGE + debt`, high confidence. Campaign's actual completion direction
is `toolu_01PMAPJmqa8eQpqmCn13r85n`; direct confirmation is
`toolu_01FtkRbxxFYvMWS8H47FBdps`. The reconciler independently read
provider merge/closure metadata, Git geometry and the complete gate,
not product source or CI logs. The complete Local packet is retained at
8,019 bytes, SHA-256
`2d3b9d8b651085ee169069e2880a4242251774e2edf89b7b29d701b553401dbe`.
The full gate is 1,930 bytes, SHA-256
`5ae938300e7a24e5d29e5e3cecf83e2a6aefddc0ed90cb24577dc6c28131c36b`.

### Accepted bounded result and evidence limits

The gate accepts the correction for a killed predecessor whose pipe
remains open: only a provable exit is classified as retryable
`NotRunning` within the existing bounded probe/spawn loops. Access denied
or an unclear state remains terminal `ForeignDaemon`. The gate records
conservative PID-reuse and `STILL_ACTIVE` (259) handling, no authentication
relaxation, new variants, wire/schema change, release or #203 surface.
These are the accepted gate's findings, not a new reconciler source review.

The exact-head source/design receipts and 21/21 green attempt-1 PR CI at
run `40258493286` remain Local/gate-attributed pre-merge evidence, not
proof of subsequent main CI. The classifier regression is not deterministic
fresh-connect-during-rundown coverage and does not force every Win32 branch.
The diagnostic trace and pre-fix failure are not exact-fix-head runtime
results. Earlier observations and launch history remain unchanged; no
new occurrence, common-cause claim or broader acceptance follows.

### All four gate debts remain retained

1. The PR body has no consequence line (normal) and no "no discoveries"
   statement.
2. In a contrived same-user case, a squatter whose pipe-creating process
   has exited while another process holds the pipe gets bounded
   `Timeout`/`Unavailable` instead of `ForeignDaemon`. The peer is still
   refused; only the diagnostic class gets worse.
3. Neither the `ERROR_INVALID_PARAMETER` branch nor a connection during
   handle rundown is forced. Observe Windows InstalledCurrent on main;
   recurrence requires a new item, not a retry or automatic #204 reopening.
4. Evidence that the test fails before the fix is from diagnostic head
   `4f133e02`, not the final reviewed head.

The complete gate body is projected unchanged in the ledger alongside
these debt dispositions. No missing coverage, body correction or debt
clearance is inferred from acceptance.

### Pending follow-through is not a completion hold

The existing #204 owner adds the consequence line and no-discoveries
statement to the PR body, and posts the #204 field report after main's
Windows InstalledCurrent runs. These actions remain pending in this
acceptance record; no body edit, report or successful main result is
claimed here.

Local reports deduplicated exact-merge CI `40260369255` and Docs
`40260369359` waits under the existing sole coordinator
`2a4bc4c8-1211-49d4-ba68-9d05d5d7530d` and one scheduler. The packet's
queued/in-progress states are historical at capture, not a current live
scheduler audit or a terminal outcome. Observe through that arrangement;
do not create a duplicate observer or authorize retries. InstalledCurrent
failure routes to Local and the same #204 owner, not #193/#198.

Campaign directs d7 to merge current main into PR #196, obtain fresh full
CI at the new head and confirm whether #196's Windows required job clears.
Local reports forwarding the direction. Integration and validation are
pending follow-through, not old-head CI carry or #196 merge authorization.
#203 remains separate. This update does not edit #166's record, task or
node, or infer a graph dependency.

Both #193/#198 wakes remain unchanged and pending. They still require
Local's verification of the actual #166 product-repair merge, not #204
acceptance, artifact landing, comparison success or readiness. No new
owner, occurrence, retry, runtime permission, #166/Watcher acceptance,
CI/debt clearance, hardening/closure, release or freeze credit follows.
