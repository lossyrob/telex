# Application Client Contract Convergence Plan

## Revision and source freeze

This is revision 23 of the execution plan. Its approval identity is the Git
commit that contains these exact `Plan.md` bytes and the lowercase SHA-256 of
those bytes encoded as UTF-8 without a BOM, LF line endings, and exactly one
trailing LF.

Revision 23 supersedes rejected revisions 21 and 22 and is the human-floor
repair plan for PR #123. It moves only the
requirements crosswalk from the normative design layer to
`docs/notes/application-client/requirements-crosswalk.md`, where it is
non-normative requirements traceability and provenance. The normative
Application Client contract, ADR 0049, bundle manifest, design index entry, and
exact historical issue #12 snapshot remain under `docs/design/`.

The documentation-placement repair is sourced from operator feedback `908` and
authorized by Application Client messages `909`, `911`, `912`, and `913`.
The separate human-floor destination override was directly instructed by the
human operator outside Telex, durably recorded by Application Client decision
`901`, raised to campaign as decision request `902`, and accepted by campaign
in `904`.
Revision-21 campaign approval `916` is stale because Application Client
rejected that plan in `917`; revision-22 requests `919`/`920` are stale because
Application Client rejected that plan in `921`.
Human-floor request `897` was not approved. The current PR head
`d921d6cdd58db856df3b8eacef908c11cf46ed25`, all Gate 3 through Gate 7
approvals/evidence, and the published issue #12 body are historical evidence
only after repair begins; none can authorize merge. Durable repository bytes
must not change until revision-23 Gate 2 approvals are current.

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
unchanged through revision 23:

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
| Current planning/branch base | `f6e0deec043308971029ddefc50411ee455fd27a` | Preserves the Operator and Watcher consumer gate updates plus Application Client builder-resume/ADR-0049 reconciliation. These planning-only movements do not revise W-01 through W-15 or AC-01 through AC-15. |
| Builder resume | Telex `1857` relayed by `1860` | Supersedes the scope pause. Application Client shared artifacts now record active planning and ADR 0049 reservation; further reconciliation remains orchestrator-owned and is not performed in this branch. |
| Revision-23 repair baseline | PR #123 head `d921d6cdd58db856df3b8eacef908c11cf46ed25` | Six-path design-only PR is green and reviewed but held after human-floor changes requested. Its bundle, publication, review, and floor evidence are stale for the relocation repair. |

At revision-23 plan freeze, the live issue #12 body is the previously approved
publication revision 1: 7,059 canonical bytes with SHA-256
`a7857aebd125e94c487b2ddac6e807f5dc9df7a4d934c2dd9277268c9093e14e`.
It remains live while planning and Gate 2 run. The repair must not edit issue
#12 until a new publication revision passes the complete Gate 5 sequence.

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

Every T-A operational approval or review request uses the literal URI address
and first verifies that the target is attended. For PR #123 only, the human
operator directly instructed Application Client orchestration to use the exact
durable address `operator:rob`; campaign accepted that task-specific override
in decision `904` after request `902`, with the direct instruction durably
recorded by Application Client decision `901`. Operator feedback `908` is only
the documentation-placement concern and is not routing provenance.
`operator:rob` is not a T-A URI, must not be expanded or rewritten, and does
not establish a rule for any other PR or future floor. It is direct durable
operator routing and is not modeled as a Copilot `attended_push` station.

| Role | Exact Telex address |
|---|---|
| Application Client workstream | `telex://lossyrob/telex/T-A:application-client-orch` |
| Campaign | `telex://lossyrob/telex/T-A:campaign-orch-devbox` |
| Operator Station consumer | `telex://lossyrob/telex/T-A:operator-station-orch` |
| Telex Watcher consumer | `telex://lossyrob/telex/T-A:watcher-orch` |
| Paired reviewer | `telex://lossyrob/telex/T-A:app-client-review-118` |
| Human merge floor for PR #123 (workstream-owned sender) | `operator:rob` |

## Bounded Human-Attention Routing

The worker never sends an `attention.*` message or sends directly to
`operator:rob`. It sends authoritative evidence, blocker, decision, and
lifecycle packets to Application Client and campaign orchestration. Application
Client orchestration independently verifies those packets and owns the bounded
human-value routing to exact `operator:rob` for PR #123; campaign mediates the
human disposition. Historical `attention:rob` routing evidence is superseded
for this task-specific floor and does not carry forward.

Workstream-owned human-attention milestones are:

- PR opened;
- review-ready;
- current-head approved;
- merge-floor;
- merged or closed;
- node/workstream completion and reconciliation;
- current blockers and `decision-needed` items;
- material blocker changes.

`attention.merge-floor` is disposition-required. Other workstream-authored
attention milestones use the campaign profile appropriate to the event and
preserve the originating worker packet IDs. The workstream does not route every
CI transition, detector attempt, polling result, duplicate status, unchanged
blocker, or low-value progress tick. The approval ledger and field report
distinguish worker-authored T-A packets from workstream-authored human-attention
messages.

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

The final non-normative traceability note records the per-row source mapping,
rationale, disposition history, provenance, owner, and downstream blocking
evidence. `docs/design/application-client.md` is the sole normative semantic
authority. This matrix prevents Gate 1 and Gate 2 from dropping a source
requirement before that note exists. Every mapping must demonstrate that each
semantic facet is preserved by normative contract clauses; the note does not
create or modify those semantics.

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

Create `docs/notes/application-client/requirements-crosswalk.md` with one row for every
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

The crosswalk is durable, non-normative traceability/provenance. It explains
how immutable source requirements map to the normative clauses in
`docs/design/application-client.md`, but it does not itself define intended
system behavior. `docs/design/application-client.md` is the sole normative
semantic authority. The contract, ADR, and design index must label this
boundary consistently.

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
2. `docs/notes/application-client/requirements-crosswalk.md`
   - complete Watcher and Operator requirement dispositions;
   - historical issue #12 proposal dispositions and blocking effects;
   - explicitly non-normative requirements traceability/provenance;
   - a header naming the immutable source freeze, manifest membership, and the
     rule that any byte change requires regenerating
     `docs/design/application-client.bundle.json` and rerunning affected exact
     approval gates. This distinguishes tracked evidence from informal
     spike/research notes elsewhere under `docs/notes/`.
3. `docs/design/history/application-client-issue-12-original.md`
   - exact canonical bytes of the pre-convergence issue #12 body;
   - historical evidence only, not normative contract authority.
4. `docs/design/index.md`
   - normative Application Client entry;
   - link the note only as supporting non-normative traceability/provenance from
     the Application Client entry, or from a clearly separate Supporting
     Traceability section;
   - do not add the note as an independent design-layer document or numbered
     normative Reading order item;
   - remove the current Reading order wording that requires the crosswalk to be
     read alongside the contract;
   - preserve the Scope note that `docs/design/` is the intended-system design
     layer and make clear that the linked evidence lives outside it under
     `docs/notes/`.
5. `docs/design/DECISIONS.md`
   - ADR 0049, using campaign allocation request `1812`, handled disposition
     `909`, and allocation response `1817`.
6. `docs/design/application-client.bundle.json`
   - canonical manifest for the approved Application Client-owned repository
     content.

The repair also updates:

- `docs/design/application-client.md` so its authority language points to the
  non-normative note and states that the note does not define semantics;
- ADR 0049 consequences so the traceability link uses the new path and
  classification;
- the exact issue #12 publication body so its immutable crosswalk link uses the
  new path at the newly approved bundle source head;
- the manifest generator, membership, path ordering, and closure checks.

After `commit-and-clean`, the final PR #123 diff against `main` must contain
exactly these six paths:

1. `docs/design/DECISIONS.md`;
2. `docs/design/application-client.bundle.json`;
3. `docs/design/application-client.md`;
4. `docs/design/history/application-client-issue-12-original.md`;
5. `docs/design/index.md`;
6. `docs/notes/application-client/requirements-crosswalk.md`.

`docs/design/application-client-crosswalk.md` and all `.paw` paths must be
absent from the final PR diff. Existing PR #123 is updated in place; no duplicate
PR is created.

Temporary PAW planning, review, publication, and evidence artifacts remain
under `.paw/work/application-client-contract-118/` while gates are active and
are removed from the final PR according to `commit-and-clean`.

Temporary evidence includes exact `inputs/issuecomment-*.md` source snapshots;
`publication/issue-12-body.pre.md` and the approved replacement body;
`approvals/ledger.json`; `evidence/` records for repairs, publication,
reviewer identity, and checkpoint-revalidation mode; and
`tools/canonicalize.ps1`.

The approval ledger is append-only by gate attempt and records gate, request
kind, plan revision or source head, digest, approver address/principal, request
and approval message IDs, timestamp, digest-reproduction attestation, and any
superseding attempt. Before `commit-and-clean`, export an exact evidence
snapshot for worker-local recovery and send its digest and complete approval
inventory to Application Client and campaign orchestration. The durable
retrieval record is the resulting Telex delivery plus its acknowledgment and
ledger message ID; the local snapshot is not approval evidence.

## ADR 0049 Allocation Authority

Campaign allocation request `1812`, handled disposition `909`, and allocation
response `1817` are the sole authority for ADR 0049:

`One API-neutral Application Client contract governs explicit station
capabilities and forbids private fallbacks`.

The stable decision slug is `application-client-semantic-boundary`; the
allocation base is `7a568c43413fc7aeab6a484b07dce0f0db11d68f`, where both
main and the allocation ledger high-water were 0048. Workstream reconciliation
at `f6e0deec043308971029ddefc50411ee455fd27a` records ADR 0049 as reserved
and not landed.

The allocation ledger's revision-19 use-condition is a frozen historical
annotation from before revision 19 was superseded without an approval request.
Revision-20 approval satisfied the prior repair cycle and revision-21 campaign
approval `916` covered rejected bytes; both are stale for this path relocation.
For current consumption, revision-23 dual exact-plan approval supersedes the
revision pin; latest-main and allocation-ledger/high-water revalidation remain
mandatory. Campaign decision `735` and revision authorizations `736` and `913`
designate `telex://lossyrob/telex/T-A:campaign-orch-devbox` as the current
custodian for decisions formerly routed to
`telex://lossyrob/telex/T-A:campaign-orch`. The historical sender address stays
immutable, and the ADR authority remains the message-ID tuple `1812`, `909`,
`1817`.

After dual exact-plan approval and immediately before editing
`docs/design/DECISIONS.md`, refresh remote `main`, re-read the campaign
allocation ledger/evidence, and reverify that ADR 0049 is still unconsumed,
reserved to the same decision title/slug, and that no unexpected high-water
movement or competing reservation exists. Use ADR 0049 only. Do not request
another number or a `no-adr-required` decision. On collision, invalidation,
unexpected high-water movement, or ambiguity, hold without editing and ask
campaign orchestration for an explicit disposition that references request
`1812`, disposition `909`, and response `1817`; never renumber, reuse, omit, or
silently reallocate the ADR.

## Canonical Contract Bundle

`docs/design/application-client.bundle.json` will be written as UTF-8 without a
BOM, LF line endings, and exactly one trailing LF. Its schema is:

- `schemaVersion`: `1`;
- `checkpointScope`: `design-only`;
- `adrAllocation`: ADR number, decision slug, request message ID, allocation
  title, disposition ID, allocation response ID, allocation base commit,
  pre-allocation high-water, and `reserved-not-landed` status;
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
`schemaVersion`, `checkpointScope`, `adrAllocation`, `sourceProvenance`,
`historicalIssue12`, `files`. `adrAllocation` uses fixed key order `number`,
`title`, `decisionSlug`, `requestMessageId`, `dispositionId`,
`responseMessageId`, `allocationBaseCommit`, `allocationHighWaterBefore`,
`status`. Each
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

The file set includes the normative design, non-normative traceability note,
design index, allocated ADR contribution, and exact historical issue #12
artifact. The traceability entry uses
`docs/notes/application-client/requirements-crosswalk.md`; its presence in the
bundle records exact traceability evidence and does not make it normative.
The manifest schema remains version 1; only file membership/path identities
change. The bundle excludes `.paw`, shared Streamliner artifacts, the
replacement issue body, review output, and generated transport evidence.

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
- the complete non-normative requirements traceability link at
  `docs/notes/application-client/requirements-crosswalk.md` and disposition
  totals;
- accepted, deferred, and rejected items with owners and blocking effects;
- the exact meaning of `application-client-ready`;
- implementation work that remains;
- explicit rejection of CLI parsing, raw daemon IPC, spike helpers, and
  product-private clients as fallback seams.

Before replacement, fetch the existing issue #12 body (publication revision 1)
into `publication/issue-12-body.rev1.pre.md`, canonicalize it, and record its 7,059
byte / `a7857aebd125e94c487b2ddac6e807f5dc9df7a4d934c2dd9277268c9093e14e`
identity as the repair pre-body. Do not copy it over
`docs/design/history/application-client-issue-12-original.md`. That historical
artifact must remain the exact 16,175-byte pre-convergence body with SHA-256
`c0a5fed4a1fa894ccc6accedee3e0e66af318a10df4ad27e6f2f065f6881c3dc`,
remain in the candidate manifest, and remain linked from the
replacement issue body as durable historical input. Publication revision 2
must replace only the crosswalk location/classification and immutable repaired
source identities needed by this path move; it must preserve the accepted
contract semantics, 30/0/0 disposition totals, historical snapshot link, and
design-only checkpoint meaning.

The revision-1 publication bytes remain durably addressable after cleanup via
source head `ee999dab46b0a8a67f09f3a4bdf6b8203ee21c2d` and blob
`89f22602d61e77464a98a224ad2505708a2e0f9c`. Revision-2 publication approval
and evidence records must cite those identities in addition to the repair
pre-body digest.

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
   - model: `claude-opus-4.8-high`;
   - interaction: `parallel`;
   - perspectives: `premortem`, `retrospective`;
   - perspective cap: `2`.
   The review must verify that the task-specific PR #123 human-floor
   destination bytes are exactly `operator:rob`, follow Application Client
   decision `901`, decision request `902`, and campaign acceptance `904`, and
   are never normalized to a T-A URI. It must verify that this is direct durable
   operator routing: no Copilot daemon-member/push-health precondition, a
   `delivered` or `queued-unoccupied` send receipt creates a pending request,
   only an explicit direct human reply/disposition can approve it, and a send
   error blocks the floor. It must also verify that only Application Client orchestration
   authors bounded human-attention milestones and that the worker sends only
   authoritative T-A evidence/status packets. Before human approval, only
   `merge-floor-ready` / `node-merge-floor-ready` are permitted with
   `humanFloorStatus: pending` and `mergeAuthorized: false`; `merge-ready` /
   `node-merge-ready` are post-human only.
   It must additionally verify that the crosswalk path is exactly
   `docs/notes/application-client/requirements-crosswalk.md`, the file is
   consistently characterized as non-normative traceability/provenance, no
   `docs/design/application-client-crosswalk.md` artifact remains, the
   manifest schema stays version 1 while membership/path identities change,
   and every invalidated Gate 3 through Gate 7 approval is rerun.
4. Resolve every blocking planning finding.
5. Re-canonicalize and commit the exact reviewed `Plan.md`.

Before Gate 2, record the final revision-23 byte length and digest in the
approval ledger. Any later byte change makes that Gate 1 identity stale and
requires canonicalization, both configured perspective reviews, and a new
planning-gate record before approval is requested. A byte change after a Gate 2
request requires a new plan revision and dual reapproval.

If the pinned model is unavailable, do not substitute silently. Request a
campaign-orchestration disposition for a successor model, revise
`WorkflowContext.md` and this plan, and repeat the affected exact-plan approval.

### Gate 2: External Exact-Plan Approval

Commit the reviewed plan, calculate its SHA-256 from the exact byte definition
at the top of this document, and send the plan revision, plan source-head
commit, digest, and full exact bytes separately to:

1. `telex://lossyrob/telex/T-A:application-client-orch`.
2. `telex://lossyrob/telex/T-A:campaign-orch-devbox`.

Each Telex message will use a subject identifying the plan review,
`next-checkpoint` attention, required disposition, metadata containing plan
revision, artifact path, plan source head, and lowercase SHA-256, and a
body loaded directly from the reviewed `Plan.md`.

Before sending, calculate the complete Telex payload size. If the full plan
cannot fit below Telex's IPC cap, send the plan source head, the immutable
Git blob ID and path for `Plan.md`, its SHA-256, and retrieval instructions
instead. An approver must retrieve that exact blob, reproduce the SHA-256, and
attest both facts; digest-only approval is forbidden.

Durable relocation edits begin only after both recipients approve revision 23
and the same digest. Approval records include the approving Telex
address/principal, message ID, timestamp, and exact-byte attestation and are
single-use for that revision. Revision-20 approvals `748` and `749`, revision-21 campaign approval `916` /
rejection `917`, and revision-22 requests `919`/`920` / rejection `921` remain
historical and do not authorize this repair. A byte change invalidates both
revision-23 approvals.

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
2. Revalidate the existing ADR 0049 allocation evidence (`1812`, `909`, `1817`),
   its title/slug/base, latest `DECISIONS.md`, and the campaign allocation
   ledger/high-water before any decision-log edit. On collision, invalidation,
   unexpected high-water movement, competing reservation, or ambiguity, hold
   for explicit campaign disposition; do not request or choose another number.
3. Move the crosswalk to
   `docs/notes/application-client/requirements-crosswalk.md`; update the
   contract authority/link wording, ADR 0049 consequences, design index
   classification/link, manifest generator and membership, exact issue #12
   publication revision 2, and all path-based verification evidence. Keep the
   historical snapshot under `docs/design/history/`.
4. Commit the candidate.
5. Generate and commit the canonical bundle manifest.
   Verify bundle closure across the union of Application Client-owned
   `docs/design/**` paths and
   `docs/notes/application-client/**` paths changed from the original planning
   base. That set must equal the manifest's listed paths plus
   `docs/design/application-client.bundle.json`. The old
   `docs/design/application-client-crosswalk.md` path must be absent.
   Unexpected paths block approval until removed, assigned to another owner, or
   deliberately added to the manifest and traceability rationale.
   Run a repository-wide literal search for
   `docs/design/application-client-crosswalk.md`. The only permitted hits are
   historical/supersession evidence under `.paw`; no durable repository path may
   reference the old location. Any out-of-scope durable hit is escalated to both
   orchestrators before approval rather than left dangling.
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
`telex://lossyrob/telex/T-A:campaign-orch-devbox`. Require both to approve the same
source head and bundle digest, record approver identity and message metadata,
and require each approver to attest retrieval and digest reproduction. Internal
PAW review and consumer approval do not substitute for this gate.

### Gate 5: Exact Issue #12 Publication Approval

Before requesting publication approval:

1. commit and push the new bundle source head and publication source head to
   `origin/feature/app-client-contract-118`;
2. verify both commit SHAs resolve through the GitHub API;
3. verify every embedded immutable `blob/<sha>/...` link, including
   `docs/notes/application-client/requirements-crosswalk.md`, resolves and
   reproduces the expected Git blob;
4. record the remote ref/head, commit lookups, link/blob results, and
   reachability timestamp in the ledger.

Send the complete exact publication bytes separately to
`telex://lossyrob/telex/T-A:application-client-orch` and
`telex://lossyrob/telex/T-A:campaign-orch-devbox` as disposition-required
`application-client-contract-publication-review-requested` messages. Include
publication revision, artifact path, and SHA-256. Require both approvals for the
same revision and digest, record approver identity and message metadata, then
perform the pre-body snapshot, re-read, on-disk preflight, post, fetch,
canonicalize, and digest verification sequence.

Immediately before `gh issue edit`, repeat the commit/link reachability check.
Any missing commit or link blocks publication without changing the approved
bytes.

Before sending, calculate the complete Telex payload size. If the exact
publication body cannot fit below the IPC cap, commit the canonical publication
file to the candidate source head and send its immutable Git blob ID, path,
SHA-256, and retrieval instructions. Each approver retrieves and hashes that
exact blob before attesting; digest-only approval is forbidden.

### Gate 6: Final PAW Review and PR

1. Verify all implementation work TODOs are complete.
2. Run repository documentation checks applicable to changed Markdown and JSON.
   Include a repository-wide literal search proving no durable reference to
   `docs/design/application-client-crosswalk.md` remains, and diff all three
   `docs/design/index.md` touch points against the base.
3. Run the configured non-interactive society-of-thought final review over the
   branch diff with the same specialist, model, interaction, and perspectives
   as planning review.
4. Resolve all blocking findings and rerun every invalidated consumer,
   shared-bundle, and publication gate after any semantic or byte change.
5. Classify each repair in the approval ledger. Any normative-clause change,
   bundle-listed file byte or path change, crosswalk
   disposition/rationale/classification change,
   manifest membership change, or checkpoint-scope change is semantic. A
   publication-only byte change reruns publication approval; a bundle-listed
   byte change reruns consumer and shared-bundle approval. Batch all known
   repairs before re-requesting approval, but do not create an editorial bypass
   from exact-digest approval.
6. Export the durable evidence snapshot and send its digest and approval
   inventory to Application Client and campaign orchestration.
7. Use `paw-pr` for artifact cleanup, selective staging, and push to the
   existing `feature/app-client-contract-118` branch. Refresh existing PR #123;
   do not open a duplicate PR.
8. Use `Closes #118` only if every contract, checkpoint, publication, approval,
   and evidence gate is complete.

### Gate 7: Paired Review and Merge-Floor Revalidation

Before paired review, the worker sends authoritative `pr-opened` evidence to
Application Client and campaign orchestration. Application Client orchestration
owns any corresponding human-value PR-opened message to exact `operator:rob`
for PR #123.

1. Verify the configured reviewer address is attended, then after push, green
   CI, and clean mergeability send `review-ready` to
   `telex://lossyrob/telex/T-A:app-client-review-118`. If it is unattended, notify Application Client
   and campaign orchestration and hold rather than selecting an unapproved
   fallback reviewer. Send matching review-ready evidence to Application Client
   and campaign orchestration; the workstream owns any human-attention
   review-ready milestone.
2. Read every real GitHub review and require a verified `PAW Review: +1` for the
   current head. The assigned paired reviewer posts that current-head GitHub
   review after its Telex review request; a local SoT artifact is not a
   substitute. Record the reviewer Telex address, GitHub identity, approval
   message ID, and reviewed commit. Send current-head approval evidence to
   Application Client and campaign orchestration; the workstream owns any
   human-attention approved milestone.
3. Treat semantically relevant review repairs as bundle/publication changes and
   repeat the affected exact-digest gates.
4. Verify green CI, clean mergeability, exact PR head, and zero unresolved
   review threads.
5. Immediately before floor readiness, run the canonical terminal PR-state
   check. Merged or closed PRs stop the floor sequence.
6. Use the campaign-approved non-background `checkpoint-revalidation` contract
   from decision-needed `885` and decision response `886`. Record the exact
   check timestamp, PR head, checks, mergeability, unresolved-thread count,
   review identity, bundle/publication identities, and recipient health.
   Set `sentryMode: checkpoint-revalidation`, `sentryActive: false`,
   `watcherUnavailable: true`, and `loopFallbackDeclined: true`. Do not start a
   background lifecycle waiter, `telex wait`, push-to-pull fallback, or polling
   task. The production Watcher remains unavailable until this checkpoint is
   merged, and the background Loop fallback previously harmed Telex push
   coordination.
7. After a successful one-shot check, the worker sends one
   disposition-required `merge-floor-ready` packet containing the full
   technical/checkpoint/revalidation
   evidence to Application Client orchestration and a
   `node-merge-floor-ready` status to campaign orchestration, then holds. Both
   packets carry `humanFloorStatus: pending` and `mergeAuthorized: false`.
   Neither packet is named `merge-ready`. The evidence contains exact PR URL
   and head, current `PAW Review: +1`, CI conclusion, mergeability,
   unresolved-thread count, checkpoint evidence, bundle and publication
   identities, and the checkpoint-revalidation contract/evidence.
8. Application Client orchestration independently rechecks that packet and is
   the only owner authorized to send the disposition-required
   `attention.merge-floor` request to exact `operator:rob`. Application Client
   decision `901`, decision request `902`, and campaign decision `904` make
   this the task-specific destination for every fresh PR #123 evidence packet
   unless the human operator explicitly revokes it. The override is not a
   global rule for another PR. Immediately before sending, the workstream
   queries and records Telex status on backend `pg-rde-telex` with
   `--address operator:rob` for audit, but does not require a daemon member,
   push delivery mode, station health, push registration, or backlog fields.
   It rejects any normalized or expanded address, then sends exactly one
   request. A successful `delivered` or `queued-unoccupied` Telex receipt is a
   valid pending human-floor request; `queued-unoccupied` is not approval and
   requires waiting for the direct human reply/disposition. A send error is a
   hard blocker that is reported to campaign and the worker. The workstream
   records the status snapshot, exact send receipt, and eventual direct human
   reply/disposition. It does not send a duplicate request for the same
   evidence packet. The worker records the request and evidence but never
   authors the request.
9. Campaign orchestration mediates the explicit human disposition. Campaign
   technical approvals, recommendations, or prior PR #115/#116 outcomes never
   satisfy this floor.
10. Any campaign merge authorization must cite both the workstream-authored
    `attention.merge-floor` request message ID and the human approval disposition
    ID. An authorization missing either reference is non-authoritative, and the
    PR remains held.
11. No background sentry runs while the workstream and human review are pending.
    Human approval is single-use for the exact evidence packet. Application
    Client reruns the complete terminal check immediately before authoring the
    floor request. Campaign reruns it before accepting a human disposition and
    before merge authorization. Any PR head change, new review, pending or
    failing check, mergeability change, unresolved-thread change, checkpoint
    drift, bundle drift, publication drift, or recipient-health change
    invalidates the floor. The worker sends a fresh `merge-floor-ready` packet;
    the workstream must independently recheck and author a new floor request,
    and campaign must obtain a new human disposition.
12. After human-floor approval and authoritative campaign authorization, the
    worker sends the disposition-required `merge-ready` field report to
    Application Client orchestration and `node-merge-ready` to campaign
    orchestration. Both cite the workstream-authored floor request ID, human
    disposition ID, and campaign authorization ID and carry
    `humanFloorStatus: approved` and `mergeAuthorized: true`.
13. Do not merge the PR. Hold it healthy for orchestration.
14. After merge, send one disposition-required `reconciliation-requested` packet
   to Application Client orchestration and concise `node-merged` status to
   campaign orchestration. Require durable received/disposition confirmation;
   if it remains unconfirmed, escalate the original message ID to campaign
   orchestration without sending a duplicate reconciliation request. Keep the
   checkpoint/evidence record until confirmation or explicit campaign
   disposition.
   Application Client orchestration owns human-attention merged/closed,
   node/workstream completion, and reconciliation milestones.
15. Campaign orchestration performs a post-merge immutable-link durability
   check before branch deletion or closure: every issue #12 permalink must
   resolve and reproduce the manifest-listed blob. If squash merge leaves the
   approved source head outside `main`, retain the PR commit as the frozen
   checkpoint and record the successful link check; if a link fails, mark the
   publication stale and run the approved supersession process.
16. The same campaign-owned post-merge check recomputes
   `docs/notes/application-client/requirements-crosswalk.md` and
   `docs/design/application-client.bundle.json`, requires the note's
   byteLength/SHA-256 to match the manifest entry, and confirms the issue #12
   supporting-traceability link resolves to that exact note. Any mismatch marks
   the checkpoint stale and starts the supersession process.

At every stage, the worker sends current blockers, `decision-needed` evidence,
and material blocker changes to Application Client and campaign orchestration.
Application Client orchestration decides whether they warrant human-attention
routing. Unchanged blockers and low-value progress ticks are not repeated.

## Approval and Repair Accounting

Approval freshness is verified against the current Telex message, source head,
and digest at each gate. An approval is never replayed for a newer revision even
when the digest happens to match.

Repairs are coalesced before requesting another review cycle. Exact-byte
approval remains mandatory for every affected artifact; whitespace,
punctuation, and non-normative prose do not receive a silent exemption.

Revision 23 invalidates every prior approval whose identity includes the plan,
bundle membership/path, publication links, PR head, review, or floor packet:

- revision-20 Gate 2 approvals `748` and `749`;
- revision-21 requests `914`/`915`, campaign approval `916`, and Application
  Client rejection `917`;
- revision-22 requests `919`/`920` and Application Client rejection `921`;
- consumer approvals `813` plus supplements `817`/`820`, and `814`;
- shared-bundle approvals `823` and `824`;
- publication approvals `829`, `832`, and `834`, plus live publication
  verification `837`;
- Gate 6 review/evidence acceptances `840` and `841`;
- PR head review evidence `863`, merge-floor packets `890`/`891`, acceptances
  `893`/`894`, and human-floor request `897`.

The ledger preserves these as historical/superseded records. Revision-23 Gate 2
must complete before any durable path edit. After the repair, Gates 3 through 7
rerun in order against new exact identities. The currently published issue #12
body remains visible but is stale for the repaired path until publication
revision 2 is approved, posted, fetched, and verified.

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

The ledger records the worker-authored `merge-floor-ready` evidence packet ID
and evidence digest, the workstream-authored `attention.merge-floor` request ID,
human approval disposition ID, and campaign merge-authorization message ID.
The authorization record is valid only when it cites the workstream request and
human disposition for the same exact evidence identity. The worker never
appears as sender of `attention.merge-floor`; no technical approval or
recommendation is recorded as a human-floor substitute. Invalidation records
identify the changed evidence axis and superseding packet/request/disposition.
The floor-request record also captures the exact destination, pre-send status
snapshot (including absent optional station fields), Telex send receipt
(`delivered` or `queued-unoccupied`), direct human reply/disposition evidence,
and confirmation that no duplicate request was sent.
After human approval, the ledger separately records the worker-authored
`merge-ready` and `node-merge-ready` message IDs, their
`humanFloorStatus: approved` / `mergeAuthorized: true` metadata, and the three
authorization evidence IDs they cite. Pre-human and post-human messages are
never conflated.
For bounded human-attention routing, the ledger links each worker-authored T-A
milestone/blocker packet to any workstream-authored `operator:rob` message for
PR #123 and
records the routing category. It records no human-attention row for excluded
CI transitions, detector attempts, polling results, duplicate status, unchanged
blockers, or low-value ticks.

## Work Items

1. **Freeze and approve the plan**
   - Internal SoT review, exact-byte digest, and dual orchestration approval.
2. **Author and commit the candidate contract**
   - Normative design, complete non-normative traceability note, index,
     campaign-allocated ADR 0049, downstream decomposition, publication draft,
     and manifest.
3. **Obtain exact consumer and shared-bundle approval**
   - Operator and Watcher approval, then Application Client and campaign
     approval for one source head and bundle digest.
4. **Publish and verify issue #12**
   - Dual exact-byte approval, source re-read, `--body-file` publication, and
     fetched-body digest verification.
5. **Complete PAW final review and final PR**
   - Resolve review findings, clean PAW artifacts, push, open the final PR, and
     ensure CI and mergeability.
6. **Complete paired review and checkpoint-revalidation handoff**
   - Current-head `PAW Review: +1`, zero unresolved threads, explicit human
     merge-floor approval requested only by Application Client orchestration,
     campaign authorization citing both floor IDs, worker-authored
     full `merge-floor-ready` report, campaign-approved non-background
     checkpoint revalidation throughout human review, and post-merge
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
- approving addresses/principals, timestamps, and digest-reproduction
  attestations;
- source-comment snapshot digests and the pre-publication issue #12 digest;
- exact checkpoint meaning and downstream node recommendations;
- design-index and ADR impact;
- ADR 0049 authority evidence: request `1812`, handled disposition `909`,
  allocation response `1817`, title, slug, allocation base, latest-main
  collision check, and allocation-ledger/high-water check;
- old issue #12 proposal elements accepted, replaced, deferred, or rejected;
- risks, boundary pressure, and explicit no-private-fallback confirmation;
- repair/reapproval cycle counts and checkpoint-revalidation mode evidence;
- worker-authored `merge-floor-ready` evidence packet ID and exact evidence
  digest covering PR/head, review, CI, mergeability, threads, checkpoint,
  bundle, publication, and revalidation state;
- post-human worker-authored `merge-ready` and `node-merge-ready` message IDs
  with `humanFloorStatus: approved`, `mergeAuthorized: true`, and all three
  authorization evidence IDs;
- workstream-authored `attention.merge-floor` request ID, explicit human
  approval disposition ID, and campaign merge-authorization ID with both
  required citations;
- exact `operator:rob` pre-send status snapshot, send receipt outcome, and
  direct human reply/disposition evidence for the workstream-authored floor
  request;
- confirmation that the worker recorded but did not author the human-floor
  request;
- any floor invalidation axis and the superseding packet, workstream request,
  and human disposition IDs;
- worker T-A packet IDs and workstream-authored `operator:rob` message IDs for
  PR opened, review-ready, approved, merge-floor, merged/closed,
  completion/reconciliation, blockers, decisions, and material blocker changes;
- confirmation that excluded CI/detector/polling/duplicate/low-value events were
  not routed to human attention;
- confirmation that campaign technical approval and PR #115/#116 were not used
  as merge authorization;
- process feedback for PAW, Telex, paired review, and checkpoint revalidation.
- a downstream recommendation for a lightweight CI/main manifest-reproduction
  check so future edits to
  `docs/notes/application-client/requirements-crosswalk.md` cannot silently
  desynchronize `docs/design/application-client.bundle.json`.

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
4. ADR 0049 uses the sole campaign allocation authority (`1812`, disposition
   `909`, response `1817`); the frozen title/slug/base match, and latest-main
   plus allocation-ledger/high-water revalidation passed. No alternate number
   or no-ADR determination was requested.
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
10. The worker delivered a disposition-required `merge-floor-ready` evidence
    packet containing the full technical/checkpoint/revalidation evidence with
    `humanFloorStatus: pending` and `mergeAuthorized: false`, with no background
    sentry or waiter active, then held. Application Client orchestration
    independently rechecked it and authored the disposition-required
    `attention.merge-floor`
    request to exact `operator:rob` after recording a direct-address status
    snapshot and obtaining a successful `delivered` or `queued-unoccupied`
    send receipt without normalization; the worker recorded but did not author
    that request. An explicit direct human reply/disposition exists before the
    human approval is accepted.
11. The workstream-authored floor request has explicit human approval routed by
    campaign, and any campaign merge authorization cites both that request ID
    and the human approval disposition ID. Technical campaign approval alone and
    PR #115/#116 are not precedent or substitutes. The approval is single-use
    for the exact evidence identity; any head, review, CI, mergeability, thread,
    checkpoint, bundle, publication, or recipient-health change has a fresh packet,
    workstream request, and human disposition.
12. The canonical field report and approval inventory have durable Telex
   retrieval records. Only after human approval and authoritative campaign
   authorization are `merge-ready` and `node-merge-ready` delivered with
   `humanFloorStatus: approved` and `mergeAuthorized: true`; exactly one
   checkpoint-revalidation mode remains enforced through repeated terminal
   checks by the worker, Application Client orchestration, and campaign.
13. Required human-value milestones and material blocker/decision changes are
    routed to exact `operator:rob` for PR #123 only by Application Client
    orchestration,
    with worker T-A provenance preserved. Excluded CI transitions, detector
    attempts, polling results, duplicate status, unchanged blockers, and
    low-value ticks are not routed.
14. The worker has not merged the PR. After orchestration merges it, exactly one
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
- Merge authorization includes workstream-owned human-floor evidence; the
  worker never authors the floor request, and technical campaign approval alone
  never authorizes merge.
- The workstream floor request uses exact `operator:rob` for PR #123, records a
  fresh status snapshot without imposing Copilot push-health requirements,
  records a successful `delivered` or `queued-unoccupied` send receipt, avoids
  duplicates, and waits for an explicit direct human reply/disposition.
- Application Client orchestration, not the worker, owns the bounded
  human-attention milestones and blocker routing; excluded low-value events are
  not sent to `operator:rob`.
- Human-floor approval is single-use for the exact pre-human
  `merge-floor-ready` evidence and is invalidated and reacquired after any
  listed evidence change.
- The PR is not merged by this worker.
