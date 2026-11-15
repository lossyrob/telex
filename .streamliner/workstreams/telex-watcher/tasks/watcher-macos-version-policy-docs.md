# watcher-macos-version-policy-docs

## Goal and scope

Deliver one docs-only PR that says `Closes #254` and `Relates to #172`.
The [#254 body](https://github.com/lossyrob/telex/issues/254) is the item
specification, under [operator clarification 6835298564](
https://github.com/lossyrob/telex/issues/172#issuecomment-6835298564) and
[steward adoption 6835839361](
https://github.com/lossyrob/telex/issues/172#issuecomment-6835839361).

Status: planned, watch attention. The coordinator-assigned docs owner is
`d6ee8a1d-e581-4f6c-a42b-77c84bfccc8b`, using its existing managed branch.
This docs item has no dependency edge to or from #172 and no checkpoint
membership. It does not gate MacBook product preparation.

## Deliverables

1. Align only ADR 0057's heading, dated Status clarification, Admission and
   directly following version-range/conformance paragraph in
   `docs/design/DECISIONS.md`, plus the matching admission-domain paragraph
   in `docs/design/watcher.md`.
2. Incorporate the sole reconciler's exact current #172 task admission
   paragraph, current-design admission wording, #172 CURRENT graph-summary
   admission sentence, coverage qualifications, and this item's node/task.
   The docs owner does not independently edit canonical paths.
3. After merge, update the admission paragraph of the #172 issue body.

## Required policy

- No macOS minimum, maximum, per-major, exact-build or UUID admission
  restriction, or warning solely for a newer, older or unvalidated version.
  Version collection, value and absence are diagnostic only, not a required
  capability probe or admission failure. Conformance is coverage, not a
  runtime allowlist. P1 is resolved with no version-policy approval hold.
- Retain native arm64/x86_64, architecture consistency for runtime and guard,
  development/KASAN and Rosetta refusal, and genuine capability, identity
  and signal-safety checks. Failed or indeterminate required checks fail
  closed with the concrete reason. `sysctl.proc_translated` ENOENT is the
  native/Intel case; a translated detector descendant remains an ordinary
  group member, and hosted runners or VMs are not admission failures.

## Boundaries and acceptance

#252 remains completed/watch; its task, graph summary, body, field report,
earlier decisions and existing brief remain historical and unchanged.
#172 stays in progress/focus with its canonical owner, dependency edges and
checkpoint membership unchanged. #246 and Local release separation remain.
No product code, new safeguard, native run, proof work, new worker or product
hold follows. The implementation safeguards and the two documentation
corrections in the supplement remain with the #172 product PR.

Exact-head steward contract review and ordinary CI must pass. This item
does not establish product acceptance, native evidence, merge authority,
installation, release/tag authority or campaign completion.
