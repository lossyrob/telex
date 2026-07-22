# Application Client Contract Convergence Plan

## Revision and source freeze

This is revision 11 of the execution plan. Its approval identity is the Git
commit that contains these exact `Plan.md` bytes and the lowercase SHA-256 of
those bytes encoded as UTF-8 without a BOM, LF line endings, and exactly one
trailing LF.

## Objective

Converge the merged Telex Watcher and Operator Station requirements into one
normative, API-neutral Application Client semantic contract; record an explicit
accepted, deferred, or rejected disposition for every exported requirement; and
publish the exact `application-client-ready` checkpoint through issue #12.

This node is design and tracker work only. It does not implement a production
client core, language binding, daemon protocol, product integration, or private
fallback seam.

## Authoritative Inputs

The domain-source freeze is the following immutable provenance tuple. Issue
comments are the requirement exports; the named final source heads are
canonical when an export predates the merged contract.

| Domain | Requirement export | Merged-source addendum | Merge commit | Canonical final source head |
|---|---|---|---|---|
| Watcher | `5042702401` | `5043498697` | `09aa6f45f213b45207adc4cf80676dcce91250da` | `e007a8067b3b91b5c57a2a756ce878e310595a05` |
| Operator Station | `5042612298` | `5044388908` | `0722051760bab569d3f947fd7b29f2dabe13ef77` | `2d99e552292a4401d3403540b6d2eaa90272282d` |

Canonical source-comment body digests refetched from GitHub at revision 5 and
unchanged through revision 11:

| Comment | SHA-256 |
|---|---|
| `5042702401` | `9a037f94af84516592a56dc9c0c701ce0277e305c83dad368227fc25a5b18d9a` |
| `5043498697` | `fa02b844c62eef17f4c08b9bc1d7d94539e525034e3f0d474b2bfe2d45caed94` |
| `5042612298` | `adf2f8e439e5c224059ca51142701f604a82203c39c9829d1323a88c58889f7e` |
| `5044388908` | `702ebdb1ea81329294c35a452670b2313625142bc0281c9100d8ae892890c9ea` |

Canonical design-file Git blob identities:

| Source | Canonical blob | Current worktree blob |
|---|---|---|
| `e007a806...:docs/design/watcher.md` | `e861119d8f26e7efaad9628558436dca789b948d` | `e861119d8f26e7efaad9628558436dca789b948d` |
| `2d99e552...:docs/design/operator-station.md` | `82dc6de625a795140edbfec605e8b526d742118e` | `82dc6de625a795140edbfec605e8b526d742118e` |

| Planning metadata | Commit | Effect |
|---|---|---|
| Formation base | `0db1b1839c1fea62507b593f2b2c96e50bdc529a` | Application Client workstream formation. |
| Current planning/branch base | `7a568c43413fc7aeab6a484b07dce0f0db11d68f` | Preserves the Operator Station consumer graph update to `application-client/application-client-ready-gate`; this integration-only movement does not revise AC-01 through AC-15. |

The Watcher export references
`9df7d25c41b2eca827361db11a7a01c416721d36`, which predates the final
Watcher source. Before drafting this plan, the worker compared
`9df7d25c...` with `e007a806...` and confirmed the final-source changes
clarify omitted event `nextState`, add the pre-send evidence fence, and make
actionable inbound backlog degrade `productionReady`. The working tree's
Watcher and Operator design content has no diff from the respective final
source heads. The candidate must preserve those final semantics.

At every source-freeze boundary, fetch the repository, verify these object
identities and their design-file content, reread the four exports/addenda, and
compare the current issue #12 body. A later `main` commit is not an authority
to substitute for either canonical domain source.

At plan freeze, fetch the four linked issue comments as exact UTF-8 payloads
under `.paw/work/application-client-contract-118/inputs/`, canonicalize them,
and record their lowercase SHA-256 values in the approval ledger. A stable
comment ID is not treated as immutable content. Gate 3 and Gate 5 must refetch
and compare each digest. Any relevant change to these comment bodies, the
canonical design sources, issue #118, or issue #12 requires a new `Plan.md`
revision and complete Gate 2 re-approval before contract work continues.

Record the same evidence in
`.paw/work/application-client-contract-118/inputs/source-freeze.json`: the
source tuple, the canonical comment digests, and the reproducible
`git diff --no-ext-diff` result for `docs/design/watcher.md` against
`e007a806...` and `docs/design/operator-station.md` against `2d99e552...`.
Gate 3 and Gate 5 reproduce this file before any approval or publication; a
non-identical result is source drift, not a reviewer's judgment call.

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

## Control-channel recipients

Every operational approval or review request uses the literal URI address and
first verifies that the target is attended. A short-form alias is not
equivalent on this backend.

| Role | Exact Telex address |
|---|---|
| Application Client workstream | `telex://lossyrob/telex/T-A:application-client-orch` |
| Campaign | `telex://lossyrob/telex/T-A:campaign-orch` |
| Operator Station consumer | `telex://lossyrob/telex/T-A:operator-station-orch` |
| Telex Watcher consumer | `telex://lossyrob/telex/T-A:watcher-orch` |
| Paired reviewer | `telex://lossyrob/telex/T-A:app-client-review-118` |
| Human merge floor | `telex://lossyrob/telex/T-A:attention:rob` |

## Required Semantic Outcome

The contract will preserve the strongest compatible pressure from both domains:

1. Stable application responsibility and configured addresses remain distinct
   from fresh, never-reused runtime or process identity. Lifecycle is explicit:
   attach, detach, typed reattach/recovery, and deliberate detach are distinct
   operations.
2. Process-bound liveness supports typed PID plus process-start-time predicates.
3. Multi-address attach, reconcile, and detach must support the source-required
   atomic-or-compensable outcome: success is atomic, while failure returns
   explicit per-address partial results, collision evidence, and compensation
   handles. Partial attachment is never application-ready.
4. The contract supports both caller-selected policies: strict typed
   membership loss and bounded automatic repair. Repair preserves liveness
   predicates and never silently converts strict membership into generic
   registration. A caller-selected bounded reconcile-and-send operation
   preserves those predicates and retry budget while returning typed failure
   rather than parsing CLI output.
5. Membership-loss reasons distinguish daemon restart, predicate death,
   collision, deliberate detach, unknown or `NeedsAttach`, and owner demotion.
   Collision exposes current owner and epoch plus bounded retry/reset guidance;
   the client never hides force takeover or silently replaces another live
   application. Restart recovery uses explicit reattachment without a resident
   per-application waiter, and deliberately detached membership is never
   automatically resurrected.
6. Sender selection is explicit for every message operation.
7. Send-only and bidirectional capabilities are explicit. Send-only membership
   does not advertise inbound application attendance. A send addressed to a
   send-only application responsibility receives the address policy's
   unoccupied or rejected result, never a false application-delivered result.
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
    aliases a same-number message from another store; resolution distinguishes
    authoritative, captured-only, mismatch, and unavailable states.
16. Backend or profile selection preserves one semantic contract for SQLite and
    credentialed Postgres and carries authenticated-principal provenance when
    available.
17. Lifecycle and health projection covers registration, runtime identity,
    predicates, epoch and owner, readiness, membership loss, reconciliation,
    compensation, detach, pending work, and acknowledgment state.
18. Local discovery, bounded retry or throttling, receipt identity
    cross-checking, and application-scope cleanup are available without CLI
    parsing or raw daemon IPC.
19. Delta-oriented message, delivery, disposition, health, and recovery events
    have an explicit snapshot fence or monotonic per-axis ordering and
    resync/backfill path that cannot regress workflow state.
20. Compound reply, disposition, notification, and route-back operations
    preserve durable ordering, expose partial or indeterminate results and
    recovery handles, and support the Operator requirement for a
    machine-readable raw-thread outcome before a non-stale terminal assisted
    closure. The client supplies general primitives; it does not own Station
    routing policy or human UX.

Watcher detector/runtime behavior and Operator Station UX, mediation,
notification, routing-policy, and presentation semantics remain outside the
shared client. Domain policy may rely on the contract primitives but is not
implemented by the client.

## Plan Traceability Matrix

The final crosswalk supplies the normative per-row rationale and disposition.
This matrix prevents Gate 1 and Gate 2 from dropping a source requirement
before that artifact exists. Every mapping preserves every semantic facet of
the source requirement; a row cannot be accepted because only one facet maps.

| Source requirement | Required semantic outcomes |
|---|---|
| W-01 | 1 |
| W-02 | 2 |
| W-03 | 3, 17 |
| W-04 | 4 |
| W-05 | 5, 17 |
| W-06 | 4, 14 |
| W-07 | 6, 13 |
| W-08 | 3, 5, 17, 18 |
| W-09 | 8 |
| W-10 | 7, 8, 13, 17 |
| W-11 | 9, 10, 13 |
| W-12 | 17 |
| W-13 | 1, 4, 5, 17 |
| W-14 | 11, 14 |
| W-15 | 15, 16 |
| AC-01 | 1, 3, 5 |
| AC-02 | 15 |
| AC-03 | 3 |
| AC-04 | 9 |
| AC-05 | 10, 17 |
| AC-06 | 11, 19 |
| AC-07 | 12 |
| AC-08 | 6, 13 |
| AC-09 | 14, 20 |
| AC-10 | 20 |
| AC-11 | 15 |
| AC-12 | 17 |
| AC-13 | 16 |
| AC-14 | 19 |
| AC-15 | 18 |

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
- source provenance and exact `source_digest`;
- a strength-preservation assertion showing that the mapped contract item is at
  least as strong as the source requirement.

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
3. `docs/design/history/application-client-issue-12-original.md`
   - exact canonical bytes of the pre-convergence issue #12 body;
   - historical evidence only, not normative contract authority.
4. `docs/design/index.md`
   - normative Application Client entry and links.
5. `docs/design/DECISIONS.md`
   - only if campaign orchestration confirms that a new load-bearing ADR is
     required and allocates its number dynamically.
6. `docs/design/application-client.bundle.json`
   - canonical manifest for the approved Application Client-owned repository
     content.

Temporary PAW planning, review, publication, and evidence artifacts remain
under `.paw/work/application-client-contract-118/` while gates are active and
are removed from the final PR according to `commit-and-clean`.

Temporary evidence includes exact `inputs/issuecomment-*.md` source snapshots;
`publication/issue-12-body.pre.md` and the approved replacement body;
`approvals/ledger.json`; `evidence/` records for repairs, publication,
reviewer identity, and sentry mode; and `tools/canonicalize.ps1`.

The approval ledger is append-only by gate attempt and records gate, request
kind, plan revision or source head, digest, approver address/principal, request
and approval message IDs, timestamp, digest-reproduction attestation, and any
superseding attempt. Before `commit-and-clean`, export an exact evidence
snapshot for worker-local recovery and send its digest and complete approval
inventory to Application Client and campaign orchestration. The durable
retrieval record is the resulting Telex delivery plus its acknowledgment and
ledger message ID; the local snapshot is not approval evidence.

## Dynamic ADR Allocation

The one supported API-neutral Application Client semantic boundary, including
the explicit send-only/bidirectional capability split and prohibition on
product-private fallbacks, is a proposed load-bearing decision. After both
external plan approvals and before editing `docs/design/DECISIONS.md`, request
campaign orchestration to allocate an ADR number or explicitly determine that
an ADR is not required. The request names the decision but never proposes a
number. It includes issue `118`, a stable decision slug, the approved plan
revision and digest, the current highest ADR number, and an idempotency key. The
response must be `allocated` with a reserved number, `no-adr-required` with
rationale, or `blocked` with an owner. Use only a returned allocation.

If campaign orchestration determines that the normative design is sufficient
without a new ADR, do not edit the decision log. Any change in the ADR decision
changes the candidate bundle and its manifest.

Before editing, verify that current remote `main` has not consumed or passed the
reserved number. On collision or an unanswered request, send one
disposition-required escalation to campaign orchestration and hold; never guess,
reuse, or silently omit a required ADR.

## Canonical Contract Bundle

`docs/design/application-client.bundle.json` will be written as UTF-8 without a
BOM, LF line endings, and exactly one trailing LF. Its schema is:

- `schemaVersion`: `1`;
- `checkpointScope`: `design-only`;
- `sourceProvenance`: the two domain export/addendum comment IDs, merge commits,
  canonical final source heads, and exact source-comment digests;
- `historicalIssue12`: committed path, UTF-8 byte length, and SHA-256 of the
  exact canonical pre-convergence issue #12 body;
- `files`: repository-relative Application Client-owned paths, sorted by
  byte-lexicographic order of their UTF-8 path bytes;
- each file entry: `path`, UTF-8 byte length, and lowercase SHA-256.

Unknown manifest schema versions must not be approved. Additive fields within
version 1 are permitted only before approval; any structural change after an
approval requires a schema-version decision and complete bundle re-approval.

The manifest is compact JSON with no insignificant whitespace and exactly one
trailing LF. Object keys use this fixed ordinal order:
`schemaVersion`, `checkpointScope`, `sourceProvenance`, `historicalIssue12`,
`files`. Each
`sourceProvenance` entry uses `domain`, `requirementExportComment`,
`mergedSourceAddendumComment`, `mergeCommit`, `canonicalFinalHead`, and
`sourceCommentDigests`; the domain entries sort by UTF-8 byte-lexicographic
`domain`. `sourceCommentDigests` is an array of exactly two objects. Each object
uses fixed key order `commentId`, `role`, `sha256`; `commentId` is a JSON
integer, `role` is `requirement-export` or `merged-source-addendum`, and entries
sort by ascending numeric `commentId`. Each `files` entry uses fixed key order
`path`, `byteLength`, `sha256`. JSON strings use standard JSON escaping without
Unicode normalization. The manifest generator is the sole writer;
independently reproducing these bytes is required before approval.

`historicalIssue12` uses fixed key order `path`, `byteLength`, `sha256`. Its
path is `docs/design/history/application-client-issue-12-original.md`, and its
digest must match the corresponding `files` entry.

The manifest does not list itself or embed its own digest, avoiding circular
identity. The review identity is the tuple:

`(candidate source-head commit, SHA-256 of exact manifest bytes)`.

Every approval request and ledger row carries both tuple values in a fixed
metadata envelope. A manifest digest without its candidate source head is not
an approval identity.

The file set includes the normative design, crosswalk, design index, and any
allocated ADR contribution plus the exact historical issue #12 artifact. It
excludes `.paw`, shared Streamliner artifacts, the replacement issue body,
review output, and generated transport evidence.

Any byte change to a listed file requires:

1. regenerating the manifest;
2. committing the changed candidate;
3. using the new source head and manifest digest;
4. invalidating all earlier consumer and shared approvals.

The candidate source head is the commit containing the listed contract files
and manifest. The immutable domain-source provenance remains part of the
manifest and is independently rechecked before every approval request.

## Exact-Byte Canonicalization

`.paw/work/application-client-contract-118/tools/canonicalize.ps1` is the sole
reproducer for exact approval bytes. It canonicalizes `Plan.md`, source
snapshots, the bundle manifest, the issue #12 publication body, and any ADR
contribution to UTF-8 without BOM, LF line endings, and exactly one trailing LF,
then prints byte length and lowercase SHA-256.

Canonicalization is a closed transform:

1. reject ill-formed UTF-8 before processing;
2. remove a leading UTF-8 BOM if present;
3. convert CRLF and lone CR to LF;
4. remove all trailing CR/LF characters;
5. append exactly one LF;
6. perform no Unicode normalization, whitespace trimming, Markdown rendering,
   or other transformation.

Immediately before every Telex `--body-file`, manifest generation, GitHub
`--body-file`, or digest comparison, rerun the reproducer and require the
on-disk digest to match the approved or recorded digest.

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

Before replacement, fetch the existing issue #12 body into
`publication/issue-12-body.pre.md`, canonicalize it, and record its digest in
the approval ledger and historical-proposal crosswalk provenance. Copy those
exact canonical bytes without wrapper text to
`docs/design/history/application-client-issue-12-original.md`, include that
artifact and digest in the candidate manifest, and link it from the replacement
issue #12 body as the durable historical input.

Publication review identity is:

`(publication revision, SHA-256 of exact publication bytes)`.

Immediately before posting:

1. refresh and re-read issue #12;
2. refresh and re-read both merged domain exports and addenda;
3. compare them with the approved provenance;
4. stop and reconfirm if any relevant source changed;
5. recompute the on-disk body-file digest and stop if it differs from the
   approved digest;
6. post only with `gh issue edit 12 --body-file <approved-file>`;
7. fetch the published body;
8. apply only the closed canonicalization transform above;
9. verify the fetched digest equals the approved digest;
10. on mismatch, preserve both byte sequences and transport evidence, hard-stop
    publication completion, and submit revised bytes through the full
    publication approval gate; never widen canonicalization or patch the issue
    body without approval.

Every `gh` command will set
`$env:GH_CONFIG_DIR = "$env:APPDATA\gh-pub"`. Publication writes use
`--body-file` and never console-round-trip the approved Markdown. No PR
assignees will be added or modified.

If any approved source comment, canonical source file, ADR, or issue #12 body
drifts after publication but before merge, mark the checkpoint publication
stale in the approval ledger, notify Application Client and campaign
orchestration with the drift evidence, and hold downstream readiness. Replace
the body only by regenerating it and repeating the affected consumer,
bundle, and exact-publication approval gates. After merge, campaign
orchestration owns the same supersession process; this worker does not silently
edit a published checkpoint.

## Gate Sequence

### Gate 1: Internal PAW Planning Review

1. Commit this plan under the configured artifact lifecycle.
2. Create the deterministic canonicalizer, canonicalize `Plan.md`, and record
   its exact revision, byte length, and digest.
3. Run the configured non-interactive society-of-thought planning-docs review:
   - specialist: `general-reviewer`;
   - model: `claude-opus-4.7-high`;
   - interaction: `parallel`;
   - perspectives: `premortem`, `retrospective`;
   - perspective cap: `2`.
4. Resolve every blocking planning finding.
5. Re-canonicalize and commit the exact reviewed `Plan.md`.

If the pinned model is unavailable, do not substitute silently. Request a
campaign-orchestration disposition for a successor model, revise
`WorkflowContext.md` and this plan, and repeat the affected exact-plan approval.

### Gate 2: External Exact-Plan Approval

Commit the reviewed plan, calculate its SHA-256 from the exact byte definition
at the top of this document, and send the plan revision, candidate source-head
commit, digest, and full exact bytes separately to:

1. `telex://lossyrob/telex/T-A:application-client-orch`.
2. `telex://lossyrob/telex/T-A:campaign-orch`.

Each Telex message will use a subject identifying the plan review,
`next-checkpoint` attention, required disposition, metadata containing plan
revision, artifact path, candidate source head, and lowercase SHA-256, and a
body loaded directly from the reviewed `Plan.md`.

Before sending, calculate the complete Telex payload size. If the full plan
cannot fit below Telex's IPC cap, send the candidate source head, the immutable
Git blob ID and path for `Plan.md`, its SHA-256, and retrieval instructions
instead. An approver must retrieve that exact blob, reproduce the SHA-256, and
attest both facts; digest-only approval is forbidden.

Implementation begins only after both recipients approve the same revision and
digest. Approval records include the approving Telex address/principal, message
ID, timestamp, and exact-byte attestation and are single-use for that revision.
A byte change invalidates both approvals.

Conflicting feedback is sent to all affected orchestrators as
`decision-needed`; no conflict is resolved silently. After two unresolved
decision rounds, campaign orchestration is the binding escalation authority.
There is no default acceptance while a decision is pending.

### Gate 3: Candidate Contract and Consumer Approval

1. Refresh remote `main` and inspect changes to all required sources and shared
   files. Revalidate the immutable source tuple; preserve latest-main content
   without treating it as a substitute for the final domain heads or
   overwriting orchestrator changes. Recompute the four source-comment digests.
   If any authoritative input changed since Gate 2 approval, revise `Plan.md`
   and repeat Gate 2 before continuing.
2. Request the ADR decision and number from campaign orchestration before any
   decision-log edit.
3. Create the design, crosswalk, index contribution, optional allocated ADR,
   downstream checkpoint/decomposition, and exact issue #12 publication draft.
4. Commit the candidate.
5. Generate and commit the canonical bundle manifest.
   Verify bundle closure: the Application Client-owned `docs/design/**` paths
   changed from the current planning base equal the manifest's listed paths
   plus `docs/design/application-client.bundle.json`; unexpected paths block
   approval until removed, assigned to another owner, or deliberately added to
   the manifest and crosswalk rationale.
6. Send the complete candidate separately to Operator Station and Telex Watcher
   orchestration at `telex://lossyrob/telex/T-A:operator-station-orch` and
   `telex://lossyrob/telex/T-A:watcher-orch` as disposition-required
   `consumer-contract-review-requested` messages.
7. Include source head, bundle digest, manifest path, contract paths,
   disposition totals, immutable domain-source provenance, and the complete
   candidate content. If Telex size limits prevent inline content, use a
   content-addressed git commit and blob/path representation. The consumer must
   record that it retrieved the exact source head, recomputed the manifest and
   listed-file digests, and matched the requested bundle digest. Digest-only
   approval is forbidden.
8. Resolve every finding. Require both consumers to approve the same source
   head and bundle digest, with approver address/principal, message ID,
   timestamp, and digest-reproduction attestation recorded in the ledger.

Semantically relevant repairs invalidate both consumer approvals and restart
this gate. Conflicting consumer findings are sent as `decision-needed` to both
consumers, Application Client orchestration, and campaign orchestration.
After two unresolved decision rounds, campaign orchestration provides the
binding disposition or leaves the node blocked.

### Gate 4: Shared Bundle Approval

Send the consumer-approved final bundle separately to
`telex://lossyrob/telex/T-A:application-client-orch` and
`telex://lossyrob/telex/T-A:campaign-orch`. Require both to approve the same
source head and bundle digest, record approver identity and message metadata,
and require each approver to attest retrieval and digest reproduction. Internal
PAW review and consumer approval do not substitute for this gate.

### Gate 5: Exact Issue #12 Publication Approval

Send the complete exact publication bytes separately to
`telex://lossyrob/telex/T-A:application-client-orch` and
`telex://lossyrob/telex/T-A:campaign-orch` as disposition-required
`application-client-contract-publication-review-requested` messages. Include
publication revision, artifact path, and SHA-256. Require both approvals for the
same revision and digest, record approver identity and message metadata, then
perform the pre-body snapshot, re-read, on-disk preflight, post, fetch,
canonicalize, and digest verification sequence.

Before sending, calculate the complete Telex payload size. If the exact
publication body cannot fit below the IPC cap, commit the canonical publication
file to the candidate source head and send its immutable Git blob ID, path,
SHA-256, and retrieval instructions. Each approver retrieves and hashes that
exact blob before attesting; digest-only approval is forbidden.

### Gate 6: Final PAW Review and PR

1. Verify all implementation work TODOs are complete.
2. Run repository documentation checks applicable to changed Markdown and JSON.
3. Run the configured non-interactive society-of-thought final review over the
   branch diff with the same specialist, model, interaction, and perspectives
   as planning review.
4. Resolve all blocking findings and rerun every invalidated consumer,
   shared-bundle, and publication gate after any semantic or byte change.
5. Classify each repair in the approval ledger. Any normative-clause change,
   bundle-listed file byte change, crosswalk disposition/rationale change,
   manifest membership change, or checkpoint-scope change is semantic. A
   publication-only byte change reruns publication approval; a bundle-listed
   byte change reruns consumer and shared-bundle approval. Batch all known
   repairs before re-requesting approval, but do not create an editorial bypass
   from exact-digest approval.
6. Export the durable evidence snapshot and send its digest and approval
   inventory to Application Client and campaign orchestration.
7. Use `paw-pr` for artifact cleanup, selective staging, push, and final PR
   creation.
8. Use `Closes #118` only if every contract, checkpoint, publication, approval,
   and evidence gate is complete.

### Gate 7: Paired Review and PR Sentry

1. Verify the configured reviewer address is attended, then after push, green
   CI, and clean mergeability send `review-ready` to
   `telex://lossyrob/telex/T-A:app-client-review-118`. If it is unattended, notify Application Client
   and campaign orchestration and hold rather than selecting an unapproved
   fallback reviewer.
2. Read every real GitHub review and require a verified `PAW Review: +1` for the
   current head. The assigned paired reviewer posts that current-head GitHub
   review after its Telex review request; a local SoT artifact is not a
   substitute. Record the reviewer Telex address, GitHub identity, approval
   message ID, and reviewed commit.
3. Treat semantically relevant review repairs as bundle/publication changes and
   repeat the affected exact-digest gates.
4. Verify green CI, clean mergeability, exact PR head, and zero unresolved
   review threads.
5. After the exact-head technical state and checkpoint evidence are complete,
   send one disposition-required `attention.merge-floor` request to
   `telex://lossyrob/telex/T-A:attention:rob`. Campaign orchestration mediates
   the request and must return an explicit human approval disposition. Campaign
   technical approvals, recommendations, or prior PR #115/#116 outcomes never
   satisfy this floor.
6. Any campaign merge authorization must cite both the
   `attention.merge-floor` request message ID and the human approval disposition
   ID. An authorization missing either reference is non-authoritative, and the
   PR remains held.
7. Immediately before adding watches, run the canonical terminal PR-state
   check. Merged or closed PRs skip Watcher registration and Loop fallback.
8. Load and follow the canonical `paw-pr-lifecycle` state machine. Prefer the
   shared Watcher runtime, use pinned private detector copies only as review
   tooling rather than a product/client seam, and use issue-scoped watch IDs.
   Record runtime identity, detector digest, watch ID, health probe, selected
   sentry mode, and whether Loop fallback was used. Never run Watcher and Loop
   supervision in parallel and never stop the shared runtime.
9. After human-floor approval and authoritative campaign authorization, send
   the full
   disposition-required `merge-ready` field report to Application Client
   orchestration and `node-merge-ready` to campaign orchestration.
10. Do not merge the PR. Hold it healthy for orchestration.
11. After merge, send one disposition-required `reconciliation-requested` packet
   to Application Client orchestration and concise `node-merged` status to
   campaign orchestration. Require durable received/disposition confirmation;
   if it remains unconfirmed, escalate the original message ID to campaign
   orchestration without sending a duplicate reconciliation request. Keep the
   sentry/evidence record until confirmation or explicit campaign disposition.

## Approval and Repair Accounting

Approval freshness is verified against the current Telex message, source head,
and digest at each gate. An approval is never replayed for a newer revision even
when the digest happens to match.

Repairs are coalesced before requesting another review cycle. Exact-byte
approval remains mandatory for every affected artifact; whitespace,
punctuation, and non-normative prose do not receive a silent exemption.

The field report is written at
`.paw/work/application-client-contract-118/evidence/final-field-report.md`,
canonicalized and hashed, then copied without modification to
`C:\Users\robemanuele\.copilot\session-state\ac0ce967-6551-4d69-8d4f-cd89c8ecaab7\files\application-client-contract-118-evidence\`.
The local copy is worker-recovery evidence only. The report is sent verbatim as
disposition-required Telex deliveries to Application Client and campaign
orchestration. The approval ledger records both message IDs, acknowledgments,
terminal dispositions, report digest, and the local recovery path before
`commit-and-clean`. Post-cleanup durable retrieval is through those Telex
message IDs, the committed bundle manifest and PR head, and the published issue
#12 body; the local session artifact is not an approval authority.

The ledger also records the `attention.merge-floor` request ID, human approval
disposition ID, and campaign merge-authorization message ID. The authorization
record is valid only when it cites the first two IDs. No technical approval or
recommendation is recorded as a human-floor substitute.

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
   - Current-head `PAW Review: +1`, zero unresolved threads, explicit human
     merge-floor approval, campaign authorization citing both floor IDs,
     merge-ready field report, Watcher-backed lifecycle monitoring, and
     post-merge reconciliation request.

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
- approving addresses/principals, timestamps, and digest-reproduction
  attestations;
- source-comment snapshot digests and the pre-publication issue #12 digest;
- exact checkpoint meaning and downstream node recommendations;
- design-index and ADR impact;
- old issue #12 proposal elements accepted, replaced, deferred, or rejected;
- risks, boundary pressure, and explicit no-private-fallback confirmation;
- repair/reapproval cycle counts and PR sentry mode evidence;
- `attention.merge-floor` request ID, explicit human approval disposition ID,
  and campaign merge-authorization ID with both required citations;
- confirmation that campaign technical approval and PR #115/#116 were not used
  as merge authorization;
- process feedback for PAW, Telex, paired review, and Watcher sentry.

## Definition of Done

The node is done only when all of the following are true:

1. Both external plan reviewers approved the same exact plan revision and
   digest before any contract, crosswalk, ADR, or publication content was
   edited.
2. The crosswalk contains exactly 30 source rows with every required
   disposition, rationale, owner, blocking effect, source digest, and
   strength-preservation field.
3. Source-freeze evidence, canonicalization tests, candidate manifest, listed
   file digests, and bundle closure reproduce from the approved source head.
4. The campaign-owned ADR determination is recorded, and any allocated ADR uses
   the dynamically returned number without collision.
5. Operator Station and Watcher approved the same source head and bundle
   digest, followed by matching Application Client and campaign approvals.
6. Application Client and campaign approved the same exact issue #12
   publication bytes, and the fetched published body matches that digest.
7. The exact pre-convergence issue #12 body is committed as a retrievable
   historical artifact, linked from the replacement body, and its digest
   matches both `historicalIssue12` and the manifest file entry.
8. The checkpoint text and machine-readable manifest state design-only scope
   and do not claim implementation, conformance, consumer integration, or
   production readiness.
9. Final configured PAW review is complete; the current PR head has verified
   `PAW Review: +1`, green CI, clean mergeability, and zero unresolved review
   threads.
10. A disposition-required `attention.merge-floor` request has explicit human
    approval routed by campaign, and any campaign merge authorization cites
    both the merge-floor request ID and human approval disposition ID. Technical
    campaign approval alone and PR #115/#116 are not precedent or substitutes.
11. The canonical field report and approval inventory have durable Telex
   retrieval records, `merge-ready` and `node-merge-ready` are delivered, and
   exactly one Watcher-or-Loop sentry mode is active under canonical lifecycle
   handling.
12. The worker has not merged the PR. After orchestration merges it, exactly one
    reconciliation request and the campaign merge status are sent as specified.

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
- The exact historical issue #12 body remains retrievable from the committed
  history artifact and its manifest identity matches the replacement body's
  link and provenance.
- `application-client-ready` unlocks semantic downstream promotion and planning
  only; it does not claim implementation, conformance, consumer integration, or
  production usability. The manifest records `checkpointScope: design-only`,
  and downstream consumers acknowledge that scope.
- The branch contains no production client code and no private fallback seam.
- Merge authorization includes the mandatory human-floor evidence; technical
  campaign approval alone never authorizes merge.
- The PR is not merged by this worker.
