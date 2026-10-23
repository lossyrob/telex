# watcher-macos-containment

## Goal

Complete the operator-selected macOS Watcher contract for trusted
non-detaching detectors. Preserve the native ARM64 and x64 Phase 1 evidence,
reconcile and merge the canonical product design, then implement and prove
ordinary-user containment of the Watcher-assigned attempt process group. The
runtime-core sequencing prerequisite was satisfied by PR #164 merge
`d0237dbb0b50dafd5518d403a5ff8d81cd5ee25c`.

Windows and Linux retain their existing detached-descendant guarantees. The
Windows-path five-minute gate remains independent. #162 continues to fail
closed on macOS until this task lands.

## Status and sequencing

Status: in progress.

Phase 1 investigation is complete. On 2026-10-22 the operator selected
`Revise macOS to the non-detaching detector contract` in issue #165. The
runtime-core merge prerequisite is satisfied; PR #170 merged #162's independent
documentation correction as main 2513ccda, and issue #162 may remain open for
its bounded unsupported-v1 removal repair, post-merge validation, and field
report. On 2026-10-23 the design steward approved the candidate Phase 2 proof
shape, subject to corrections, and kept product implementation on hold. Product
implementation remains held until PR #168 merges the clarified ADR 0054 and
normative Watcher text, the Phase 2 plan incorporates that disposition, pinned
XNU source reasoning and native ARM64/x64 evidence establish the
fork/group-empty proof, and the owner returns the selected algorithm and
evidence for implementation authorization. The node remains in progress with
focus attention; it is not blocked on #162 closure.

At exact proof head 810bd24fe7ca3415a4b32c19b762ed32a68d952b, native ARM64 and
x86_64 run 39904383638 rejected the one-shot SIGKILL completion candidate: a
barrier-controlled in-group descendant survived the accepted group SIGKILL and
answered two post-seal challenges one second apart. Both post-reap observations
reported presence, and ESRCH appeared only after private teardown. The result
preserves the pinned-source old-group closure argument and fail-closed receipt
rule but proves that one accepted group SIGKILL is not a completion condition.
Product implementation remains held. Any successor must retain the destructive
anchor and nonzero-signal authority until a separately justified completion
transition, then seal, exactly reap, and observe under the existing proof rules;
if no ordinary-user completion condition can be proved, return decision-needed.

The owner session is `d6ee8a1d-e581-4f6c-a42b-77c84bfccc8b`, worktree
`lossyrob-jubilant-bassoon`, branch `lossyrob-macos-watcher-containment`, base
`ec98b3ee`. The merge gate is shadow; the operator merges.

## Phase 1 evidence

Native ARM64 and x64 probes established that:

- recursive kqueue descendant tracking is unavailable because `NOTE_TRACK`,
  `NOTE_TRACKERR`, and `NOTE_CHILD` return `ENOTSUP`;
- an ordinary process group cleans up descendants only while they remain in the
  group;
- `setpgid`, `setsid`, and double-fork escapes survive process-group cleanup;
- surviving escaped descendants answered a private challenge after runtime
  death and reap; and
- the probes cleaned up only through their private control pipe and never by
  signaling discovered orphan PIDs.

Authority: commit `8545dcfd882b3f6386849f47afe70b03d5af91e6`, run
`39855202278`, ARM64 archive SHA-256
`0689a4cce00df132f82ea4382b8a9fef85d4795a7b12b692eec6344da7f975a5`, and
x64 archive SHA-256
`68f5a136e61fdccd7a7e9f8330c7bfd9ab30ab7e77ebf4d1b4bfc555454b9c6f`.

## Selected macOS contract

A conforming macOS detector and every local descendant it creates must remain
in the Watcher-assigned attempt process group until the attempt ends. Ordinary
fork/exec that preserves the group is supported. Daemonization, `setsid`,
process-group changes, spawn attributes that create another group or session,
and attempt-scoped local background work outside the group are nonconforming.
Work that escapes the group is outside the cleanup guarantee.

This is a trusted behavioral precondition, not a sandbox. Watcher may not
detect every violation. A macOS terminal receipt proves only that the assigned
process group ended.

## Phase 2 implementation obligations

The implementation must:

1. preserve #162's fail-closed macOS behavior until the complete #165 change is
   accepted;
2. durably record, before detector release, a versioned identity containing the
   boot session, guard PID/start identity, assigned PGID, PID/PGID-reuse-safe
   group anchor, receipt path, nonce, and `assigned-process-group` scope;
3. keep the containment owner outside the detector group and hold the detector
   launch gate until handshake and durable identity recording complete;
4. establish the gated trampoline that becomes the detector root as the assigned
   process-group leader before durable launch release; retain
   PID/PGID-reuse-safe destructive authority through every nonzero group signal;
   irreversibly seal all signaling and numeric-authority reconstruction before
   exact leader reap; and permit only read-only former-group observation
   afterward;
5. observe Watcher death through a private liveness channel and clean the whole
   assigned group on runtime death, root exit, timeout, cancellation, output
   bound, or explicit cleanup;
6. never infer cleanup from leader exit, never issue a nonzero signal after the
   anchor is released, and never accept numeric-group absence without the
   pinned-source and native fork/reuse proof required by this task;
7. write and sync a terminal receipt only after exact leader reap and race-safe
   ESRCH proof that the assigned group is absent;
8. treat guard failure before launch as no launch and guard failure after launch
   as unproved unless an identity-matched terminal receipt exists;
9. retain `orphan-containment-unproven`, pause the watch, and start no
   replacement after missing, stale, malformed, or ambiguous evidence;
10. keep containment evidence independent of send reconciliation, detector-state
    commit, resume, store binding, and production-readiness fences; and
11. report `decision-needed` rather than claim success if ordinary-user macOS
    APIs cannot provide race-safe ownership and terminal proof.

## Runtime machinery boundary

Extending the existing private `watcher_process_guard` pattern is within this
node's runtime authority only if the process is ordinary-user, per-attempt,
ephemeral, shipped with Watcher, controlled only by inherited private handles,
not installed or launchd-managed, not shared between attempts or runtimes, not
independently addressable, and exited after evidence finalization.

Any privilege, entitlement, installed or persistent helper, shared broker,
public/reusable IPC endpoint, cross-principal command surface, system service,
or VM returns to the operator before implementation.

## Required validation

Run native ARM64 and x64 macOS validation covering:

- launch-gate failure before detector release;
- a root plus multiple generations of conforming descendants that preserve the
  assigned group;
- root exit while descendants remain;
- timeout, cancellation, normal completion, output-limit termination, and
  abrupt Watcher runtime death;
- guard loss before and after detector release;
- runtime/guard death races and restart while cleanup is in progress;
- PID and PGID reuse pressure, anchor release ordering, and stale identity;
- mechanically prove that no timeout, retry, cancellation, error, Drop, restart,
  or finalization path can reconstruct nonzero signal authority after the seal;
- pin the XNU group lookup, membership removal/destruction, group creation, fork
  inheritance, and kill iteration paths used by the proof, and verify the same
  semantics on both tested native OS releases;
- use barriers, not sleeps, to cover fork insertion versus final group signal,
  leader reap, ESRCH observation, and receipt publication;
- durable receipt creation, fsync, nonce/identity mismatch, malformed receipt,
  and boot-session change;
- unrelated-process survival and no process-table scan followed by arbitrary
  PID signaling;
- a deliberately detaching fixture proving that any receipt is scoped to the
  assigned group and is not reported as arbitrary-descendant cleanup; and
- SQLite and credentialed PostgreSQL lifecycle/health parity without weakening
  sender, receipt, store-binding, or readiness fences.

The selected destructive anchor, signal-seal, and group-empty algorithm require
pinned XNU source reasoning plus barrier-controlled native race evidence; stress
alone is insufficient. An unreaped direct group leader held through every
nonzero signal, followed by irreversible seal, exact reap, and read-only ESRCH
observation, is an admissible candidate, not accepted proof, until that evidence
establishes fork closure and PID/PGID reuse safety.

## Success criteria

- ADR 0054 and the normative Watcher design define the selected host contract
  without weakening Windows/Linux.
- #162 remains fail-closed on macOS until the full #165 implementation lands.
- Conforming detector attempts are cleaned up after every required termination
  path and abrupt runtime death on native ARM64 and x64 macOS.
- Terminal evidence is durable, identity matched, group scoped, and never
  overclaims cleanup of nonconforming escaped work.
- Guard failure and restart ambiguity fail closed with no replacement attempt.
- No privileged, entitled, installed, persistent, brokered, launchd-managed, or
  VM architecture is introduced.
- Required CI, independent implementation review, exact-head design review, and
  shadow merge-gate evidence pass at the final head.

## Invalidation conditions

Return `decision-needed` and do not call the node ready if:

- the implementation assumes that group-leader exit proves group cleanup;
- the destructive-ownership anchor can be released before every nonzero signal
  is complete and every signal-authority reconstruction path is irreversibly
  sealed, any nonzero signal can occur after the seal, or a reused PID/PGID can
  be signaled as the old attempt;
- fork/exit/kill races can leave a conforming in-group descendant after a
  successful receipt;
- runtime death can close the liveness path without causing group cleanup;
- guard failure is treated as success or a missing receipt is inferred as
  ended;
- restart can launch a replacement while containment remains ambiguous;
- evidence claims cleanup of a deliberately detached nonconforming process;
- the implementation requires privilege, entitlement, installation,
  persistence, launchd management, a shared broker, or a VM without a new
  operator decision; or
- Windows/Linux guarantees, receipt-gated state, store binding, sender
  reconciliation, or readiness fences are weakened.
