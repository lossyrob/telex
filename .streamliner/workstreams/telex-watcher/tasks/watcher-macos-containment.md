# watcher-macos-containment

## Goal

Document the macOS non-detaching detector contract. PR #168 records the
operator-selected contract for trusted non-detaching macOS detectors in
`docs/design/watcher.md` and ADR 0054. The contract selects no guard or
termination algorithm and does not enable macOS execution. Implementation and
proof belong to `watcher-macos-containment-runtime` under issue #172.

Windows and Linux retain their existing detached-descendant guarantees. #162
continues to fail closed on macOS until #172 lands.

## Status and sequencing

Status: in progress, with focus attention, until PR #168 merges; then completed,
with watch attention.

On 2026-10-23 the design steward approved splitting the former #165 scope under
the operator's one-item, one-PR rule. This node now covers only the macOS
non-detaching detector contract. All Phase 2 work moved to
`watcher-macos-containment-runtime` under issue #172: source arguments, native
probes, the completion-predicate decision, implementation, runtime tests,
receipt and restart integration, and the implementation authorization gate.

Phase 1 investigation is complete. On 2026-10-22 the operator selected
`Revise macOS to the non-detaching detector contract` in issue #165.

Owner session `d6ee8a1d-e581-4f6c-a42b-77c84bfccc8b` holds this node through the
PR #168 merge. Campaign runs the merge gate and merges at the exact cleared SHA.

## Delivery

PR #168 delivers this node. It must change exactly `docs/design/watcher.md` and
ADR 0054 in `docs/design/DECISIONS.md`, and it must say `Closes #165`. At this
reconciliation, PR #168 was an open draft at
`d4dad4e73bf65cd3957a3a9560877865cb57058e` that referenced issue #165 without
closing it.

## Outcome

The merged contract records:

- the operator-selected contract: the detector root and all local descendants
  stay in one Watcher-assigned attempt process group, escape is nonconforming
  and outside the cleanup guarantee, and Windows and Linux are unchanged;
- the algorithm-neutral proof boundary;
- fail-closed guard-loss and restart behavior;
- the private ephemeral guard authority boundary; and
- the independence of pending receipt and store reconciliation.

It selects no guard or termination algorithm and does not enable macOS
execution.

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

## Success criteria

- PR #168 merges ADR 0054 and the normative Watcher design with the selected
  host contract, without weakening Windows or Linux.
- The merged text covers every outcome item above and selects no guard or
  termination algorithm.
- The merged change enables no macOS execution. #162 remains fail-closed on
  macOS until #172 lands.
- PR #168 changes exactly the two named documents and says `Closes #165`.
- Required CI, independent review, and design-steward approval all pass at the
  final exact head; Campaign runs the merge gate and merges at the exact cleared
  SHA.

## Invalidation conditions

Return `decision-needed` and do not call the node ready if PR #168 would:

- select a guard or termination algorithm, or enable macOS execution;
- claim cleanup of deliberately detached nonconforming work;
- require privilege, entitlement, installation, persistence, launchd management,
  a shared broker, or a VM without a new operator decision; or
- weaken Windows/Linux guarantees, receipt-gated state, store binding, sender
  reconciliation, or readiness fences.
