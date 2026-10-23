# Repair cooperative station-intent partial-page completion

- **Workstream:** `local-daemon`
- **Node:** `cooperative-partial-page-repair`
- **Type:** implementation
- **Status:** in progress; v2 preparation complete and real kickoff delivered
- **Attention:** focus
- **Depends on:** none
- **Blocks:** `telex-watcher/watcher-cli-ux`, issue #179 / PR #180
- **Tracker:** [#181](https://github.com/lossyrob/telex/issues/181)
- **Owner coordination:** Local `aeae4fa9-c5fc-4da9-b60e-bcf5b1a98f57`;
  implementation owner `9d4fee6c-63a6-4aa7-b384-a4df1cbcbb4c`
- **Parent workstream:** [#32](https://github.com/lossyrob/telex/issues/32)
- **Campaign:**
  [Addressable Attention #102](https://github.com/lossyrob/telex/issues/102)

## Outcome

Restore the accepted cooperative partial-page completion/progress behavior
through a discriminating diagnosis and minimal complete correction. Deliver
one separate repair PR that closes issue 181; no unrelated marker repair.

## Authority

Campaign `e06813c4-1db2-4d77-b8d3-3addc6a1824c` authorized this bounded
repair on 2026-10-23T16:44:24-04:00. Local submitted the exact directive and
task scope in `issue181-active-repair-promotion-20261023.json`.

> Authorized: one separate bounded Local root-repair owner for
> local-daemon-windows-cooperative-partial-page-overrun, using the scope you
> proposed. Diagnose the completion/permit/worker boundary first. Preserve
> single-slot behavior and the absolute D+grace and mandatory partial-page
> semantics. Make the minimal correction, then validate at the exact head on
> hosted CI. No third unchanged-head retry, skip, waiver, or unsupported
> timeout increase. Apply the discovered-work rule: the reconciler promotes
> the candidate to an active Local node with a GitHub issue that the repair PR
> closes, and records it as an upstream blocker of watcher-cli-ux (#179).
> PR #180 stays as is: no absorption, no Watcher-side change. Launch with
> gpt-6.1-sol, high reasoning, default context, under the existing workstation
> freeze. The marker cases stay separate candidates. When the repair PR is
> ready, it comes to the Campaign gate like any other. Tell the Watcher
> coordinator (fe2e883c) that #180 waits on this repair.

This is explicit implementation/launch and work-geometry direction, not a
merge authorization or hardening/closure decision. No product design or
contract change is authorized by this task.

## Inputs

- Canonical issue 181 body and Campaign's quoted authorization in this
  packet.
- Candidate local-daemon-windows-cooperative-partial-page-overrun and its
  retained attempt-1/2 evidence in the Local ledger.
- Associated PR head
  8826fad763f12ed6cde30a3f976a87e8807de775; both retained CI logs record
  provider test merge da52c4d687b517ec10aa3d673bc50e9863225156.
- Attempt 1 run 40008577194/job 120602732487, full log SHA256
  21aa0585d47364173ded4bd4b0742f480e7c3ddb3388e0802bb07a476a1dc8db:
  562 passed, 3 failed, 4 ignored.
- Attempt 2 same run/job 120608948530, full log SHA256
  4e6ec7a29337c0a27c2ddd8796fe212b8de2c6103177f1c78f16e9f4e38fdf62:
  partial-page assertion src/station_intent.rs:3865:14 recurred; 564 passed,
  1 failed, 4 ignored. The two marker tests passed; their first failures
  remain separate candidates.
- Existing test/helper/source and accepted Local Daemon design. Earlier
  52abb30 source review is historical context, not proof of this runtime
  cause.

The retained checkout identity is a provider test merge, not direct PR-head
execution. The exact logs and their custody history remain in
[`../discovered-work.json`](../discovered-work.json); no source-equality,
binary-rebuild or causal claim follows from the image identity.

## Diagnosis

Identify which completion boundary is responsible: permit acquisition,
worker scheduling/execution/result, cooperative deadline handling, or
another evidenced mechanism. Record a bounded discriminating account and
distinguish current observations from hypotheses. Do not assume runner load
or a previous cause merely from test names.

## Boundaries

### In scope

- The minimal source or test synchronization correction that satisfies the
  original partial-page outcome.
- Directly related regression coverage and documentation needed to
  demonstrate and explain the correction.
- Exact-head hosted Windows validation of the affected root-library test
  and relevant required suites, with truthful source/test-merge provenance.
- One implementation owner, one isolated branch, one node PR, direct
  review-response fixes and final Campaign gate handoff.

### Out of scope

- The idle-drain repush and live-CC marker candidates, unless separately
  dispositioned based on new evidence.
- Watcher implementation changes or incorporation into PR 180.
- Issue 153 transactional authority, broader station-intent redesign or new
  persistence mechanisms.
- Third unchanged-head retries, disabled tests, check waivers, unsupported
  timeout increases or degraded mandatory partial-page guarantees.
- Hardening/closure acceptance, release/tag, fleet adoption, merge without
  Campaign authorization or historical incident clearance.

## Invariants

- At most one cancellable filesystem phase owns the process-local permit.
- Admission and completion share the existing absolute D+grace budget; no
  sequential fresh grace windows.
- Original cooperative deadline, cancellation/generation fencing and
  truthful truncation remain.
- The mandatory partial-page/progress contract is preserved, not replaced
  by accepting Overran as success.
- No false claim that async timeout interrupts an entered blocking syscall
  or erases late completion.

## Success criteria

- A precise evidence-backed explanation distinguishes the selected
  corrective mechanism from plausible alternatives.
- A discriminating regression checks the actual corrected requirement, not
  merely compilation or retry success.
- Affected behavior and required suites pass on the exact repair head in
  isolated hosted Windows CI, with other relevant platform/profile coverage
  preserved.
- Complete implementation review, exact-head design inspection and Campaign
  repair-PR gate are satisfied; issue 181 closes with the authorized repair
  PR.
- Watcher dependency remains pending until actual accepted repair delivery;
  PR 180's own current-head requirements are not waived.

## Workstation freeze

No local runtime/test/proof execution, daemon/child process control,
PageHeap/registry or ACL/inheritance mutation, fixture-root actions,
restoration or cleanup. Read-only analysis, ordinary isolated
source/worktree edits, commit/push and permitted compile-only checks are
distinct from runtime validation. Runtime proof must use the authorized
hosted disposable runner. Do not touch installed user/campaign Telex state.

## Launch

Canonical promotion landed at
`3d517e17fba88559fbd5e69557ceb6264300aeff`. On 2026-10-23 Local completed
Streamliner v2 preparation and delivered the real kickoff to App owner
`9d4fee6c-63a6-4aa7-b384-a4df1cbcbb4c` in checkout directory
`lossyrob-expert-doodle`.
The launch-time branch was `lossyrob-expert-doodle`, initially at that main
commit. A later reconciler App inspection reports branch
`lossyrob-cooperative-partial-page-repair` and the same owner/checkout.

The requested runtime profile is `gpt-6.1-sol`, high reasoning, default
context tier and interactive mode; this records the request, not independent
verification of effective runtime settings. The canonical v2 binding is
`prompts\v2\launches\cooperative-partial-page-repair-20261023.json`
in the Telex prompt pack.
The reconciler read its `launched`/`kickoffDelivered` receipt and matched the
12,819-byte kickoff at SHA-256
`dfec5f6b8ff3e147096bb7e98dbb5ea52113fab3973363953663235ab5aba4b8`.
The binding snapshot is 3,560 bytes at SHA-256
`5af3fa466a2104fdf534f1ba09cad1e3e641e44047f7d60c4b17c867bd1bc0d8`.
Local's receipt records completed preparation checks and accepted immediate
kickoff delivery. The reconciler independently observed the same App owner
and checkout with busy activity; delivery acceptance/activity do not prove
implementation or CI completion.

This v2 launch corrects and supersedes the earlier preparation-pending and
session-online/write-authorized identity-exchange requirements in this task.
Local reports no API/SDK/v1 handshake; none is required for this v2 launch.
No duplicate writer, prepared-worktree copying or hand-written preparation
bypass is authorized. The workstation freeze remains in force.

## Workflow

Use the full canonical App implementer role and paw-lite planning. The
Campaign directive and workstation freeze override older model and
local-runtime defaults. Independent review and hosted CI begin from
complete pushed scope. The existing wait coordinator owns recurring
observation; worker must not create a poller. Source-only approval, design,
runtime, merge and hardening remain separate.

## Ownership and handoff

Local aeae4fa9-c5fc-4da9-b60e-bcf5b1a98f57 coordinates launched owner
9d4fee6c-63a6-4aa7-b384-a4df1cbcbb4c under the v2 receipt above. Existing
Watcher c90bafbb preserves its branch/evidence; current Watcher coordinator
fe2e883c-debc-4573-a855-2cf1e0f89407 has been told PR 180 waits.

The separate repair PR must explicitly close
[#181](https://github.com/lossyrob/telex/issues/181). Return its complete
exact-head evidence and review/design results to Campaign. Reconcile
completion and release the pending Watcher dependency only from actual
accepted terminal delivery; this task grants no immediate merge or
hardening/closure acceptance.
