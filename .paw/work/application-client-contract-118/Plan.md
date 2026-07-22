# Application Client Contract Convergence Plan

## Objective

Converge the merged Telex Watcher and Operator Station requirements into one
normative, API-neutral Application Client semantic contract; record an explicit
accepted, deferred, or rejected disposition for every exported requirement; and
publish the exact `application-client-ready` checkpoint through issue #12.

This node is design and tracker work only. It does not implement a production
client core, language binding, daemon protocol, product integration, or private
fallback seam.

## Authoritative Inputs

The work starts from remote `main` commit
`7a568c43413fc7aeab6a484b07dce0f0db11d68f` and must refresh remote `main`
at every source-freeze boundary.

Normative and planning sources:

- Issue #118 and the Application Client workstream artifacts.
- Issue #12 as the sole publication and checkpoint owner. Its existing
  TypeScript API sketch is historical input, not accepted authority.
- Watcher requirements export
  `issuecomment-5042702401` and merged-source addendum
  `issuecomment-5043498697`.
- Operator requirements export
  `issuecomment-5042612298` and merged-source addendum
  `issuecomment-5044388908`.
- `docs/design/daemon.md`, `docs/design/watcher.md`,
  `docs/design/operator-station.md`, and ADRs 0046 through 0048.
- `.streamliner/shaping/roadmap.md` and the Application Client, Operator
  Station, and Telex Watcher workstream briefs and graphs.

The worker may change only its dedicated worktree, branch, PR, issue #12 body,
and issue #118 comments. Shared Streamliner roadmap, brief, graph,
reconciliation, and campaign checkpoint artifacts are orchestrator-owned and
will not be edited in this branch. Recommended reconciliations will be included
in the field report.

## Required Semantic Outcome

The contract will preserve the strongest compatible pressure from both domains:

1. Stable application responsibility and configured addresses remain distinct
   from fresh, never-reused runtime or process identity.
2. Process-bound liveness supports typed PID plus process-start-time predicates.
3. Multi-address attach, reconcile, and detach are atomic or return explicit
   per-address partial results, collision evidence, and compensation handles.
4. Callers explicitly select strict typed membership loss or bounded automatic
   repair. Repair preserves liveness predicates and never silently converts
   strict membership into generic registration.
5. Membership-loss reasons distinguish daemon restart, predicate death,
   collision, deliberate detach, unknown or `NeedsAttach`, and owner demotion.
6. Sender selection is explicit for every message operation.
7. Send-only and bidirectional capabilities are explicit. Send-only membership
   does not advertise inbound application attendance.
8. Durable acceptance, occupancy at acceptance, push attempt or acceptance,
   exact recipient delivery-row transport acknowledgment, and workflow
   disposition remain separate typed axes.
9. Bidirectional receive yields the complete message, opaque metadata,
   delivery-role context, exact recipient or delivery-row identity, and an
   acknowledgment capability bound to that exact delivery.
10. Ack occurs only after durable application ingest; ack-pending,
    pending-unconsumed, inbound-actionable, deaf, and backlog states are
    observable.
11. At-least-once redelivery identity is per recipient. Restart reconciliation
    and ordered resync use a snapshot fence or monotonic per-axis versions and
    cannot regress workflow state.
12. Unresolved obligations, bounded recent history, and thread history are
    queryable without full-store materialization or pre-filter limit ambiguity.
13. Send, metadata-bearing reply, read-thread, exact-recipient disposition,
    source resolution, and compound application operations return typed,
    identity-checkable results.
14. Application-authored operations are retry-safe through stable operation
    identity, an explicit accepted-send/local-commit duplicate window, and
    post-restart operation-result and receipt reconciliation.
15. Source identity is `(opaque logical-store identity, message ID)` and never
    aliases a same-number message from another store.
16. Backend or profile selection preserves one semantic contract for SQLite and
    credentialed Postgres and carries authenticated-principal provenance when
    available.
17. Lifecycle and health projection covers registration, runtime identity,
    predicates, epoch and owner, readiness, membership loss, reconciliation,
    compensation, detach, pending work, and acknowledgment state.
18. Local discovery, bounded retry or throttling, receipt identity
    cross-checking, and application-scope cleanup are available without CLI
    parsing or raw daemon IPC.

Watcher detector/runtime behavior and Operator Station UX, mediation,
notification, routing-policy, and presentation semantics remain outside the
shared client. Domain policy may rely on the contract primitives but is not
implemented by the client.

## Requirement Crosswalk and Disposition Rules

Create `docs/design/application-client-crosswalk.md` with one row for every
Watcher W-01 through W-15 requirement and every Operator AC-01 through AC-15
requirement. Each row will contain:

- source requirement ID and an accurate summary;
- shared contract item or items that satisfy it;
- disposition: `accepted`, `deferred`, or `rejected`;
- rationale;
- owner of any remaining work;
- downstream blocking impact;
- source provenance.

Overlapping requirements may point to the same contract item, but no source row
may disappear. Stronger constraints such as process-bound liveness, strict
versus bounded repair, exact recipient delivery identity, ack-after-ingest,
restart-safe result reconciliation, raw-thread route-back evidence, and
no-regression resync remain explicit.

The intended outcome is to accept all merged shared semantic requirements. If
research or consumer review shows that any requirement cannot be accepted, the
row must state the deferral or rejection, owner, and affected blocked consumer.
No non-accepted requirement authorizes an undocumented product-private seam.

The crosswalk will also disposition the old issue #12 proposal elements:

- accept runtime-agnostic application integration and semantic operations;
- replace the historical holder/waiter, client-owned heartbeat/cursor, and raw
  socket assumptions with current daemon membership, `NeedsAttach`,
  per-recipient acknowledgment, and at-least-once semantics;
- defer package names, TypeScript or napi-rs priority, C ABI, public socket
  protocol, binding API shape, delivery ergonomics, SDK interrupt policy, and
  language sequencing to later implementation nodes;
- reject the old API sketch as contract authority.

## Repository Artifacts

The candidate repository bundle will contain:

1. `docs/design/application-client.md`
   - normative API-neutral semantic contract;
   - identity, capability, lifecycle, receipt, receive, operation, source,
     backend, health, and delta/resync models;
   - explicit product-boundary exclusions;
   - exact `application-client-ready` meaning;
   - downstream decomposition for supported client core, first binding,
     conformance, consumer integration, and hardening.
2. `docs/design/application-client-crosswalk.md`
   - complete Watcher and Operator requirement dispositions;
   - historical issue #12 proposal dispositions and blocking effects.
3. `docs/design/index.md`
   - normative Application Client entry and links.
4. `docs/design/DECISIONS.md`
   - only if campaign orchestration confirms that a new load-bearing ADR is
     required and allocates its number dynamically.
5. `docs/design/application-client.bundle.json`
   - canonical manifest for the approved Application Client-owned repository
     content.

Temporary PAW planning, review, publication, and evidence artifacts remain
under `.paw/work/application-client-contract-118/` while gates are active and
are removed from the final PR according to `commit-and-clean`.

## Dynamic ADR Allocation

After both external plan approvals and before editing
`docs/design/DECISIONS.md`, send campaign orchestration a
disposition-required ADR allocation request describing the proposed
load-bearing decision. Use only the number returned by campaign orchestration.

If campaign orchestration determines that the normative design is sufficient
without a new ADR, do not edit the decision log. Any change in the ADR decision
changes the candidate bundle and its manifest.

## Canonical Contract Bundle

`docs/design/application-client.bundle.json` will be written as UTF-8 without a
BOM, LF line endings, and exactly one trailing LF. Its schema is:

- `schemaVersion`: `1`;
- `files`: repository-relative Application Client-owned paths, sorted by
  ordinal path order;
- each file entry: `path`, UTF-8 byte length, and lowercase SHA-256.

The manifest does not list itself and does not embed a source commit or its own
digest, avoiding circular identity. The review identity is the tuple:

`(source head commit, SHA-256 of exact manifest bytes)`.

The file set includes the normative design, crosswalk, design index, and any
allocated ADR contribution. It excludes `.paw`, shared Streamliner artifacts,
issue bodies, review output, and generated transport evidence.

Any byte change to a listed file requires:

1. regenerating the manifest;
2. committing the changed candidate;
3. using the new source head and manifest digest;
4. invalidating all earlier consumer and shared approvals.

## Exact Issue #12 Publication

Materialize the proposed issue #12 body at
`.paw/work/application-client-contract-118/publication/issue-12-body.md` as
UTF-8 without BOM, LF line endings, and exactly one trailing LF. It will
include:

- merged-source provenance and immutable comment or commit references;
- the accepted shared semantic contract;
- the complete requirement crosswalk link and disposition totals;
- accepted, deferred, and rejected items with owners and blocking effects;
- the exact meaning of `application-client-ready`;
- implementation work that remains;
- explicit rejection of CLI parsing, raw daemon IPC, spike helpers, and
  product-private clients as fallback seams.

Publication review identity is:

`(publication revision, SHA-256 of exact publication bytes)`.

Immediately before posting:

1. refresh and re-read issue #12;
2. refresh and re-read both merged domain exports and addenda;
3. compare them with the approved provenance;
4. stop and reconfirm if any relevant source changed;
5. post only with `gh issue edit 12 --body-file <approved-file>`;
6. fetch the published body;
7. canonicalize transport line endings and trailing newline only;
8. verify the fetched digest equals the approved digest;
9. stop on mismatch rather than repairing the body without approval.

Every `gh` command will set
`$env:GH_CONFIG_DIR = "$env:APPDATA\gh-pub"`. No PR assignees will be added or
modified.

## Gate Sequence

### Gate 1: Internal PAW Planning Review

1. Commit this plan under the configured artifact lifecycle.
2. Run the configured non-interactive society-of-thought planning-docs review:
   - specialist: `general-reviewer`;
   - model: `claude-opus-4.7-high`;
   - interaction: `parallel`;
   - perspectives: `premortem`, `retrospective`;
   - perspective cap: `2`.
3. Resolve every blocking planning finding.
4. Commit the exact reviewed `Plan.md`.

### Gate 2: External Exact-Plan Approval

Send the exact reviewed `Plan.md` bytes separately to:

1. Application Client workstream orchestration.
2. Campaign orchestration.

Each Telex message will use:

- kind `plan-review-requested`;
- attention `next-checkpoint`;
- disposition required;
- metadata containing plan revision, artifact path, and lowercase SHA-256;
- body loaded directly from the reviewed `Plan.md`.

Implementation begins only after both recipients approve the same revision and
digest. A byte change invalidates both approvals. Conflicting feedback is sent
to all affected orchestrators as `decision-needed`; no conflict is resolved
silently.

### Gate 3: Candidate Contract and Consumer Approval

1. Refresh remote `main` and inspect changes to all required sources and shared
   files. Preserve latest-main content; do not overwrite orchestrator changes.
2. Request the ADR decision and number from campaign orchestration before any
   decision-log edit.
3. Create the design, crosswalk, index contribution, optional allocated ADR,
   downstream checkpoint/decomposition, and exact issue #12 publication draft.
4. Commit the candidate.
5. Generate and commit the canonical bundle manifest.
6. Send the complete candidate separately to Operator Station and Telex Watcher
   orchestration as disposition-required
   `consumer-contract-review-requested` messages.
7. Include source head, bundle digest, manifest path, contract paths,
   disposition totals, and the complete candidate content or an exact immutable
   representation within Telex size limits.
8. Resolve every finding. Require both consumers to approve the same source
   head and bundle digest.

Semantically relevant repairs invalidate both consumer approvals and restart
this gate. Conflicting consumer findings are sent as `decision-needed` to both
consumers, Application Client orchestration, and campaign orchestration.

### Gate 4: Shared Bundle Approval

Send the consumer-approved final bundle separately to Application Client
workstream and campaign orchestration. Require both to approve the same source
head and bundle digest. Internal PAW review and consumer approval do not
substitute for this gate.

### Gate 5: Exact Issue #12 Publication Approval

Send the complete exact publication bytes separately to Application Client
workstream and campaign orchestration as disposition-required
`application-client-contract-publication-review-requested` messages. Include
publication revision, artifact path, and SHA-256. Require both approvals for the
same revision and digest, then perform the re-read, post, fetch, canonicalize,
and digest verification sequence.

### Gate 6: Final PAW Review and PR

1. Verify all implementation work TODOs are complete.
2. Run repository documentation checks applicable to changed Markdown and JSON.
3. Run the configured non-interactive society-of-thought final review over the
   branch diff with the same specialist, model, interaction, and perspectives
   as planning review.
4. Resolve all blocking findings and rerun consumer, shared-bundle, and
   publication gates after any semantically relevant repair.
5. Use `paw-pr` for artifact cleanup, selective staging, push, and final PR
   creation.
6. Use `Closes #118` only if every contract, checkpoint, publication, approval,
   and evidence gate is complete.

### Gate 7: Paired Review and PR Sentry

1. After push, green CI, and clean mergeability, send `review-ready` to
   `telex://lossyrob/telex/T-A:app-client-review-118`.
2. Read every real GitHub review and require a verified `PAW Review: +1` for the
   current head.
3. Treat semantically relevant review repairs as bundle/publication changes and
   repeat the affected exact-digest gates.
4. Verify green CI, clean mergeability, exact PR head, and zero unresolved
   review threads.
5. Immediately before adding watches, run the canonical terminal PR-state
   check. Merged or closed PRs skip Watcher registration and Loop fallback.
6. Prefer the shared Watcher runtime and canonical `paw-pr-lifecycle` behavior
   with pinned private detector copies and issue-scoped watch IDs. Never run
   Watcher and Loop supervision in parallel and never stop the shared runtime.
7. At the first approved, green, mergeable, zero-thread state, send the full
   disposition-required `merge-ready` field report to Application Client
   orchestration and `node-merge-ready` to campaign orchestration.
8. Do not merge the PR. Hold it healthy for orchestration.
9. After merge, send one `reconciliation-requested` packet to Application
   Client orchestration and concise `node-merged` status to campaign
   orchestration.

## Work Items

1. **Freeze and approve the plan**
   - Internal SoT review, exact-byte digest, and dual orchestration approval.
2. **Author and commit the candidate contract**
   - Normative design, complete crosswalk, index, optional dynamically
     allocated ADR, downstream decomposition, publication draft, and manifest.
3. **Obtain exact consumer and shared-bundle approval**
   - Operator and Watcher approval, then Application Client and campaign
     approval for one source head and bundle digest.
4. **Publish and verify issue #12**
   - Dual exact-byte approval, source re-read, `--body-file` publication, and
     fetched-body digest verification.
5. **Complete PAW final review and final PR**
   - Resolve review findings, clean PAW artifacts, push, open the final PR, and
     ensure CI and mergeability.
6. **Complete paired review and sentry handoff**
   - Current-head `PAW Review: +1`, zero unresolved threads, merge-ready field
     report, Watcher-backed lifecycle monitoring, and post-merge
     reconciliation request.

## Verification and Evidence

The final field report will include:

- accepted, deferred, and rejected totals for both requirement sets;
- unresolved or deferred blockers and downstream owners;
- source head, canonical manifest path, manifest digest, and listed file
  digests;
- issue #12 publication revision, approved digest, fetched digest, and issue
  URL;
- Operator and Watcher consumer approval message IDs;
- Application Client and campaign bundle/publication approval message IDs;
- exact checkpoint meaning and downstream node recommendations;
- design-index and ADR impact;
- old issue #12 proposal elements accepted, replaced, deferred, or rejected;
- risks, boundary pressure, and explicit no-private-fallback confirmation;
- process feedback for PAW, Telex, paired review, and Watcher sentry.

## Success Criteria

- Every W-01 through W-15 and AC-01 through AC-15 requirement has an explicit
  disposition, rationale, owner, blocking effect, and contract mapping.
- No overlap is weakened and no product-specific policy leaks into the shared
  client.
- Capability, identity, receipt axes, exact delivery identity, retry and
  restart behavior, ordered resync, backend parity, and source identity are
  precise and API-neutral.
- Both consumer orchestrators approve one exact candidate source head and
  bundle digest.
- Application Client and campaign orchestration approve the same exact bundle
  and exact publication bytes.
- Issue #12 is published from approved bytes and the fetched canonical body
  matches the approved digest.
- `application-client-ready` unlocks semantic downstream promotion and planning
  only; it does not claim implementation, conformance, consumer integration, or
  production usability.
- The branch contains no production client code and no private fallback seam.
- The PR is not merged by this worker.
