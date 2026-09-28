# PostgreSQL wait-reset recovery

- **Workstream:** `local-daemon`
- **Node:** `postgres-wait-reset-recovery`
- **Type:** implementation
- **Status:** completed; PR #156 merged exact head `eda9ac24` at
  2026-09-28T19:32:21Z as `62291a78`, and issue #155 closed as completed.
  Release publication remains a separate operator gate
- **Attention:** focus
- **Depends on:** none
- **Blocks:** `schema3-release-preparation`
- **Owner:** worker `38d183fb-df5d-4e65-88f3-e27b62e3c92f` under direct
  end-to-end solution, experiment, implementation, test, push, and review-fix
  authority
- **Tracker:** [lossyrob/telex#155](https://github.com/lossyrob/telex/issues/155)
- **Adopted PR:** [lossyrob/telex#156](https://github.com/lossyrob/telex/pull/156)
- **Merged branch/head:** `copilot/fix-postgres-connection-reset` at `eda9ac24ddbad29251f1141c68c5cd687ebd3599` (branch retained)
- **Merge commit:** `62291a788f33c948ecfa6cca8e3fdb2b9374b7fd`
- **Parent workstream:** [lossyrob/telex#32](https://github.com/lossyrob/telex/issues/32)
- **Campaign:** [Addressable Attention #102](https://github.com/lossyrob/telex/issues/102)

## Outcome

Complete issue #155 by adopting PR #156 in place. Preserve its code, branch,
commits, and review history. Do not replace the branch or recreate the repair.

Recover active waiters from transient PostgreSQL query and `LISTEN` connection
resets while pull and push stations share one daemon. Preserve finite wait
deadlines during recovery, bound retry, and return an actionable terminal outcome
when recovery is exhausted. The operator selected a repair, not an accepted
limitation.

## Inputs

- Issue #155 incident sequence and expected recovery behavior.
- PR #156 at published head
  `25aea118e04c4e93eb4e748fe7c1989338931ac2`.
- Historical source-quiet worker checkpoint
  `5d9566182429066ee8b51cd8dd569065f9abdb5a`, with unpublished M1, C1, and C2
  commits and intentionally red untracked M2 evidence in
  `tests/credential_command.rs`. At that checkpoint no M2 production mechanism
  existed; current mutable implementation and experiment results are not proof
  in this artifact.
- Existing daemon reconnect, watchdog, finite-deadline, status, and exit-code
  contracts.

## Accepted M2 lifecycle policy

The operator selected `Approve the one-shot invocation contract (Recommended)`
for `--password-command`.

- Normal, error, and cancellation completion clean up helpers that Telex owns and
  that remain within the supported invocation scope.
- Intentionally persistent, escaped, or broker-launched background work is outside
  this command lifecycle contract.
- Querying an already-running external credential agent does not give Telex
  ownership of that agent and does not authorize Telex to terminate it.
- Windows jobs and Unix process groups have different scope and escape limits.
  The contract does not promise universal all-descendant containment or
  fail-closed detection of every Unix escape.

This is policy authority only. PR #156 must later promote the accepted contract
into normative product documentation and code through its original review and
merge path.

## Accepted M2 intended technical design

Campaign accepted the exact reviewed intended model on
2026-09-25T13:41:31-04:00. The operator selected the Windows job-terminal
receipt on 2026-09-28T09:47:17-04:00. These decisions select the intended
engineering design and limits. They do not mean the design is fully
implemented, measured, runtime-proven, reviewed at a new exact head, merged, or
released.

- Use one credential-specific, process-local admission and ownership registry
  across all password-command calls and Tokio runtimes in that host. Do not add a
  global service or general process manager.
- Use aggregate capacity `C=2` and cleanup observation target `B=3s`. Both
  targets are approved intended limits and remain unmeasured. `B=3s` reuses the
  existing recovery grace; it is not a measured OS reap guarantee. The two slots
  permit one canceling or cleaning invocation while an independent source
  proceeds; the same-source barrier still prevents one source from using both.
- Count setup, running, output collection, cancellation, completed-but-unjoined
  work, and `FAILED_HELD` toward capacity. Keep the same-source barrier through
  cleanup.
- A caller waiting for admission uses its existing budget. Before admission, it
  starts no process, native worker, or reader and enters no internal payload
  queue. Capacity pressure is not bad credentials and does not reset a caller
  clock.
- The native owner retains process scope, stdout/stderr pipes, cancellation,
  result publication, cleanup receipt, and worker-thread completion independently
  of Tokio. Cancellation and result publication use one consistent transition;
  no partial or post-cancellation credential may publish.
- Collect stdout and stderr through owned nonblocking readers, with no detached
  readers. Preserve inherited environment and working directory, existing shell
  parsing, complete UTF-8 output and trimming, and ordinary nonzero behavior. Do
  not add an output truncation policy. Diagnostics must not expose commands,
  credentials, environment, DSNs, or unredacted helper stderr.
- Normal, error, and cancellation completion clean up owned in-scope helpers.
  Cleanup observation timeout or failure enters `FAILED_HELD`: retain the exact
  invocation registry record, including its slot, source reservation, owned
  Windows process and job handles or Unix unreaped leader, pipes, and
  worker-thread obligation. The owning host retains that record, closes affected
  admission, and reports a sanitized failure naming the obligation. Do not claim
  a receipt, restart automatically, discard ownership, start a hidden forever
  worker, or create a replacement cleanup queue. Permanently holding every
  ordinary Unix completion is not an adequate implementation.

### Accepted intended Windows receipt

The operator answered the following question through the designated worker at
2026-09-28T09:47:17-04:00:

> May I adopt the explicit Windows job-terminal completion contract described
> above, accepting its residual kernel/driver/I/O limitations instead of
> requiring every former descendant process handle to be signaled before
> completion?

Exact choice:
`Approve Windows job-terminal completion with the documented limits
(Recommended)`.

Use an invocation-private noninheritable single-use kill-on-close job. Retain
documented process and primary-thread handles, create suspended, assign before
resume, allow no breakaway or uncontained fallback, and clean up setup failures
explicitly. Before eligible atomic credential-result publication and
source/admission release, require:

1. successful checked termination of the exact owned private job;
2. a successful checked query reporting zero active job processes;
3. `WAIT_OBJECT_0` for the exact launched leader process handle;
4. checked completion or closure of owned I/O and handles; and
5. native owner completion and join.

Finalization yields an explicit success or named failure before publication and
release. Pending, cancellation, receipt-ready, and `FAILED_HELD` arbitration is
consistent: cancellation or failure prevents credential eligibility, close or
join failure retains the still-valid obligation, and no closed handle is
reconstructed, retried, or resurrected. A termination request, job close, or
zero active count alone is not receipt. No debugger is used.

Normal credential success still requires the original successful shell status,
complete stdout and stderr through EOF, full UTF-8 decoding, and the existing
trim. A finite inherited helper that still writes keeps the invocation
collecting. Cancellation or error may close owned I/O but cannot publish a
partial or post-cancellation credential.

This is an explicit weakening of the earlier all-descendant-signaled condition.
Former descendant handles may remain nonsignaled during kernel or driver
rundown. Receipt does not prove every former process object signaled, every
kernel or driver operation or previously issued external I/O finished, every
external reference disappeared, or a finite bound on those residual objects.
`C=2` bounds Telex invocation owners and reservations, not descendant count or
all residual Windows resources. Preserve the original stronger A/B observations
as historical red evidence; they are not retroactive passes or measured harm.

For the advisory diagnostic, an external observer or guardian owns a separate
challenge channel and retained observation handles. Before cancellation, that
controller or guardian, outside the invocation job, must complete a successful
round trip over the separate diagnostic channel, not credential stdout, with
the same ready, membership-verified ordinary child inside the job; no external
proxy may answer. Channel setup or round-trip failure is failure or inconclusive
evidence, and the guardian retains exact known fixture cleanup ownership.
Generate a fresh unpredictable challenge only after actual publication
following join and invocation-owned handle finalization. A valid fresh reply
falsifies the operational interpretation for that run. Nonresponse supports no
universal quiescence claim. An observer-retained job handle means the
invocation closes its last owned handle, not the last system handle; explicit
job termination drives receipt.

Canonical Windows receipt references:

- [Job objects](https://learn.microsoft.com/en-us/windows/win32/procthread/job-objects)
  and [nested jobs](https://learn.microsoft.com/en-us/windows/win32/procthread/nested-jobs)
  define association, inheritance, accounting, and termination scope.
- [`TerminateJobObject`](https://learn.microsoft.com/en-us/windows/win32/api/jobapi2/nf-jobapi2-terminatejobobject)
  defines scoped termination; it is not a synchronous wait for every former
  descendant handle.
- [`QueryInformationJobObject`](https://learn.microsoft.com/en-us/windows/win32/api/jobapi2/nf-jobapi2-queryinformationjobobject)
  and
  [`JOBOBJECT_BASIC_ACCOUNTING_INFORMATION`](https://learn.microsoft.com/en-us/windows/win32/api/winnt/ns-winnt-jobobject_basic_accounting_information)
  define the checked exact-job active-process observation.
- [`WaitForSingleObject`](https://learn.microsoft.com/en-us/windows/win32/api/synchapi/nf-synchapi-waitforsingleobject)
  and [terminating a process](https://learn.microsoft.com/en-us/windows/win32/procthread/terminating-a-process)
  define exact leader signaling separately from object lifetime.
- [`CloseHandle`](https://learn.microsoft.com/en-us/windows/win32/api/handleapi/nf-handleapi-closehandle)
  supplies the checked Telex-owned handle-release result.

### Accepted intended Unix receipt sequence

Campaign accepted the steward's technically feasible Unix receipt design with
the following supported-API conditions. Acceptance is not implementation or
runtime proof.

1. Before exec, establish a dedicated group with leader PID equal to PGID and
   greater than one.
2. Give the native owner exclusive wait authority for the leader. Retain the
   waitable unreaped leader through every mutating group signal; nonconsuming
   leader observations may be used.
3. Send the final termination signal while the identity anchor remains valid.
   Then enter an irreversible no-more-group-signals state.
4. Observe and reap only the exact owned leader. `ECHILD` or evidence of a
   competing reaper is an ownership error, not receipt.
5. After reap, use the former numeric PGID only as a read-only observation key
   within the same cleanup budget. No normal, error, retry, cancellation, Drop,
   or operator-recheck path may send a nonzero signal to that identifier.
6. Release ownership and admission only after exact leader reap, conclusive
   platform group absence, completed or closed owned I/O, and native owner
   completion and join.

An ordinary non-escaped member keeps the original group extant after leader
reap. Conclusive absence therefore proves the original group ended. Later
numeric reuse can cause a conservative presence or error hold, but cannot
restore the ended group or confer signaling authority. Direct-child reap is not
grandchild reap, and a kill request, shell exit, or pipe EOF alone is not
receipt.

### Linux read-only absence predicate

Call `getpriority(PRIO_PGRP, P)`, never `setpriority`, in the native PID
namespace and a supported syscall context. Clear thread-local `errno` before
each call.

- Return `-1` with `errno == ESRCH` is the only conclusive absence result.
- Any successful result is presence. Nice value `-1` with `errno == 0` is valid
  presence; other returned priority values are also presence.
- `EINTR` permits retry of the same read-only query within the same absolute
  budget.
- `EPERM`, `EACCES`, `EINVAL`, `ENOSYS`, unexpected errors, unknown context, or
  known sandbox or interposer fabrication are not absence and cannot produce
  receipt.

This native group query avoids the Linux `kill(-P, 0)`
`security_task_kill` existence-concealment ambiguity. It does not promise
universal detection of fabricated syscall results.

### macOS read-only absence predicate

Use the supported libc POSIX/UNIX03 `kill(-P, 0)` binding. Do not use a private
raw syscall or legacy non-POSIX variant. Implementation review must verify the
actual binding and supported-target conformance.

- Return zero is presence.
- Return `-1` with `ESRCH` is conclusive absence under the supported POSIX-mode
  semantics.
- `EPERM`, including found-but-unsignalable, MAC-denied, or zombie-only cases,
  is presence or inconclusive and must never become absence.
- Unknown ABI, denial, unexpected errors, or query limitations retain pending
  ownership or enter `FAILED_HELD`; they cannot produce receipt.

Do not generalize the Linux `getpriority` predicate to macOS or the macOS
POSIX/UNIX03 null-signal predicate to Linux.

### Errors, races, and retained ownership

Final `SIGKILL` failure is not absence. Partial delivery, changed permissions,
missed concurrent forks, ordinary surviving helpers, zombies, reparenting, and
slow adopter reaping can retain the group through or beyond `B=3s`. Telex reaps
only its exact owned child. Reparenting does not erase process-group membership.
Telex sends no post-reap catch-up signal.

Presence, possible reuse, inaccessible observation, or unexpected error retains
the cleanup obligation. After reap, `FAILED_HELD` retains the exact invocation
record, slot, source reservation, responsible host, observation key, pipes, and
native thread obligation, but no signal authority. It reports a sanitized
failure and permits no automatic restart, ownership discard, growing retry
queue, or clean-exit claim.

### Canonical receipt references

- POSIX process groups and lifetime:
  [Base Definitions 3.283](https://pubs.opengroup.org/onlinepubs/9799919799/basedefs/V1_chap03.html#tag_03_283)
  and [Process ID Reuse](https://pubs.opengroup.org/onlinepubs/9799919799/basedefs/V1_chap04.html#tag_04_17).
- POSIX operations:
  [`waitid`](https://pubs.opengroup.org/onlinepubs/9799919799/functions/waitid.html),
  [`kill`](https://pubs.opengroup.org/onlinepubs/9799919799/functions/kill.html),
  and Linux [`setpgid`](https://man7.org/linux/man-pages/man2/setpgid.2.html).
- Linux predicate:
  [`getpriority`](https://man7.org/linux/man-pages/man2/getpriority.2.html),
  [`kernel/sys.c` v6.12 lines 297-359](https://github.com/torvalds/linux/blob/v6.12/kernel/sys.c#L297-L359),
  [`kernel/signal.c` lines 830-864](https://github.com/torvalds/linux/blob/v6.12/kernel/signal.c#L830-L864),
  and [`kernel/pid.c` lines 346-367](https://github.com/torvalds/linux/blob/v6.12/kernel/pid.c#L346-L367).
- macOS predicate:
  [libc kill wrapper](https://github.com/apple-oss-distributions/xnu/blob/f6217f891ac0bb64f3d375211650a4c1ff8ca1ea/libsyscall/wrappers/kill.c),
  [`kern_sig.c` lines 1642-1721](https://github.com/apple-oss-distributions/xnu/blob/f6217f891ac0bb64f3d375211650a4c1ff8ca1ea/bsd/kern/kern_sig.c#L1642-L1721),
  [`kern_proc.c` lines 2947-2967](https://github.com/apple-oss-distributions/xnu/blob/f6217f891ac0bb64f3d375211650a4c1ff8ca1ea/bsd/kern/kern_proc.c#L2947-L2967),
  and [`cdefs.h` lines 757-771](https://github.com/apple-oss-distributions/xnu/blob/f6217f891ac0bb64f3d375211650a4c1ff8ca1ea/bsd/sys/cdefs.h#L757-L771).

## Selected M2 shutdown policy

The operator selected
`Approve bounded orderly-exit drain and explicit failure hold (Recommended)` for
the following question:

> For normal Telex host shutdown, may credential cleanup add up to 3 seconds
> after logical command/Wait completion, with admission closed and owned workers
> drained/joined; if receipt is still missing at that bound, should Telex report
> cleanup failure and retain ownership rather than report a clean exit?

Only the host that owns credential work performs this drain. Logical command and
Wait result deadlines remain unchanged. Normal host exit may take up to three
additional seconds to close admission, cancel owned invocations, and drain and
join all workers concurrently against one absolute current cleanup budget. The
budget is not renewed per worker and does not grant `C * B` latency. A client for
a remote command does not drain work owned by the remote host.

Waiting three seconds is not proof of cleanup. If receipt remains missing, Telex
must report cleanup failure and retain named ownership and escalation,
potentially beyond three seconds. It must not report a clean exit or promise an
unconditional three-second OS process-exit bound.

Campaign technical acceptance selects `C=2`, the native owner and registry,
admission behavior, internal `B=3s` observation target, runtime-shutdown
ownership, `FAILED_HELD`, and the receipt designs with all conditions above.
The process-local owner must survive Tokio runtime drop while its embedding
process stays alive, but it cannot outlive host process exit. The same worker
now proceeds autonomously with design, isolated experiments, implementation,
tests, ordinary pushes, and review fixes. This artifact proposal, advisory
review, and reconciliation do not gate that ordinary work. Runtime validation,
M2 closure, thread resolution, merge, release, and publication remain pending.

## Required M2 proof after implementation

- Run genuine Windows, Linux, and macOS lifecycle tests for every claimed support
  target. Compile-only or skipped coverage is not runtime proof.
- Cover ordinary helper and shell completion with inherited and redirected
  pipes. Observe helper liveness before cleanup and production receipt before
  independent fixture fallback. On Unix, never issue receipt while an ordinary
  in-scope group member remains. On Windows, apply the selected job-terminal
  predicate and continue to report independently retained descendant-handle
  signaling as a separate oracle.
- For finite-deadline and recovery-grace cancellation, use a readiness barrier
  and measure response, cancellation signal, cleanup receipt, and process exit
  separately.
- Prove queued cancellation launches no process, worker, or reader. Cover
  cancellation during setup, resume, output collection, cleanup, and atomic
  result publication.
- Instrument every normal, error, retry, cancellation, Drop, and operator-recheck
  path so any nonzero Unix group signal after leader reap fails the test.
- On Linux, distinguish valid nice value `-1` with `errno == 0` from `ESRCH`,
  and cover read-only error handling. On macOS, prove the actual UNIX03 binding,
  permission denial, and zombie semantics.
- Cover controlled fork, reparent, zombie, and adopter-delay cases with
  conservative holds. Use a safe labeled observation seam for deterministic
  identifier-reuse behavior; do not churn host PIDs or signal guessed
  replacements.
- Cover Windows setup and nested-job failures, checked exact-job termination,
  zero active accounting, exact leader signaling, checked I/O and handle
  finalization, native join, and atomic publication/release ordering. Preserve
  the stronger A/B all-descendant-signaled failures as historical negative
  evidence. Record retained-handle lag independently, and use the external
  fresh-challenge oracle without treating nonresponse as universal proof.
- Under saturation and repeated cancellation, prove active owners never exceed
  the campaign-accepted capacity, one source cannot occupy both slots, no task,
  handle, reader, or cleanup-queue growth occurs, healthy established stores
  remain unaffected, and unrelated preexisting credential agents survive.
- Cover Tokio runtime drop while the embedding process stays alive, one absolute
  owning-host exit drain, and retained `FAILED_HELD` slot, identity, diagnostic,
  and escalation behavior.
- Preserve exact UTF-8 trimming, quoting, environment, working directory,
  nonzero exit, invalid-output, and read-error behavior.
- Label intentional escape and broker examples unsupported and use an independent
  fixture guardian for cleanup; never claim runtime ownership of escaped work.
- Preserve the existing negative Windows probe chronology. Existing probes and
  tests do not count as proof of the new mechanism.

This artifact role runs none of these product tests.

## Boundaries

### In scope

- Structural classification of transient query and `LISTEN` failures.
- Bounded reconnect/retry that keeps an eligible waiter armed.
- Preservation of the original finite wait deadline across recovery.
- Actionable exhausted-recovery status and documented exit behavior.
- Fault-injection coverage with pull and push stations on one daemon.
- Compatibility and design-impact inspection, exact-head review, and required CI.

### Out of scope

- Replacing, rebasing, force-pushing, or rewriting PR #156 or its branch.
- Treating the failure as an accepted limitation.
- Relaxing PostgreSQL identity, attendance, delivery, deadline, or schema guards.
- PR #138, issues #152/#153, Watcher/Station runtimes, and unrelated features.
- A global process manager, subreaper, service, process scan, unsupported kill,
  schema/auth/IPC-required-capability change, provider-state mutation, or shared
  database operation.
- Treating issue #155's same-image historical proof as a substitute for issue
  #157's genuine v0.1.2 ordered upgrade, daemon replacement, and fresh-install
  proof.

## Success criteria

- Transient query and `LISTEN` resets recover for representative pull and push
  delivery without dropping station registration.
- Recovery never extends a finite wait beyond its original deadline.
- Retry is bounded and exhausted recovery produces an actionable documented
  outcome instead of generic exit 1.
- Fault-injection tests exercise query reset and `LISTEN` reset while pull and push
  stations share one daemon.
- The exact head passes required CI, implementation review, and compatibility and
  design inspection before campaign-authorized merge.
- The selected one-shot credential policy is implemented without terminating an
  already-running external credential agent or claiming unsupported escaped or
  broker-owned cleanup.
- The campaign-accepted intended mechanism has genuine Windows, Linux, and macOS
  runtime proof; technical acceptance alone does not satisfy M2.

## Engagement

- The existing #155 worker owns autonomous end-to-end solution selection,
  bounded isolated experiments, code, documentation, tests, ordinary pushes,
  and review fixes. Do not require another plan, acknowledgement, preparation,
  artifact landing, or per-experiment permission for ordinary work.
- Material new user-visible guarantees or risk choices go directly to the
  operator. Advisory design review and Local artifact reconciliation proceed in
  parallel and do not suspend the worker.
- Final independent product review, required CI, design inspection, campaign
  merge authorization, and explicit operator publication approval still apply.
- Every new session or delegated agent must explicitly set
  `model=gpt-6-astra`, `reasoning_effort=high`, and
  `context_tier=long_context`; silent downgrade is not authorized. This artifact
  role creates or delegates none.
- Review checkouts must be physically distinct and read-only. Never use
  `open_pr_session` for a reviewer.
- Register external waits only through WATCHER
  `2a4bc4c8-1211-49d4-ba68-9d05d5d7530d`.
- Merge only the exact reviewed, green, inspected PR #156 head after campaign
  merge authorization.

## Terminal evidence

Statements above that describe implementation, proof, review, or merge as
pending or active are the contract as written before merge. The facts below
supersede them. The full record is on ledger item
local-daemon-postgres-wait-reset-recovery.

- Campaign authorized exact head da9ac24, and the Local orchestrator merged
  it with an ordinary guarded merge at 2026-09-28T19:32:21Z as 62291a78
  (parents 4eb95b4 and da9ac24). The first-parent diff is exactly 33
  paths (9326 insertions, 229 deletions) with no .streamliner paths. Issue
  #155 closed as completed at 19:32:23Z.
- Review: the original COMMENT review 5311857915 at 25aea118 is preserved.
  COMMENT review 5342232604 covered a full M2 rebaseline at 836efbc plus the
  causal notify delta at c71befc, with one LOW proof warning (S1). The clean
  delta at  38eb102 resolved S1 with an actual nested-hierarchy proof, and
  the clean delta at da9ac24 kept the cumulative result at 0 blockers and 0
  warnings. All four original threads were replied to directly and resolved.
- CI run 36470043028 on da9ac24 passed all 15 jobs, including Live
  PostgreSQL, Windows, macOS credential, feature combinations, both fallback
  E2Es, and eight alignment profiles. Earlier failures in runs 36448934676
  (836efbc, Live PostgreSQL) and 36464472486 ( 38eb102, Windows readiness)
  remain historical and are recorded as
  local-daemon-postgres-listen-ci-proof and
  local-daemon-credential-helper-readiness-ci.
- Design inspection by steward f271672 at da9ac24 passed with 0 blockers
  and 0 warnings (report SHA-256
  170696e70ec1685c8a6c769f2c44fc3aea7dbcd0310a79f816ad080e0efe5c4e).
- Hosted macOS runtime proof exists: job 109018355750 at 836efbc and the
  dedicated macOS job in the final run. The 2026-09-25 compile-only checkpoint
  remains dated history. Worker-reported Linux 42/0 evidence is source-pinned
  and historical, not final-head proof.
- Limits that remain: no unconditional three-second exit, since FAILED_HELD
  may hold longer; the selected Windows job-terminal residual limits; macOS
  EPERM is never absence; R2 transport was inconclusive; legacy-process proof
  is same-image and source-pinned, not a genuine v0.1.2 deployment; and the old
  A and B strong red packets remain red.
- Campaign independently verified both repair merges, both closed issues, and
  their ancestry in 62291a78, closing the dependencies of
  schema3-release-preparation. That node stays planned until an actual
  worker launches. No release gate, publication, hardening, or closure claim
  follows.
