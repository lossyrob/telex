# Repair Windows intent-seed acquisition preconditions

## Current workstation run policy (2026-10-26)

At 12:44 ET the operator lifted the I161-1 blanket workstation freeze.
Local builds, tests and runtime runs are allowed within the task's authorized
scope. Use isolated `TELEX_*` roots and a temporary `LOCALAPPDATA`, except for
the separately authorized #201 dogfood against the real installation. That
exception permits real-installation use, not real-directory ACL or permission
changes. I161-1 preservation remains: no ACL restoration or reset.

Exact-head hosted CI remains the gate evidence. Local results support it; they
do not replace it or retroactively change earlier validation/debt dispositions.
Older freeze, compile-only and hosted-only records below remain historical,
including older sections labelled current; they do not renew a blanket local
run prohibition. Other scope, dependency, merge, incident and retained-debt
decisions are unchanged. This policy does not launch #201, satisfy its
dependencies or grant unrelated runtime authority.

Authority: Campaign `toolu_01JDdrjgTsK2e181xhhLdeYY`, relaying the operator:
"Also what is the 'freeze'? If it's blocking then remove it." On this operator's
workstation, the existing `gh-pub` profile now resolves to `lossyrob`; personal
GitHub writes can use it directly without a token override.


- **Workstream:** `local-daemon`
- **Node:** `intent-seed-precondition-repair`
- **Type:** implementation
- **Status:** in progress; one owner authorized, pending Local v2 launch
- **Attention:** focus
- **Depends on:** none
- **Tracker:** [#193](https://github.com/lossyrob/telex/issues/193)
- **PR:** pending; one separate repair PR must close #193
- **Owner coordination:** Local `aeae4fa9-c5fc-4da9-b60e-bcf5b1a98f57`;
  concrete implementation owner is pending preparation and launch
- **Parent workstream:** [#32](https://github.com/lossyrob/telex/issues/32)
- **Campaign:**
  [Addressable Attention #102](https://github.com/lossyrob/telex/issues/102)

## Outcome

Establish the fixture's successful-acquisition preconditions so its
registration-proof test reaches the intended corruption/refusal/no-member
assertions reliably. Preserve the accepted bounded production lock
contract. Deliver the smallest justified setup/precondition repair and
discriminating hosted coverage in one PR closing #193.

## Authority

Campaign accepted Local's narrower fixture-defect assessment and authorized
one immediate bounded owner through `toolu_01X4YTjtnHnE76rNwKipSSUn`.
Owning Local supplied the exact scope in
`issue193-active-repair-promotion-20261025.json`.

> Campaign: your #193 assessment is accepted. Don't serialize staffing
> behind the steward. Main is red, and Watcher #180 needs this job strictly
> green, so launch the bounded owner now for the setup/precondition repair.
> The repair must keep the 480ms budget, non-theft and fencing, with no
> widening, retry or flake exception, and needs a hosted check that tells
> the causes apart. The steward's contract-boundary confirmation can run
> in parallel and gate the PR, not the launch. My earlier "if real defect"
> condition is met: a fixture that unconditionally expects bounded
> acquisition to succeed, and turns main red, is a real defect.

The earlier no-owner/conditional-staffing snapshots remain historical.
Steward contract confirmation has already been received; another steward
launch review is not required. This is not merge authority or approval of
a particular production-policy change.

## Inputs and evidence limits

Main CI `40220058150`, Windows default workspace job `121249478906`, at
`34cfdee7fb1f31eef6f0025f7a7b5b1af8bd2a11` failed in
the `daemon::p3_tests` test
`a_register_whose_proof_cannot_be_written_is_refused_and_leaves_no_member`.
`seed_pending_intent` panicked at `src/daemon.rs:8511:52`, called from line
8692, with the combined locked-writer/acquisition-budget error. The root
target reported 571 passed / 1 failed / 4 ignored in 21.89 seconds.
That duration is not the acquisition elapsed time.

Local matched the retained 320,580-byte log, SHA-256
`403046b0afffec29ff94c48b2ea16cd50ed86b86c08d8efc0e6ae685df7179bd`,
and read selected direct-checkout, panic/call-stack and result excerpts.
The original intake is
`main40220058150-intent-seed-refusal-intake-20261025.json`; the later
assessment is `issue193-seed-precondition-assessment-20261025.json`.
These are Local-attributed source/evidence reads, not a new reconciler
source inspection, full-log read, CI inventory audit or execution.

The failure precedes registration, deliberate manifest corruption and the
refusal/no-member assertions. Local's source assessment finds an
atomic-sequence test root and initially empty intent runtime, with no
other in-process writer started before the seed. That does not establish
timely acquisition or exclude every external/environment condition.
First acquisition includes owner-private creation/open, security and
local-volume checks within the production budget. The shared error covers
pre-attempt expiry, a late successful guard and WouldBlock/attempt
exhaustion; no exact failing path, holder or per-stage duration is known.
Source is unchanged from accepted `65f5846d` to tested `34cfdee7`.

Steward `af271672-5d33-418c-bb9c-0f4d4570c7d7` provisionally confirms a
setup-success precondition problem, not proved contention or a production
policy violation. An expect on fallible setup is not inherently wrong;
this fixture must establish the success needed for its downstream proof.

## Scope and invariants

- Make the minimal setup/precondition correction and directly related
  diagnostic/regression coverage; document only necessary related changes.
- Add a hosted check that distinguishes budget-consumption paths and
  unexpected contention/sharing instead of presuming a flake.
- Retain actual pending-intent creation and the intended registration,
  corruption, refusal and no-member assertions.
- Keep the production 480ms elapsed budget: no widening, pausing,
  refreshing or bypass. Preserve persistent owner-private locks,
  non-theft, generation fencing and late-guard disposal.
- Do not skip the seed/error, accept missing intent as success, add a
  retry loop or grant a flake exception.
- Any clock/admission seam is test-only. Native contention, non-theft,
  persistence/release and production-budget tests must remain meaningful,
  not become tautological.

## Validation and PR gate

Hosted Windows default workspace build/test must pass at the exact final
head, together with the discriminating check and relevant platform/profile
coverage. Record exact source/head/job/feature provenance. Targeted or
all-features success does not waive a failed required default suite.

An independent source reviewer must post its own exact-head public PR
receipt, including for zero findings. The new reviewer profile is
`gpt-6.1-sol` / high reasoning / default context. Steward inspection may
run in parallel with source review and CI; it gates the PR, not launch.
One repair PR closes #193 and returns to the Campaign gate. Neither this
task nor a passing check alone grants merge or Watcher readiness.

## Launch and coordination

Local coordinates exactly one owner through the existing v2 adapter:
one held isolated worktree, `gpt-6.1-sol` / high reasoning / default
context / interactive, no idle notification, immediate binding, packet 05
general-purpose Task in that exact checkout, verified handoff and one real
kickoff. Do not use an API/SDK/v1 handshake or a sibling worktree.
The reconciler prepares canonical artifacts and does not launch an owner.

Use direct source-review/steward coordination and existing wait coordinator
`2a4bc4c8-1211-49d4-ba68-9d05d5d7530d` for external CI; do not create a
duplicate observer. Report a matching PR #180 failure to Campaign with
its exact head/job/evidence. PR #180 retains its own strictly green
current-head Windows default build/test and all other gates.

## Workstation freeze and exclusions

Only read-only source/evidence, ordinary isolated-worktree edits, commits,
pushes and compile-only local checks are permitted. No local tests,
runtime/proof execution, daemon/test-child control, ACL/registry/real-root
operations, fixture cleanup or new local proof. Runtime validation belongs
on authorized hosted disposable CI. The separate previously authorized
Watcher proof exception does not apply to this root repair.

Do not claim PR #192 causality or change its consolidation here. Do not
absorb earlier #181 Overran, marker or optional-Clippy work, or reopen
accepted #181/#187 delivery by inference. No Watcher code, graph edge,
dependency/status/gate change, issue #153 acceptance, hardening/closure,
release or broader freeze authority follows.

## Actual single-owner launch and branch qualification - 2026-10-25

The promotion-time pending-owner and PR lines above are historical. Local
reports one actual launch of `519693a5-0607-4aba-a968-345f3b1cd1b5` in the
existing `lossyrob-upgraded-system` checkout. The requested profile was
`gpt-6.1-sol` / high / default / interactive, with no idle notification;
this records configuration, not independent effective-capacity proof.
The original launch branch remains `lossyrob-upgraded-system`.

Local verified package `ctx-20261025185147-10f8a3ea`, the full owner guide
and current overrides, same-checkout handoff, installed context and only
two preparation `.paw` files. It delivered the 14,556-byte kickoff once
by file reference, SHA-256:
`ffd1f36d7e835d0f8243ef3b3d8f6fa39d05ac32806f0c8afe19f79ff6d2440f`.
The same session, checkout and launch branch were observed busy after
admission at `2026-10-25T18:59:23.4882067Z`. The immutable launched-binding
snapshot is 4,238 bytes, SHA-256:
`ce9969d90ba623e1b5669f39399766b43bcbd300037927c0375f073528dc1c22`.

Preparation remains pinned to
`4deaf30cc6656d4807918fa42870db742d453bc4`. Later main
`0d5347a6e1b8a1716b524c537c4886d3f4d49b65` includes Watcher, Cargo.lock
and design changes, not only artifacts. Local reports the selected node,
task and launch policy unchanged and notified the existing owner; no
integration acceptance or old review/CI evidence carries forward.

A separate Local observation received at `2026-10-25T19:01:48.042Z`
verified the same App session and checkout busy on current branch
`lossyrob-intent-seed-preconditions`. The 2,289-byte immutable supplement
has SHA-256:
`a52c4a66e9f81c2cacd5c8a0339418c196e8f59e529bf3ae20fc711fc084eded`.
This is an App-managed rename, not a second owner, checkout or preparation.
The original branch/head, binding and kickoff remain launch history.
Integration at `0d5347a6` and the test-only seed-clock/real-locking/
diagnostic direction are owner reports, not source/design/CI approval.

Local subsequently resolved App alias
`1185656f-ec05-4f88-aacc-f09b2431f9ee` to the same canonical owner
`519693a5-0607-4aba-a968-345f3b1cd1b5` using both App lookups. Both
returned the same creation, parent, checkout and current branch. Local
reports an App association to open
[#194](https://github.com/lossyrob/telex/pull/194), not provider-head,
source/design/CI or readiness verification. This is an observed alias,
not a second owner or rebinding; source `call_UtzzXkcm6TehxV3uVwfqtPMU`.

Local's later independent provider read at
`2026-10-25T19:10:41.4572621Z` observed PR #194 OPEN/non-draft to main,
head `f8b42d20acd3ae358a71a576d00c76b3f571ac6c`, branch
`lossyrob-intent-seed-preconditions`, author `lossyrob`. Its 2,286-byte
immutable identity/metadata packet has SHA-256:
`eb2240d119727ff7e7d889b6683f0a7b060a3e9f0763c9bfe4cfb0b2a755e3be`.
Source `call_W0agCbytXP9sVD4SUyKsNafs` qualifies the earlier App-only
association, not source/design/CI or readiness. No alias namespace is
inferred. The durable launch binding advanced only after snapshot custody;
the original binding, branch observation and kickoff are not rewritten.

Local sources are `call_vmhmemA3eGud4ZyEIR0cpqdr` and
`call_PA0Ng9Q6cfSjeNBotN1A2KE1`; the owner branch report is
`call_B8ehaIZT5wXISiDchvg2iXIH`. The reconciler read and byte-captured
the immutable source packets; it did not inspect the owner's checkout,
contexts, source, runtime or hosted validation, or launch another owner.
All production 480ms/non-theft/fencing, no-widening/retry/flake-exception,
hosted discrimination, reviewer-publication, design and Campaign gates
remain. Launch is not implementation completion, a PR, readiness or debt
clearance; Watcher state and the workstation freeze are unchanged.
