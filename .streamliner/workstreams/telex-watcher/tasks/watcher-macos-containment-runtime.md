# watcher-macos-containment-runtime

## Goal

Implement and prove ordinary-user, per-attempt macOS containment for the trusted
non-detaching detector contract owned by `watcher-macos-containment` under
issue #165. Deliver one product PR that says `Closes #172`.

Windows and Linux retain their existing detached-descendant guarantees. #162
continues to fail closed on macOS until this node lands. The Windows-path
five-minute gate does not depend on this node; the product-level usable-watcher
checkpoint and closure do.

## Status and sequencing

Status: in progress, with focus attention. Source and native proof work is
active. Product integration is held until the implementation authorization gate
below clears.

This node was split from #165 on 2026-10-23 under the operator's one-item,
one-PR rule. Issue #172 holds the full successor specification. #165 now covers
only the macOS non-detaching detector contract delivered by PR #168.

The node depends on `watcher-macos-containment`, `watcher-runtime-core`, and
`watcher-host-containment-doc-alignment` under #177. #177 is a prerequisite
for #172 product-PR readiness and closure only; it does not stop the active
source and native proof work and grants no implementation authorization. The
runtime-core merge prerequisite was satisfied by PR #164 merge
`d0237dbb0b50dafd5518d403a5ff8d81cd5ee25c`; PR #170 merged #162's independent
documentation correction as main 2513ccda. PR #173 merged #162's bounded repairs
on 2026-10-23 as main `55cb6b116607bceccf3ce0b72550e2d849ca5426`, and issue #162
closed on merge; its post-merge validation and field report remain outstanding.
This node does not depend on that validation or report. On 2026-10-23 the design
steward approved the candidate Phase 2 proof shape, subject to corrections, and
kept product implementation on hold.

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

The owner session is `d6ee8a1d-e581-4f6c-a42b-77c84bfccc8b`. Campaign runs the
merge gate and merges at the exact cleared SHA.

## Outcome

Implement ordinary-user, per-attempt macOS containment for the trusted
non-detaching detector contract:

- establish the gated trampoline as the detector root and the assigned group
  leader;
- durably record boot, guard, leader, PGID, anchor, nonce, and receipt identity
  before release;
- keep the guard outside the group, with private runtime-liveness observation;
- terminate the whole assigned group on every cleanup path;
- retain reuse-safe destructive authority through every nonzero signal;
- seal authority irreversibly before the exact leader reap;
- accept terminal evidence only after the approved completion predicate and an
  ESRCH-only proof that the old group is absent; and
- preserve fail-closed restart and guard-loss recovery without affecting
  independent pending-operation reconciliation.

## Implementation authorization gate

Product code stays held until all of these are true:

1. PR #168 has merged.
2. The plan incorporates every design-steward correction.
3. The pinned XNU reasoning is mapped to both target releases.
4. Barrier-controlled native ARM64 and x64 evidence proves the selected
   completion algorithm, fork/spawn/exec closure, PID/PGID reuse safety, and
   that no signal authority is reconstructed after the seal.
5. The owner returns the selected algorithm and evidence to the design steward,
   who gives final authorization.

If no ordinary-user completion predicate can be proved, this node returns
`decision-needed`.

## Boundaries

- In scope: only an unprivileged, ephemeral, per-attempt, shipped guard that is
  controlled through an inherited private handle and is nonpersistent,
  nonshared, and non-addressable.
- Returns to the operator: privilege, entitlements, a launchd installation, a
  persistent or shared helper, a broker or public IPC, any cross-principal
  surface, or a VM.
- Unchanged: Windows and Linux behavior, and all receipt, store, and readiness
  fences.
- The normative contract is owned by #165 (`docs/design/watcher.md` and ADR
  0054).
- The Windows-path five-minute gate does not depend on this node. Product-level
  usable-watcher and closure do.

## Runtime machinery boundary

Extending the existing private `watcher_process_guard` pattern is within this
node's runtime authority only if the process is ordinary-user, per-attempt,
ephemeral, shipped with Watcher, controlled only by inherited private handles,
not installed or launchd-managed, not shared between attempts or runtimes, not
independently addressable, and exited after evidence finalization.

Any privilege, entitlement, installed or persistent helper, shared broker,
public/reusable IPC endpoint, cross-principal command surface, system service,
or VM returns to the operator before implementation.

## Phase 2 implementation obligations

The implementation must:

1. preserve #162's fail-closed macOS behavior until the complete #172 change is
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

## Evidence and holds carried from #165

None of these is implementation authorization.

- Phase 1 native investigation: commit
  `8545dcfd882b3f6386849f47afe70b03d5af91e6` and run `39855202278`, recorded in
  `watcher-macos-containment.md`.
- Anchor/source disposition: SHA-256
  `da1928575fdf2c22ab939fd39bac9a7d85db9b6d454f6d2eb8762ebf6e70f2e7`.
- Residual disposition: SHA-256
  `e5fae25bf1e2fe521ccf243b63fe4d18d6aff885895706893d8d0d50273facef`.
- Pinned-source old-group closure argument, with the replacement-group
  correction:
  https://github.com/lossyrob/telex/issues/165#issuecomment-6357895152
- Native run `39904383638` at `810bd24fe7ca3415a4b32c19b762ed32a68d952b`, which
  rejects one-shot SIGKILL completion without a false absence:
  https://github.com/lossyrob/telex/issues/165#issuecomment-6358097317
- Source approval of the sole-terminal-anchor census proposal (`int[2]`, a fresh
  WNOWAIT plus census conjunction, sole non-shadow public representative
  semantics, and conservative ESRCH for inert shadows):
  https://github.com/lossyrob/telex/issues/165#issuecomment-6358285871

## Required validation

Run native ARM64 and x64 macOS validation covering:

- launch-gate and setup failure before detector release;
- a root plus multiple generations of conforming descendants that preserve the
  assigned group, through fork/exec, `posix_spawn`, and public `vfork` routes;
- root exit while descendants remain;
- timeout, cancellation, normal completion, output-limit termination, and abrupt
  Watcher runtime death;
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
- completion-census zero, error, saturation, malformed, and mismatch results,
  and exec-shadow overlaps;
- a negative control that preserves the one-shot SIGKILL failure;
- durable receipt creation, fsync, nonce/identity mismatch, malformed receipt,
  and boot-session change;
- unrelated-process survival and no process-table scan followed by arbitrary PID
  signaling;
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

- #162 remains fail-closed on macOS until the full #172 implementation lands.
- Conforming detector attempts are cleaned up after every required termination
  path and abrupt runtime death on native ARM64 and x64 macOS.
- Terminal evidence is durable, identity matched, group scoped, and never
  overclaims cleanup of nonconforming escaped work.
- Guard failure and restart ambiguity fail closed with no replacement attempt.
- No privileged, entitled, installed, persistent, brokered, launchd-managed, or
  VM architecture is introduced.
- When the capability ships, the same product PR updates the macOS tracker
  reference in `docs/watcher-runtime.md` from #165 to #172.
- The shipping PR must remove the temporary fail-closed sentence in
  `docs/design/watcher.md` and rewrite the macOS containment bullet into
  unqualified present tense only if the complete runtime implementation,
  source/native proof, guard-loss/restart behavior, and final design
  authorization land together at that exact head.
- One product PR says `Closes #172`. Required CI, independent implementation
  review, and exact-head design review pass at the final head; Campaign runs the
  merge gate and merges at the exact cleared SHA.

## Invalidation conditions

Return `decision-needed` and do not call the node ready if:

- no ordinary-user completion predicate can be proved;
- the implementation assumes that group-leader exit proves group cleanup;
- the destructive-ownership anchor can be released before every nonzero signal
  is complete and every signal-authority reconstruction path is irreversibly
  sealed, any nonzero signal can occur after the seal, or a reused PID/PGID can
  be signaled as the old attempt;
- fork/exit/kill races can leave a conforming in-group descendant after a
  successful receipt;
- runtime death can close the liveness path without causing group cleanup;
- guard failure is treated as success or a missing receipt is inferred as ended;
- restart can launch a replacement while containment remains ambiguous;
- evidence claims cleanup of a deliberately detached nonconforming process;
- the implementation requires privilege, entitlement, installation, persistence,
  launchd management, a shared broker, or a VM without a new operator decision;
  or
- Windows/Linux guarantees, receipt-gated state, store binding, sender
  reconciliation, or readiness fences are weakened.
