# Plan - Production Operator Station Contract

- **Issue:** lossyrob/telex#114
- **Workstream / node:** operator-station / station-contract
- **Plan revision:** 3
- **Base / target:** `main` -> `feature/production-station-contract`
- **Outcome:** Publish an accepted production Operator Station and mediated-attention
  contract that is precise enough to detail `station-app` and `operator-broker`,
  while exporting shared Application Client requirements to issue #12.

## Approach

Create a normative `docs/design/operator-station.md` contract that promotes the
accepted spike and dogfood lessons without promoting the experimental runtime,
namespace, campaign metadata, or UI. The document will define the product and
application-layer behavior, identify the Telex core semantics it relies on, and
separate Station/operator requirements from the shared Application Client seam.

Record two load-bearing candidate decisions separately in the decision log:

- Operator Station is an optional application-layer Station plus operator-agent
  role; Telex core does not acquire human UI, semantic routing, or workflow
  execution.
- Direct and assisted routing use one exclusive ingress registration at a time;
  transition sequencing is application-owned while exclusivity, epoch fencing,
  durable queueing, and membership are daemon-owned.

The detailed product rules remain in the normative Station design.
Do not reserve ADR numbers during planning because Watcher contract node #110
is editing the same design layer concurrently. Allocate the next available
numbers only after synchronizing with the latest shared design head and
inspecting #110's current/final decision-log and index changes.

Publish a GitHub comment on issue #12 containing only the exact shared semantic
requirements needed by Operator Station. It will not propose package names,
language bindings, wire formats, or a competing public API.

## Contract decisions to resolve

The production design will make the following choices explicit:

1. **Attendance and human availability:** Station registration, receive/ack
   health, operator-agent occupancy, and human availability are separate facts.
   Queueing or application occupancy never implies that a human saw a message.
2. **Routing modes and address configuration:** direct and assisted are routing
   topologies. Direct mode gives the Station exclusive ingress attendance.
   Assisted mode gives the operator agent exclusive ingress attendance and the
   Station a distinct configured human address. Quiet is an assisted-mode policy
   posture, not a third occupancy topology; direct+quiet is therefore not a
   production mode. The issue uses "routing modes" as an umbrella term; the
   contract will call out this refinement rather than silently changing the
   charter vocabulary. Ingress and human addresses are explicit deployment
   configuration, must be distinct in assisted mode, and are not derived from
   the spike's `attention:rob` / `operator:rob` examples.

   A direct/assisted transition composes existing daemon semantics rather than
   defining a new Telex handoff protocol: the old session performs
   `Detach`/`station stop`, the application verifies through station status that
   the old registration no longer owns the ingress address, and the new session
   performs `Register`/attach. A collision fails closed. Messages may queue
   durably during the unoccupied gap. The design will cite `DESIGN.md` address
   lifecycle and lease collision plus `daemon.md` sections 4, 5, 11.2, and 14;
   daemon section 11.4's daemon-upgrade handoff is not reused as a session mode
   transition.
3. **Operator authority:** the operator agent may resolve, clarify, aggregate,
   recommend, escalate, route back, and disposition within its assignment. It
   may not impersonate sources, execute arbitrary commands, mutate authoritative
   artifacts directly, or move semantic judgment into Telex core.
4. **Production extension convention:** retire the
   `operator-station-spike.*` namespace. Define extension ID
   `urn:telex:operator-station:v1`, shortname `operator-station`, and this
   Station-interpreted kind inventory:

   | Kind | Direction | Purpose | Required to process | Safe to ignore |
   |---|---|---|---|---|
   | `operator-station.escalation` | operator agent -> Station | New human obligation with recommendation and source references | yes | no |
   | `operator-station.human-reply` | Station -> operator agent | Human response requiring route-back or explicit stale-origin resolution | yes | no |
   | `operator-station.digest` | operator agent -> Station | Aggregated informational summary in quiet posture | no | yes |

   The production design will map every spike string to accepted, renamed,
   role-local, or retired status. Clarification and routed-outcome messages
   remain ordinary raw-thread replies or operator-role conventions rather than
   Station-interpreted kinds. Unknown fields inside the supported v1 envelope
   are preserved and ignored safely; an unsupported extension/version is shown
   as a feed-only raw diagnostic and never triggers automatic action. The design
   document is the stable descriptor until a general extension packaging
   mechanism is accepted. Campaign-local `attention.*` kinds and
   `campaignAttention` remain opaque source conventions: the Station may render
   their raw kind/metadata as source evidence but does not interpret them as the
   production schema or grant them a toast override.
5. **Provenance and store identity:** mediated messages are authored by the
   operator address and carry source references as `(opaque logical-store
   identity, message ID)` plus a captured display snapshot. The required store
   identity is a shared Application Client semantic: stable for the logical
   store across process/daemon restart, equality-comparable, opaque to the
   application, and free of raw paths, credentials, or connection strings. No
   current `store_key`, path fingerprint, daemon singleton, or owner instance is
   accepted as the production identity without #12 adopting that contract.
   Station source resolution is gated on this shared type. Current-store
   authoritative records, captured snapshots, and unavailable/unverified
   sources are presented as distinct trust states.
6. **Reply, disposition, and stale origins:** the default response to a human
   obligation is an explicit **Reply & Handle** operation. It is a higher-level
   Application Client requirement, not a claim that current Telex has a
   cross-message transaction. The reply must be durably accepted before the
   selected human-facing obligation becomes terminal. The new human reply
   remains a disposition-required operator obligation until one of these
   outcomes is recorded:

   - route-back is durably accepted for the source address, including durable
     queueing while that active address is unoccupied;
   - the source obligation is already terminal and the operator records that no
     route-back was needed, with a human-visible explanation;
   - the source store/message cannot be resolved, the source address is retired
     or rejects delivery, or the source has been superseded; the operator
     records a stale-origin outcome and either defers for human repair or
     terminally handles/rejects it with an explicit note according to policy.

   The design will enumerate reply-failed, reply-succeeded/handle-failed,
   handle-succeeded/reply-failed (forbidden ordering), restart-mid-operation,
   route-back-failed, operator-replaced, source-store-replaced, and duplicate
   retry states. Each state remains visible and retryable. Plain Reply, Handle,
   Defer, Reject, and Close remain explicit alternatives. The Station command
   handler is the enforcement point: **Reply & Handle** must not issue the
   terminal Handle until it has a durable reply receipt. A shared-client
   compound operation may enforce the same invariant, but `station-app` still
   fails closed if the client reports an indeterminate or partial result.
7. **Notifications:** the feed is authoritative. The design will publish a
   deterministic policy and precedence table:

   - `interrupt` + disposition-required: toast eligible and prominent feed row;
   - `next-checkpoint` + disposition-required: toast eligible by default for
     `operator-station.escalation`, otherwise actionable feed;
   - `background` + disposition-required: actionable feed and badge, no toast by
     default;
   - non-disposition `background` and all `fyi`: feed/history only;
   - `operator-station.digest`: feed only by default;
   - unsupported kinds/extensions: feed-only raw diagnostic.

   Precedence is OS/user-disabled notifications, explicit source mute, quiet
   schedule, supported-kind override, then attention/disposition default.
   `interrupt` never bypasses OS or explicit user suppression. Quiet mode changes
   operator escalation/aggregation policy but does not erase individual
   obligations. An aggregate toast may summarize many messages, but every
   obligation retains a feed row and identity. The Station records the resolved
   notification decision and submission attempt; OS-level focus suppression
   (currently Windows Focus Assist) is reported when observable and otherwise
   remains `unknown`, never inferred as human receipt. Focus Assist behavior is
   explicitly unvalidated spike carry-forward.
8. **Restart, ingest, replacement, and idempotency:** ack occurs only after
   durable application ingest, defined as a restart-replayable write of the
   message envelope, delivery role, disposition requirement, metadata, and
   receive cursor into Station-owned recovery state. Rendering, toast
   submission, or transient in-memory insertion is not ingest. Live/backfill
   merging dedupes by logical-store identity and message ID.

   Station and operator replacement reattach explicitly and rehydrate unresolved
   obligations plus bounded recent history. Operator-authored escalation,
   human-reply, route-back, and compound reply/disposition operations carry a
   retry-safe operation identity persisted across restart. The contract will
   define its wire carrier and scope without prescribing a client API; #12 owns
   the shared idempotent-operation semantic and explicit duplicate-window
   outcome.
9. **Identity, safety, and observability:** show sender address and any
   backend/authenticated principal separately. A principal is labeled verified
   only when the Application Client supplies an authenticated identity with
   provenance; otherwise it is `unverified` or `unavailable`. Cryptographically
   verified cross-principal identity remains hardening work. Display link
   destinations, restrict actionable links to local policy, and never execute
   message content, metadata, commands, custom schemes, or fetched instructions
   automatically.

   The contract will define health as separate observable axes rather than one
   success label: Station receive registration, delivery/ack backlog, operator
   ingress occupancy, source resolution, and notification posture. Each axis
   has healthy/recovering/degraded/stopped-or-unknown states, relevant counts or
   oldest-pending age, and a human-visible explanation. Application attendance
   never implies human availability.

## Work items

### 1. Author the normative Station contract

Create `docs/design/operator-station.md` with:

- scope, terminology, responsibilities, and explicit non-goals;
- direct, assisted, and quiet routing topology and transition procedure;
- operator-agent authority, raw/mediated lifecycle, and non-impersonation;
- production kinds and opaque metadata, including explicit disposition of all
  spike and campaign-local conventions, the exact v1 kind table above,
  unknown-field/version handling, and the descriptor location;
- source/thread provenance and source-availability/trust states;
- feed, thread, unresolved-history, read-state, receive/ack, and health behavior;
- the deterministic notification matrix and precedence above, aggregation
  identity/window rules, per-delivery decision observability, and OS focus
  posture limitations;
- reply/disposition operations, finite terminal outcomes, and the complete
  partial-failure state matrix above;
- restart, replacement, duplicate, delayed-reply, stale-origin, and route-back
  recovery, including durable-ingest and retry-safe operation identity;
- sender/principal presentation, safe links, and arbitrary-execution boundary;
- an accepted/deferred/rejected table for every issue open question and dogfood
  carry-forward item, with downstream owner, tracker, rationale, and impact; the
  table is an accepted-contract snapshot updated only when a later design change
  supersedes it;
- a stable observable health-state table for Station receive, ack backlog,
  operator occupancy, source resolution, and notification posture;
- a separate, exact list of shared Application Client requirements and
  downstream obligations for `station-app`, `operator-broker`, and hardening.

The document will be normative because #114 is the accepted post-viability
contract. Its Status section will state that the extension envelope uses the
current opaque-kind/metadata mechanism while the general packaging/discovery
proposal remains forward-looking; #12 remains the owner of shared client
realization.

### 2. Integrate the design layer

- The domain document `docs/design/operator-station.md` may be authored
  independently after plan approval. Before creating or editing shared
  `docs/design/index.md` and `docs/design/DECISIONS.md`, fetch the latest
  `origin/main`, synchronize this branch with that shared head, and inspect the
  active/final #110 Watcher contract diff for proposed index entries and ADR
  titles/numbers.
- Observe the current `DECISIONS.md` high-water number and `origin/main` commit,
  then send campaign orchestration a disposition-required
  `adr-allocation-requested` message containing issue/node/workstream, requested
  count `2`, the two proposed titles and one-line rationales, and the observed
  high-water commit/number.
- Wait for `adr-allocated` containing exact numbers and the campaign's base
  commit. Campaign holds those numbers until this node publishes or explicitly
  releases them. Use only the allocated numbers. If the base, high-water, or a
  sibling change conflicts, return to campaign; never renumber or overwrite
  silently. If allocation is unavailable, contradictory, or cannot remain
  collision-free, send `decision-needed` to campaign and the workstream
  orchestrator and hold shared-file editing; do not invent numbers or continue
  with an unrecorded reservation.
- Link the new contract from `docs/design/index.md` and add it to the reading
  order.
- Append separate ADRs titled **Operator Station mediation remains application
  logic outside Telex core** and **Direct and assisted routing use exclusive
  ingress attendance** using only the campaign allocation.
- Immediately before final agent review and PR readiness, fetch/rebase the
  latest `origin/main`, preserve every existing ADR and any merged #110 domain
  contract/index entry, and verify the allocation remains collision-free. If
  this is the second contract PR to integrate, perform an explicit
  cross-contract consistency pass without changing the first contract's
  accepted domain semantics.
- Record durable coordination provenance in the PR description and field
  report: final `origin/main` integration SHA, inspected #110 PR/head or merge
  SHA (or explicit "not published"), ADR allocation message ID, allocated
  numbers/base commit, and any index reconciliation performed.
- Do not edit `.streamliner` artifacts or the read-only Streamliner Desktop
  checkout.

### 3. Obtain workstream approval of the Station domain contract

After the initial `operator-station.md`, allocated ADR entries, design-index
integration, accepted/deferred/rejected matrix, and downstream obligations are
complete:

1. Run the initial consistency checks from Work Item 5, commit the complete
   initial domain design, and ensure the owned files are clean. Prepare a
   canonical JSON bundle manifest serialized as UTF-8 without BOM, sorted keys,
   no insignificant whitespace, LF line endings, and exactly one trailing LF.
   The manifest contains:
   - schema version and committed source head;
   - full path, Git blob ID (`HEAD:<path>`), and SHA-256 for
     `docs/design/operator-station.md`;
   - each allocated Operator Station ADR number/title plus the SHA-256 of that
     exact canonical ADR entry text, not the whole shared `DECISIONS.md`;
   - each Operator Station index contribution identified by stable anchor plus
     the SHA-256 of its exact canonical text, not the whole shared `index.md`;
   - ADR allocation message ID and allocation base commit.
   Compute the domain bundle digest from those manifest bytes. This scopes
   approval to Operator Station-owned content, so sibling ADR/index edits do not
   invalidate the domain contract.
2. Send a disposition-required `station-contract-review-requested` message to
   the workstream orchestrator with attention `next-checkpoint`. Include the
   complete canonical manifest and digest, current head, design/index/ADR diff,
   spike/dogfood
   evidence links, the accepted/deferred/rejected matrix, downstream
   obligations, and focused review prompts for routing modes, provenance,
   notification, Reply & Handle, recovery, identity, and safety.
3. Wait for `station-contract-approved` naming that head and bundle digest or
   address every `station-contract-feedback` point. Batch non-urgent fixes into
   one candidate head before re-requesting approval. If feedback conflicts or
   cannot be resolved within #114 authority, send `decision-needed` to the
   workstream and campaign orchestrators and hold.
4. Define the approved source anchor as
   `{ stationContractApprovalMessageId, sourceHead, domainBundleDigest }`. The
   canonical manifest is kept in immutable session-private state and included
   in the durable Telex review request; the request message ID is its durable
   audit reference.

This engagement checkpoint is the issue #114 domain-contract review. Internal
society-of-thought review does not replace it. It must complete before the #12
draft gate and before final PAW/PR review.

Later change classification is explicit and owned by the workstream
orchestrator. The design maps each shared-client-relevant section to the
numbered #12 requirement bullets below; the worker proposes a class and the
workstream approval confirms it:

- **Class A - integration-only:** sibling/shared-file movement outside the
  Operator Station ADR/index snippets, or unrelated commits. Update coordination
  provenance only; no domain or #12 reapproval.
- **Class B - editorial domain change:** owned bytes change without semantic
  meaning. Recompute the bundle and obtain batched workstream reconfirmation;
  no #12 reconfirmation.
- **Class C - domain semantic change, not shared-client affecting:** obtain
  batched workstream reconfirmation; no #12 reconfirmation.
- **Class D - shared-client affecting:** obtain workstream reconfirmation and
  the dual #12 source/comment reconfirmation below. Update the comment only when
  its bytes change.

Uncertain or disputed classification is `decision-needed` to workstream and
campaign orchestration. Reconfirmation is per candidate head/batch, not per
commit.

### 4. Export Operator Station requirements to issue #12

Draft a UTF-8 GitHub issue comment that:

- states #114 is the Operator Station source;
- lists these shared semantic requirements:
  1. stable application station identity with explicit attach, detach,
     reattach/recovery, and typed membership-loss outcomes;
  2. opaque stable logical-store identity with no path, credential, or
     connection-string exposure;
  3. multi-address lifecycle with explicit partial results and compensation;
  4. streaming/callback/async receive yielding message, delivery-role context,
     metadata, and an ack capability;
  5. ack-after-durable-ingest and observable ack-pending/deaf/backlog state;
  6. at-least-once duplicate/redelivery identity and restart-safe cursor
     semantics;
  7. unresolved-obligation query plus bounded recent/thread history without
     full-store materialization;
  8. typed send, reply, read-thread, and per-recipient disposition operations
     with explicit sender selection;
  9. a retry-safe application operation identity/idempotency semantic with an
     explicit accepted-send duplicate window;
  10. Reply & Handle and route-back compound semantics with durable ordering,
      partial-failure outcomes, and recovery handles even if implemented as
      coordinated operations rather than one backend transaction;
  11. source-reference resolution using logical-store identity + message ID,
      with authoritative/captured/unavailable trust states;
  12. lifecycle/health projection covering registration, epoch/owner,
      receive-path health, pending unconsumed, inbound actionable, ack pending,
      and detach/recovery outcomes;
  13. backend-profile selection without backend-specific message semantics,
      covering the current SQLite and credentialed Postgres implementations,
      with authenticated principal provenance when available;
  14. delta-oriented application events and explicit resync/backfill behavior;
  15. receipt identity cross-checks, bounded retry/throttling, and local scope
      discovery/cleanup;
- separates those requirements from Station-specific UX and operator policy;
- identifies spike shortcuts that are explicitly not requirements;
- avoids choosing bindings, package names, protocol framing, or API signatures.

Before posting:

1. Materialize the exact comment as an immutable UTF-8-without-BOM file in
  session-private state with LF line endings and exactly one trailing LF.
  Compute SHA-256 from those canonical file bytes and use that same file for
  both Telex request bodies and `gh ... --body-file`; do not round-trip through
  console text or regenerate the file between approval and publication.
2. Send the complete draft separately to the workstream orchestrator and
  campaign orchestrator as disposition-required
  `application-client-requirements-review-requested` messages. Both carry issue
  #114, current branch head, accepted Station design path, evidence references,
  draft revision, SHA-256 digest, the complete approved source anchor and
  canonical bundle manifest, the observed #12 comment high-water, and
  inspected #110 PR/head or merge SHA.
3. Require campaign orchestration to compare the draft with #110's
  current/final Watcher requirements and preserve #12 as the sole shared
  contract owner. The comment states that campaign/#12 convergence, not this
  domain export, accepts the eventual Application Client contract. The
  workstream orchestrator separately verifies that the draft is faithful to
  the accepted Station design and does not export Station UX/operator policy
  as shared-client semantics.
4. Publication requires `application-client-requirements-approved` from both
  orchestrators for the same revision and digest. If either sends feedback,
  resolve every point, increment the draft revision, recompute the digest, and
  request two fresh approvals. Ignore and disposition stale replies. If
  feedback conflicts, or #110 is too unstable for a bounded comparison, send
  `decision-needed` to both and hold rather than inventing a fallback.
5. Immediately before posting, re-read #12 and the inspected #110 reference. If
  either changed since the approval evidence, request reconfirmation from both
  orchestrators even when the draft bytes are unchanged.
6. Post the comment to GitHub only after the exact-text dual approval. Fetch the
  resulting body and canonicalize only its textual transport representation to
  UTF-8 without BOM, LF line endings, and exactly one trailing LF. Verify that
  canonical SHA-256 matches the approved digest and capture the issue-comment
  URL plus both approval message IDs. Any difference beyond CRLF/LF or trailing
  newline normalization is a publication mismatch: stop, report it to both
  approvers, and obtain explicit reconfirmation before replacing or accepting
  the comment. The durable Telex request bodies are the audit copies of the
  exact approved bytes; the session-private file is only the immutable
  transport source.
7. Treat the Operator Station and Watcher comments as independent domain
  requirement exports. Neither supersedes the other; campaign/#12 convergence
  resolves overlaps and accepts the eventual shared contract. A campaign
  convergence edit elsewhere on #12 does not mutate or invalidate this
  historical domain export. A requested correction to this Operator Station
  comment is a Class D change and follows the same dual-approval/update path.

Plan approval is not approval of the later #12 requirements text. The PR cannot
be finalized until this campaign seam review and GitHub publication complete.

Classify later PAW, workstream, paired-review, or PR-review repairs using the
Class A/B/C/D rules above. For Class D:

- recompute the domain bundle and obtain workstream contract reconfirmation;
- send both #12 approvers the replacement approved source anchor and request
  reconfirmation, even if the comment bytes remain unchanged;
- if the comment bytes change, increment the draft revision, obtain two fresh
  exact-digest approvals, update the existing GitHub comment, and re-verify its
  canonical digest.

Before every `review-ready`, `rereview-requested`, and `merge-ready` handoff,
recompute the current Operator Station-owned domain bundle and verify it matches
the active approved source anchor; a different current head is acceptable only
for recorded Class A changes. Verify the published #12 comment URL/body/digest
and recorded source anchor remain faithful. Record these fields in handoff
metadata and the field report: current head, approved source head, domain bundle
digest, station-contract approval message ID, #12 comment URL/digest, both #12
approval message IDs, change classification since the anchor, and fidelity
result. Stop and repair the review/publication chain on mismatch.

### 5. Validate completeness and consistency

- Cross-check every #114 success criterion, open question, and spike
  carry-forward item against an explicit design disposition.
- Check consistency with `PRODUCT-THESIS.md`, `DESIGN.md`, `daemon.md`,
  `EXTENSIONS.md`, `DISPATCH.md`, the accepted spike report, issue #12, and the
  campaign mediation convention.
- Maintain and verify an explicit mapping from each
  shared-client-relevant `operator-station.md` section/ADR decision to the
  numbered #12 requirement bullets, so Class B/C/D change classification is
  reviewable rather than subjective.
- For each contract decision, cite and verify the governing substrate:
  - attendance/health: daemon sections 4, 5, 9, 13.2, and 14; ADRs 0023, 0027,
    0031, 0039, and 0042;
  - exclusive occupancy/transitions: DESIGN address lifecycle/lease collision,
    daemon sections 5, 11.2, and 14; ADRs 0014, 0023, and 0027;
  - delivery/ack/dedup: DESIGN delivery/disposition, daemon sections 5.1,
    11.3, and 13; ADRs 0011, 0013, 0032, 0033, and 0034;
  - replies/threads: DESIGN messaging/threading and ADR 0035;
  - trust/safety: daemon section 7 and DESIGN multi-user coordination;
  - extension behavior: EXTENSIONS opaque-envelope, mandatory-to-understand,
    and versioning guidance without promoting extension parsing into core.
- Verify internal links and references, run `git diff --check`, and inspect the
  final diff. No code build is required for documentation-only changes.

## PAW and coordination gates

1. Run the configured society-of-thought planning-docs review with the
   `general-reviewer` specialist and resolve blocking findings.
2. Send the exact reviewed Plan.md bytes to both orchestrators as
   `plan-review-requested`, revision 3, with a SHA-256 digest.
3. Begin design editing only after both orchestrators approve the same revision
   and digest. Any later Plan.md byte change increments the revision and resets
   both approvals.
4. After implementation, run the configured society-of-thought final review
   with `general-reviewer` only after the Station domain-contract review and
   #12 draft publication gates complete. Resolve findings, batch and classify
   any source change using Class A/B/C/D, complete the required reconfirmation,
   then use `paw-pr`.
5. Use the Telex reviewer handshake after CI is green; verify source/comment
   fidelity before every review or merge-ready handoff; do not merge the PR.

## Expected artifacts

- `.paw/work/production-station-contract/Plan.md`
- `docs/design/operator-station.md`
- updated `docs/design/index.md`
- updated `docs/design/DECISIONS.md`
- one requirements comment on GitHub issue #12
- final PR closing #114 only if all contract and tracker requirements are met

## Main risks

- **Accidental API freeze:** mitigated by specifying semantic requirements and
  invariants, not method signatures or bindings.
- **Spike leakage:** mitigated by an explicit accepted/deferred/rejected
  inventory and production namespace replacement.
- **Ambiguous human completion:** mitigated by explicit Reply & Handle semantics,
  partial-failure state, and a still-actionable operator reply obligation.
- **Competing occupancy during mode changes:** mitigated by exclusive ownership
  and application sequencing over existing detach/status/attach behavior with a
  durable queueing gap; daemon-upgrade handoff is not repurposed.
- **Overclaiming identity or notification:** mitigated by separate trust/health
  states and by treating OS notification submission as an attempt, not receipt.
