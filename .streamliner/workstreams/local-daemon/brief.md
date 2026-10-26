# Local presence/transport daemon (eliminate the per-session holder)

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

## 2026-10-26 - Operator choice for the existing #166 repair

Campaign confirms the operator's conditional choice of Mio 1.2.4 as the
fix and judges its comparison-job condition met. The public Mio 1.2.4
job `121354231722` in run `40254795271`, attempt 1, at PR #196 event
head `b0ab5f2cf067f08afbb20bd807a89af1e0affb46` succeeded at
`2026-10-26T01:48:20Z`; the overall run failed. d7's report of three
clean 400,000-round configurations remains owner-attributed, not a
reconciler archive/log finding or causal proof for the five Telex cases.
The exact Local packet and direct Campaign confirmation are retained;
no originating ask_user transcript, source, archive or runtime replay
is claimed.

The already-authorized sole writer remains
`d7a17cd2-08c2-4760-b1a6-1209c2d0f80c` on
`lossyrob-windows-heap-av-repair`. Bookkeeping must not delay that work.
Upgrade Mio to 1.2.4 with the Windows normal-dependency floor, validate
cross-platform/features, remove diagnostic-only scaffolding except the
retained capture, and use `Closes #166` in PR #196. Any recurrence reopens
#166. Prior evidence and B/no-push holds remain history; only this
implementation hold is superseded. No five-case common cause, new
occurrence, owner or experiment extension is accepted.

Hosted-only runtime and no retry/waiver/flake exception continue.
Exact-final-head source/design review, reviewer-origin public receipt,
strict green CI and the Campaign merge gate remain. #203 and #204 must
clear as far as they affect #196's required jobs; a green run alone does
not independently accept those repairs. Both pending #193/#198 wakes
still require Local's verification of the actual #166 repair merge.
No graph edge, status change, repair/readiness/merge, debt/HOLD or
workstation-freeze credit follows from this decision recording.

## 2026-10-26 - ForeignDaemon #204 accepted complete with debt

Campaign explicitly accepts the bounded #204 repair as completed with
debt under [gate 6407030627](
https://github.com/lossyrob/telex/pull/205#issuecomment-6407030627),
`MERGE + debt`, high confidence. PR #205 merged as
`faf4f883631c7aaec057dde4beca36daa29d29dd`, tree
`1b754b186622b384bc9adaf3a835be7c6faa4df7`, with ordered parents
`7e91a233` and reviewed `e11c56ce`; #204 closed completed.
The same owner `8afc363c-1af3-447d-a472-336f92d57998` retains follow-through.
The ledger/node are completed; pending main observation and field work
must not keep this accepted repair artificially in progress.

All four debts remain: the missing PR-body consequence/no-discoveries
statements; the contrived same-user squatter's worse diagnostic class
while still refused; unforced `ERROR_INVALID_PARAMETER`/handle-rundown
branches; and pre-fix failure evidence from diagnostic rather than final
head. The owner must add the body statements and post the field report
after main's Windows InstalledCurrent runs. Debt-3 recurrence creates a
new item, not a retry or automatic #204 reopening. No debt is cleared here.

The complete 8,019-byte Local acceptance packet and full 1,930-byte public
gate are retained and projected by hash. Merge/closure/Git geometry/gate
were independently read; pre-merge 21/21 CI and source/design findings
remain Local/gate-attributed, not new product review or main CI proof.
The packet's exact-merge CI/Docs waits share one existing scheduler;
their captured states are historical, with terminal results uncredited.

d7 must integrate main into PR #196, obtain fresh full CI and confirm
its Windows required job clears. That remains follow-through, not #196
readiness or merge authority; #203 remains separate. All prior #204
evidence/launch history, the entire #166 record and both pending #193/#198
wakes stay exact. Only Local-verified actual #166 repair merge triggers
those wakes. No new owner, occurrence, graph edge, runtime/retry, broader
acceptance, debt/HOLD reversal or workstation-freeze clearance follows.

## 2026-10-26 - Credential-writer repair #167 already staffed

Campaign authorized one public-record-only owner for existing
[#167](https://github.com/lossyrob/telex/issues/167), with launch first
and artifact recording afterward. The same
`local-daemon-windows-credential-writer-timeout` record is promoted to
the `credential-writer-timeout-repair` task. The actual owner is
`9a868a31-2c8d-429a-aee3-7e195076a4dd`, coordinated by Local, in checkout
`lossyrob-curly-potato` on current branch
`lossyrob-fix-167-delayed-writer-timeout`. Local reports the actual launch;
the reconciler's read-only App observation confirms the same session,
creator/coordinator, checkout, current branch and observed busy state.
No exact starting checkout head or cleanliness report was received in
this frozen launch snapshot. Public-main `be8b9910` is preflight evidence,
not a verified owner checkout. Requested Opus 5.5/high/default/interactive
settings are not independent effective-capacity proof.

Original Campaign source `toolu_014vkRgy6ysjNiGD88kQeR6L` and direct
confirmation `toolu_01Jw6FHd8GL21QG41ruVtEnr` authorize this separate
repair, not a new tracker or second launch. The complete 5,274-byte Local
launch packet is retained by SHA-256
`ff74705a9428f38b9276eed956ba21cc50ecacbffa24232e26ea0ce887910ad9`.
Only the private machine path is replaced by its checkout name in the
canonical packet projection. All earlier candidate/unassigned fields and
all evidence, including the four existing successor entries, remain exact
history; promotion adds no occurrence. Campaign's "four times" is its
record-based count, not a new independent census.

The owner must diagnose lifecycle, test-budget and hosted-scheduling
possibilities without guessing, preserve delayed inherited-writer output
after shell exit, output/error semantics, effective negative assertions
and owned-process cleanup, and deliver a separate PR using `Closes #167`.
Product/test/daemon runtime is hosted-only. No retry, rerun, cosmetic-head
retry, waiver, flake exception, unbounded experiment or workstation/
installed-state change is authorized.

Coordinate overlapping credential/daemon/CI/dependency edits with d7
before publishing; do not write PR #196's branch or absorb #166's repair.
Use the existing sole external wait coordinator, not another scheduler.
Exact-final-head independent source review with reviewer-origin public
receipt, design inspection as applicable, strict required hosted CI and
Campaign gate remain separate. No autonomous merge.

The Local ledger remains 48 records; one new repair node makes 29 Local
nodes with no dependency edges or edits to existing nodes, attention,
status or design links. All other records, Watcher and both pending
#193/#198 wakes remain exact. Only Local-verified actual #166 repair merge
triggers those wakes. Staffing/recording supplies no cause, source/runtime,
CI/readiness, repair/accepted-delivery, debt/HOLD or freeze credit and does
not delay the already-authorized implementation.

## 2026-10-26 - Accepted #166 completion with debt and both wakes sent

Campaign accepts the bounded #166 repair as completed with retained debt,
following [PR #196](https://github.com/lossyrob/telex/pull/196)'s actual
product merge `8d7815a8d0fb8c6f465d441904044bd20f7dcc7b`, issue closure,
the [merge + debt gate](https://github.com/lossyrob/telex/pull/196#issuecomment-6416830413)
and the owner's [field report](https://github.com/lossyrob/telex/issues/166#issuecomment-6416863991).
Direct Campaign confirmation `toolu_01LZnnNNdNrKTw4NLAmyyBgg` expressly
requires the existing repair node to be completed with retained debt,
following the #204 precedent. The same owner d7 remains recorded. This
is not a new owner, node or causal finding.

The reviewed head is `9b538e4fc2047bf7b2e4940a334cfff673643cc9`;
the product merge's ordered parents are prior artifact main `a0bf2cf0`
then that reviewed head. The merge tree is
`30aac58eb2f76fab2800308fa68a1c22658a2a05`. Its `.streamliner` tree is
unchanged from the first parent. This artifact successor is separate
from the product merge; it does not count that merge as artifact work.
All original observations, owner/launch/authority history and prior
pending-wake snapshots remain intact.

The accepted repair is the operator-authorized Mio 1.2.4 upgrade and
Windows dependency floor for the reached Mio defect. Neither the gate
nor this recording maps it causally to the six retained native
observations. A recurring native crash reopens #166; preserve exact
run/job/artifact and any dump, without retrying past the failure.

All eight gate debts remain **retained and open**: ProcDump availability
and signature-only verification; conditional SDK/symbol downloads;
live-index/resolver-wording dependence in the fail-closed negative test;
text-only capture coverage and the exact-1.2.4 assertion; Windows-only
floor scope; unproved capture for `0xc0000409`/`0xc0000374`; stale
#203/#204 gate wording and omitted isolation-env description; and the
Windows Tokio 1.39.1+ next-release-note requirement. The full gate and
field report are retained; these summaries clear none of the debts.
The gate expressly makes #203 separate, superseding the earlier
precondition without rewriting its history. #204's prior completion
remains unchanged.

After Local independently verified the actual product merge, Campaign
`toolu_01VsA55QAZu6tR6v4erTrbHu` directed both waiting wakes immediately.
Local reports exactly one immediate, no-mode-override send to each
existing recipient: #193 / PR #194 owner `519693a5-0607-4aba-a968-345f3b1cd1b5`
and #198 / PR #202 owner `a96926b1-3e4f-4578-a197-33de51d669c7`.
The App accepted both. These are sent/admitted dispositions only, not
owner execution, main integration, CI success, readiness or merge
authority. Earlier `delivered:false` snapshots remain history; no wake
is resent by recording.

Each awakened owner integrates main and obtains fresh exact-head hosted
CI/review and its own gate through the existing sole wait coordinator.
Watcher's intervening delay proposal was not adopted; its later report
that Campaign upheld the original wake remains attributed. No duplicate,
revocation, PR #202 failure or conditional later wake is inferred.

Main CI `40328433964` and Docs `40328433981` at `8d7815a8` are registered
follow-through for Local and the sole coordinator; no terminal result or
debt clearance is accepted here. Campaign's later confirmation reports
#207 passed its merits gate but still awaits main integration and fresh
CI; this records no #203/#207 or #167 acceptance. Counts stay 48 Local
records and 29 nodes, with no edge, other status, owner, runtime or
workstation-freeze change. Recording does not delay either awakened owner.

## 2026-10-26: Same-head CI reruns and Sentinel investigation

Campaign `toolu_01BAMphx5UxVR6KEHAWxh5Ud` records the operator's
09:25 ET directive: "Reruns are fine." The prior #167 and #203
no-retry/flake-exception clauses are superseded for same-head CI reruns.
Record each failure's run, job and test on its existing tracker or
discovered-work record before rerunning failed jobs at the unchanged head.
A green rerun at that exact head satisfies CI; merging red is still
forbidden. Repeated failure of the same test is a finding, not noise.
Do not waive, skip or weaken a required check or manufacture a new head.
This changes neither local-runtime restrictions nor product retry behavior.

Existing test sentinel `fc772c8f-2a38-40de-a868-69a5397dbbc5` now owns the
cross-family Windows flake/environment investigation and approach decision;
R15 is amended for that purpose. #167's B/C decision and #175/LockFileEx-33
staffing are deferred to its recommendation. #167 remains held with no
merge; the two candidates stay unstaffed. This records an assignment, not
a new session launch, implementation owner, recommendation or result.

Campaign's `74721a829f1dda93d1d2acc869a60fa2266c45f1` is an owner-guide
and merge-gate process-document commit in the private planning repository,
not a telex main commit or product merge. Only this directive/rule summary
is projected; private planning-document content stays private.

All earlier failures and policy statements remain history, not current
no-rerun authority. A green rerun is not a causal fix, repair acceptance
or debt clearance. Existing repair owners, statuses, attention, graph
geometry and all evidence remain unchanged. No CI result, #166 debt
clearance/causality, #167/#203/#207 acceptance, new wake, release,
hardening or freeze credit follows. Owners, reruns and Sentinel do not
wait on this recording.

## Main 8d7815a8: one store-lock occurrence and later green CI/Docs

Local intake `call_1UttL409xWlB8RRLaWbch40q` records the ordinary Windows Build job
[121591124574](https://github.com/lossyrob/telex/actions/runs/40328433964/job/121591124574)
at exact main `8d7815a8d0fb8c6f465d441904044bd20f7dcc7b`. Retain exactly one
attempt-1 occurrence on `local-daemon-windows-station-restart-lockfileex-33`.
The test is
`station_intent_a_partial_multi_store_drain_reports_each_store_and_restores_only_the_provable_one`.
The CLI exited 1 with an explicit SQLite store-lock / LockFileEx os-error-33
refusal, not an outer timeout. The post-drain side-station attach, source anchors,
failure segment and diagnostics are Local-attributed. The prior not-running
check does not establish kernel rundown, lock release or a holder. The 61.18
seconds is target duration, not attach latency.

The complete original 6,077-byte packet is retained with its original
pending-attempt-2 state. The later Local supplement
`call_32zBsiFzX792fUpGLID5Z6Ny` forwards Campaign's terminal direction
`toolu_01UPPDy9BrhDdh9Pbx9yhMyA`; that Campaign report remains Local-attributed.
The later terminal supplement's inference of 21 fresh executions is retained
as history but explicitly superseded by Local correction
`call_Ikpc7KuKK6HUqUKU7vt2bvCN` and direct Campaign direction
`toolu_01MWLGRfT4D1iSaxC2faBPhi`. Independent bounded timestamp and full-step
receipt comparisons, without logs or runtime replay, confirm:

- Run 40328433964, attempt 2, succeeded at unchanged `8d7815a8`: 21 successful
  aggregate entries represent **one re-executed Windows Build** (121598826679)
  and **20 reused prior successes**. Those 20 pairs have identical job name,
  head, start/end, status, conclusion and every step receipt. New IDs and
  `run_attempt=2` alone do not establish fresh execution.
- The original attempt-1 job 121591124574 remains one retained failure.
- Docs run 40328433981 succeeded at the same head, attempt 1, with both actual
  attempt-1 jobs 121591123899 and 121593338791.

Only Git blob identity was checked for the source anchors. No product source,
raw log, archive, dump, native runtime, scheduler or recipient was replayed.
Earlier same-test evidence with different head/job/source anchors remains exact.
This is not #193 intent admission, #204 ForeignDaemon or a reported native
#166 recurrence. No common cause or causal repair is inferred from green CI.

#166 remains completed under the same owner. Its full prior completion and
pending-CI snapshots, all eight open debts and both already-sent #193/#198 wakes
remain exact. Neither wake is resent or reinterpreted as downstream execution.
The prior candidate/proposed fields stay exact in this evidence-only pre-promotion
predecessor; they do not renew a current staffing hold. Local reports later
authorized #212 staffing, recorded separately and neither blocked nor revoked
here. Its owner does not wait for this recording. No lock theft/removal,
retry-budget weakening, local runtime, new owner/tracker/node/edge, debt
clearance, downstream acceptance, release or workstation-freeze credit follows.
Recording does not delay owners, authorized reruns or the Sentinel.

## Accepted #193 completion with six retained debts and hosted follow-through

Campaign direction `toolu_01MWLGRfT4D1iSaxC2faBPhi` accepts [#193](https://github.com/lossyrob/telex/issues/193) completed after [PR #194](https://github.com/lossyrob/telex/pull/194) merged as `b90eacc26f2dcbad90352d58ece25dddd4e9fde2` from reviewed head `6567bb497802f872680195e10249383488f7c98e`. The existing node is completed/watch; owner `519693a5-0607-4aba-a968-345f3b1cd1b5` is unchanged. All six Campaign debts remain retained under that owner. There is no field-report hold. Prior launch, preparation, source scope and evidence remain exact history, not a renewed pending assignment or implementation instruction.

The first Campaign gate held only the body: a negated closing-keyword sentence would also close #208, and the Deferred section was missing. Clearance verified GitHub parsed closing references exactly `[193]`, an unchanged source head and the added section, while expressly carrying debts 1-6. Debt 4 below is the original historical wording, not a claim that the merged body still lacks a section. The owner field reports the body floor resolved; this reconciler does not infer a separate retirement of the carried debt record.

Independent public metadata verifies PR CI 40338051684 attempt 1 at `6567bb49` and merged-main CI 40343448362 attempt 1 at the exact `b90eacc2` merge, each with all 21 jobs successful. Complete frozen and live job identity/timing/ordered-step receipts match. This is bounded hosted metadata, not source/log/per-test/artifact replay or proof of historical cause. Owner `call_t3nKy7rMpMdCev7VFJNXUz31` reports delivery complete, consequence low, design impact updated-docs, sole wait done and scheduler absent. Those source/per-test/artifact/process conclusions remain attributed; no scheduler or recipient was inspected. No rerun or local runtime is triggered here.

**#208 is NOT repaired and remains open.** Its Live-fixture setup and the broader #209 routing remain separate from #193. The field report below predates Campaign's later #209 completion; that later completion and its five retained debts are separate intake, not pilot-stage-2 authority or broader fixture acceptance. #212 staffing, the seven-native historical packet and the two distinct non-native candidates are not absorbed here. #166 completion, all eight debts and both already-sent wakes remain exact. No new owner, launch, wake, graph edge, native reopening or debt clearance is granted.

### Original Campaign body-only hold and six-debt gate

[Original Campaign body-only hold and six-debt gate](https://github.com/lossyrob/telex/pull/194#issuecomment-6418988704), raw 2674 bytes / SHA-256 `20381682e7815f2bc5504ebcd08a704f1e7ff791501602aea0bba8c840bcf1b0`. Retained verbatim.

```text
## Campaign merge gate: changes-required (body only); merits merge + debt (high)

HOLD at `6567bb497802f872680195e10249383488f7c98e`. No fork, no hard stop: the change is a test seam plus refusal diagnostics, and production admission policy is unchanged.

**Blocking (floor)**
- The body sentence "Closing #193 through this PR does not close #208." contains the GitHub closing keyword `close #208`. `closingIssuesReferences` currently returns [193, 208], so merging would close #208, which this PR explicitly does not repair.
- Fix: reword it without a closing keyword before #208, e.g. "#208 stays open after this PR merges". Also add a Deferred work section.
- A body edit does not move the head. Campaign will re-check that `closingIssuesReferences == [193]` and merge at this head without gating the merits again.

**Floor checks**
- **CI:** run 40338051684, attempt 1, at the exact head: 21/21 jobs succeeded.
  - The target test passed on hosted Windows (station job 121624833423, default workspace 121624833354), macOS and Ubuntu.
  - The 4 new lock tests ran on all 3 OSes.
- **Source review at the head:** issuecomment-6418317623, by an independent reviewer session; it names 6567bb49.
- **Design:** issuecomment-6418511627, at the head.
- **PR body and state:** Closes #193; consequence low; #208 listed under Discoveries as NOT repaired and recorded in discovered-work.json; 0 threads; MERGEABLE/CLEAN.
- **Main drift:** since d8a382a4, only `.streamliner/` files changed. No conflicts with #206, #211 or #213.
- **ci.yml:** one additive step (the exact proof-write refusal test, on 3 OSes); all existing steps are retained.

**Process debt:** none from local runs. The owner's local checks were compile-only, and no credit was claimed. Separately, the #208 closing keyword passed the owner's, reviewer's, steward's and Local's text checks because none of them checked GitHub's parsed closing references.

**Debt (non-blocking)**
1. The historical cause is still unknown. The diagnostics only help with future failures, and one green attempt is not proof. Watch main Windows CI.
2. The same family of production-clock setup remains at `daemon.rs:8656` and `:8876`, and in copilot.rs and station_intent tests. Route it to #208/#209.
3. Registration's proof-commit admission still uses production timing. A stall there could yield the same typed refusal the test expects.
4. The body has no Deferred work section.
5. Refusal text nit: "1 contended attempts".
6. The new native tests leak their temp roots on panic.

**Deferred:** #208 (scoped-scan Live-fixture seed, not repaired) and #209 (the cross-family CI approach). The #193 field report is due after main CI.
```

### Campaign clearance with all six debts retained

[Campaign clearance with all six debts retained](https://github.com/lossyrob/telex/pull/194#issuecomment-6419049801), raw 304 bytes / SHA-256 `10dd8bebd607ca8382df0f40eaadaa3a90443b715db851ca52cf051ac80a1f1d`. Retained verbatim.

```text
Campaign: the hold is cleared. The body edit is verified: closingIssuesReferences == [193], the head is unchanged at 6567bb49, and there is a Deferred work section. Main drift is still .streamliner only. Merging at the exact head; the merits verdict and debts 1-6 from issuecomment-6418988704 carry over.
```

### Owner field report

[Owner field report](https://github.com/lossyrob/telex/issues/193#issuecomment-6419556610), raw 5873 bytes / SHA-256 `95d307c7241bf91be34c5ac9ca1fbcc7bcdc62d82c7584141547d0fe842004d0`. Display normalizes CRLF to LF: 5839 bytes / SHA-256 `b309b9abef2b7868addbf74cdd59d54af07fae3f701b25921105781542c389de`; raw capture remains exact.

```text
## Field report: merged repair and hosted main validation

[PR #194](https://github.com/lossyrob/telex/pull/194) merged at `b90eacc26f2dcbad90352d58ece25dddd4e9fde2`, tree `914277a2134f3382f30470c9aeeaad2d4388dfb5`. [Main push CI 40343448362, attempt 1](https://github.com/lossyrob/telex/actions/runs/40343448362) completed successfully at that exact merge: all 21 actual jobs passed. This is fresh merged-main evidence, separate from the earlier PR-head run.

The sole CI coordinator retained the complete terminal receipt, including Windows default workspace job `121643973933`, heap diagnostics `121643973926`, all three station-intent boundaries (`121643973764`, `121643973812`, `121643973821`) and all eight Windows token-user profiles. The station-intent boundary includes the exact pending-seed/proof-write-refusal target and independent native/budget coverage. No job was omitted or waived, and no rerun was used. The observation is done; no active waits remain and the scheduler was cleared and verified absent.

Retained hosted artifacts: [windows-heap-diagnostics, 12660592540](https://api.github.com/repos/lossyrob/telex/actions/artifacts/12660592540/zip), 64,875,533 bytes; [released-v021-upgrade-proof, 12660297001](https://api.github.com/repos/lossyrob/telex/actions/artifacts/12660297001/zip), 14,377,479 bytes. No artifact download or independent dump inspection is claimed.

## What changed and what held

Pending-proof fixture setup now has an explicit test-only fixed acquisition clock while retaining owner-private filesystem validation, real native locking and shared generation-safe pending persistence. Setup reloads the real manifest and checks generation 1, Pending state and absence of armed proof before registration. Deliberate corruption, typed refusal and the no-member assertion remain.

Production retains its real monotonic 480ms budget, 25-attempt cap, WouldBlock-only retries, capped sleeps, persistent locks, non-theft, generation fencing and late-guard disposal. Refusal diagnostics now identify the acquisition branch; a late guard is explicitly released before refusal. Independent native and real-clock coverage remains separate from fixture setup.

Consequence: low. Production admission policy and durable format are unchanged. This successful main run supports the bounded fixture outcome; it does not prove the historical cause or eliminate unrelated fixture/timing failures. No local tests, runtime proofs or wider live soak were performed.

## Campaign debts 1-6 and successor disposition

The following preserves every debt in [Campaign's merge gate](https://github.com/lossyrob/telex/pull/194#issuecomment-6418988704):

1. **Historical cause unknown; retained debt.** The original combined error did not identify contention, expiry, native-call latency or scheduling cause. New diagnostics apply to future observations only. The required post-merge Windows observation passed here, but one green attempt establishes neither a cause nor future reliability. Cross-family evidence remains with [#209](https://github.com/lossyrob/telex/issues/209).
2. **Other production-clock setup sites remain.** Campaign identified `src\daemon.rs:8656` and `:8876`, plus cases in `src\commands\copilot.rs` and station-intent tests (locations at the reviewed source). This repair did not change those setup paths. Scoped-scan Live-fixture work remains with [#208](https://github.com/lossyrob/telex/issues/208); the broader fixture inventory and bounded hosted approach remain with [#209](https://github.com/lossyrob/telex/issues/209). Do not automatically generalize this test seam or widen production admission.
3. **Proof-commit admission timing remains a possible assertion confounder.** Registration still uses production timing. A refusal there could satisfy the same typed-refusal assertion without demonstrating the intended corruption branch. This is retained coverage debt for the #209 investigation, not a claimed diagnosis of either historical failure and not repaired here.
4. **Missing Deferred work section was resolved before merge.** The PR body now lists the separate trackers, remaining setup sites and this post-main-CI field report. Campaign also caught an unintended GitHub closing reference in a negated sentence. The body was reworded and GitHub's parsed `closingIssuesReferences` was verified as exactly `[193]`, without moving the accepted source head. Successors should inspect parsed closing references rather than infer them from prose.
5. **Diagnostic wording nit remains.** A singular count can produce `1 contended attempts`. It is recorded non-blocking wording debt in the gate and Deferred work section; no follow-up code change was included.
6. **Panic-path native-test temporary roots remain.** The new native tests can leave their temporary roots after panic. This remains recorded cleanup debt in the gate and Deferred work section. It was not repaired or locally exercised/cleaned during this item.

**#208 is NOT repaired by PR194 and stays open.** Its scoped-scan fixture seeds Live records through `write_atomic` and production-clock locking, outside the pending-proof helper. Its original failed attempt is retained despite a green same-head rerun; shared refusal text does not establish a common cause. The repair in this issue must not be treated as resolving that separate finding.

## Design impact and completion

Design impact: **updated-docs**. The daemon design now explains the setup-versus-admission coverage boundary; the production lock contract is unchanged.

The requested hosted merged-main validation and this field report complete the bounded #193 delivery. #193 remains completed; #208 and #209 retain their separate unresolved work. Reopen #193 only if evidence shows its stated outcome failing, rather than for those distinct successor findings.

```

Only this existing Local node and record receive the completion projection; the brief is append-only. All 48 Local records, 29 Local nodes, 15 Watcher records and 22 Watcher nodes retain their geometry and identities. Other records, owner bindings, workstreams, design links, pending/completed evidence and debt/wake histories remain unchanged. Recording does not delay authorized owners.


## #224 planned node before owner, diagnosis and validation boundaries (2026-10-26)

Campaign `toolu_01JR27dM8djhYUzuMCGXmgYN` accepts the existing `local-daemon-postgres-fallback-candidate-fetch-exhaustion` family and tracker [#224](https://github.com/lossyrob/telex/issues/224) as planned/watch node `postgres-fallback-recovery-exhaustion`. Later `toolu_012dMoYnEa5LdQd2wUwFehr1` makes this the NEXT publication, ahead of #223 with no edge. Local verifies the canonical node before launching exactly one owner; this packet contains no launch, owner assignment, requested/effective profile, diagnosis, product acceptance or readiness. The earlier candidate intake remains source history, not an invented previous canonical row. Discovery source stays null. All four ledgers were searched across every item field before adding this one record.

The exact two controller originals are preserved privately at 6,296 bytes / `147fe912a4d8acd4e4ebc707d2940e80ec45bb888365ecf07ff8d607f40e06e9` and 5,879 bytes / `f216aad74d8688698a9c4f44a3e43b7a0ebce90e6c14f2fd5da68ff59261bc5f`. Their complete public object projections are retained in the new row: the first removes only the private run-directory field and replaces its one occurrence in Campaign wording with a marker; an object-level inverse restores the original. The second object is unchanged. No raw directory or recipient reference was followed. All original Campaign scope and prohibitions remain, not a claim that any diagnostic operation ran.

### Initial Campaign authority (only the private directory redacted)

```text
#224 is routed as you proposed: a separate Local node, postgres-fallback-recovery-exhaustion, at top priority within Local, because it blocks the #201 real-root criterion.

Sequence:
1. The reconciler publishes the node.
2. You launch its owner. No further Campaign approval is needed.
3. The owner diagnoses first, against the filed outcome and your stated boundaries, and returns either an evidence-only disposition for my decision or a closing PR with its own exact-head and hosted gates.

The steward adjudicates the #201 to #224 edge. My position for that adjudication is that #201's root confirm-and-ring criterion cannot pass while #224 is open, and I am not narrowing that criterion.

Evidence for the owner (I am the owner of these root runs). All three were on v0.3.0-gate201-9e971fd9, root-201, against pg-rde-telex. Each exited 7 backend-unavailable with "backend recovery grace expired while fetching wait candidates ... backend query or reconnect attempt timed out" after roughly 28 to 58s:
- 1793042572542-c8903866107167a1
- 1793042640067-5700450788a59922
- 1793042940764-4b4f25eb407601ab, at 19:29:59Z. This was a single re-arm prompted by a turn guard.

Run directories are under [private run directory withheld]. `status` on the same backend succeeds every time. wait-coordinator and gate201-owner fail identically. Root stays attached and unattended, and I am making no further retries until #224 reports.
```

### Later exact Campaign diagnosis/validation and next-publication authority

```text
#224 routing decision, given that the steward has made #201 depend on #224 (6425382009).

Sequence: node first, then owner. Ask the reconciler to publish postgres-fallback-recovery-exhaustion now, as the next publication. Launch the owner as soon as the node is on canonical main. #224 goes ahead of #223, with no edge between them.

Diagnosis authority:
- Read-only work against pg-rde-telex is allowed: status, schema and catalog reads, EXPLAIN without ANALYZE on the candidate-fetch query, connection and latency timing, and server-side activity views.
- Isolated reproduction is allowed, using a separate TELEX_HOME/TELEX_CONFIG and a disposable local Postgres seeded to production-like history size.
- Reading the existing run directories and the custody packet is allowed.
- Not allowed: real `telex wait` retries on campaign addresses, timeout, auth, PIM or install changes, ACL restores, detector-event staging, and writes to production rows.

Validation authority:
- Path (a), a product or guidance fix: a closing PR with its own exact-head review, design and hosted gates, which I will gate.
- Path (b), environmental closure: allowed only after diagnosis gives specific evidence that the cause was transient or environmental and has cleared. The owner then asks the steward to approve exactly one supported same-profile wait that must outlast the failure window, and runs it only with that approval. I will not authorize that real wait myself.

Either way, the disposition comes to me. After it is accepted, I re-arm root once and report station_health for #201.
```

Read-only real-profile status/catalog/schema, EXPLAIN without ANALYZE, connection/latency timing and activity views are diagnosis authority for the future assigned owner. Isolated reproduction uses separate TELEX_HOME/TELEX_CONFIG, disposable local Postgres and temporary LOCALAPPDATA. Real campaign-address waits, timeout/auth/PIM/install changes, ACL restoration, event staging and production-row writes remain prohibited. No I161-1 real-ACL restore/reset is allowed. Environmental closure requires specific diagnosis/clearance first and then the steward, not Campaign, approves exactly one sustained same-profile wait. Product/guidance work needs its own exact-head source/design/hosted gates and Campaign disposition. Local runs do not replace hosted gate evidence.

Campaign root runs 1793042572542-c8903866107167a1, 1793042640067-5700450788a59922, 1793042940764-4b4f25eb407601ab are source-reported and distinct from the two member-owner runs and other coordinator run in the sanitized custody below. Raw status/exit/binary hashes, installation/runtime identities and timing/health claims remain actor-attributed; the reconciler did not inspect or rehash raw runs. Prepare/status timestamps are not process durations, no numeric recovery-grace value was emitted, and before/armed owner station_health is missing. Successful status reads do not prove candidate-fetch health. Typed exit 7 is exhaustion reporting, not itself a regression, auth/PIM, environment/root-cause or fleet finding.

### Original Watcher routing snapshot, before the conditional dependency triggered

https://github.com/lossyrob/telex/issues/224#issuecomment-6425323208

Raw 1228 bytes / SHA-256 `63be896a50ed9f775f9f23f78ef0c1a4aa5587dc399c41ceb162972628ca8434`; LF display 1227 bytes / SHA-256 `9aefabef8464155e714d8bd4b0cfcebb828c53080062b055f684ebe120a9b4d0`.

```text
Design steward decision: this routes to the Local Daemon workstream, not the Watcher geometry.

Why: pull fallback wait lifecycle, backend recovery grace, Postgres candidate-fetch and reconnect timeouts, and station health reporting are Local Daemon surfaces. The closest prior work is that workstream's completed `postgres-wait-reset-recovery` (#155). Whether this was a transient environment observation or a lifecycle defect is for Local Daemon to decide. The expected outcome stands as filed.

Watcher dependency: none for now. #201 continues under 6425143128 condition 7. This changes into a modeled upstream dependency only if both hold:
1. the supported re-arm also fails, or later fallbacks keep failing, so that the Campaign root cannot receive the real detector event; and
2. no other supported delivery path for the root is available within the #201 conditions.
In that case #201 gains an external dependency on the Local Daemon node that closes this issue, and #201 waits. The gate is not narrowed. The #201 owner brings that evidence to me, and I will make the change.

A transient failure followed by a successful re-arm does not create a dependency. #201's report records it as observed, with this issue linked.

```

### Later formal #224 upstream condition

https://github.com/lossyrob/telex/issues/224#issuecomment-6425382766

Raw 818 bytes / SHA-256 `d422f70857b52456689a88e104e359257b389494c545c18e182fdf10eeb39ace`; LF display 817 bytes / SHA-256 `b04c3ba37b6d461bd3d8b5dd4393a8fd697400323ea845ad8373ec560691d59c`.

```text
Design steward note: the conditional Watcher dependency in 6425323208 has triggered. #201 (`stall-detection-workstream-gate`) now depends on this issue's resolution and waits for it: https://github.com/lossyrob/telex/issues/201 (decision comment linked there). Both root pull waits and both owner-member waits exited 7/backend-unavailable, the root has no supported alternative (#223), and status queries kept working.

Satisfaction from Watcher's side: Local Daemon closes this issue in one of two ways.
- An accepted resolution, merged with exact-head hosted validation and Campaign acceptance.
- Closed as environmental, with evidence that a supported pull wait on the same Entra Postgres profile stays armed past the observed failure window.
Prioritization within Local Daemon remains that workstream's decision.

```

### Watcher-owned #201 dependency decision

https://github.com/lossyrob/telex/issues/201#issuecomment-6425382009

Raw 2071 bytes / SHA-256 `81280e282f91625ac2b129b458ab76ce370e416cc3d185fcae2301c6c658f5ba`; LF display 2070 bytes / SHA-256 `50ae04df2cb599eb53ed4acf055b19f114291b30a8228c775188b161695f1623`.

```text
Design steward decision: #201 now depends on #224, as provided in https://github.com/lossyrob/telex/issues/224#issuecomment-6425323208.

Trigger, both conditions met:
1. Supported pull waits are failing. The Campaign root's first wait and its single re-arm both exited 7/backend-unavailable, as did the owner member's wait and its re-arm, and a previously armed Watcher member wait expired the same way. The live root shows pull, unattended, 0 waiters, while status queries succeed.
2. The root has no other supported delivery path: the App cannot load the bridge (#223), and 6425143128 accepted the pull fallback as the only route.
No real detector event can reach the root, so the root cannot confirm and ring. The fourth pass criterion is unprovable.

Change:
- `stall-detection-workstream-gate` gains an external dependency on the Local Daemon work that closes #224 (Postgres pull-fallback recovery), status pending. Local Daemon has not yet modeled a node for #224. The reconciler records the dependency against #224 and binds it to that node's id once Local assigns one.
- #201 stays unpassed and is not narrowed. Its pass criteria and my conditions 6424158582 and 6425143128 are unchanged.

The dependency is satisfied when either:
- Local Daemon delivers an accepted #224 resolution: merged with exact-head hosted validation and Campaign acceptance; or
- Local Daemon closes #224 as environmental, with evidence that a supported pull wait on the same Entra Postgres profile stays armed past the earlier failure window.
Then #201 resumes with a fresh supported prepare/arm for the root before any real event is staged.

Meanwhile:
- No timeout, auth, install or check changes.
- No further retries.
- Do not stage a real detector event.
- The background-work case stays held.
- The installed runtime, registry and branch are retained under the operator's existing dogfood authorization. Retention needs no new approval. Any change to them does.
- Automated isolated-root coverage on the branch may continue. It does not substitute for the real root confirm/ring.

```

### Sanitized member evidence and custody, not raw verification

https://github.com/lossyrob/telex/issues/224#issuecomment-6425409170

Raw 3769 bytes / SHA-256 `0b4e5147effd3ea5ae7875d3a19c5cc2eeed44240d43830babb64a4aea1440c6`; LF display 3745 bytes / SHA-256 `e71c48195babbfa130f6697c9e1418b61ea814b57a5f40ca24b889156eb5717a`.

```text
Bounded #201 evidence/custody packet for Local

Source identity: main 9e971fd9ee877a20d13c97ad59b25d477b5c6ad0; installed Telex 0.3.0 with Entra support. Installed Telex SHA-256 fe3d09c80afe0ba294d9f77936af6df280f196e615d1194b48b374e23b7d761c. Running Watcher SHA-256 343b9d9509d72cb28dacd9be0f9fe3ad51822d079e8e44b5bd07fa624b24521a.

Original run: 1793042657859-83154eafc09756e3. Generated prepare identifier begins at 1793042657859 ms; status written_at_ms 1793042689524. exit.code is 7; status.exit_code is 7; outcome backend-unavailable. SHA-256 of original raw status: fe86bb98d25501033d675cd16c61c546b552e4487863a8469e7ab41734c49cae.

Single supported re-arm: 1793042722283-01fcd5d2b4bc7687. Prepare identifier begins at 1793042722283 ms; status written_at_ms 1793042760337. exit.code is 7; status.exit_code is 7; outcome backend-unavailable. SHA-256 of original raw status: c3ce26f269093add6ce54ea6ed2c5c71394d1067cd446804177cf1047f039f07.

Both raw exit.code files have SHA-256 10159baf262b43a92d95db59dae1f72c645127301661e0a3ce4e38b295a97c58.

Exact diagnostic structure, with only the address redacted:
"backend recovery grace expired while fetching wait candidates for [member address]: backend query or reconnect attempt timed out"

Both prepares used idle timeout 1800000 ms. The failure artifacts emit no numeric recovery-grace value. No duration is inferred from the prepare timestamp to represent process runtime. No timeout, recovery, auth/PIM, daemon, install or polling changes were made for the re-arm. No third attempt is running or proposed.

Before launch: preparation returned mode pull-fallback and distinct generated run/artifact/launcher identities. No owner before/armed station_health snapshot was captured; preparation is not evidence of sustained attendance. After the original run: delivery_mode pull, station_health unattended, live_waiters_count 0, pending_unconsumed_count 0; last waiter outcome backend-unavailable/7. The re-arm also ended backend-unavailable/7, not a message delivery. Status queries continued to work.

Independent campaign receipts, not owner-captured raw evidence: root initial run 1793042572542-c8903866107167a1 and one re-arm 1793042640067-5700450788a59922 both ended backend-unavailable/7, last root exit_at_ms 1793042688793. Wait coordinator run 1793042546147-82d8d1bea621074e ended the same way at 1793042688234; its owner later detached its unattended address. Both reported pull/unattended/zero live waiters. This establishes repeated observations across these three addresses, not fleet-wide prevalence, auth/PIM cause, or a #155 regression.

No actual detector event was deliberately staged or received for this request. The real stall watch's public events endpoint was queried before hold and returned an empty array. Its ready/idle attempts are monitoring evidence, not root-delivery evidence. Background-shell, root confirm/ring and reply proof remain held. Separate isolated synthetic tests emitted and stored their synthetic events; those are not real campaign events.

Custody: original generated per-run exit/status artifacts remain in their original private directories; copies of each owner status and the before/after status observation are retained in #201's private session artifacts. Shared copies contain no credentials, tokens, user paths or backend connection details. No access to the installation or raw files is needed to consume this packet.

The steward has now explicitly modeled #224 upstream of #201: https://github.com/lossyrob/telex/issues/201#issuecomment-6425382009 . Gate is unpassed. Resume requires accepted resolution or environmental closure with evidence a supported wait stays armed, then a fresh root prepare/arm before staging a real event.

```

The owning #201 hold report is https://github.com/lossyrob/telex/issues/201#issuecomment-6425427162. Its same owner, runtime/registry/branch and validation-only checkpoint remain source reports; no PR or completion is claimed. Watcher owns the separate blocked-status/external-edge projection. The #201 criterion is not narrowed, and this Local packet does not modify its graph or credit any validation.

Only this one new row, one planned node/task, a root-summary prefix retaining the full old suffix, and this brief append change. All 48 old Local rows and 29 old nodes, null discovery provenance, dependency/checkpoint/presentation/design state, #166 eight debts/two sent wakes and all other workstreams remain exact. #155 remains completed/related only. #223 and #222 stay separate. Parked promote212v2 and every receipt remain immutable historical evidence; no technical/role/final authority carries to a successor base. No extra tracker, failure experiment, observer, wake, root re-arm or debt clearance follows.


## #223 planned capability-based delivery path, no launch or wake proof (2026-10-26)

Campaign `toolu_01Y5o2oxjb7MD82B7hCRAj5r`, quoted in actual Local `call_kUHJSXbVmnRG8PJlpwsuplKL`, accepts existing [#223](https://github.com/lossyrob/telex/issues/223) and family `local-daemon-copilot-app-first-time-delivery-path` as planned/watch node `copilot-app-delivery-path`. Local verifies canonical publication before launching one owner without another Campaign approval. No implementation owner or requested/effective profile is recorded. Initial candidate intake remains historical private reporting, not a fictitious previous canonical row; sourceNode remains null. #224 published first; no dependency edge between them is inferred.

### Exact Campaign decision in Local custody

```text
Routing decision for #223: accepted as proposed. It is a separate node, copilot-app-delivery-path, owned by the Local workstream, with the scope and boundaries you posted in 6425315392. It has no dependency on #201 or #222. Have the reconciler publish the node, then launch its owner yourself; no further Campaign approval is needed. Note for the owner: the real root cannot currently arm the pull fallback, because `telex wait` times out fetching wait candidates on pg-rde-telex. The #201 owner is filing that separately. #223 fixes the docs and recovery text only; it must not claim fallback wake works and must not absorb that bug.
```

The capability distinction is explicit: an App session without a supported live bridge loader gets the existing Telex-owned fallback; loader-capable and already-live push is not forced to pull. Provisioning is not loading proof. Preserve the supported one-wait procedure, completion artifacts, ack/dedup, re-arming and stop/mode-transition behavior. No custom shell polling, new transport or invented host API. Local source/guidance reads and App capability/fallback observations remain attributed; this artifact projection replayed no product source, runtime or recipient state.

### Watcher routes to Local, no Watcher dependency

https://github.com/lossyrob/telex/issues/223#issuecomment-6425261961

Raw 881 bytes / SHA-256 `e001e67d741596207da1c10abd3d926f7eb3cf66a9785fc443949c3e923e29ad`; LF display 880 bytes / SHA-256 `5f1fdfedef777735ef422fd4de32f9a8a1a75f2cc4b6cf7343673a4d56bc81c2`.

```text
Design steward decision: this does not join the Watcher geometry. It routes to the Local Daemon workstream.

Why: first-time Copilot bridge loading (`telex copilot attach --copilot-bridge`, then `extensions_reload`) and the push/pull delivery contract belong to Local Daemon's `push-delivery` design (docs/design/copilot-bridge-push.md, guides/copilot-push.md). Whether App-hosted sessions get a supported first-time bridge path, or the guide states pull fallback as the App path, is that workstream's decision.

Watcher dependencies: none. #201 uses the supported pull fallback (6425143128). The Watcher stall guide naming push as the only root delivery mode is Watcher's own wording; the fix moves into #222's outcome, item 3, and does not wait on this issue.

The Watcher coordinator hands this off to Local Daemon routing. The outcome text in this issue body stands as filed.

```

### Full original Local routing decision; last pending-Campaign paragraph is now history

https://github.com/lossyrob/telex/issues/223#issuecomment-6425315392

Raw 2635 bytes / SHA-256 `32dd52e68c6f6ea26235606a07bbdfc7bd2945e512616fd0109050ac8a68f59f`; LF display 2616 bytes / SHA-256 `e98550e718903aaae68671266749605557b9f6e4a7793541f192cfd58ebf2305`.

```text
Local routing decision: document the supported pull fallback as the first-time delivery path for App-hosted sessions that do not expose a supported live bridge loader.

This keeps the issue's expected outcome as filed. It does not promise a new App extension-loading API or treat bridge provisioning as evidence that a bridge loaded. The decision is capability-based: a host with the supported reload capability can use bridge push; an already-loaded, live bridge is not forced into pull merely because its host is the App.

The current source at 9e971fd9 shows the guidance mismatch:
- docs/guide/src/guides/copilot-push.md opens with a CLI-and-App push promise and asks unavailable-tool users to enable /experimental and retry extensions_reload.
- copilot/COPILOT.md:25-27 and :69-78 makes the same assumption; its supported fallback procedure already exists.
- docs/design/copilot-bridge-push.md describes extensions_reload as available to every agent.
- src/commands/copilot.rs:31 emits the same recovery advice, and its skill contract tests currently require that recovery sequence.

The bounded repair should align the binary-owned skill, operator guide, bridge design/overview and directly related recovery text/tests with actual host capabilities. For an App session without a loader, direct it to Telex-owned `copilot fallback prepare` and exactly one supported pull-mode wait, with completion artifacts, acknowledgment/deduplication, explicit re-arming and stop/mode-transition semantics preserved. Do not prescribe CLI-only /experimental or a nonexistent reload tool as an App requirement. Do not invent a shell polling loop or a new bridge transport.

This is guidance and recovery-message alignment, not proof of App bridge push. Source checks cannot establish that the generated launcher wakes a real App session. #201 retains its separately authorized real-root pull-fallback validation and must report that mode and its actual evidence; bridge push remains untested there. A fallback wake/delivery failure is a separate product failure, not something this wording can waive.

Ownership and sequencing:
- Local Daemon owns this issue under its push-delivery contract.
- No Watcher dependency is added. #201 continues using the steward-approved fallback.
- #222 separately owns the Watcher stall guide's push-only prerequisite wording and installation/bootstrap outcome. Do not duplicate that change here.
- Implementation staffing and any new workstream node are pending Campaign routing. This decision launches no worker, runtime experiment, installation change or observer, and does not close #223.

```

Campaign node acceptance above supersedes the original pending-staffing paragraph prospectively; the old public body is retained exactly, not edited. #201 remains governed by its own #224 upstream condition; this task creates no #201/#222 dependency or gate waiver. #224 product/environment diagnosis stays separate and this wording cannot claim fallback wake works. Existing push-delivery design is normative ownership, not discovery provenance or #53 reopening.

Open-PR plus named-section overlap coordination is required with #222 for copilot/COPILOT.md: #222 owns stall-group installation/root-mode wording; #223 owns host-capability/first-load/fallback/recovery text. No duplicate repair or silent overlap. Local tests follow current isolated TELEX_* and temporary LOCALAPPDATA policy; no real ACL/permission change or I161-1 restore/reset, no reinstated blanket freeze and no replacement of exact-head hosted gates.

Only this new row/node/task, root-summary prefix with full old suffix and append-only brief change. All 49 old rows/30 old nodes and every other workstream/dependency/checkpoint/design/presentation/debt/wake remain exact. No owner launch, source/runtime/recipient proof, broader family intake/repair/acceptance, observer/wake or debt clearance follows.


## #223 actual owner and later review-phase qualification (2026-10-26)

Local reports the existing owner `a78ad6b1-1f7d-4936-85d7-6cf660022386` was launched at `2026-10-26T20:17:58.585Z` after its verified publication prerequisite. The requested mode was interactive; model, reasoning and context overrides were null, with no base-branch override or idle notification. The complete supplied controller is retained in this same ledger row, changing only its private App-observation path to checkout name `lossyrob-bookish-train`. Restoring that one value recovers the complete original object. This is the provided controller/assignment report, not a raw create_session response or complete kickoff payload. Busy for three seconds and no awaiting input are the controller temporal snapshot, not continuing activity or effective capacity. No recipient checkout, HEAD, cleanliness, model, runtime, referenced private path or actual delivery was inspected.

The later actual Local report changes this same node and row from planned to in-progress; attention stays watch, with no new node, row, tracker, occurrence or worker. PR #226 was open and non-draft at captured head `274c1195db078a70b6fc5f546d29b351c6b8a935`, with parsed closing reference exactly [223]. Its public branch is `lossyrob-223-app-delivery-guidance`; the launch controller remains unchanged. No rename mechanism/time or current owner checkout state is inferred.

The historical P2 review at `0bc480eae8bcc22c7d7d7d56e46bc19c2d2a09a6` remains history: https://github.com/lossyrob/telex/pull/226#issuecomment-6426679183. The later reviewer-authored source verdict at https://github.com/lossyrob/telex/pull/226#issuecomment-6426763756 resolves that P2 at the new head, retaining its prior ten-file inspection and verified three-file delta; this is not source approval at an unreviewed head or merge authorization. Local reports CI 40397079900 attempt 1 succeeded with 21 unique jobs for candidate/event head 274c1195. The observer terminal https://github.com/lossyrob/telex/pull/226#issuecomment-6427256198 separately identifies Windows provider/checkout `37cc92d6e903376a180e52106611471eef4e0608`, tree `632f2ad3123b6035b29c92b93d43803b8d83c7c7`, parents `331c4da1ed9970771e3d7213d13dc2ed068d31e7` and `274c1195db078a70b6fc5f546d29b351c6b8a935`. Bounded public Git metadata was captured, not execution. Actual checkout/execution remains observer-log-attributed through Local. Docs was not scheduled for this PR and is not passed; the workflow-source assessment and scheduler-clearance report remain Local-attributed. No CI run/job/log or product source was inspected by this reconciler.

Formal design inspection was requested under its separately reported exact tuple; no design PASS/provider receipt or Campaign gate existed at the later report. Reported owner 21 unit and three guidance checks remain attributed support, not actual App wake/bridge loading or #201 acceptance. No merge, completion, operator readiness, #224 repair, debt clearance, five-minute hold release or operational action follows. #222 / PR #228 named-section coordination remains mandatory and separate; no new dependency, scope absorption or review of #228 is inferred. The owner work is not held on this recording. #224, #201, parked #212 and other queued residuals remain separate.

The initial planned/unassigned text above is the preserved prelaunch snapshot, not a denial of the later owner. Actual launch intake `call_zru2oYv7FN9jbHBzw8pTWUSC`, origin `eea3f9c9-c4e8-4ee6-8342-3c2f01e60187`; later Local source `call_WSt05WJGuvIa4JQyLPAIIV9t`, origin `3bea7132-7608-4bd5-9e53-71920e1a24fe`. Complete later source body follows verbatim; compressed source spacing is quoted, not newly authored operative prose.

```text
Actual later same-owner #223 phase, freshly checked now: a78ad6b1-1f7d-4936-85d7-6cf660022386 owns OPEN/non-draft PR226 https://github.com/lossyrob/telex/pull/226 at head274c1195db078a70b6fc5f546d29b351c6b8a935/tree02e053f5c271d34a4a9fdee75e788325fd67e4f0, recordedbasea3a788063b558927bfe914c50613d421a50a60af; parsedclosingrefs[223]. Mainfc962a6ab7d12f40b004b4eed6b5d18700292a34. This is implementation/review in progress, not merge/readiness/Campaign acceptance or issue completion.

Actual source sequence: reviewer77597d21-e23e-4b66-ab48-02418854e927 AUTHORED historicalreceipt6426679183at0bc480ea withP2missingexistingexit6/7guidance/contracts; ownercorrectedskill/guide/tests inone+56line3filecommit274c1195. Reviewer-authoredfinal https://github.com/lossyrob/telex/pull/226#issuecomment-6426763756 resolvesP2/no source blockers atnewhead, coveringprior10fileinspection+verifieddelta;Localreadfullreceipt/identity,notcode/runtimereplay.

Hosted: CI40397079900attempt1pull_request candidate/EVENT274c1195 completedSUCCESS/21uniquejobs independentlyverifiedbyLocal. Observer-authoredterminal https://github.com/lossyrob/telex/pull/226#issuecomment-6427256198 retainsfullinventoryandWindowsprovider/CHECKOUT37cc92d6e903376a180e52106611471eef4e0608;Localverifiedthatcommittree632f2ad3123b6035b29c92b93d43803b8d83c7c7/parents331c4da1ed9970771e3d7213d13dc2ed068d31e7+274c1195,executionclaimobserverlog-attributed. DocsPRN/A/notscheduled/notpassed:exactdocs.ymlnoPRtrigger,pushmain+dispatchonly/Pagesdeploy;Localreadsource,no dispatch/newwatch. Scheduler clearedperobserver.

Formaldesign-pr226-274c1195-20261026requestedbyLocal withcanonicalmainfc962a6a/designba29e6b7b99d5c3990f56bf7a5ee1ed3a84dc3d3/currentbridge394b0e7edc4ae8af61979d08b453cf2d9f4d0989/proposed9165f24edfb625b55555bedf04aadeb30eb3a946. Stewardcall_NMLH1LNIPeueueR8mCnwqFED independentlyagreesprereqs21:03:42Zandislaunchingonefreshreadonlyinspector;NOdesignPASS/providerreceipt orCampaigngateyet. Ownerlocal21unit+3guidancepassesareattributedsupportonly,notrealAppwake/bridge-load/#201proof. #222sharedsectioncoordinationandPR228remainseparate;onlyscopeoverlapreport,notreviewof228. Preservecompleteoriginallaunch/prelaunch/historicalP2andalllimitsadditively;no oldheadcarry/newworker/debtclearance oroperationalhold.
```


### Later posted formal design qualification

After the earlier report, actual Local `call_0F9MTGeJqsX6Ws8PTJWu8kah`, origin `b0ec9aa8-aedf-4edd-9fe1-78d52ba25123`, reports the same-owner, same-head formal inspection posted and verified at https://github.com/lossyrob/telex/pull/226#issuecomment-6427438907. The complete 4,085-byte ASCII public body has SHA-256 `81f6a245ac5ff3bfef25b823a0a2e1a6265b871ebd7c46977e09a4730189e5b9`. Its raw bytes remain exact; the following display normalizes CRLF to LF only. This supersedes only the earlier pending-design phase. Source review, observer CI and distinct provider/checkout identity remain separate; Docs PR N/A is not passed. The public receipt asks for a stale PR-body status refresh; Local reports a final design-link refresh before future readiness routing, not that either operation or Campaign acceptance has occurred. No merge, completion, readiness, runtime/App wake, #201/#224 release, debt credit, second worker or hold on existing work follows. Complete public receipt:

```text
Design Inspection: +1

Initial design inspection for local-daemon / copilot-app-delivery-path, closing #223. Correlation: `design-pr226-274c1195-20261026`. The Local Daemon design steward adjudicated this result and authorized this one receipt.

Exact binding:
- Head: `274c1195db078a70b6fc5f546d29b351c6b8a935`; tree: `02e053f5c271d34a4a9fdee75e788325fd67e4f0`.
- Base: `a3a788063b558927bfe914c50613d421a50a60af`.
- Canonical design revision: `fc962a6ab7d12f40b004b4eed6b5d18700292a34`; `current-design.md` blob: `ba29e6b7b99d5c3990f56bf7a5ee1ed3a84dc3d3`.
- Canonical `docs/design/copilot-bridge-push.md` blob: `394b0e7edc4ae8af61979d08b453cf2d9f4d0989`; proposed blob: `9165f24edfb625b55555bedf04aadeb30eb3a946`.

**No design blockers.** The complete #223 guidance outcome matches the accepted capability-based routing. Already-live push remains supported, including in App; hosts with a supported loader can load the bridge; first-time App setup without one uses existing Telex-owned prepared pull. Provisioning and registration are not loading proof, and restart/clear do not substitute for an unavailable loader.

The single-shot detached launcher contract preserves completion artifacts and exit.code-first interpretation, exits 0-7 and legacy exit 1, quarantine metadata/no-ack/conditional continuation, recovery before re-arming backend failure, message-id acknowledgment/deduplication/disposition and the CC exception, explicit re-arm/stop, intentional --force downgrade, and push/pull exclusion. See the [binary-owned skill](https://github.com/lossyrob/telex/blob/274c1195db078a70b6fc5f546d29b351c6b8a935/copilot/COPILOT.md#L198-L318) and [design boundary](https://github.com/lossyrob/telex/blob/274c1195db078a70b6fc5f546d29b351c6b8a935/docs/design/copilot-bridge-push.md#L39-L65).

Proposed durable documentation promotion is adequate across the design of record, skill, guide, bridge overview, README, troubleshooting and recovery messages. No daemon protocol, ADR or canonical current-design rewrite is needed for this bounded repair. No new API, transport, polling loop or readiness mechanism is introduced. #222 retains Watcher bootstrap/stall wording; #224 retains backend repair; #201 retains real-root delivery acceptance.

Prerequisites are separately evidenced by the [reviewer-authored source receipt](https://github.com/lossyrob/telex/pull/226#issuecomment-6426763756) and [observer-owned hosted receipt](https://github.com/lossyrob/telex/pull/226#issuecomment-6427256198). The parent steward independently verified CI run `40397079900`, attempt 1, pull_request event/API candidate head `274c1195db078a70b6fc5f546d29b351c6b8a935`, and all 21 successful jobs. The observer's Windows provider/checkout SHA is the distinct synthetic merge `37cc92d6e903376a180e52106611471eef4e0608`, with ordered parents `331c4da1ed9970771e3d7213d13dc2ed068d31e7` and `274c1195db078a70b6fc5f546d29b351c6b8a935`. I verified that commit geometry; execution attribution remains observer-log-derived, not my or the parent's log replay. Candidate and checkout SHAs are not equated; aggregate CI is not per-test, causal or App-runtime proof.

PR Docs is N/A, not passed: exact-head `.github/workflows/docs.yml` blob `ef2945ac216695eabde74a1d324f78efea0602d4` has only path-filtered push to main and workflow_dispatch, not pull_request. Any applicable post-merge Docs run is separate.

The owner should refresh the PR body's stale source/CI status before readiness. This separately routed evidence bookkeeping is not a source/design blocker; this receipt does not claim the body was updated.

Confidence is high for source/design alignment only. This was read-only exact-source design inspection, not another implementation review. I did not rerun the owner's 21 unit/3 guidance checks, execute or poll CI, replay logs, perform runtime/installation/auth/ACL actions, inspect recipients, or merge. No real App wakeup, bridge loading, #224 repair or #201 acceptance is certified. This receipt grants no merge authority. Any head/base or relevant canonical-design change invalidates coverage.
```


### Subsequent accepted product merge and pending follow-through

Actual Campaign `toolu_01SzupWWGaZeqoNRuZgTMCPb` reports PR #226 merged as `f4c58eed5fe9336408750d69d59ae446707c9f88`, closing #223. Local `call_v7l5SEzgZfdIqX5Bbx07zR65` independently reports ordered parents `fc962a6ab7d12f40b004b4eed6b5d18700292a34` + `274c1195db078a70b6fc5f546d29b351c6b8a935`, tree `28dc7120912323257f6ab07d6255c2ea447c63c1`, and public issue closed/completed at 21:20:42Z. The bounded public/metadata capture confirms that merge and the unchanged complete `.streamliner` tree; no product source was inspected. This new product boundary invalidated the unpublished old-base tuple; both earlier owner packets remain frozen, unreviewed, exactly reversed history with no authority carry. A clean fast-forward adopted the accepted merge without creating a merge commit or pushing.

The same owner still owes exact-main CI, any applicable main Docs and a #223 field report at this source. Public issue closure does not automatically complete this node: lifecycle remains in-progress, attention watch. Earlier review/design/CI receipts remain their exact premerge identities, not new-main execution proof. PR Docs N/A is not a main Docs pass. No post-merge completion, runtime/App wake, bridge loading, #224 repair, #201 acceptance or debt credit follows.

The merge gate separately preserves a nonblocking operator report that startup extension discovery may load session-scoped App extensions even without advertised reload/manage tools. Only if the Campaign root restart test actually confirms a live bridge should an evidence-backed restart/resume/first-time wording correction be filed. This is conditional source history, not observed behavior, a new row/family, an experiment, root action or current runtime authorization.

Complete Campaign gate (raw body retained exactly; this display changes CRLF to LF only):

```text
## Campaign merge gate: PR #226 (Closes #223) - MERGE

Head 274c1195db078a70b6fc5f546d29b351c6b8a935 (tree 02e053f5). PR base a3a78806; current main fc962a6a.

**Floor**
- OPEN, non-draft, MERGEABLE/CLEAN; closing refs exactly [#223]; 0 unresolved review threads.
- Hosted CI: run 40397079900 attempt 1, pull_request, head 274c1195, completed/success, 21/21 jobs. Synthetic merge parents 331c4da1 + 274c.
- Final-main drift: 331c4da1..fc962a6a touches only .streamliner/workstreams/telex-watcher/{brief.md,discovered-work.json,graph.json}. Disjoint from the 10 PR files; disjoint-drift exception applies, no refresh.
- Docs: PR Docs not applicable (docs.yml has no pull_request trigger). Main Docs is post-merge follow-through, not credited as passed.
- Receipts: source review 6426763756 (no blockers), design 6427438907 (+1, no blockers), observer 6427256198.

**Merits**
Capability-based delivery guidance replaces the CLI-only /experimental assumption for App sessions; keeps live and loader-capable push; adds exit 6/7 fallback handling and states that docs/contract tests prove neither App wake nor bridge loading. No transport/API/runtime change claimed.

**Follow-up (non-blocking, recorded):** the operator reports that in Copilot App, startup extension discovery still loads session-scoped extensions even though extensions_reload/extensions_manage are not advertised. If the Campaign root restart test confirms the bridge goes live that way, the sentence "Restarting or clearing a session is not a substitute for an unavailable loader" and the App first-time routing to pull fallback need a follow-up correction naming restart/resume startup discovery as an App first-time path. To be filed with evidence, not assumed here.

**Post-merge obligations (owner a78ad6b1):** exact-main CI and Docs at the merge commit, field report on #223. No #224 repair, #201 acceptance, or App wake/bridge-load proof is credited by this merge.
```


## #224 actual owner, implementation phase and additive identity correction (2026-10-26)

Local reports the existing owner `769a992c-7319-4d2e-b93a-4977094d7715` launched at `2026-10-26T19:59:29.847Z` after verified canonical publication `51a94f8ef0126e74f89f0f4b7fcd6201d718b793`. The complete supplied controller is retained in the same ledger row, replacing only its private App-observation path with checkout name `lossyrob-crispy-giggle`; restoring that one value recovers the full original object. Interactive was requested, with null model/reasoning/context overrides, no base override and no idle notification. Local-reported busy for fourteen seconds/no input is a temporal observation, not current activity, effective capacity or current owner checkout HEAD/cleanliness. The supplied controller is not a raw create_session result or full kickoff payload. No recipient/private referenced path, runtime or actual delivery was inspected here.

The same node and row become in-progress, attention watch unchanged. Local reports Campaign `toolu_01QLkaykjbzhsK7TeNYSpRyS` accepted the evidence-only diagnosis and selected path (a): one closing #224 PR fixing shared-client contention with phase-specific exhaustion diagnostics, retaining the 3000 ms grace. This is a Local-attributed Campaign phase decision, not independently captured raw Campaign authority. No second family, row, node, tracker, occurrence or implementation worker is created. The complete original planned/unassigned snapshot and all existing authority/history remain retained; recording does not hold the owner work.

The diagnosis packet remains reported as 17,464 bytes / `10cc8a269de279a2f749a2a02050412cdcee2b276363a2ca90e23843e39d48db`. Sufficient isolated contention and a distinct slow-credential alternative do not establish the historical production cause or calibrated production replay. Failed setup assertions and partial-run limits remain evidence, not erased by later progress. The 12,669-byte cleanup command/timestamp packet / `b7c7b70995efec266947ca3ba07c396cf5bf2929c5b823f7658199f5062c3029` does not independently establish external-command outcomes: actual Docker stop and resource absence remain owner-attested. This reconciler read no diagnosis, manifest, nineteen-file set, command transcript, experiment or production evidence. Local reports its byte-custody checks; no experiment/production/cleanup replay is inferred.

The original diagnosis owner_session `0a11050e-77b7-4b4d-8eed-f8ac937e9957` stays unchanged. Local reports independently querying both IDs and receiving canonical/active owner `769a992c-7319-4d2e-b93a-4977094d7715`, same checkout/current branch and Local creator. The workspace alias is not another owner. The complete separately provided 1,239-byte correction / `0dcf579c8c847156506d4e3db142a104bfab2c593edda6c46249cbbfaa922795` is retained unchanged in the ledger; its complete object follows. Local reports the unchanged 3,391-byte manifest with SHA-256 prefix `b6c15760` and all nineteen file hashes matching; the prefix is not promoted to a full hash and no referenced file or App lookup was read/repeated here.

```json
{
  "kind": "additive-owner-identity-binding",
  "original_packet": "issue224-diagnosis-disposition.json",
  "original_bytes": 17464,
  "original_sha256": "10cc8a269de279a2f749a2a02050412cdcee2b276363a2ca90e23843e39d48db",
  "original_packet_and_manifest_preserved": true,
  "canonical_app_owner_session": "769a992c-7319-4d2e-b93a-4977094d7715",
  "original_owner_session_value": "0a11050e-77b7-4b4d-8eed-f8ac937e9957",
  "binding_evidence": "The initial workspace envelope supplied 0a11050e as project_session_id. On 2026-10-26, get_session invoked independently with 0a11050e and with 769a992c returned the same canonical id/active_session_id 769a992c, project d53b5ea1-97fc-4e7c-a48c-8c2dbee13d54, original checkout path, current branch and Local creator. Thus the original field used a resolvable workspace alias rather than the canonical App session ID; it is not evidence of a second owner.",
  "cleanup_provenance": "Cleanup was observed by this owner in tool output. The retained Docker transcript records commands/timestamps but is not claimed to contain every external-command receipt. Cleanup remains owner-attested for an independent packet reader. No restart or experiment was performed to reconstruct receipts."
}
```

Environmental closure and #201 root re-arm remain blocked until BOTH PR merge and installed-build upgrade. Neither a successful experiment nor this record grants upgrade permission, environmental clearance, supported-wait authorization, root retry/re-arm, #201 validation or runtime repair. #223 guidance, #222 bootstrap, parked #212 and all other families/debts/wakes stay separate; #155 remains completed related history, not a proved regression or reopened node.

Actual sources: launch `call_snSu89ZCyTsGxy7QTsNISKt3`; phase `call_v9EqKx99uvyHMavvBRiF5rFq`; additive correction `call_QNYGcQ0UNjFkBw8WJnFiZ9sy`. Full captured phase source follows verbatim; compressed spacing is quoted source, not newly authored operative prose.

```text
Same-node #224 phase decision, NOT a shape change: Campaign toolu_01QLkaykjbzhsK7TeNYSpRyS accepts owner769a992c's evidence-only diagnosis and chooses path(a): one bounded closingPR fixing shared-client contention plusphase-specificexhaustiondiagnostics, unchanged3000msgrace. Environmentalclosureand201rootrearmremainblockeduntilPRmergeANDinstalledbuildupgrade. No installed-upgradepermission or outcomecredit follows;samefamily/node/owner.

Ownerfinaldiagnosispacket17464/SHA10cc8a269de279a2f749a2a02050412cdcee2b276363a2ca90e23843e39d48db readbyLocal;mechanismis sufficientisolatedcontention+separateslowcredentialalternatives,NOThistoricalcauseorcalibratedproductionreplay. I requested additiveidentitycorrectionbecausepacketowner_session0a11050e differsAppowner769a992c;do notsilentlybindit/overwriteoriginalorcopyprivatepaths. Cleanupcommand/timestampspacket12669/SHAb7c7b70995efec266947ca3ba07c396cf5bf2929c5b823f7658199f5062c3029 bindsrecordedcommands,whileactualDockerstop/resourceabsencearemereownerattestationinLocalreview. Keepfailingsetupassertions/partialrunlimits. Existingactual-ownerqualificationcanrecordthisnewphaseadditivelywhenin scope;no duplicateowner/shape/publicationhold.
```


## #223 bounded guidance follow-through accepted (2026-10-26)

Campaign `toolu_01RtX3dmYB4DhkWdLrKE9wqf` accepts the same owner a78ad6b1-1f7d-4936-85d7-6cf660022386 follow-through: [field report 6428012639](https://github.com/lossyrob/telex/issues/223#issuecomment-6428012639), main CI `40403556376` and Docs `40403556481` succeeded at exact product merge `f4c58eed5fe9336408750d69d59ae446707c9f88`. This later acceptance completes the bounded #223 guidance node/row, attention watch unchanged, without reopening it or replacing its launch/review/design/merge/pending history. The owner and complete actualOwnerQualification remain exact; the original brief/task text stays complete.

Complete public field body is 3734 bytes / `b2b3defb2ee80b333664dce9a92ad3ad0ee9de02b1c80ad77b0951292bb2a8dc`. Public field readback and run metadata independently match exact push/attempt 1/completed/success. CI21/21 and Docs build/Pages inventories, Windows event/provider/checkout equality and observer terminal/scheduler/support-test claims remain owner/observer-attributed. No job/log/source/runtime/recipient or scheduler was replayed. Earlier PR Docs N/A is not rewritten as passed; later exact-main Docs is separate evidence.

No real App fallback wake or bridge loading, #224 repair, #201 acceptance, installed migration, backend/transport repair or debt credit follows. The complete field retains its conditional startup-discovery follow-up at its source; later Campaign root observations and #230 routing are separate qualifications, not a denial/rewrite of that field or scope absorbed by #223. No new worker, observer, rerun/dispatch, wait/root test/rearm, installation/auth/ACL operation, cleanup/environmental clearance, five-minute/195 hold release or successor/parked authority is granted here.

### Complete field report (CRLF-to-LF display only)

```text
## Field report: bounded guidance repair and merged-head follow-through

[PR #226](https://github.com/lossyrob/telex/pull/226) merged at `f4c58eed5fe9336408750d69d59ae446707c9f88` on 2026-10-26 at 21:20:40Z; this issue closed at 21:20:42Z. The merge tree is `28dc7120912323257f6ab07d6255c2ea447c63c1`, with ordered parents `fc962a6ab7d12f40b004b4eed6b5d18700292a34` and reviewed candidate `274c1195db078a70b6fc5f546d29b351c6b8a935`.

The bounded change aligns the binary-owned skill, operator/bridge documentation, README, troubleshooting and recovery messages with host capabilities. It preserves live push and the existing prepared single-shot pull procedure, including completion artifacts, exits 0-7, acknowledgment/deduplication/disposition, explicit re-arming/stopping and mode exclusion. No transport, host API, backend repair or polling loop was introduced. The Watcher stall subsection and its mirrored guide remain unchanged for #222.

### Exact-merge checks

The existing observer reported terminal success, and I independently read back the run identities and complete job conclusions through GitHub:

| Workflow | Exact merge | Attempt | Result |
|---|---|---|---|
| [CI 40403556376](https://github.com/lossyrob/telex/actions/runs/40403556376) | `f4c58eed5fe9336408750d69d59ae446707c9f88` | 1 | push; completed/success; 21/21 jobs successful |
| [Docs 40403556481](https://github.com/lossyrob/telex/actions/runs/40403556481) | `f4c58eed5fe9336408750d69d59ae446707c9f88` | 1 | push; completed/success; build and Pages deployment both successful |

The observer's Windows Build/test execution-context read records event, provider and checkout SHA all equal to this actual merge. That execution attribution is observer-log-derived, not my independent log replay or an every-job checkout claim. This post-merge evidence is distinct from the earlier candidate CI, whose Windows checkout was a generated PR merge ref. PR Docs was not scheduled; the actual main Docs result above supplies the post-merge follow-through.

The observer reports all waits terminal and its scheduler cleared. No rerun, manual dispatch or local runtime mutation was performed for this follow-through.

### Review and limits

- [Independent source review](https://github.com/lossyrob/telex/pull/226#issuecomment-6426763756): no blockers at the reviewed candidate, with the exit 6/7 documentation gap resolved.
- [Formal design inspection](https://github.com/lossyrob/telex/pull/226#issuecomment-6427438907): +1, no design blockers.
- [Campaign merge gate](https://github.com/lossyrob/telex/pull/226#issuecomment-6427601103): authorized the actual merge with disjoint final-main drift and retained follow-up.

Local support checks passed 21 unit and three guidance contracts using isolated roots. Those checks, hosted CI and this report establish neither real App fallback wakeup nor bridge loading. They do not repair #224 or satisfy #201 real-root delivery acceptance. No real installation, campaign waiter, bridge reload, detector, backend probe or real-user ACL change was part of this work.

The gate's non-blocking startup-discovery follow-up remains explicit: the operator reports that App startup discovery can load session-scoped extensions even without an advertised live reload tool. Campaign owns the root restart test and evidence-backed filing. If confirmed, the restart wording and App first-time routing need a separate correction. This report does not claim that experiment ran or infer its result, and no speculative correction was added here.

The assigned exact-merge CI/Docs follow-through is complete. Campaign's bounded delivery acceptance and the separately owned runtime evidence remain distinct.

```

### Complete actual Campaign acceptance

```text
#223 follow-through accepted. Owner a78ad6b1 finished the field report (6428012639), and main CI (40403556376) and Docs (40403556481) passed at f4c58eed. Nothing in this credits #224, #201, or App wake/bridge loading.
```
