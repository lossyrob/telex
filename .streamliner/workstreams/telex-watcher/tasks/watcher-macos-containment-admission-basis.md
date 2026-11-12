# watcher-macos-containment-admission-basis

## Goal and scope

Deliver one docs/decision PR that says `Closes #252` and relates to #172.
The [#252 body](https://github.com/lossyrob/telex/issues/252) is the item
specification, under [operator decision 6830371292](
https://github.com/lossyrob/telex/issues/172#issuecomment-6830371292) and
[steward adoption 6830611753](
https://github.com/lossyrob/telex/issues/172#issuecomment-6830611753).
The product PR remains separate and says `Closes #172`.

Status: planned, watch attention. The coordinator-assigned docs owner is
`d6ee8a1d-e581-4f6c-a42b-77c84bfccc8b`, using its existing managed docs branch.
This item depends only on the completed `watcher-macos-containment` contract
item (#165). #172 adds this docs dependency; all its other dependencies and
in-progress/focus state remain unchanged. Documentation sequencing adds no
metadata/proof prerequisite for ordinary authorized investigation or parallel
Mac product development.

## Deliverables

1. Add ADR 0057 in `docs/design/DECISIONS.md`. Amend only ADR 0054's
   former-group-observation and race-safe-proof wording, native-proof and
   `decision-needed` consequences, and admission basis. Keep its other
   decisions unchanged.
2. Make the matching narrow macOS MUST edits in `docs/design/watcher.md` and
   directly dependent sentences. Do not claim the unshipped capability is
   supported or remove the product's temporary fail-closed status.
3. Incorporate the sole reconciler's exact canonical #172 task, graph,
   this task, and directly contradictory `design/current-design.md` text in
   the same PR. The docs owner does not independently edit these paths.
4. After the docs PR merges, update #172's issue-body Status, Implementation
   authorization gate, and Required validation framing.

## Required contract

- Admit native arm64 or x86_64 on macOS 15 or later through runtime version,
  kernel/process architecture, translation and capability checks. Refuse
  development/KASAN kernels, Rosetta and versions below 15. Failed checks fail
  closed before detector release and establish neither receipt nor absence.
  Add newer major versions through conformance, not kernel mapping.
- Base Ended on documented `waitid`, `proc_listpids` and `kill` contracts plus
  native conformance. Keep fresh terminal WNOWAIT evidence, an unsaturated
  `int[2]` `PROC_PGRP_ONLY` census showing only the owned unreaped leader,
  irreversible seal, exact reap and `kill(-PGID, 0) == -1/ESRCH`.
  Zero or EPERM is never absence.
- State that the receipt records this predicate, not kernel-proved absence of
  every transient member. A transient fork/exec/corpse member invisible to both
  census and `kill` may leave a stray trusted detector with closed output,
  overlapping the next attempt. This cannot authorize signaling an unrelated
  process. The twenty-D and R4/R5 obligations close as accepted risk, not proof.
- Keep anchor-held nonzero signaling, no post-seal signals or numeric authority
  reconstruction, durable identity before release, receipt/restart/recovery
  rules, independent pending reconciliation, trusted-code boundary,
  Windows/Linux behavior and the runtime-machinery boundary unchanged.
  One-shot SIGKILL, quiet time and signal counts are not completion conditions.
- Replace #172's old gates with native conformance on the operator's Apple
  Silicon Mac and hosted macOS CI, normal independent implementation/design
  review, and full CI plus SQLite/PostgreSQL lifecycle and health parity.
  Ordinary local native and PR CI runs need no finite, freeze or steward grant.

## Boundaries and acceptance

Docs only: no product code, CI job, native run, new worker, ownership transfer,
proof-ref push, merge or release action follows from this item. Never push
`lossyrob-macos-containment-proof`. Retain old proof attempts and ref records as
history; no R6, decoder, KDK, mapping or finite work continues.

The existing MacBook product lane is coordinated publicly through #172; this
item creates no duplicate Windows product implementation. macOS remains
unshipped and fail-closed until #172's product PR is accepted. #246 remains
retired/parked, its closure edge retained, and the Windows checkpoint planned.
Local prerelease preparation remains separate.

Independent review and design-steward contract review must pass at the exact
docs head. Campaign runs the merge gate. Neither this decision nor the docs
item completes #172 or the campaign, or establishes native product evidence.
