# watcher-macos-containment-runtime

## Goal

Implement and validate ordinary-user, per-attempt macOS containment for the trusted
non-detaching detector contract owned by `watcher-macos-containment` under
issue #165. Deliver one product PR that says `Closes #172`.

Windows and Linux retain their existing detached-descendant guarantees. #162
continues to fail closed on macOS until this node lands. This node gates
`closure-gate` and `closure-ready`, not `usable-watcher`, the initial Watcher
release or Local prerelease preparation.

## Status and sequencing

Status: in progress, with focus attention. The [operator decision of
2026-11-12](https://github.com/lossyrob/telex/issues/172#issuecomment-6830371292)
authorizes the version-range design below and supersedes the exact-build proof
HOLD, twenty-D obligations, and pending A/B/C question through explicit accepted
risk. This is neither native proof nor product completion. Investigation, the
docs/decision PR, and ordinary authorized product development do not wait for
metadata or proof reconciliation.

This node was split from #165 on 2026-10-23 under the operator's one-item,
one-PR rule. Issue #172 holds the full successor specification. #165 now covers
only the macOS non-detaching detector contract delivered by PR #168.

The node depends on `watcher-macos-containment`, `watcher-runtime-core`, and
`watcher-host-containment-doc-alignment` under #177. The
[steward adoption](https://github.com/lossyrob/telex/issues/172#issuecomment-6830611753)
adds `watcher-macos-containment-admission-basis` under #252 for the separate
docs/decision PR (`Closes #252`). That documentation dependency does not block
ordinary authorized investigation or parallel product development; the product
PR still says `Closes #172`. #177 is a prerequisite
for #172 product-PR readiness and closure only; it did not stop the earlier
source and native proof work or itself grant implementation authorization. The
documentation prerequisite is now satisfied: PR #178 merged on 2026-10-23
as main `b6eac6801eb9e6185868e0059640ceef5fe49eb7`, and the Watcher
coordinator completed #177 after exact-merge main CI and its field report.
This historical prerequisite completion did not clear the then-operative HOLD.
The 2026-11-12 decision now replaces that authorization standard; the successor
documentation duties remain.

The runtime-core merge prerequisite was satisfied by PR #164 merge
`d0237dbb0b50dafd5518d403a5ff8d81cd5ee25c`; PR #170 merged #162's independent
documentation correction as main 2513ccda. PR #173 merged #162's bounded repairs
on 2026-10-23 as main `55cb6b116607bceccf3ce0b72550e2d849ca5426`, and issue #162
closed on merge. After post-merge validation and the #162 field report, the
Watcher coordinator completed `watcher-runtime-core` on 2026-10-23, so this
node's runtime-core dependency is satisfied. On 2026-10-23 the design steward
approved the candidate Phase 2 proof shape, subject to corrections, and kept
product implementation on hold.

At exact proof head 810bd24fe7ca3415a4b32c19b762ed32a68d952b, native ARM64 and
x86_64 run 39904383638 rejected the one-shot SIGKILL completion candidate: a
barrier-controlled in-group descendant survived the accepted group SIGKILL and
answered two post-seal challenges one second apart. Both post-reap observations
reported presence, and ESRCH appeared only after private teardown. The result
preserves the pinned-source old-group closure argument and fail-closed receipt
rule but proves that one accepted group SIGKILL is not a completion condition.
That rejection remains a required negative control. The later exact-build proof
holds are superseded, not satisfied by this evidence. The selected mechanism
still retains the destructive anchor through every nonzero signal, then
irreversibly seals authority, exactly reaps, and observes under the documented
API and conformance basis below.

The retained issue/docs owner session is
`d6ee8a1d-e581-4f6c-a42b-77c84bfccc8b`. The single product lane is the operator's
already-running MacBook agent, developed and tested on the physical Apple
Silicon Mac running macOS 26.7.1 / 25G241 and coordinated publicly through #172.
This task update does not transfer the canonical owner or launch a parallel
Windows implementation. Campaign runs the merge gate and merges at the exact
cleared SHA.

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
- accept terminal evidence only after the selected completion predicate and an
  ESRCH-only absence result, under the documented API/conformance basis and
  explicit transient-invisibility residual below; and
- preserve fail-closed restart and guard-loss recovery without affecting
  independent pending-operation reconciliation.

## Implementation authorization gate

The 2026-11-12 operator decision replaces the former exact-build gate items
3-5. Product work follows the retained safety contract and version-range
admission below. A docs/decision PR may land before product code: ADR 0057
amends only ADR 0054's former-group-observation and admission basis, with
matching `docs/design/watcher.md`, task, and issue-body text. It need not wait
for metadata reconciliation or completion of the superseded proof track.

The product PR must meet these gates:

1. The native conformance suite passes on the operator's Apple Silicon Mac
   (macOS 26.x) and on hosted CI macOS runners (`macos-15` / `macos-26` arm64,
   and x64 where available), whatever builds those runners currently serve.
   This is coverage, not a runtime allowlist.
2. Normal independent implementation and design review pass on the product PR
   against decision 6830371292 and ADR 0054 as amended by ADR 0057.
3. Full CI and SQLite/PostgreSQL lifecycle and health parity pass without
   weakening the existing receipt, sender, store-binding, or readiness fences.

Ordinary local native and PR CI runs need no per-run finite, freeze, or steward
launch grant. Runtime admission and capability failures still fail closed before
detector launch. These gates do not claim that implementation, conformance,
review, or product acceptance has already completed.

## Boundaries

- In scope: only an unprivileged, ephemeral, per-attempt, shipped guard that is
  controlled through an inherited private handle and is nonpersistent,
  nonshared, and non-addressable.
- Returns to the operator: privilege, entitlements, a launchd installation, a
  persistent or shared helper, a broker or public IPC, any cross-principal
  surface, or a VM.
- Unchanged: Windows and Linux behavior, and all receipt, store, and readiness
  fences.
- The normative non-detaching contract is owned by #165
  (`docs/design/watcher.md` and ADR 0054); decision 6830371292 and ADR 0057 amend
  only the former-group-observation and admission basis.
- This node gates `closure-gate` and `closure-ready` only, not `usable-watcher`,
  the initial Watcher release or Local prerelease preparation.

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
   runtime admission, capability checks, and selected completion predicate;
7. require fresh `waitid(P_PID, leader, WEXITED|WNOHANG|WNOWAIT)` terminal
   evidence and an unsaturated `int[2]` `PROC_PGRP_ONLY` census showing only the
   exclusively owned, unreaped leader; irreversibly seal non-cloneable signal
   authority before the exact reap; then write and sync a terminal receipt only
   after `kill(-PGID, 0)` returns `-1/ESRCH` (zero or EPERM is never absence);
8. treat guard failure before launch as no launch and guard failure after launch
   as unproved unless an identity-matched terminal receipt exists;
9. retain `orphan-containment-unproven`, pause the watch, and start no
   replacement after missing, stale, malformed, or ambiguous evidence;
10. keep containment evidence independent of send reconciliation, detector-state
    commit, resume, store binding, and production-readiness fences; and
11. preserve reuse-safe signal ownership and the public documented
    API/conformance terminal-evidence predicate; a conformance failure blocks
    acceptance and must be corrected, without reopening `decision-needed` or
    the accepted residual as a kernel-proof prerequisite.

## Evidence and holds carried from #165

These are retained historical records, not proof that the new product gates
have passed. Later R1-R5 attempts, mappings, and ref records remain history;
the twenty-D and R4/R5 open obligations are closed as accepted risk, not native
proof. No R6, decoder, KDK, mapping, or finite work continues. The
`lossyrob-macos-containment-r2` reference at `1c5b88e1` and the old proof branch
remain archived references. Never push `lossyrob-macos-containment-proof`.
The retained C/test harness may seed conformance tests; expiring run artifacts
may lapse and are no longer prerequisites.

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

Use the native conformance suite and normal CI/review/backend gates above.
Public documented `waitid`, `proc_listpids`, and `kill` contracts plus native
conformance replace per-build kernel/API equivalence proof. Cover:

- launch-gate and setup failure before detector release;
- a root plus multiple generations of conforming descendants that preserve the
  assigned group, through fork/exec, `posix_spawn`, and public `vfork` routes;
- root exit while descendants remain, including pipe-holding descendants;
- timeout, cancellation, normal completion, output-limit termination, and abrupt
  Watcher runtime death;
- guard loss before and after detector release;
- runtime/guard death races and restart while cleanup is in progress;
- PID and PGID reuse pressure, anchor release ordering, and stale identity;
- mechanically prove that no timeout, retry, cancellation, error, Drop, restart,
  or finalization path can reconstruct nonzero signal authority after the seal;
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

Retain the exclusively owned, unreaped direct-child leader through every
nonzero group signal, then irreversibly seal, exactly reap, and perform only
read-only ESRCH observation. A mechanical type-level proof must show that no
signal authority exists after the seal. Repeating the census after group
SIGKILL and before sealing is permitted as defense in depth, not proof.
Exercise the accepted residual cheaply where possible, but do not require
per-build kernel, KDK, DWARF, libsystem, or twenty-D closure.

## Success criteria

- #162 remains fail-closed on macOS until the full #172 implementation lands.
- Native conformance covers cleanup after every required termination path and
  abrupt runtime death on ARM64 and x64 macOS, under the selected predicate and
  explicitly accepted transient-invisibility residual.
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
  native conformance, guard-loss/restart behavior, and independent final design
  review against the amended contract land together at that exact head.
- One product PR says `Closes #172`. Required CI, independent implementation
  review, and exact-head design review pass at the final head; Campaign runs the
  merge gate and merges at the exact cleared SHA.

### Successor documentation duty

The shipping PR also reconciles the present-tense macOS passages in
`docs/design/watcher.md` (lines 209-215 and 612-616 at `b6eac680`) with the
shipped capability: it keeps the normative non-detaching contract they state,
and it leaves no sentence that implies macOS detectors run or produce
containment receipts before the accepted capability ships. If #172 closes
without shipping a macOS capability, the closing change instead qualifies
these passages as the accepted contract for an unshipped capability and keeps
macOS fail-closed.

The steward added this duty on 2026-10-23 from the #177 field report
(https://github.com/lossyrob/telex/issues/177#issuecomment-6369949388);
evidence: `docs/design/watcher.md` lines 209-215 and 612-616 at `b6eac680`.

## Invalidation conditions

Do not call the node ready if:

- runtime admission, capability checks, or the selected completion predicate
  are bypassed;
- the implementation assumes that group-leader exit proves group cleanup;
- the destructive-ownership anchor can be released before every nonzero signal
  is complete and every signal-authority reconstruction path is irreversibly
  sealed, any nonzero signal can occur after the seal, or a reused PID/PGID can
  be signaled as the old attempt;
- observed fork/exit/kill failures violate the selected predicate or retained
  signal, receipt, or recovery invariants; the documented transient-invisibility
  residual is excepted, not reopened as a kernel-proof obligation;
- runtime death can close the liveness path without causing group cleanup;
- guard failure is treated as success or a missing receipt is inferred as ended;
- restart can launch a replacement while containment remains ambiguous;
- evidence claims cleanup of a deliberately detached nonconforming process;
- the implementation requires privilege, entitlement, installation, persistence,
  launchd management, a shared broker, or a VM without a new operator decision;
  or
- Windows/Linux guarantees, receipt-gated state, store binding, sender
  reconciliation, or readiness fences are weakened.

## Support scope and accepted residual - 2026-11-12

[Operator decision 6830371292](
https://github.com/lossyrob/telex/issues/172#issuecomment-6830371292) supersedes
the [2026-10-23 support-scope selection 6374496868](
https://github.com/lossyrob/telex/issues/172#issuecomment-6374496868),
[steward clarification 6372381064](
https://github.com/lossyrob/telex/issues/172#issuecomment-6372381064), the later
R4/R5 exact-build proof standard, and A/B/C. The former physical-class targets
(Apple Silicon and Intel on 24G830 and 25G83) and their evidence remain
historical, not the current admission domain.

The subsequent [operator clarification 6835298564](
https://github.com/lossyrob/telex/issues/172#issuecomment-6835298564) supersedes the former
macOS 15 floor and below-15 refusal. P1 is resolved; no version-policy
approval hold remains. Admission retains native arm64 and x86_64,
kernel/process architecture consistency for the runtime and authority-holding
guard, and refusal of development/KASAN kernels and Rosetta-translated
runtimes or guards. A translated detector descendant remains an ordinary
assigned-group member. `sysctl.proc_translated` ENOENT is the native/Intel
case; hosted runners and VMs are not admission failures.

No macOS minimum, maximum, per-major, exact-build or UUID restriction
applies. Do not warn solely because a version is newer, older or not
conformance-validated. Version collection is optional diagnostic information;
its value or absence is neither a required capability probe nor an admission
failure. Conformance records coverage, not a runtime allowlist.

Before detector launch, retain required translation and capability checks for
the `proc_listpids` ABI, `int` / `pid_t` sizes and `waitid` WNOWAIT support,
alongside the required identity and signal-safety checks. A failed or
indeterminate required capability, identity or safety check fails closed with
the concrete reason. Admission refusal is neither a terminal receipt nor
group-absence evidence.

The exclusively owned unreaped direct-child leader prevents PID/PGID reuse
through every nonzero signal. No discovered PID is signaled, no authority is
rebuilt from numbers, and no signal authority survives the irreversible seal.
Durable boot/guard/leader/PGID/anchor/nonce/receipt identity precedes release.
Restart clears containment only from the exact finalized receipt or a changed
boot-session UUID; otherwise it remains Unproven. Independent pending-operation
reconciliation is unchanged.

The accepted residual is a transient fork, exec, or corpse-state member that
is invisible both to the terminal-leader census and the post-reap `kill` probe.
After leader exit and group SIGKILL this could produce a false Ended receipt:
a stray trusted detector process may remain with closed stdout/stderr pipes
and overlap the next attempt. This does not grant unrelated-process signal
authority. If the member remains visible to `kill`, ESRCH fails and containment
remains `orphan-containment-unproven`. Detectors remain trusted same-user code,
not a sandbox.

The twenty D items (D-G2 through D-G10, D-TYPE, X and V) and R4/R5 open
obligations are closed as explicit accepted risk, not native proof or product
completion. An Ended receipt records that the selected predicate held; it is
not kernel-proved absence of every transient member. The native evidence in run
[39909079250](https://github.com/lossyrob/telex/actions/runs/39909079250)
(128 cleanups across ARM64/x64 and about 99k churn children) found no
counterexample; it does not prove the residual impossible. One-shot SIGKILL,
quiet time, or signal counts are never completion conditions.

Windows/Linux guarantees, receipt/store/readiness fences, the non-detaching
contract, and the no-privilege/entitlements/launchd/persistent-or-shared-helper/
broker/public-IPC/VM boundary remain unchanged. Until the accepted runtime
implementation ships, macOS stays unsupported and fail-closed. The node stays
in progress/focus with its canonical owner unchanged. The only adopted geometry
change is the separate #252 docs node and its #172 dependency. Neither #172 nor
the campaign is complete, and no separate metadata/proof prerequisite is
created for authorized work.
