# Local presence/transport daemon (eliminate the per-session holder)

## Purpose

Most of telex's recurring staleness (orphaned holders, zombie `occupied` leases,
holder/waiter startup races, dismiss leaving a holder attached, a forever-listener
starving an orchestrator's turn loop) traces to one structural choice: a
**per-session resident holder** whose lifetime must track a fuzzy agent session.
This workstream eliminates that holder by introducing an **auto-spawned per-user
local daemon** that owns presence and delivery for all locally-attended addresses,
and ships the surrounding pieces (Copilot plugin, seamless upgrade) needed for a
real, end-to-end unblock so idle long-lived sessions stay wakeable and stations
stop going stale. It resolves [issue #32](https://github.com/lossyrob/telex/issues/32).

## Approach

The work is a single complete deliverable across **both SQLite and Postgres** (the
operator runs both and has stations idle waiting on this), not a thin V1 slice. A
SQLite-only spike is an internal step inside the daemon-core node, not the shippable
boundary.

Formation orders the work as confidence transitions expressed through the node DAG.
**design-foundation** (a research node, written and spar-pressure-tested) locked the
hard contracts up front - the daemon-scoped capability/version-handshake IPC, the
**server-side lease-epoch fence**, the **seen-dedup redesign**, explicit membership
+ non-destructive liveness, negative-only watched-process evidence, the daemon
**singleton identity** + **lifecycle contract**, and daemon-native session RPCs -
behind a builder **design-gate**. Then the **daemon core**
(the centerpiece: daemon process, durable buffer, one-shot verbs, server-side
epoch-fenced delivery, the lifecycle contract, and a minimal upgrade floor) on
SQLite, which with the **Copilot plugin** is the first slice that can unblock the
operator (reached when the plugin lands on SQLite). A distinct **fencing-proof** gate
(epoch-guarded emission + ordered handoff, proven on SQLite) then blocks downstream
work. **Postgres parity** extends the core under that proof and adds the cross-machine
reclaim (competing daemons); **seamless upgrade** (#6) lands
**last**, after Postgres and the plugin, so the full upgrade platform never blocks
the unblock. The original large validation-harness and AKS-scale shape was later
replaced by a practical **release-confidence-validation** node, which is complete.
Issue #106 / PR #138 is the hardening repair discovered after that
validation; it merged on 2026-10-08. The operator accepted persistent
OS-lock containment and a truthful
degraded-enumeration contract for that PR. The mandatory downstream
**station-intent-transactional-authority** node
([#153](https://github.com/lossyrob/telex/issues/153)) closes the accepted gap before
the final **closure gate**, without blocking PR #138 or the builder
**hardening gate**. Nodes are coarse and PAW-sized; the completeness split is
justified by a transactional migration boundary and an independently useful,
safe PR #138 outcome.

The schema-3 recovery release is a separate, bounded chain. Issue #154 covers
the two unsafe Windows `TOKEN_USER` reads on exact main. Issue #155 adopts draft
PR #156 for PostgreSQL query and `LISTEN` reset recovery. After both repairs merge
and the campaign verifies dependency closure, one isolated worker prepares an
immutable candidate for the campaign/operator-owned
`schema3-release-gate`. The v0.2.0 tag run failed on Windows, so v0.2.0 is held
unpublished; the corrective v0.2.1 was prepared by the same node and published
on 2026-09-30 after a new operator decision. This chain does not reopen the
earlier completed release nodes and
does not accept hardening or closure.

The richer design rationale and the full decision ledger that led here live in
[`docs/initial-shaping.md`](docs/initial-shaping.md). The brief stays current and
distilled.

## Design References

The authoritative design layer (merged from `design-foundation`) lives under
`docs/design/`:

- `telex:docs/design/daemon.md` - the **normative daemon contract** the implementation
  nodes build against (17 sections + the sec.17 gating tests).
- [`design/current-design.md`](design/current-design.md) - the canonical integrated
  workstream design. It summarizes merged authority and keeps accepted issue #106 /
  PR #138 direction explicitly behind a product-promotion boundary.
- `telex:docs/design/DESIGN.md` - the local-exchange architecture.
- `telex:docs/design/DECISIONS.md` - the ADR log; **0014-0024** are this workstream's
  decisions (0023 = the minimal session/presence/delivery model; 0021 = the
  `docs/design/` relocation).
- `telex:docs/design/index.md` / `docs/design/ARCHITECTURE.md` - the entry point and the
  5-diagram visual on-ramp.
- `telex:PRODUCT-THESIS.md` (root) - the "no server" -> "auto-spawned local exchange"
  framing.

## Boundaries

- **In scope:** the per-user daemon (presence + transport) for SQLite **and**
  Postgres; one-shot `attach`/`detach`/`wait` against the daemon; durable buffer
  (reuse 0011/0013) with the **seen-dedup redesign** for a long-lived daemon; the
  **lease-epoch fencing token** with a **server-side fence on delivery emission +
  ordered handoff** (`mark_delivered_if_current_owner`) proven by a distinct
  **fencing-proof** gate; the daemon **singleton identity** (user SID + config root +
  protocol-major) and **lifecycle contract** (spawn-lock, connect-or-spawn, readiness
  ACK, `wait` reconnect-on-EOF grace, exit codes, Status surface); the **daemon-scoped
  capability + version-handshake IPC**; the ADR 0023 liveness model
  (authoritative, non-destructive `sessionEnd`, negative-only watched-process
  evidence with start-time, and a non-destructive idle backstop); explicit-only
  membership removed only by explicit **`Detach`**; the Copilot CLI plugin as the
  harness adapter and one shared source for `telex skill`; the **minimal upgrade
  floor** (versioned shim + `daemon stop
  --drain` + next-call respawn + legacy/non-epoch cutover rule) in `daemon-core` with
  full seamless upgrade (#6) last; retiring superseded mechanisms (#3 relay, pid-watch
  as a per-session holder, the re-arm dance) and updating the docs **with**
  `daemon-core`, not at closure; desired station-intent recovery with bounded
  OS-lock safety and a degraded partial-scan contract; and the downstream
  transactional-authority closure. The bounded recovery-release addition covers
  issue #154 Windows token-buffer alignment, issue #155 PostgreSQL wait-reset
  recovery, and isolated schema-3 candidate preparation and publication gating
  after both repairs merge (v0.2.0, then corrective v0.2.1).
- **Out of scope:** the embeddable SDK client (#12) - it shares the
  collapse-into-one-process theme and should reuse the stabilized Layer-1 IPC, but
  is a separate solve; response windows / TTL deadlines (#2); the `store_key` helper
  (#25). The completed recovery-release packet also excluded PR #138, issues
  #152/#153,
  Watcher and Operator Station runtimes, campaign closure, schema-policy redesign,
  shared or production database mutation, and operator-installed daemon operations.
- **Deferred:** a richer non-binary occupant status policy beyond the accepted
  non-destructive liveness states; the pid-reuse-immune fd-over-IPC backstop
  (#28-flavored), awkward with a singleton daemon (the accepted process evidence
  uses PID + start-time); and the daemon subsuming directory/occupancy reads
  (`address list`).

## Current State

On 2026-10-25 Campaign accepted the fixture/precondition assessment for
[#193](https://github.com/lossyrob/telex/issues/193) and authorized one
immediate bounded Local repair owner. The new
`intent-seed-precondition-repair` node establishes successful seed setup
while preserving the production 480ms budget, non-theft and generation
fencing. Its hosted check must distinguish relevant admission causes; no
widening, retry, skipped seed or flake exception is authorized.
Local owns the single v2 launch after canonical preparation; the concrete
implementation owner is pending that launch. Steward contract confirmation
is already available and gates the PR, not staffing. The repair delivers
one PR closing #193 and returns to the Campaign gate. Watcher PR #180 still
needs its own strictly green current-head default Windows build/test;
critical-path priority adds no Watcher graph edge or readiness waiver.
Accepted #181/#187 delivery, retained debts and the workstation freeze
remain unchanged. Original candidate/no-owner and conditional-staffing
source snapshots are historical, not a further launch gate.

On 2026-10-23 Campaign promoted the repeated Windows partial-page candidate
into active `cooperative-partial-page-repair`, tracked by
[#181](https://github.com/lossyrob/telex/issues/181). After canonical
promotion landed as `3d517e17`, Local completed v2 preparation and delivered
the real kickoff to owner `9d4fee6c-63a6-4aa7-b384-a4df1cbcbb4c` in
checkout `lossyrob-expert-doodle`, initially on that branch and main commit.
The requested profile is `gpt-6.1-sol` / high reasoning / default context
tier / interactive. The launched v2 binding supersedes the earlier pending
preparation and v1 identity-exchange wording; no API/SDK/v1 handshake is
required. Launch does not establish implementation or hosted-validation
completion.
Draft [PR #185](https://github.com/lossyrob/telex/pull/185) now carries the
proposed repair at head
`7b11b92630d0b2cf63423a77fc19232aff368825`, base `main`, branch
`lossyrob-cooperative-partial-page-repair`; its body says `Closes #181`.
The PR is OPEN/draft. The owner reports the DI-185-01 same-origin fixture
correction. A later exact-head steward inspection approves its bounded
design-only source resolution, with zero open design findings in that
scope. Independent App review and new-head hosted validation remain
unresolved, not Campaign-ready. Earlier head `57fdc975` and its registered
CI wait are historical, not current coverage. This association establishes
no runtime acceptance or historical root-cause conclusion; no old-head
acceptance carries over.
The repair blocks delivery of Watcher CLI #179 / PR #180, whose existing
owner and Watcher-only scope remain intact. Diagnosis precedes correction;
the repair PR must close #181 and earn exact-head hosted validation and
Campaign merge authorization. The workstation freeze remains in force.
The two marker candidates, issue #153, and hardening/closure decisions are
unchanged. This is bounded repair authority, not product-design acceptance
or a merge authorization.

On 2026-10-24 Campaign authorized the separate
`macos-persistent-lock-repair`, tracked by
[#187](https://github.com/lossyrob/telex/issues/187), upstream of #181.
Issue #181 is blocked; owner `9d4fee6c` and OPEN/draft PR #185 remain
unchanged at `7b11b92630d0b2cf63423a77fc19232aff368825`. One new v2
`gpt-6.1-sol` / high / default / interactive owner is authorized but not yet
prepared or launched; Local acts after canonical landing verification.
No further retry, skip or threshold workaround. Accepted upstream delivery
permits reconsidering the prerequisite, not automatic #181 or Watcher
completion. Earlier launch/review/wait statements above are historical;
all terminal, retry, recurrence and delayed-writer evidence is preserved.
The workstation freeze and every unrelated gate remain unchanged.

The pending-launch statement above is the promotion-time snapshot. Local
later reports completed v2 preparation and one real kickoff delivered to
owner `5d9b3155-0327-40d1-936d-7a3652ea90c3` from main `970be041`, initial
checkout/branch `lossyrob-stunning-telegram`. The same checkout now uses
`lossyrob-macos-persistent-lock-repair`; a receipt-time App read observed
that same owner/checkout/current branch busy. The requested
`gpt-6.1-sol` / high / default / interactive profile is not effective-capacity
proof. The launched binding qualifies only old pending preparation/launch
text: no implementation, runtime, CI result or gate acceptance follows.
Issue #181 stays blocked and PR #185 stays draft and unchanged; the freeze
and every unrelated gate remain.

PR [#189](https://github.com/lossyrob/telex/pull/189) now carries the proposed
#187 repair, OPEN/draft at `d0b1010594aff7c27f7629ceb43ddad3c79118be`,
with the same owner/branch, base `main` and `Closes #187`. Local absorbed the
bounded canonical-parent fixture correction in that source, preserving the
eight-creator race and production privacy behavior; no new upstream exists.
Old `add79d51` module passes and design approval remain historical. The
current-head delta design receipt is APPROVED only for design; App source
review, hosted CI, wait-registration receipt, delivery/readiness and every
downstream gate are not credited. #181 remains blocked, PR #185 unchanged,
and the workstation freeze and all unrelated boundaries remain.

The preceding PR #189 draft/review/CI-pending paragraph is historical.
Local now qualifies the same `d0b1010`/`b8bd08f3` OPEN/non-draft/CLEAN PR
as ready for the operator: finalized App source verdict has zero #187
blockers, zero ready comments and one skipped advisory, exact-head design
is APPROVED, and all 21 reported CI entries succeeded in `40138318787`.
The retained macOS log verifies direct head, 58+13 passing tests and
514.511584ms contention below the unchanged 1,540ms assertion, not a hard
480ms return guarantee. Local supplies bounded verification; the reconciler
matched retained sources without independently querying CI or the scheduler.
Later coordinator-attributed wait receipt qualifies the old receipt-pending
wording; the existing lifecycle-only merge wait grants no merge permission.
This is operator-ready, not merged/done. Main CI, field report and accepted
delivery remain pending; #187 stays in progress, #181 blocked and PR #185
draft. All ledger records, owners, dependencies, Watcher, freeze and every
unrelated gate remain unchanged. Head movement invalidates this readiness.

The OPEN/operator-ready and PR #185 unchanged-hold statements above are
historical snapshots. PR [#189](https://github.com/lossyrob/telex/pull/189)
merged on 2026-10-25T16:55:54Z as
`d03492ea425f001df530f4405808d3a817558291`, from unchanged reviewed head
`d0b1010594aff7c27f7629ceb43ddad3c79118be`; issue #187 closed as completed
at 16:55:56Z. Local and the reconciler verified those provider identities.
Campaign directed the existing #181 owner to merge main into PR #185 and
run CI again, superseding the unchanged hold for that integration and
validation only. No new-head review/CI success or readiness is established.
The existing #187 owner retains post-merge main CI and field-report
obligations; neither is verified or accepted here. Node #187 remains in
progress and #181 blocked on accepted upstream delivery, with the existing
dependency intact. The direction permits integration/revalidation, not
automatic dependency clearance, Watcher readiness or gate acceptance.

Campaign's prospective Local source-review floor took effect on
2026-10-25T12:56:34.591-04:00: the source reviewer itself must post an
exact-current-head verdict receipt on the reviewed PR, retaining scope and
limits distinct from design, CI and merge authority. This also applies to a
finalized `no-comments-to-post` review with no inline findings or formal
GitHub approval. An owner attestation, private artifact or coordinator relay
alone does not satisfy publication; an older-head receipt stays historical.
Before future floor-complete readiness, verify the reviewer-origin receipt
at the current reviewed head. Missing publication is now a floor miss.
Include this additive instruction in new Local owner/reviewer kickoffs and
delta-review requests, preserving `gpt-6.1-sol` / high / default for new
sessions. Campaign reports PR #189's owner-only publication gap was accepted
as debt this time, like the gap it names on PR #184. Prior private review
and readiness records remain historical, not retroactively floor-complete.
No receipt backfill, review, empty approval, posting for the reviewer, debt
clearance, merge reversal, issue reopening or runtime action is authorized.
This supplement changes no ledger, owner, status, dependency, workstation
freeze, #153, Watcher or hardening/closure gate.

A later Local qualification records OPEN/draft
[PR #185](https://github.com/lossyrob/telex/pull/185) at
`130be3b257e8208f521f9e4e91bc6795b991e023`,
tree `fd53e11c478ae8e4b7ca19855734735803feac79`, with ordered parents
`7b11b92630d0b2cf63423a77fc19232aff368825` and actual PR #189 merge
`d03492ea425f001df530f4405808d3a817558291`. Local and the reconciler
verified PR and immutable commit metadata, not the complete merge diff.
This observes the authorized integration; the older `7b11b926` and
integration-pending statements are historical. Local verified the identity
of run `40214432369`, `pull_request`, attempt 1 at this head, not its
results; the reconciler did not query CI. The owner's public receipt
reports both three-OS matrices and ten real partial-page repetitions kept,
no workaround or threshold change, compile-only checks passed, and new-head
source/design requests pending. Its reported later Campaign routing defers
consolidation assessment until after #185 lands; this authorizes no
consolidation or matrix removal. Sole-observer registration is pending in
this snapshot. Fresh head-bound review/design/CI evidence and the source
reviewer's own exact-head public receipt remain pending; no readiness is
credited. The owner's accepted-upstream/blocker-resolved language does not
establish #187 main-CI/field/terminal acceptance or remove #181's dependency.
All statuses, owners, ledger, gates, Watcher and freeze boundaries remain.

A later source/design-only qualification supersedes that pending wording
for unchanged `130be3` / `fd53e11c`, product base `d03492ea`.
The source reviewer's public
[receipt](https://github.com/lossyrob/telex/pull/185#issuecomment-6400606700)
records zero blocking and zero non-blocking findings, with R1 still resolved.
Local and the reconciler read the full public receipt; Local separately
received reviewer `2d1c11d8`'s direct confirmation of its own `gh` post and
existing REST body/head checks. Shared account identity alone is not
publisher proof; raw execution history was not independently audited.
This supports the reviewer-origin exact-head publication sub-floor, not
overall readiness. The renewed exact-head steward design report is
APPROVED with zero new/open findings and DI-185-01 resolved; Local and the
reconciler matched its 17,092 bytes/hash and read it fully. Its earlier
source-publication-pending statement is historical, not rewritten.
Source and design remain distinct bounded verdicts, not CI/runtime proof.
Run `40214432369` results and observer-registration completion are not
credited. #187 main-CI/field/terminal acceptance, #181's dependency, all
statuses/owners/ledger/gates, Watcher, freeze and historical #189 receipt
debt remain unchanged. Any PR-head movement invalidates these receipts.

On 2026-10-25, Local and Campaign accepted bounded #187 terminal delivery
at PR #189 merge `d03492ea425f001df530f4405808d3a817558291`,
tree `39a45aa5e238fcb86e67eebef85a08f00030ebf1`.
The separate 10,480-byte Local qualification, SHA-256
`d85bf58b789a643d5d6ca05c0d6c143d086d747d50e8202ff83a7923a3fff0fa`,
independently verifies main CI `40214028177` attempt 1, all 21 jobs,
Docs `40214028168`, both jobs, and the full
[field report](
https://github.com/lossyrob/telex/issues/187#issuecomment-6400771517).
The reconciler read the report, matched the retained main macOS log and
read only verifier/measurement/named-result excerpts: direct merged SHA,
510.866708ms below the unchanged 1,540ms ceiling, 58+13 passing tests.
This is not a full-log read, repeated CI query, hard 480ms return guarantee
or historical timeout-cause conclusion. Design impact is `updated-docs`.
The later 2,821-byte Campaign supplement, SHA-256
`fdf82497a84ca6b14bce0ae42a274b428967415d64661c464f93c4a7b76e1109`,
confirms express terminal acceptance; #185's gate does not wait on
artifact publication. #187 and only its two linked ledger records are
completed; #181 is in progress with its completed #187 edge retained as
satisfied lineage. This receipt supplies no #181 readiness, merge,
accepted delivery or Watcher #179 / PR #180 release. Earlier pending and
blocked statements above/below remain historical, not rewritten.
PR #189 publication debt, CI-overlap/sentinel and optional Clippy evidence,
all other ledger records, owners, attention, gates and freeze remain.

A separate 2026-10-25 merge-history qualification records actual #185
merge `65f5846d50bc4c07b3eaf9ffed0800f4cb19a17b`,
tree `144382f5b9bc4d5951113e3b588c4e23cab0530d`, and completed #181
provider closure; ordered parents are `ea623032` and reviewed `130be3`.
The immutable 12,565-byte readiness packet,
SHA-256 `cb8f3b5253f169e5116c46af931e076c7d7250d22c20a65b44775a195ff59b2d`,
is explicitly pre-merge history, including Local's PR-head 24/24 and
three-platform 62+10 repetition checks, not current OPEN/ready or main CI.
The separate 5,195-byte merge packet,
SHA-256 `366424cf4f838e16e1a04f6b9727746e740e713c29ba08b92c26585447c38370`,
and full [Campaign gate](
https://github.com/lossyrob/telex/pull/185#issuecomment-6400899418)
record **merge + debt, confidence high**. Main CI/field/Local acceptance
were distinct follow-through, not inferred from provider closure.
The only ledger addition is `mergeDebtAndFollowThrough` on the existing
partial-page overrun record, retaining routed/null-source/accepted and all
prior evidence. Local retains the real-time 200ms positive-test coverage
gap and unknown historical `Overran` cause; recurrence means a new item,
not retry. Sentinel CI consolidation/repetition reduction and optional
Clippy remain separate; fewer repetitions are reduced exposure.
#187 stays completed and #181 in progress with their edge retained for
this merge-only receipt. Original pending/observer/ready snapshots stay
historical. Later accepted #181 delivery/Watcher resumption is a separate
successor; this receipt supplies no such release, readiness, debt
clearance, product/runtime/CI action or freeze change.

A later 2026-10-25 terminal receipt accepts bounded #181 delivery at
`65f5846d50bc4c07b3eaf9ffed0800f4cb19a17b` / `144382f5`.
Local's immutable 13,796-byte qualification, SHA-256
`efa69b137efb54b6de605cd72165eb9d407374bdf9df3e17f99dc675813f0f2d`,
independently verifies actual main CI `40217242404` push attempt 1,
24/24 successful jobs, Docs `40217242455` with 2/2, four actual main
Windows/macOS captures and the full
[field report](
https://github.com/lossyrob/telex/issues/181#issuecomment-6401070057).
Local/reconciler hashes and selected verifier/result mechanics match
cooperative 62+10 each, station 62 each, filesystem Windows 14/macOS 13,
zero ignored; no full-log read, CI requery or local execution is claimed.
Lock timings 480.445/539.0605ms < 1,540ms do not measure partial-page
completion or prove a hard 480ms return. Campaign directly accepts
delivery; Local independently accepts it. #181 and its original
partial-page record complete with one additive terminal qualification,
retaining every old field/evidence and the completed #187 lineage.
The Local real-time 200ms positive-coverage gap/unknown historical cause
and recurrence-as-new-item rule, sentinel and optional Clippy debts stay.
Only the existing Watcher Local prerequisite is satisfied. Owning Watcher
separately selects #179 in-progress/focus; the immutable 2,185-byte Local
supplement qualifies its older preserve-watch instruction. Owner, PR,
runtime-core lineage and Watcher's own current-head gates stay intact.
Earlier ready/merge/pending snapshots remain historical; no Watcher
readiness, merge, delivery, debt clearance, new runtime/CI/observer or
freeze/closure authority follows. Owners do not wait on publication.

The design foundation, daemon core, fencing proof, Postgres parity, Copilot plugin and
push bridge, lifecycle hardening, versioned/release upgrade paths, public release, and
release-confidence validation are merged and recorded complete. The normative design
remains `docs/design/daemon.md`, with ADR 0023 governing explicit-only membership and
non-destructive liveness; merged PR #139 additionally defines the Copilot App
turn-idle and bridge-host lifecycle behavior.

Dogfooding then exposed issue #106: daemon replacement can preserve durable messages
while losing a still-live bridge's desired push registration. Existing PR #138 is the
adopted `station-intent-reconciliation` repair. The operator selected persistent
owner-private OS advisory locking to prevent stale pathname mutation and accepted a
degraded contract for bounded partial directory scans. PR #138 was
excluded from the completed schema-3 recovery release. Its existing writer
resumed on 2026-10-08 in parallel with Application Client issue #152, and the
PR merged later that day as `6ab6143a` after campaign authorization. The
**hardening gate is not accepted**: the merged narrowed repair still has to be
presented with isolated restart/drain/upgrade and push-recovery evidence.

Unconditional transactional generation authority, seekable fair discovery and
garbage collection, exact counts, and exact over-cap recovery belong to the
XL `station-intent-transactional-authority` node
([#153](https://github.com/lossyrob/telex/issues/153)). That node follows PR #138 and
blocks the final **closure gate**, not PR #138 or the hardening gate.

Exact main `ed417c6b938f92fe3bfb84f3b0cc0bea719fbbd0` is schema 3 and protocol
1.5, while released v0.1.2 supports schema 2 and protocol 1.4. Exact main also
contains unsafe `Vec<u8>`-backed `TOKEN_USER` reads in both
`src/backend/sqlite.rs` and `src/daemon.rs`; the daemon alignment repair exists
only on unmerged PR #138. Draft PR #156 is published at
`5c302dacb1c3e659e7f89c3c9670e3cf5cbc5105` for issue #155 and must be adopted
without replacing its branch or history. The recovery release prepares v0.2.0 only
after issues #154 and #155 merge, using disposable isolated roots and databases.
Tagging and publication remain a separate explicit operator decision.

Both recovery repairs launched against main
`7ed886b07620e8ab8adba68249ab84f96ea26013` and are in progress. Issue #154 has one
worker on `feature/windows-token-buffer-alignment`, with no PR yet. Issue #155 has
one worker completing adopted draft PR #156 in place on
`copilot/fix-postgres-connection-reset`; its local starting head `3423cd6` merges
main into published head `5c302dac` without rewriting history. At launch, neither
repair had completed review, CI, or merge. Issue #157 is not launched, and the
release gate, hardening gate, and closure gate remain planned.

As of 2026-09-25T00:09Z, PR #156 was no longer draft, and its complete candidate
`25aea118` was under initial full PAW review (`8cda7626`). Exact-head CI run
36071395738 succeeded on that head. The #154 candidate `f8363be` is pushed and
clean but has no PR; publication is held on an operator-owned App link. Both nodes
remain in progress.

The full independent PAW review of `25aea118` is complete and was posted as
GitHub COMMENT review 5311857915. Its verdict is changes requested: two P2
blockers, no warnings, and two optional low-priority observations. The blockers
are stop-outcome precedence during drain and cancellation of the credential
helper; both are absorbed into PR #156. The same #155 worker is repairing them.
There is no new head yet, and design inspection and merge remain held. The
earlier CI success covers only `25aea118`.

The operator selected the one-shot `--password-command` lifecycle contract for
M2. Normal, error, and cancellation completion must clean up Telex-owned helpers
that remain within the supported invocation scope. Intentionally persistent,
escaped, or broker-launched background work is outside that command lifecycle.
Querying an already-running credential agent does not make Telex its owner or
authorize Telex to terminate it. This policy does not promise universal
all-descendant containment or fail-closed detection of every Unix escape.

The operator also selected `Approve bounded orderly-exit drain and explicit
failure hold (Recommended)`. Logical command and Wait result deadlines remain
unchanged. Normal exit of the host that owns credential work may take up to
three additional seconds to drain and join owned work concurrently against one
absolute current cleanup budget. The budget is not renewed or multiplied by
capacity. Waiting three seconds is not proof of cleanup. If receipt remains
missing, Telex must report failure and retain named ownership and escalation,
potentially beyond three seconds, rather than report a clean exit or promise an
unconditional three-second OS exit bound.

Campaign accepted the exact reviewed intended M2 technical design on
2026-09-25T13:41:31-04:00. The accepted engineering limits include the
credential-specific process-local native owner and registry, finite admission,
aggregate capacity `C=2`, same-source barrier, owned nonblocking I/O, atomic
cancellation and publication, platform receipt conditions, join and shutdown
ownership, and internal cleanup-observation target `B=3s`. These limits are not
measured performance or universal finite OS cleanup guarantees.

The accepted design uses the steward's technically feasible Unix receipt
sequence with supported-API conditions, superseding the earlier blanket
feasibility blocker.
Telex would retain the unreaped group leader through every mutating group signal,
send the final termination signal while that identity anchor is valid, enter an
irreversible no-more-group-signals state, reap the exact leader, and then use the
former group ID only for read-only absence observation. Linux would use
`getpriority(PRIO_PGRP, P)` with exact `errno` handling; macOS would use the
supported libc POSIX/UNIX03 `kill(-P, 0)` binding. Receipt would require
conclusive group absence, exact leader reap, closed or completed owned I/O, and
native owner completion. Presence, denial, ambiguity, or possible reuse would
retain the cleanup obligation without restoring signal authority.

The accepted intended design landed on main at `a6eabba`. Campaign then granted
the same #155 worker M2 product-write authority at 2026-09-25T14:27:39-04:00.
After a context refresh that preserved the three unpublished M1, C1, and C2
commits, the worker acknowledged the grant before editing at local head
`2e2f3b99`, which merges `5d956618` and `a6eabba`, and began the accepted plan
phases P1-P7. The intentionally red credential regression may now become an
actual regression test; its original negative evidence is retained.

The operator selected the Windows job-terminal receipt on
2026-09-28T09:47:17-04:00:
`Approve Windows job-terminal completion with the documented limits
(Recommended)`. Before eligible atomic credential-result publication and
source/admission release, Telex must successfully terminate the exact owned
private single-use job, observe zero active job processes, observe the exact
launched leader process handle signaled, finish or close owned I/O and handles
with checked results, and join the native owner. Failure, cancellation, and
`FAILED_HELD` use the same final arbitration; no close or join error is ignored
and no closed handle is reconstructed.

This selection expressly replaces the stronger current Windows requirement that
every former descendant process handle be signaled. Former descendant handles
may remain nonsignaled during kernel or driver rundown. The receipt does not
prove that all former process objects are signaled, all kernel, driver, or
previously issued external I/O has finished, all external references have
disappeared, or residual objects have a finite bound. `C=2` bounds Telex
invocation owners and reservations, not descendant count or all residual
Windows resources.

The original stronger red observations remain historical negative evidence:
job accounting reached zero while retained process handles were nonsignaled in
the recorded A and B cases. They are not retroactive passes, measured harm, or
proof of continued user-mode work. The advisory review also requires checked
finalization/publication arbitration and a separately owned diagnostic channel
for any fresh post-publication challenge. A positive fresh reply falsifies the
operational interpretation for that run; nonresponse cannot prove universal
quiescence. No new runtime proof is recorded by this artifact update.

On macOS, the frozen Unix code confirms a
conditional source defect: a final group `SIGKILL` that fails with `EPERM` on a
zombie-only group is latched as failure before the exact reap and independent
absence check. Campaign then authorized the steward's narrow correction as an
ordinary implementation fix: record the attempt, seal signals, reap the exact
leader, and require independent absence. `EPERM` is never absence. The worker
acknowledged at 2026-09-25T15:20:59-04:00 before editing. At that 2026-09-25
checkpoint no macOS program had run, and Linux and macOS runtime proof was
unrun; later worker runtime evidence belongs to Local's proof intake. Hosted
macOS runtime proof later ran in PR #156 CI, recorded below. Any material change
to intended authority requires reviewed Tier B reconciliation.

Since the operator's delegation at 2026-09-28T09:20:35-04:00, the same #155
implementer owned autonomous design, isolated experiments,
code, tests, ordinary pushes, and review fixes. This artifact proposal and its
review or reconciliation did not gate that routine work. Material new guarantee
choices still went directly to the operator.

Issue #154 is complete. The earlier App EMU 403 and quota results remain
historical evidence. On 2026-09-28 one authorized retry by the same worker
(`d13b474e`) opened [PR #158](https://github.com/lossyrob/telex/pull/158) from
`feature/windows-token-buffer-alignment` at `f8363be`. Its first CI run failed
only in the postgres and entra jobs because SQLite-only test symbols were unused
under the CI warning policy; the same PR fixed this at `74b5041` without
weakening CI. Full review 5340941652 found no blockers or warnings, CI run
36439483760 passed all 14 required jobs, and design inspection passed. After
campaign authorization, PR #158 merged at 2026-09-28T15:43:41Z as `afda9460`,
and issue #154 closed as completed. This satisfies only the #154 dependency of
release preparation.

Issue #155 is complete. After campaign authorization, PR #156 merged exact head
`eda9ac24` at 2026-09-28T19:32:21Z as `62291a78`, and issue #155 closed as
completed. Full review 5342232604 rebaselined M2 at `836efbc` and the causal
notify fix at `c71befc`; its one LOW proof warning was resolved by an actual
nested-job proof at `038eb102`, and the final readiness-fixture delta at
`eda9ac24` kept the cumulative result at 0 blockers and 0 warnings. CI run
36470043028 passed all 15 jobs on the exact head, including Live PostgreSQL,
Windows, and macOS credential jobs, and design inspection passed. Earlier CI
failures at `836efbc` (Live PostgreSQL latency oracle) and `038eb102` (Windows
readiness) were fixed in the same PR and remain historical. The selected limits
remain: `FAILED_HELD` may hold beyond three seconds, the Windows job-terminal
residual limits apply, macOS `EPERM` is never absence, and legacy-process proof
is same-image rather than a genuine v0.1.2 deployment.

Campaign has independently verified both repair merges, both closed issues, and
their ancestry in `62291a78`, closing the dependencies of release preparation.
It directed Local to launch exactly one #157 worker under the accepted scope.

Issue #157 release preparation is in progress. On 2026-09-28 campaign created
one branch-mode release worker (`ebfd215b`) on
`feature/schema3-release-preparation`, controlled by the Local orchestrator.
The worker acknowledged write authority before editing and is implementing
under routine autonomy. Its read-only audit found that `install.ps1` always
changes the user PATH; a default-preserving opt-out for disposable install
proof is accepted in the same node and still pending. Genuine v0.1.2 upgrade,
protocol 1.4-to-1.5, schema-2 and schema-3, fresh-install, CI, and built-asset
proof all remain required. `schema3-release-gate` stays planned, and tagging
and publication still require explicit operator approval.

After campaign authorized exact head `efc30214`, preparation PR #159 merged on
2026-09-28 as `1b9fc8f0`, with a tree identical to the reviewed source. This
is a source merge only: issue #157 stays open, and the node stays in progress.
The same worker now owns final proof against `1b9fc8f0`: merged-source CI, one
build-only Release run, newly built artifacts, genuine old-binary upgrade and
install, schema-2 and schema-3, native and Linux PostgreSQL coverage, cleanup,
and the immutable gate packet. Later artifact-only main movement does not
change that target.

Release preparation is complete; the two in-progress paragraphs above are dated
history. On 2026-09-28 Local accepted the preparation delivery at `1b9fc8f0`
after independently verifying the sealed packet (SHA-256 `720cd397...`) and
the posted field report. Final CI and the build-only Release matrix, including
hosted Linux PostgreSQL, passed at that commit with newly built artifacts.
`schema3-release-gate` stays planned: its dependency is met, but publishing
v0.2.0 still needs explicit operator approval, and issue #157 stays open until
campaign decides its disposition.

On 2026-09-30 the operator authorized publishing v0.2.0 from `1b9fc8f0` only.
The same worker pushed tag `v0.2.0` at that commit, and tag Release run
36734022444 (attempt 1) failed. The tag-version check and four native builds
passed, the Windows x64 job failed, and the Linux PostgreSQL and Publish jobs
were skipped. No release was published; v0.1.2 remains the latest release. The
Windows proof hit a local-file PermissionError (Errno 13) reading the successor
cap for a preexisting schema-3 root. The native Windows error and the underlying
cause are unknown. The tag stays in place, and no retry, tag move, deletion, or
withdrawal is authorized.

The operator then chose to hold v0.2.0 and prepare a reviewed corrective v0.2.1
candidate. Release preparation is reopened and in progress under the same
worker (`ebfd215b`). The completion paragraph above is dated v0.2.0 history,
and the `1b9fc8f0` packet is not v0.2.1 proof. No v0.2.1 source or PR exists
yet. Publishing v0.2.1 needs a new explicit operator decision at
`schema3-release-gate`, which stays planned, and issue #157 stays open.

The operator decision is also recorded on #157 as comment 5914989191. The same
worker has since opened [PR #160](https://github.com/lossyrob/telex/pull/160)
with the corrective v0.2.1 source at `4b097171`, so the sentence above about no
v0.2.1 source or PR is now dated. The PR sets version 0.2.1 and adds proof-only
Windows diagnostics that retry only a native sharing violation within the
existing readiness budget. Worker native controls show that the original
Errno 13 could mean either a sharing violation or access denied; the hosted
cause remains unknown. Independent review, hosted CI and Release proof, merge,
and final merged-source proof are pending. GitHub currently lists #157 as a
closing reference of PR #160, although its body says it does not close #157.

That provider history is now repaired and dated. Artifact commit `6dcaf045`
unintentionally closed tracker 157 (event 32182855058); Local reopened it
(event 32183059364), and the PR #160 body was corrected to zero closing
references. After campaign authorized the reviewed head `4b097171`, PR #160
merged on 2026-09-30 as `212b76a4`, with product source unchanged from that
head. This is a source merge only. The same worker now owns final proof at
`212b76a4`: merged-source CI, one build-only Release run with all native jobs,
both Windows controls, and hosted Linux PostgreSQL, a new inventory, and the
v0.2.1 packet and field report. The original Errno 13 cause remains unknown.
Tracker 157 stays open, and publishing v0.2.1 still needs a new operator
decision.

Corrective v0.2.1 preparation is complete; the pending-proof sentences above
are dated history. On 2026-09-30 Local accepted the delivery at `212b76a4`
after independently verifying the sealed packet (SHA-256 `b74c9232...`) and the
posted field report. Final CI and the build-only Release matrix, including
both Windows native controls and hosted Linux PostgreSQL, passed at that commit
with newly built artifacts. The original Errno 13 cause is still not
identified. `schema3-release-gate` stays planned: its dependency is met, but
publishing v0.2.1 needs a new explicit operator decision, and tracker 157 stays
open until campaign decides its disposition.

v0.2.1 is published; the paragraph above is dated history. The operator
authorized publishing exactly `212b76a4` and its sealed packet. The same
worker tagged `v0.2.1`, and tag run 36754203801 passed all 8 jobs, including
both Windows native controls and Linux PostgreSQL. Release v0.2.1 (ID
400301560) was published at 2026-09-30T17:57:24Z as Latest. Local verified
the live asset bytes, and isolated Windows and Ubuntu runs upgraded a real
published v0.1.2 install through the live installers. `schema3-release-gate`
is complete. Campaign accepted only this bounded gate completion, and Local then set tracker 157 to closed as completed at 2026-09-30T18:16:08Z (event 32189988997) by an explicit provider action, not a commit directive. The `v0.2.0` tag stays unpublished history, and
the original Errno 13 cause remains unknown. This does not accept hardening
or closure.

On 2026-09-28 the operator also answered "Keep #155 required; hold until a
reviewed Windows solution exists", and campaign confirmed it. The later
job-terminal choice supplied that intended Windows receipt but did not by itself
complete implementation or proof. The schema-3 recovery release did not accept
the PostgreSQL-reset limitation, PR #156 could not merge partially, and #157
stayed unlaunched until both #154 and #155 merged.

On 2026-10-08 the operator approved the post-release direction: Application
Client issue #152 is the campaign main effort, and issue #106 / PR #138 proceeds
in parallel. Campaign directive
`campaign-postrelease-execution-directive-20261008.json` (SHA-256
`a4d98af7d891efe7164839c79b4431d07acfc1ceef606c54e46b82ab7eb5613d`) records
this direction. The same PR #138 writer resumed at clean head
`6315c24a5b36989f3f9dac916458f8ea9e752e60` on
`feature/station-intent-reconciliation-106` and is merging current main
`2c084873719080b812e0d0c4bf92125afa751499` into the branch. The PR is open and
non-draft but conflicts with main; from merge base `ed417c6b` the branch is 58
commits ahead and 57 behind. No new candidate exists yet. The accepted Option A
M3 and M5 outcome is unchanged. The 2026-09-02 technical floor is not current
merge authority: merge needs fresh exact-head review, CI, design inspection,
and campaign authorization. Review sessions `41c2eef9` (Option A) and
`e5e3bc0c` (full PAW review) and design steward `af271672` are retained.
After PR #138 merges, issue #153 is promoted from the actual landed authority
with a complete task specification; it remains mandatory for closure and does
not block PR #138 or the hardening gate. Hardening and closure stay separate
builder decisions.

Later on 2026-10-08 that integration completed. PR #138 is open, non-draft,
and mergeable at `975f10efe907bb1959129a21962324b70bc00e15` (tree
`1f5f324c310b03b891cf7edbe8f6ebd0f0a0a134`), which merges current main
`233d46af77f56e698ade15847c8e1fbe731d081f`. CI run 37841292797 passed all 15
jobs on that exact head. Integration exposed two CI failures, both repaired in
the same PR. First, test code already on main used the deprecated
`fetch_update`; `9893c638` replaced it with `try_update`. Second, Rust 1.99
Clippy rejected redundant `must_use` attributes generated by `async_trait`:
four in the watcher adapter, then 72 on `Backend` across the workspace.
`21b37f9e` kept the adapter reasons, and `3d652f3c` updated only the lockfile
to `async-trait` 0.1.92 (adding `syn` 3.0.6). Ledger item
`local-daemon-pr138-exact-head-ci-failures` records both repairs. Local reports
zero findings from focused review 5462370793 at `9893c638` and from internal
reviews at `21b37f9e` and `975f10ef`; these add to full review 5095418333 and
delta review 5095588119. Design steward `af271672` is reinspecting the head
against canonical design revision `233d46af`. That result and campaign merge
authorization are pending, and the node stays in progress until PR #138
merges.

Design reinspection of `975f10ef` found one documentation error, D1
(`pr138-design-D1-reset-order`). The `Reset` row in `docs/design/daemon.md`
placed `reset_epoch_lease` before the per-binding work. The runtime does the
reverse: it enumerates durable and live bindings, then for each binding,
under that binding's admission, marks it idle, withdraws its intent, and runs
a second idle sweep. Only after every binding succeeds does it reset the
epoch. The same wrong row was present at `6315c24a`, so D1 is an omission in
the earlier design pass, not a regression from integration. The same writer
corrected only that row at `286251b6de93b2e84ccd4547b6e16b8b2472c36a` (tree
`56fa6e1155cefc832720b201f0448f2e2d5122ae`); `src/daemon.rs` is unchanged.
CI run 37844195350 passed all 15 jobs on that exact head. Local reports a
clean internal review of the 975f10ef to 286251b6 delta by `41c2eef9` with
zero findings. Steward `af271672` passed the design with D1 resolved and
zero blockers or warnings. Neither design result was posted to the PR.
PR #138 is open, non-draft, and mergeable at that head. The exact-head
review, CI, and design evidence is therefore in place, and ledger item
`local-daemon-pr138-exact-head-ci-failures` is complete. Campaign merge
authorization is still pending, and the node stays in progress until PR #138
merges.

The campaign then authorized a guarded ordinary merge of exact head
`286251b6` (authority `pr138-campaign-merge-authority-286251b-20261008.json`,
SHA-256 `59d658109d05078d35133f261edc9255971d490b9f4270412fd9d00ec672fb63`).
Local merged PR #138 at 2026-10-08T21:48:41Z as
`6ab6143ad0d0e8b9832df41e4e180c4acd1204c4`, with parents `9166fce0` (main) and
`286251b6` and tree `a9bd5f0dc4a8867b3dc04b7358d0d2fdd3eef902`. That tree
matches the one computed before the merge. Local posted field report
6069716718 on issue #106, then closed the issue as completed at 21:50:45Z by
an explicit provider action (event 32828313724, no closing commit).
`station-intent-reconciliation` is complete. The ledger items for the M3 lock
repair and the stale pending review are complete. The M5 item stays routed:
PR #138 delivered the degraded-contract slice, and issue #153 still owns
unconditional transactional authority. The post-merge CI and Docs runs on
`6ab6143a` are post-merge checks, not candidate proof. The merge is source
only. No tag or release changed, and the hardening and closure gates are not
accepted. Design steward `af271672` then
found the issue #153 task ready for one supported preparation against the
merged source, so `station-intent-transactional-authority` is ready. No worker
exists and preparation has not started.

Local then applied the reviewed issue #153 body correction once (readback at
2026-10-08T22:03:07Z). Supported preparation `beb72642` completed from
`43d5e43f`, and worker session `62bbe494` correctly recorded the requested
`gpt-6-astra`, high reasoning, `long_context` profile. The preparation
initializer, however, ran without the long-context tier and with a
272,000-token prompt limit, because Streamliner preparation does not pass
the context tier when it creates or resumes the session. The node is
therefore blocked on external condition
`streamliner-preparation-long-context-propagation`, owned by the Streamliner
service with campaign authority over the repair, and the worker is held
before any product write. No #153 implementation has started. The task
outcome, single worker, prepared branch, and completed issue #106 / PR #138
record are unchanged.

On 2026-10-08 the operator authorized the recovery (campaign authority
`153-preparation-sdk-recovery-authority-20261008.json`, SHA-256
`20a8eb1f7b76f773955af752759699041227c38d93bb315678ec25ed8bc6968d`): one
reviewed preparation-only SDK adapter repair in the Streamliner repository,
one guarded restart of the owned API after it merges, and one replacement
initializer for the same held worker on the same checkout. No exception for
the 272,000-token run was granted. The Streamliner repair session started on
2026-10-12, but no repair has merged or been deployed, and no restart,
replacement initializer, or admission has occurred. The external condition
stays pending and the node stays blocked until the new initializer proves
long context and Local admits the worker.

Workstream and design-steward branches are proposal/integration workspaces, not
silent authority. Streamliner artifact changes become durable only through the
campaign's sole artifact reconciler applying the reviewed, operator-authorized
packet directly to `main`.

## Decisions

- **One complete deliverable, both backends:** SQLite and Postgres ship together;
  the SQLite spike is an internal step, not the boundary. Rationale: the operator
  runs both with stations idle waiting; a partial cutover does not unblock them.
- **Coarse, PAW-sized nodes (~one per confidence transition):** bias to fewer,
  heavier nodes; the three completeness tracks are the one deliberate split for
  parallelism + distinct expertise.
- **Local-spec-first tracking:** node specs live under `tasks/`; promote to GitHub
  issues at wave promotion. The umbrella issue #32 is the workstream's parent
  tracker.
- **Project design authority lives under `docs/design/`:** `daemon.md` is normative,
  `DESIGN.md` supplies architecture, and `DECISIONS.md` is the append-only ADR log.
  The workstream's canonical integrated summary lives at
  `design/current-design.md`; pending proposals are not project authority.
- **Spar at arm's length:** critique informs the design but pivots are surfaced for
  builder confirmation, not auto-applied.
- **Lease-epoch fencing is the spine (from spar):** daemon-down recovery, upgrade
  handoff, and Postgres reclaim are all made safe by one monotonic
  `lease_epoch`/`owner_instance_id` rather than by timing. `design-foundation` owns
  the epoch lifecycle.
- **Fencing-first sequencing (from spar):** lock the hard contracts (fencing,
  explicit membership, non-destructive liveness, watched-process identity) in
  `design-foundation`; gate Postgres on fencing proven under competing daemons;
  land seamless-upgrade last. Keeps both backends + #6 in the deliverable while
  limiting blast radius.
- **Server-side epoch fence + a distinct `fencing-proof` gate (council):** lease-row
  fencing alone is insufficient - delivery emission is fenced server-side
  (`mark_delivered_if_current_owner`; no frame unless the daemon owns the epoch) and
  handoff is ordered; a distinct executable `fencing-proof` gate blocks
  Postgres/plugin/upgrade until proven. Verified: the holder emits the frame *before*
  `mark_delivered` commits, and per-process `seen` resets across a handoff.
- **Minimal upgrade floor early (council):** a versioned shim + `daemon stop --drain`
  + next-call respawn + a legacy-holder/non-epoch-lease cutover rule land in
  `daemon-core` (the first daemon-aware install hits the Windows binary-lock); full
  rollback/gc/UX stays last.
- **Daemon-native session ownership (revised by ADR 0023):** the daemon's
  in-memory `session->addresses` map is the authority. Reuse the hook plumbing as
  a non-destructive liveness input; explicit detach, not sessionEnd, owns
  membership removal.
- **Station-intent safety now, transactional convergence downstream:** PR #138
  replaces age-stealable intent locks with persistent owner-private OS advisory
  locks and exposes partial-scan degradation under the four-second response
  contract. A connected XL node restores unconditional transactional generation
  authority and fair maintenance before workstream closure. This preserves one
  complete, useful PR #138 outcome while keeping the accepted gap durable.
- **Recovery release is an isolated confidence chain:** Repair #154 and adopted
  PR #156/#155 first. After both merge, one isolated worker prepares one immutable
  candidate and evidence packet (v0.2.0, then corrective v0.2.1 after the v0.2.0
  tag run failed). The campaign/operator publication gate is
  distinct from hardening and closure; green evidence never implies publication
  approval.
- **Docs/SKILL cutover with `daemon-core` (council):** keep the verb names; update
  `SKILL.md` + plugin docs when behavior changes, not at closure, so instructions
  never describe a dead holder/waiter model mid-workstream.

## Resolved questions and superseded validation shape

All eight design-foundation questions are **resolved** as ADRs 0014-0024 (see
`docs/design/DECISIONS.md` and `daemon.md`): epoch lifecycle (0015), session
presence/reaping + crash durability (0017/0023), watched-process evidence (0017),
legacy cutover (0020/0024), explicit membership and agent acknowledgement
(0019/0023), and the Status freeze line (0018).

The earlier `validation-harness`, Entra multi-host campaign, and AKS scale-rig
concepts were superseded by the completed `release-confidence-validation` node
(issue #78). The operator resolved the PR #138 M3/M5 fork on 2026-09-02:
PR #138 owns safe desired push restoration with persistent OS locking and an
explicit degraded-enumeration contract; the downstream transactional node owns
unconditional generation, discovery, GC, and over-cap authority.

## Imports and Exports

### Imports

- **PR #31 / issue #23 (sessionEnd hook plumbing):** the plugin reuses the hook
  wiring, but not its filesystem `session_registry` as attendance authority. The
  daemon owns `session_id->addresses` in memory; under ADR 0023 the hook supplies
  non-destructive liveness input while explicit detach owns membership removal.
- **Decisions 0011/0013 durable delivery (`deliveries` table, `fetch_undelivered`):**
  reused as the daemon's durable buffer. Available in `main`.
- **Harness env contract (consumed only by the plugin layer):**
  `COPILOT_AGENT_SESSION_ID` and `COPILOT_LOADER_PID`, verified present and reliable
  (explicit env vars, not ppid-walk). telex core stays harness-agnostic - it takes an
  opaque `$TELEX_SESSION_ID` and one or more generic `--watch-pid`s; the Copilot
  plugin maps these env vars onto them.

### Exports

- **Stabilized Layer-1 IPC/attendance protocol:** the daemon's documented control
  protocol, intended for reuse by the embeddable SDK client (#12).
- **Seamless-upgrade install layout + launcher shim:** the versioned-install
  mechanism (#6), reusable for any future telex distribution.

### External Dependencies

- None outside telex itself. Building/installing from source on Windows is locked by
  running `telex` processes during the binary swap - the very pain #6 fixes - so
  validating `seamless-upgrade` requires care during dogfooding.

## Closeout Observations

(parking lot - populated during execution)

## Actual bounded #193 owner - 2026-10-25

Owning Local reports the single authorized owner
`519693a5-0607-4aba-a968-345f3b1cd1b5` launched in existing checkout
`lossyrob-upgraded-system`, with `gpt-6.1-sol` / high / default /
interactive requested and no idle notification. Package
`ctx-20261025185147-10f8a3ea` delivered one 14,556-byte kickoff
(`ffd1f36d7e835d0f8243ef3b3d8f6fa39d05ac32806f0c8afe19f79ff6d2440f`);
the exact launched-binding snapshot is 4,238 bytes
(`ce9969d90ba623e1b5669f39399766b43bcbd300037927c0375f073528dc1c22`).
Local observed the same session/path/launch branch busy after admission.
Earlier pending-owner statements remain historical. This records launch,
not implementation, source/design/CI or effective-capacity proof.

Launch branch `lossyrob-upgraded-system` and preparation head `4deaf30cc`
remain unchanged history. A later separate 2,289-byte Local observation
(`a52c4a66e9f81c2cacd5c8a0339418c196e8f59e529bf3ae20fc711fc084eded`)
verifies that same session/checkout busy on App-renamed branch
`lossyrob-intent-seed-preconditions`, not a second owner or preparation.
Owner-reported integration at `0d5347a6` and its test-only repair direction
are not approvals; the main advance contains Watcher/Cargo.lock/design
changes, not just artifacts. No old review or CI acceptance carries.

Local later resolved observed App alias
`1185656f-ec05-4f88-aacc-f09b2431f9ee` to the same canonical owner using
both App lookups, with the same creation, parent, checkout and branch.
Local reports App metadata associates open
[#194](https://github.com/lossyrob/telex/pull/194); that is not a second
owner/rebinding, provider-head verification or source/design/CI approval.

Local's separate later provider read observed PR #194 OPEN/non-draft to
main at `f8b42d20acd3ae358a71a576d00c76b3f571ac6c`, same owner branch,
author `lossyrob`. The 2,286-byte immutable qualification has SHA-256
`eb2240d119727ff7e7d889b6683f0a7b060a3e9f0763c9bfe4cfb0b2a755e3be`.
This is attributed metadata at `2026-10-25T19:10:41.4572621Z`, not
source/design/CI or readiness. The later durable-binding update does not
rewrite the captured launch, branch and kickoff history.

The node stays in-progress/focus; counts remain 26 nodes and 45 records.
All earlier evidence and accepted #181/#187 delivery remain unchanged.
Production 480ms/non-theft/fencing, hosted cause discrimination, no
widening/retry/flake exception, independent source/design/CI/Campaign
gates, retained debts and workstation freeze remain. No Watcher edge or
state change, root-cause finding, second launch or readiness follows.

### Priority native-AV repair, separate from #193 - 2026-10-25

Campaign authorizes one separate Local owner under existing
[#166](https://github.com/lossyrob/telex/issues/166). The active
`windows-pageheap-av-repair` node promotes the existing AV candidate;
preparation and concrete owner identity are pending the actual canonical
receipt. Existing candidate/unassigned/no-owner authority is historical.
Local reports no duplicate implementation owner. No new issue or record,
second #193 owner, automatic dependency edge or Watcher state change
follows.

Keep the status-hint, concurrent-first-use, destructor stop/drain and
busy-station status-child observations distinct. A shared `0xc0000005`
does not establish one fault location or cause. Diagnose on authorized
hosted disposable Windows infrastructure, preserve logs/symbols/dumps
actually produced, fix the confirmed defect and validate the full required
diagnostics/default workspace/relevant profiles at the final reviewed
head. Rust harness backtraces are observation boundaries, not native
fault stacks; printed `draining=true` is not proof of successful drain.

Local's corrected PR #194 evidence is the THIRD full repetition failing
48/1 after TWO 49/49 passes, not the earlier tail-only second-repetition
report. Its retained 68,029-byte log and source boundary are Local-checked;
the new artifact archive/dump contents remain uninspected. The earlier
report stays historical. #193 / PR #194 and its existing owner remain
separate, subject to strict CI. Ordinary SQLite os33, `ForeignDaemon`,
seed-budget refusal and `Overran` are not AV evidence.

No retry/rerun, flake exception, waiver, skip or abnormal-exit suppression.
Preserve owner-private locks, non-theft, fencing and the production 480ms
budget. Source-reviewer-owned exact-head public receipt, steward design
inspection, full hosted validation and Campaign merge gates apply to the
closing PR, not an extra launch vote. The workstation freeze and accepted
#181/#187 delivery/debts remain; no release, #153, admission or closure
clearance follows. The complete scoped task carries evidence distinctions,
single-owner v2 preparation/launch rules and validation boundaries.

## 2026-10-25 - Actual single #166 owner launched

Local reports existing #166 owner
`d7a17cd2-08c2-4760-b1a6-1209c2d0f80c` launched once in
`lossyrob-special-meme`, observed busy at 19:44:19Z. The requested profile is
`gpt-6.1-sol / high / default / interactive`, without idle notification.
Local's later same-identity/checkout App observation qualifies the current
branch as `lossyrob-windows-heap-av-repair`; the launch branch stays history.
Preparation stays pinned to `f0053e3b`; the observed `df37d568` movement was
three Watcher artifacts only, not source integration or review/CI carry.
The complete original 4,407-byte launch binding and 15,666-byte kickoff
are retained by exact digest; all old pending-owner snapshots remain
historical. No second launch, repair, diagnosis or readiness is claimed.
Later archive-inventory and dependency-candidate reports are not folded in.

The delivered kickoff records a future event-driven obligation: after
the #166 repair actually merges, its owner immediately reports that merge
to Local; Local verifies it and wakes the existing #193 owner to integrate
main, obtain fresh CI and request the PR #194 gate. This is not a completed
merge/wake, an automatic edge/status change or a CI/readiness waiver.
#193/#194, accepted #181/#187, all Watcher state and debts remain separate.

## 2026-10-25 - Campaign heap routing and two pending merge wakes

Campaign `toolu_011HendiW3BJzkXVtPYrCfq6` routes the PR #202 / #198
default-Windows `0xc0000374` observation to the existing #166 owner,
`d7a17cd2-08c2-4760-b1a6-1209c2d0f80c`. Local reports delivery to that
owner. This makes #166 the Campaign's top critical-path repair for
#194 and #202 under strict CI; it creates no owner, node or automatic
dependency/status change. Keep this heap-corruption/first-use observation
distinct from the retained PageHeap `0xc0000005` stages. No common cause
is established; split only if diagnosis establishes independent defects.

The later retained-evidence qualification concerns the SAME run/job,
not another occurrence. PR/event head `02d2cbbd` differs from the executed
pull-merge checkout `0cbecafc`, merging that head into `6c7ff068`.
Local's bounded retained-text/Git/source checks remain Local-attributed,
not reconciler replay. Attempt-1 process-target execution and archive
dump presence remain unestablished. The complete task/ledger preserve
the original unknown-head routing packet, later qualification and limits.

After the #166 product repair actually merges, Local must verify that
merge and separately wake BOTH existing owners: #193 / #194 owner
`519693a5-0607-4aba-a968-345f3b1cd1b5` and #198 / #202 owner
`a96926b1-3e4f-4578-a197-33de51d669c7`. Each integrates main and obtains
fresh exact-head CI/review/PR gates. Both deliveries remain pending.
This supplements, rather than replaces, the original #193 obligation.
An artifact landing, diagnostic green or readiness report is not the
trigger. No retry, waiver, repair, source/CI/readiness/debt/HOLD acceptance
or workstation-freeze exception follows.

## 2026-10-25 - Later #166 identity and diagnostic-draft metadata

Local's separate 19:51:53Z observation resolves both identifiers to the
same canonical owner `d7a17cd2-08c2-4760-b1a6-1209c2d0f80c`.
`57bcb249-2ec6-4272-be67-31b9f0f59d7d` is the observed reply alias
requested by that owner, not another owner or a rebind. The checkout
remains `lossyrob-special-meme`; the observed current branch remains
`lossyrob-windows-heap-av-repair`. Original launch history is unchanged.

Local reports [PR #196](https://github.com/lossyrob/telex/pull/196)
OPEN/draft against main at
`b0f59d408b32d61cc4cfd9c5a5310a8c522567e1`, as of that observation.
This is attributed diagnostic-draft metadata, not a completed repair,
source/design approval, causal finding, CI pass or readiness. This snapshot
supplies no actual repair merge or Local #193 wake.

The later #166 heap-routing qualification and BOTH still-pending
#193/#198 actual-repair-merge wakes remain unchanged. This older
identity/draft snapshot supplies no delivery or current gate credit.

## 2026-10-26 - Separate ForeignDaemon repair #204 already staffed

Campaign authorized a separate tracker and one public-record-only owner
for the Windows InstalledCurrent crash/reattach ForeignDaemon failure,
with launch first and artifact recording afterward. Existing candidate
`local-daemon-windows-installed-current-crash-reattach-foreign-daemon`
is promoted to [#204](https://github.com/lossyrob/telex/issues/204) and
`foreigndaemon-reattach-repair`. The actual owner is
`8afc363c-1af3-447d-a472-336f92d57998`, on its own main-based checkout
and branch `lossyrob-foreigndaemon-reattach-repair`. Local reports one
launch and clean starting head `7e1bfc3a`; App identity confirms the same
session/checkout. The requested Opus 5.5/high/default/interactive profile
and no-idle setting are not independent effective-capacity verification.

The ledger keeps both earlier Windows observations and all candidate/
unassigned source wording as history. One later public PR #196 occurrence
is appended: run `40250813915`, job `121342086267`, event head
`854855f1`, executed merge `1813c432a9a43178b17dc6909f2fe272367b5e8e`
with base `db8eaa75` and that PR head. Failure at line 508 reports
`DaemonBootstrap(ForeignDaemon)`; target 29 passed / 1 failed / 2 ignored,
139.79 seconds, after fixture 6 passed / 0 failed / 1 ignored.
The complete public issue and Local's launch/evidence binding are retained
by hash. Their log/source verification remains Local-attributed; the
reconciler did not replay logs, source, archives or runtime.

The public result does not identify an actual foreign process, failed
authentication branch, native exception or common cause. Later successor-
PID, marker and single-spawn assertions were not reached; configured
delay/recovery counts are not new retry permission. Preserve authenticated
peers, owner-private authority, non-theft, fencing, bounded recovery and
the refusal of genuinely foreign peers. Runtime work stays on authorized
disposable hosted Windows only. No retry/rerun, waiver, flake exception,
budget widening, blanket-success conversion or local runtime permission.

The owner coordinates overlapping daemon/bootstrap/test/CI edits with
`d7a17cd2-08c2-4760-b1a6-1209c2d0f80c` before publishing and does not
write PR #196's branch. d7 remains #166 only. No automatic dependency or
Watcher edge/status change is added. Both #166 actual-product-merge wakes
remain pending and unchanged. Deliver one separately reviewed PR closing
#204 with discriminating evidence, exact-final-head hosted coverage and
the normal Campaign merge gate; launch is not repair, CI or readiness.
