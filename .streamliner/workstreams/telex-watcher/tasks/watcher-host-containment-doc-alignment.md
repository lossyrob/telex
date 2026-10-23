# Align host-specific containment contract and macOS status

- **Workstream:** `telex-watcher`
- **Node:** `watcher-host-containment-doc-alignment`
- **Type:** task
- **Status:** in progress; owner session `99ea868c-1d3c-4ad7-8e1b-69bfb4d41705`
- **Attention:** watch
- **Depends on:** completed `watcher-macos-containment`
- **Tracker:** [lossyrob/telex#177](https://github.com/lossyrob/telex/issues/177)
- **Parent workstream:** [lossyrob/telex#100](https://github.com/lossyrob/telex/issues/100)
- **Campaign:** [Addressable Attention #102](https://github.com/lossyrob/telex/issues/102)

## Source and placement

This item comes from debt items 1 and 2 in Campaign's PR #168 merge gate,
carried from the c3a3cc21 gate:
https://github.com/lossyrob/telex/pull/168#issuecomment-6359852865

It corrects text that PR #168 merged as main
`bc1e02edf43490c41dbc3628b692c4cbf822c5ef`. The design steward ruled that item
1 is a documentation overstatement, not a Windows runtime gap, and that item 2
is a documentation touch-up needed now.

`watcher-macos-containment-runtime` depends on this item because the #172
shipping PR rewrites the interim macOS sentence that this item adds. #177 is a
prerequisite for #172 product-PR readiness and closure only; it does not stop
the active source and native proof work and grants no implementation
authorization. The Windows-path five-minute gate does not depend on this item.

`watcher-macos-containment` completed on 2026-10-23, so every dependency of this
item is satisfied. The Watcher coordinator launched owner session
`99ea868c-1d3c-4ad7-8e1b-69bfb4d41705` on 2026-10-23 from main
`91ff646679638d230a5f52116d5d3dcaae0d8379`. The node is in progress with watch
attention. The launch binding record is
`watcher-host-containment-doc-alignment-20261023.json` under
`streamliner/telex/prompts/v2/launches/` in the planning repository, with
kickoff SHA-256 `3d3ce235`.

## Outcome

Correct the merged Watcher containment prose so the common launch requirement
is host-specific rather than incorrectly requiring Linux/macOS guard machinery
on Windows, and so the normative design distinguishes the accepted macOS
contract from the capability's current fail-closed implementation status.
Preserve Windows and Linux detached-descendant guarantees, the Windows atomic
Job-assignment and durable-identity gate, Linux's guard handshake and receipt,
and all accepted macOS proof boundaries. Select no algorithm and grant no macOS
implementation authority.

## Scope

`docs/design/watcher.md` only. No runtime, schema, ADR, or algorithm change.

The accepted Windows mechanism is a runtime-owned, uniquely named kill-on-close
Job Object assigned atomically at CreateProcess, with the detector suspended
until the versioned identity is durably recorded; recovery verifies the exact
owner identity and job emptiness. It deliberately has no separate guard-process
handshake or durable terminal-receipt file. Do not add a Windows guard or
receipt duty.

### Replacement 1: host-neutral launch paragraph

Replace lines 690-694 at `bc1e02ed` with exactly:

> Before detector release, Watcher MUST durably persist a versioned,
> host-specific containment identity sufficient to distinguish the containment
> instance, its owner, and its assigned scope during recovery. The launch gate
> opens only after the host's accepted containment setup and durable identity
> recording succeed. Any host-required containment-owner handshake and
> terminal-receipt path/nonce identity MUST also be established before release.
> Failure of setup, any required handshake, or identity recording starts no
> detector.

### Replacement 2: macOS bullet

Replace lines 683-686 at `bc1e02ed` with exactly:

> - The accepted macOS contract is for conforming non-detaching detectors. When
>   the capability is enabled, Watcher assigns one process group to the
>   attempt, and the detector root plus every process it creates must remain in
>   that group until the attempt ends. Work moved outside the group is outside
>   the cleanup guarantee. Until issue #172's implementation and proof are
>   accepted, Watcher fails closed before detector launch on macOS and does not
>   claim macOS support.

## Success criteria

- `docs/design/watcher.md` contains the two replacements above and no other
  product-contract change.
- Windows remains correctly described by atomic Job containment, durable
  pre-resume identity recording, and fail-closed recovery without a separate
  guard or receipt file.
- Linux and macOS retain every required owner-handshake and durable-receipt
  obligation.
- macOS is explicitly fail-closed with no support claim until #172 is accepted.
- The docs-only PR says `Closes #177` and passes required CI and exact-head
  design review.

## Delivery

One item, one PR. The PR body says `Closes #177`. Campaign runs the merge gate
and merges at the exact cleared SHA.

## Related

- #165 / PR #168: merged contract text being corrected.
- #172: its shipping PR later removes the temporary fail-closed sentence and
  rewrites the macOS bullet in unqualified present tense only when the full
  capability lands at that exact head.
