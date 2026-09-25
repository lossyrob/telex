# PostgreSQL wait-reset recovery

- **Workstream:** `local-daemon`
- **Node:** `postgres-wait-reset-recovery`
- **Type:** implementation
- **Status:** in progress; source-quiet pending finalized Tier B landing,
  independent landing verification, and separate M2 write authority
- **Attention:** focus
- **Depends on:** none
- **Blocks:** `schema3-release-preparation`
- **Owner:** known worker `38d183fb-df5d-4e65-88f3-e27b62e3c92f`,
  source-quiet until separate campaign M2 write authority
- **Tracker:** [lossyrob/telex#155](https://github.com/lossyrob/telex/issues/155)
- **Adopted PR:** [lossyrob/telex#156](https://github.com/lossyrob/telex/pull/156)
- **Published branch/head:** `copilot/fix-postgres-connection-reset` at `25aea118e04c4e93eb4e748fe7c1989338931ac2`
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
- Preserved source-quiet worker checkpoint
  `5d9566182429066ee8b51cd8dd569065f9abdb5a`, with unpublished M1, C1, and C2
  commits and intentionally red untracked M2 evidence in
  `tests/credential_command.rs`. No M2 production mechanism exists.
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
2026-09-25T13:41:31-04:00. This acceptance selects the engineering design and
numeric limits for later product promotion. It does not mean the design is
implemented, measured, runtime-proven, or authorized for product writes.

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

Use an invocation-private noninheritable kill-on-close job. Retain documented
process and primary-thread handles, create suspended, assign before resume, allow
no breakaway fallback, and clean up setup failures explicitly. Assignment or
containment failure must not resume an uncontained shell. Normal receipt
requires observed process termination, zero active job processes, completed or
closed owned I/O, and worker-thread completion. A termination request or job
close alone is not receipt.

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
ownership, `FAILED_HELD`, and receipt design with all conditions above. The
process-local owner must survive Tokio runtime drop while its embedding process
stays alive, but it cannot outlive host process exit. M2 product writes remain
held pending mechanical delta review, campaign exact-blob landing authority,
independent landing verification, and separate same-worker write authority. No
runtime validation, M2 closure, thread resolution, merge, or release authority
follows from technical acceptance.

## Required M2 proof after implementation

- Run genuine Windows, Linux, and macOS lifecycle tests for every claimed support
  target. Compile-only or skipped coverage is not runtime proof.
- Cover ordinary helper and shell completion with inherited and redirected
  pipes. Observe helper liveness before cleanup and production receipt before
  independent fixture fallback. Never issue receipt while an ordinary helper
  remains.
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
- Cover Windows setup and nested-job failures, actual process and job completion,
  I/O and thread completion, and the rule that close or termination request alone
  is not receipt.
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

- Product launch is conditional on campaign-approved Tier B landing and exact
  verification. Before resuming PR #156, establish one writer and verify that no
  cloud or legacy writer overlaps.
- The Local Daemon orchestrator prepares the external repair session through
  Streamliner, registers the exact prepared checkout path in branch mode, verifies
  `session-online`, grants standalone write authority, and obtains acknowledgement
  before product writes.
- This is one of exactly three new delivery sessions in the packet: two repair
  sessions and one later release worker. Do not create dormant placeholders.
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
