# Repair Windows InstalledCurrent ForeignDaemon reattachment

- **Workstream:** `local-daemon`
- **Node:** `foreigndaemon-reattach-repair`
- **Type:** implementation
- **Status:** in progress; one owner already launched
- **Attention:** focus
- **Depends on:** none; coordinate overlap, do not infer a dependency
- **Tracker:** [#204](https://github.com/lossyrob/telex/issues/204)
- **PR:** pending; one separate PR must close #204
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
