# Repair Windows intent-seed acquisition preconditions

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
