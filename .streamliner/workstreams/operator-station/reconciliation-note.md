# Operator Station — Reconciliation

## What changed

Wave 1 began as a deliberately temporary Windows vertical spike and merged as
PR #104. The product loop held, but plan review changed the live Station from
read-only inbox polling to an application-owned wait/read/ingest/ack courier so
the experiment proved honest attendance and consumption. Review also hardened
route-back recovery, disposition ordering, source identity, restart projection,
notification evidence, and the final repository layout.

The merged node proved implementation viability. The builder subsequently
passed `viability-gate` after guided dogfood exercised human escalation, routine
resolution, clarification, route-back, restart continuity, and notification
publication. The builder also chose to use the campaign orchestrator as the
live `attention:rob` mediator during normal workstream execution.

`station-contract` subsequently completed in PR #116. The accepted
`docs/design/operator-station.md`, ADR 0047, and ADR 0048 now promote the
mediated loop into intended product design. The corrected Operator Station
AC-01 through AC-15 export and merged-source addendum are published on issue
#12.

Contract-convergence issue #118 completed through clean product-only PR #126,
merged at `62c2b23cc3d54877226f46df44d6036b7dffa380`. Polluted PR #123 was closed
without merge and preserved for protocol forensics.

The campaign-owned `application-client-ready` gate remains pending because
issue #12 still carries the pre-repair crosswalk path and manifest identity.
Campaign/operator authority must reconcile and accept the clean checkpoint
before it is exported to Operator Station.

Campaign staged execution subsequently promoted `operator-broker` as issue
#128. It may proceed from the accepted semantic contract because it packages an
agent-session role and routing policy, not the non-agent shared client.
`station-app` remains held. A proposed dependency change from the semantic gate
to Application Client `client-conformance` is recorded for review but not
applied.

Streamliner launch-preparation run
`1aec0d88-bdb2-41e8-b69f-acee8e6b47bc` created the dedicated
`feature/operator-broker-128` worktree and launched both the v2 implementer and
reviewer sessions through the broker. The implementer Telex station was
initially attended by the temporary preparation session rather than the launched
worker; orchestration detached that stale registration after the preparation
session ended. Both launched sessions remained live and graph-bound, but neither
registered its assigned Telex control station during the bounded startup wait.
That launch was treated as degraded, and the launch-profile/broker interaction
was escalated for campaign review.

Campaign ordered a fresh corrected-profile relaunch. Run
`686fc37a-43b8-496c-a086-29fc0f094527` created replacement sessions, and the
reviewer reached Telex. A later profile correction established that all active
v2 implementers must use `Artifact Lifecycle: never-commit`; the preserved
WorkflowContext still said `commit-and-clean`, with PAW-only commits `50dbe86`
and `5cff47f` already on the branch. Both replacements were stopped before
product implementation, their claims and stations were cleared, and the
worktree was preserved unchanged. The node is now blocked on an authorized
lifecycle repair.

Campaign authorized a forensic branch rename and clean derived-worktree repair.
The polluted branch remains local as `forensic/paw-init-operator-broker-128`.
Preparation-only run `6aa58815-6ca1-441a-b8df-6b763b034481` proved the clean
worktree starts at `origin/main`, uses `Artifact Lifecycle: never-commit`, tracks
no `.paw` files, creates no workflow commit, and carries `--yolo`. Final paired
launch run `24cbafa6-8708-4c12-8f96-4a495682418a` started both sessions. The
implementer reached `attended_push` but has an unconsumed scope correction; the
reviewer is live but waiting for input before Telex attachment. Readiness
therefore remains blocked without rewriting the current WorkflowContext or
stopping current work.

Campaign authorized an in-place reviewer repair. The exact reviewer window
showed its bridge had attached to local SQLite rather than `pg-rde-telex`.
Orchestration detached only the local station and re-provisioned the same
session/address on PostgreSQL. Reviewer session
`a1b159a8-4b9e-469b-99d8-baa725d394ec` is now `attended_push` with zero backlog
and handled correction `1144`. Implementer correction `1143` remains queued
behind a long-running validation turn, which was preserved rather than
interrupted.

Campaign subsequently classified that state as normal Telex behavior:
`working-with-queued-control-message`, not a readiness failure. The node returned
to `in-progress`; orchestration will not stop, interrupt, or relaunch the
implementer solely because deferred correction `1143` awaits the next natural
turn boundary.

The implementer handled correction `1143` at the next checkpoint and opened PR
#130 at `313a5b4b76e21b984ff5c9abb4951a227129148b`. All CI axes are green and
the PR is mergeable. Review ownership remains with the paired reviewer; the
workstream orchestrator records state and boundary pressure rather than
duplicating the code review.

Review 4812665371 posted two must-fix and two non-blocking findings. The blocking
items align the packaged role with already accepted Operator Station semantics:
required `outcomeType` on disposition-only routed outcomes and exact,
cross-session-reproducible operation-ID derivation bytes. They do not reopen the
workstream boundary or shared Application Client contract.

The re-review passed with the verified automated +1 marker at
`961e51e5a7d8da4a4867b2ae01efe75af47476b3`; CI is green and no blocker remains.
GitHub thread state still showed all four discussions unresolved, including one
non-outdated thread, so orchestration treated thread resolution as separate
merge-floor evidence and requested implementer reconciliation before campaign
merge authorization.

The implementer resolved all four threads without changing the reviewed head.
Final revalidation found head
`961e51e5a7d8da4a4867b2ae01efe75af47476b3`, green CI, mergeable status, the
verified current-head automated +1, and zero unresolved threads. The node is
merge-ready; merge authority remains outside the worker and reviewer sessions.

## Boundaries

- **Held:** The Station remained a separate optional application; filtering
  stayed in the operator agent; Telex core did not gain human UI or semantic
  routing; Windows/SQLite remained the spike boundary; raw and mediated threads
  stayed distinct.
- **Leaked:** Moving the report and Station late in review left authoritative
  task paths stale until the workstream orchestrator reconciled them.
- **Expanded:** Evidence grew to include Action Center publication, operator
  absence, route-back recovery, and unresolved-history stress beyond 1,000 newer
  messages. These strengthened the same viability question rather than creating
  production scope.

## Contracts and exports

The durable exports are the experimental Station and assignment under
`spike/operator-station/`, plus the evidence and Application Client requirements
in `docs/notes/operator-loop-spike-report.md`.

The accepted domain export is `docs/design/operator-station.md`, backed by ADR
0047/0048. The subprocess courier, full-history export, store fingerprint,
experimental namespace, current UI behavior, `attention.*` kinds, and
`campaignAttention` metadata remain explicitly outside the production contract.
Issue #12 remains the shared-client convergence authority.

## Context fitness

The original node boundary and engagement points were useful. Dual plan review
caught two misleading assumptions before implementation: attach plus inbox
polling was not healthy attendance, and inbox limits did not retrieve the newest
N actionable obligations. The mandatory live-demo review also caught evidence
that did not support its claims before the PR advanced.

The node spec should have named the intended final repository layout earlier.
That would have prevented the late shared-artifact path reconciliation.

For the contract wave, explicit campaign ADR allocation and exact-byte #12
draft approval were necessary because Watcher and Operator edited the same
design index, decision log, and shared-client tracker concurrently. The final
paired review also showed that replied-to findings and resolved GitHub threads
are separate merge-floor evidence.

The first Application Client PR exposed a context-authority failure: a worker
treated workstream gates, approval evidence, and workflow recovery as product
branch deliverables. The clean replacement proved that node missions must stay
separate from orchestration state, with PAW and approval evidence kept
off-branch unless the product explicitly requires it.

## Attention allocation

Operator attention was highest-leverage at plan review, the first live demo,
the domain-contract review, the exact #12 export review, and the
workstream-owned artifact reconciliations. The paired reviewer remained the
right owner for detailed lifecycle, provenance, recovery, and safety defects.

Operator attention belongs at the `operator-broker` output review while
campaign/operator authority resolves the clean issue #12 checkpoint and the
proposed `station-app` dependency. The later usability gate still owns combined
Station/broker acceptance.

## Inspired vs. recovery interventions

- **Inspired:** Treating delivery/ack health and operator-agent occupancy as
  first-class Station state came from using the real loop and improved the
  product evidence.
- **Inspired:** Distinct raw and mediated threads made provenance and route-back
  behavior clearer than a single conversation model.
- **Recovery:** Replacing inbox polling with a real delivery consumer corrected
  a planning mistake that would have produced false viability evidence.
- **Recovery:** Late layout changes required an orchestrator-owned update to the
  authoritative node spec before merge.
- **Recovery:** Review found route-back/ack/disposition ordering gaps that the
  initial implementation evidence did not expose.
- **Inspired:** The cross-contract consistency pass preserved one coherent
  Watcher/Operator design layer and made the shared-client overlap explicit.
- **Recovery:** Fixed ADR assumptions and a stale #12 export required dynamic
  allocation, exact-byte approval, Class D correction, and publication
  verification before merge.
- **Recovery:** Paired review found per-recipient delivery identity,
  terminal route-back, restart-safe metadata operations, unresolved-work
  handoff, and inert-rendering gaps in the initial contract.
- **Recovery:** PR #123 mixed worker implementation with workstream gates,
  ledgers, and approval artifacts. Clean PR #126 restored a product-only change;
  future nodes must use the Streamliner launch broker and route new gates or
  workflow rewinds back to orchestration.
- **Recovery:** The first #128 broker launches inherited `commit-and-clean` PAW
  metadata and created workflow-only commits before product work. Campaign
  corrected the profile to `never-commit`; orchestration stopped the sessions
  and preserved the branch for explicit repair rather than silently rewriting
  it.
- **Inspired:** Staging `operator-broker` separately preserves useful parallelism:
  the agent-role package can be built against accepted semantics while the
  desktop waits for a supported/conformant client export.

## Closeout observation dispositions

- Reply/disposition clarity: completed in
  [#114](https://github.com/lossyrob/telex/issues/114) at the contract level.
  `station-app` owns implementation and usability evidence.

Other deferred items remain production-contract or hardening concerns owned by
#12, #114, or `operational-hardening`.

## Promotion candidates

- Consolidate the spike's Application Client requirements with Telex Watcher
  evidence: target authority — issue #12
  - Disposition: landed as the corrected Operator Station domain export and
    merge-SHA addendum on #12; clean PR #126 is merged. The checkpoint remains
    pending campaign/operator reconciliation and acceptance against the clean
    supporting-traceability path and manifest.
- Decide whether to promote, rename, or retire the experimental message/source
  convention: target authority — issue #114
  - Disposition: landed in `docs/design/operator-station.md`; the experimental
    namespace is retired, the production v1 application convention is accepted,
    and campaign-local conventions remain evidence only.
- Preserve the lesson that application attendance must prove delivery and
  consumption, not database visibility: target authority — workstream-design
  lesson (`project`)
  - Disposition: landed in this reconciliation note, the spike report, the
    Operator Station contract, and AC-04/AC-05.
- Keep worker missions separate from workstream gates and workflow evidence:
  target authority — workstream-design lesson (`streamliner`)
  - Disposition: landed in the campaign's updated launch protocol and v2
    Operator implementer/reviewer defaults; future launches use Streamliner's
    launch-preparations API.
