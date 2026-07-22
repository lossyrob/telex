# Plan: Production Watcher Contract

Plan revision: 4

- Issue: `lossyrob/telex#110`
- Work ID: `watcher-contract`
- Base branch: `main`
- Target branch: `feature/watcher-contract`
- Scope: design and tracker artifacts only

## Outcome

Promote the successful generic Watcher spike into an accepted production design
contract without implementing the production runtime, template library, or shared
Application Client. The completed node will:

- add a normative Watcher design under `docs/design/` and link it from the design
  index;
- add machine-readable v1 schemas for the detector request/result, normalized
  Watcher event metadata, and health surface;
- record the load-bearing architecture and protocol choice in the decision log;
- resolve or explicitly defer every open question carried by the Watcher brief
  and spike report;
- publish the exact shared-client requirements to issue #12 without proposing a
  competing public API; and
- leave `watcher-runtime` and `detector-template-library` with sufficiently
  precise implementation inputs.

## Contract baseline and planned decisions

The design will preserve implemented spike evidence unless a production
narrowing is called out explicitly.

1. **Product boundary**
   - Watcher is a separately supervised, per-user, headless Telex application.
   - It executes trusted local observational detector commands.
   - Its only runtime reaction is a normalized Telex send.
   - Provider policy remains in editable detectors; arbitrary actions, remote
     executable registration, hosted ingestion, and workflow automation remain
     out of scope.

2. **Versioned detector protocol**
   - Freeze a strict versioned v1 request containing `schemaVersion`, attempt
     identity/time, watch ID and parameters, executed script mode/digest, and
     opaque prior state.
   - Freeze a strict v1 result containing `schemaVersion`, `outcome`,
     optional `nextState`, and optional event.
   - Preserve `idle`, `event`, `terminal`, and `degraded`.
   - Preserve detector event ID, namespaced kind, subject, body, and arbitrary
     JSON metadata from the implemented v1 spike. Watcher nests that value under
     its own structurally defined provenance key rather than narrowing the
     detector field under the same version.
   - Preserve strict unknown-field rejection. Any field addition, removal,
     shape change, or semantic change requires a new `schemaVersion`; v1 is not
     extended additively. Unsupported versions fail without state advancement.
   - The initial production runtime supports detector protocol v1 only. Each
     registration records that version. Concurrent-version selection is deferred
     until a later contract revision defines negotiation and migration.
   - Add registration-owned allowed event kinds or namespace prefixes. A
     detector kind outside that policy is a visible validation failure, so the
     Telex `kind` is Watcher-attested rather than merely detector-declared.
   - Any allowed-kind policy change creates a new registration revision and
     automatically pauses an active watch. Explicit resume confirms the new
     policy; later mismatch also pauses with a typed blocked reason.
   - Keep routing, sender, attention, disposition, cadence, timeout, working
     directory, environment, and actions outside detector control.

3. **State and observation meaning**
   - `idle` may commit `nextState` after successful evaluation.
   - Advancing state on `idle` means the detector observed and intentionally
     classified source data as non-actionable, including ignored observations.
   - A detector must not advance beyond observations it did not evaluate.
   - `degraded`, process failure, malformed output, timeout, script drift, and
     send failure never advance state.
   - `terminal` may carry a receipt-gated final event or commit eventless state
     and stop.

4. **Receipt-gated event transaction and deduplication**
   - Specify prior state -> bounded detector execution -> validation ->
     normalization -> fixed-route send -> typed durable acceptance -> atomic
     state/event/attempt commit.
   - Treat occupancy, push attempt, recipient consumption, and disposition as
     separate from durable send acceptance.
   - Preserve at-least-once behavior across accepted-send/local-commit
     uncertainty.
   - Bind committed event evidence to watch/event IDs, state hashes, normalized
     envelope hash, script digest, route, message ID, receipt, and attempt.
   - Scope event ID uniqueness to one watch ID for that watch ID's entire retained
     lifetime. The durable dedup key is `(watchId, eventId)`.
   - Hash state and normalized envelopes as `sha256:<hex>` over RFC 8785 JSON
     Canonicalization Scheme UTF-8 bytes. Any future algorithm change requires a
     new ledger/schema version; historical values are never rehashed in place.
   - Matching committed event evidence is a visible stale duplicate; conflicting
     evidence is an event-ID collision. Neither authorizes sending, state
     advancement, or terminal transition.
   - An event-ID collision moves the watch to a visible blocked/paused state.
     Recovery requires an explicit detector or registration revision plus
     operator resume; the committed ledger is never deleted or overwritten to
     make the collision disappear.

5. **Watch lifecycle**
   - Use `active`, `paused`, `terminal`, and provenance-retaining `removed`.
   - Watches remain durable until detector terminal outcome or explicit removal.
   - Model a single-event watch as an event-producing terminal result rather
     than a separate lifecycle engine.
   - Pause/resume provide operational suspension; remove is explicit
     cancellation.
   - Defer address-bound or inferred expiration. Occupancy loss must never
     silently cancel a watch.
   - Keep lifecycle status authoritative as `active`, `paused`, `terminal`, or
     `removed`. Non-transient failures use `paused` plus health status `blocked`
     and a typed reason. Legal recovery is update followed by explicit resume,
     or removal.

6. **Script provenance**
   - Make `pinned` the production default: the registered digest must match the
     content selected for execution.
   - Keep `follow-path` as an explicit development mode: hash immediately before
     execution and before accepting output; reject mid-run drift.
   - Preserve the detector v1 request's existing bare-hex `script.sha256` field.
     Record that digest as algorithm-qualified `sha256:<hex>` only in
     Watcher-owned attempts, ledger rows, health, and emitted metadata.
   - Freeze SHA-256 for v1. A future algorithm requires a new audit/schema
     version and explicit repinning; historical records retain their qualified
     values.
   - Treat a pinned digest mismatch as a non-transient blocked/auto-paused
     configuration failure requiring explicit update and resume.
   - Treat a follow-path mid-run drift as a failed attempt eligible for bounded
     retry; repeated drift contributes to the normal degraded health threshold.

7. **Credentials, environment, and trust**
   - State plainly that detectors are same-user trusted local code, not
     sandboxed code.
   - Start detectors from a cleared environment, restore only a documented
     minimal platform launch baseline, and inherit only explicitly allowlisted
     variable names.
   - Read credential values at execution time; never persist them in registration
     or detector requests.
   - Permit operator-selected credential wrappers as command policy, not as a
     provider-aware Watcher feature.
   - Require bounded diagnostics and best-effort redaction while acknowledging
     that provider errors and arbitrary code can still expose sensitive context.

8. **Scheduling, failure, and process lifecycle**
   - Require per-watch non-overlap, bounded global concurrency, cadence bounds,
     deterministic jitter, bounded timeout/output, process-tree termination,
     exponential backoff, and one overdue execution after restart rather than
     replaying missed intervals.
   - Require production detectors to be cursor-clean: one catch-up execution
     must query from committed opaque state and classify every observation since
     that cursor. Window-only detectors must declare their downtime gap risk and
     are not production templates unless the source provides replay/query
     semantics.
   - Add registration policy `maxSafeDowntimeSeconds` (nullable for sources with
     durable replay). If elapsed time since the last successful evaluation
     exceeds it, transition to `paused` with health
     `blocked`/`downtime-gap`, retain audited downtime timestamps, and require
     explicit operator update/resume rather than silently reporting healthy.
   - Preserve attempts and local health diagnostics for every failure.
   - Define a versioned local management interface:
     `telex-watcher status --json` returns runtime and all-watch health, while
     `telex-watcher show <watch-id> --json` returns the same per-watch projection.
     Both conform to `watcher-health-v1.schema.json`.
   - Define a minimum machine-readable runtime/watch health surface containing
     status, consecutive failures, last attempt/success/event, next attempt,
     current diagnostic category, blocked reason, sender readiness, and retained
     row/byte counts, plus observation time, runtime heartbeat time, and declared
     `staleAfterSeconds`. Heartbeat updates independently of detector execution.
     The local service supervisor/operator CLI is the initial consumer. A
     supervisor-visible stale/degraded state is required before the production
     runtime can pass its usability gate.
   - Defer automatic Telex degradation/recovery notifications to
     `operational-hardening`; any later notification must be thresholded,
     coalesced, and routed by explicit operator-health policy rather than
     implicitly spamming the event target.
   - Require the detector process tree not to outlive abrupt Watcher death.
     Startup reconciliation marks unfinished attempts interrupted and keeps an
     affected watch ineligible until the runtime proves prior containment ended;
     inability to prove that becomes a visible blocked state requiring operator
     action. The contract freezes this behavior; runtime and hardening nodes own
     platform mechanisms and destructive proof.

9. **Service identity and sender membership**
   - Use a fresh never-reused runtime incarnation ID per Watcher process and one
     application session spanning its configured sender addresses.
   - Keep sender responsibilities stable across process replacement.
   - Attach sender stations with required PID/start-time liveness, reconcile at
     startup/config change/periodically/typed membership loss, and remain
     non-ready on partial attachment.
   - Never force-take an address. Make collision, partial attachment,
     compensation, retry, and detach outcomes observable.
   - Preserve explicit sender selection and strict caller-controlled
     `NeedsAttach` recovery.
   - Treat Watcher sender stations as explicitly send-only. Dedicated sender
     addresses must not be advertised as reply-capable targets; inbound
     actionable backlog is an operational error surfaced in health and is never
     silently consumed.
   - Require the shared Application Client's send-only station mode not to count
     as inbound attendance. Sends addressed to a send-only sender receive the
     address policy's unoccupied or rejected result, never a false
     application-delivered result. Watcher never drains, acks, drops, or
     dead-letters inbound traffic itself. This is hard-gated on #12 rather than
     approximated through the spike adapter.

10. **Normalized Telex event**
    - Use the detector's registration-authorized namespaced event kind as Telex
      `kind`.
    - Use detector subject/body within Watcher bounds.
    - Use registration-owned sender, target, attention, and disposition policy.
    - Define exact top-level metadata keys: `schemaVersion`, `watcher`, and
      `detector`. `watcher` carries watch/event/attempt identity and
      algorithm-qualified script evidence; `detector` contains the detector's
      arbitrary JSON value. Watcher constructs the outer object, so detector
      metadata cannot collide with reserved keys.
    - Preserve the spike budgets: detector metadata is at most 64 KiB serialized,
      and complete normalized metadata is at most 80 KiB serialized, including
      Watcher overhead.
    - Keep state hashes and normalized envelope hash in the Watcher audit ledger;
      include only the provenance consumers need for deduplication and source
      inspection in the message.

11. **Packaging and deferred boundaries**
    - Require `fake_detector` and `fake_telex` to move behind a non-default
      test-support feature or an equivalent test-only package before production
      publishing; ship only the product binary by default.
    - Require the runtime node's packaging acceptance to prove the default
      published package exposes only the product binary and excludes test-support
      binaries/features.
    - Retain current watch state and the sent-event ledger for the lifetime of a
      watch ID; removed watch IDs are not reusable. Until operational hardening
      defines safe compaction/backup, provenance retention is intentionally
      unbounded, no destructive GC is allowed, and health must expose growth.
      Attempts and diagnostic payload retention/backup/compaction are owned by
      `operational-hardening`, with storage pressure as the explicit downstream
      impact.
    - Health exposes retained rows/bytes, configurable warning thresholds, and
      threshold state. Operational hardening owns the capacity model and default
      numeric thresholds before closure.
    - Provider/credential-wide rate budgets are owned by detector templates and
      operational hardening; the provider-neutral runtime supplies only global
      concurrency, cadence bounds, jitter, and backoff.
    - Require templates to declare detector schema version, template version,
      source provenance/digest, and known provider replay limitations. Copied
      detectors become user-owned scripts; compatibility guarantees remain owned
      by `detector-template-library`.
    - Keep sandboxing, signed catalogs, remote administration, multi-host
      failover, hosted webhooks, provider compatibility guarantees, and rich UI
      outside this contract.
    - Identify each deferred item with owner, rationale, and downstream impact.

12. **Application Client export**
    - Publish requirements, not an API design, covering stable service addresses
      plus ephemeral runtime identity, PID/start-time membership, atomic or
      compensable multi-address lifecycle, typed membership-loss reasons,
      strict versus automatic recovery policy, bounded reconcile-and-send,
      explicit sender selection, collision visibility, typed receipt separation,
      sender-only versus bidirectional station semantics, receive/cursor/ack/
      disposition/reply, lifecycle status, daemon restart recovery, and dedup
      guidance.
    - Explicitly reject promotion of CLI subprocess parsing,
      `TELEX_WATCHER_INTERNAL_SEND_ONCE_V1`, raw daemon IPC, or sender occupancy as
      proof of application consumption.
    - Hard-gate production Watcher runtime promotion on the campaign-owned
      `application-client-ready` checkpoint. There is no private-seam fallback.
      Request a per-requirement accepted/deferred/rejected disposition from #12
      so downstream workers can identify unresolved blockers.

## Work items

### 1. Write the production Watcher design

Create `docs/design/watcher.md` with:

- status, scope, terminology, component boundary, and trust model;
- exact detector request/result examples and validation rules;
- watch registration policy and lifecycle;
- script provenance and environment/credential policy;
- scheduler, failure, restart, and process-containment behavior;
- receipt/state/dedup transaction and audit evidence;
- service identity and sender membership lifecycle;
- normalized Telex event and provenance conventions;
- Application Client dependency and temporary seam exclusions;
- a disposition table for every carried open/deferred question; and
- downstream implementation obligations for runtime, templates, and hardening;
- an explicit failure/recovery table for collision, pinned mismatch, repeated
  drift, degradation, sender loss, restart interruption, and unprovable orphan
  containment;
- the minimum machine-readable health/status contract and retention-growth
  warnings; and
- a downstream-consumer checklist mapping each runtime/template implementation
  concern to a contract section or accepted deferral.

Add normative machine-readable design artifacts:

- `docs/design/schemas/watcher-detector-request-v1.schema.json`;
- `docs/design/schemas/watcher-detector-result-v1.schema.json`;
- `docs/design/schemas/watcher-event-metadata-v1.schema.json`; and
- `docs/design/schemas/watcher-health-v1.schema.json`.

The schemas will preserve implemented v1 compatibility, use strict objects for
Watcher-owned envelopes, allow arbitrary JSON only where the detector contract
already does, and make future drift mechanically detectable.

### 2. Integrate the contract into the design layer

- Link `watcher.md` from `docs/design/index.md` and place it in the reading order.
- Before editing the decision log, send a disposition-required
  `adr-allocation-requested` message to campaign orchestration with
  workstream/node/issue, requested count, proposed title/rationale, and the
  observed `DECISIONS.md` high-water number and commit.
- Wait for `adr-allocated` with exact number(s) and base commit. Never hardcode,
  renumber, or overwrite a sibling allocation.
- Append one concise accepted ADR at the allocated number because the
  detector protocol, receipt-gated state transaction, trusted-local execution
  boundary, and provider-neutral fixed-action architecture meet the repository's
  load-bearing decision threshold.
- Before final review, fetch/rebase latest `main`, verify the allocation remains
  collision-free, and preserve every sibling ADR and design-index entry.
- If a sibling domain contract merged first, run an explicit cross-contract
  consistency pass without changing its accepted domain semantics. Escalate any
  semantic conflict to both orchestrators instead of silently choosing one.
- Do not modify Streamliner roadmap, workstream graph, brief, or shared campaign
  artifacts from this worker branch.

### 3. Stabilize the contract against latest main and run final review

After drafting `watcher.md`, schemas, index changes, and the allocated ADR:

- fetch/rebase latest `main`;
- verify the campaign allocation is still collision-free;
- preserve every sibling domain contract, ADR, and design-index reading-order
  entry;
- if a sibling contract merged first, complete the explicit cross-contract
  consistency pass without changing its accepted domain semantics;
- resolve or escalate any semantic conflict rather than silently choosing
  between contracts;
- run the configured final `society-of-thought` `general-reviewer` review on the
  rebased design content; and
- resolve final-review findings before generating the #12 publication candidate.

The rebased, reviewed PR head is the semantic source for the requirements draft.

### 4. Publish the shared-client requirements

After the rebased design text passes final review, prepare the exact UTF-8,
no-BOM issue #12 comment draft in a private file. It will:

- links the production Watcher contract and issue #110;
- identifies the source design commit or PR head SHA;
- distinguishes accepted Watcher-specific semantics from shared-client needs;
- enumerates the final requirements and acceptance implications;
- identifies rejected spike-private seams; and
- requests an accepted/deferred/rejected disposition for each requirement before
  production Watcher runtime promotion; and
- leaves API shape and `application-client-ready` acceptance with the campaign
  owner.

Before publication:

- assign a requirements-draft revision and compute SHA-256 over the exact raw
  UTF-8 file bytes;
- send the complete draft separately to the Watcher orchestrator and campaign
  orchestrator as disposition-required
  `application-client-requirements-review-requested` messages with revision,
  digest, issue/repo/workstream/node, and draft purpose;
- wait for `application-client-requirements-approved` from both for the same
  revision and digest;
- treat any byte change as a new revision requiring two fresh approvals; and
- disposition stale or conflicting replies without publishing.

Only after both approvals, publish with `gh issue comment --body-file` using the
approved bytes and verify the resulting GitHub body. Do not edit issue #12's API
proposal or claim the shared checkpoint. If #12 does not accept the required
semantics, report the affected runtime/template work as blocked; do not restore a
private client seam.

### 5. Verify final alignment and prepare the PR

- Check every issue/brief/report open question has an accepted, rejected, or
  deferred disposition with owner/rationale/downstream impact.
- Cross-check protocol examples and limits against the spike implementation and
  report, calling out intentional production differences.
- Parse every new JSON Schema and validate the design examples against its
  required fields and outcome constraints. Record any intentional divergence
  from the spike validator as an explicit migration decision.
- Check links and design-index navigation.
- Inspect the final diff for design authority conflicts with `daemon.md`,
  `DESIGN.md`, and the forward-looking status of `EXTENSIONS.md`.
- Record the issue #12 comment URL for the PR description and field report.
- Verify the downstream-consumer checklist covers every carried implementation
  question and request orchestrator confirmation in the merge-ready field report.
- Record the runtime/template/hardening owner and mechanical acceptance check for
  each deferred packaging, retention, notification, provider-budget, template,
  and platform-containment item.
- Require `watcher-runtime` promotion to add CI conformance tests comparing its
  request/result/metadata/health models to the four canonical schemas, and require
  `detector-template-library` to validate every shipped fixture against them.
  Schema drift blocks those downstream promotions.
- Require the runtime packaging rubric to record the exact
  `cargo package --list -p telex-watcher` (or release-equivalent) invocation,
  default feature set, and expected artifact/bin list proving only the product
  binary ships by default.
- If runtime/template owners are assigned before merge, request a lightweight
  contract-consumable acknowledgment. Otherwise make the checklist an explicit
  launch-acceptance gate for those nodes and record that owners were unassigned.
- Compare the final PR head against the source SHA named in the approved #12
  draft. If any later rebase, final review, or PR review changes a Watcher
  semantic represented by the published requirements, the draft is stale:
  increment its revision, regenerate the exact UTF-8 bytes, obtain two fresh
  approvals, and publish an explicit superseding/correction comment before
  merge-ready. There is no semantic-drift exception.
- Confirm the merge-ready field report names the final PR head, approved
  requirements-draft revision/digest, and verified GitHub comment URL, and states
  that the final contract and published requirements remain semantically aligned.
- No runtime code is changed; project builds/tests are not a proxy for design
  consistency and are not required unless implementation files change
  unexpectedly.

## Sequencing and dependencies

The work is intentionally sequential because the tracker export depends on the
reviewed contract text:

1. Draft `watcher.md`, schemas, proposed ADR content, and index update.
2. Request and receive dynamic ADR allocation.
3. Fetch/rebase latest `main`, append the allocated ADR, preserve sibling design
   content, and complete the cross-contract consistency pass.
4. Resolve consistency findings and run configured final review on the rebased
   contract.
5. Generate the exact #12 draft from that reviewed PR head, obtain dual approval,
   and publish only the approved bytes.
6. Recheck final-head/published-requirements semantic alignment and repeat the
   draft approval/correction flow after any later semantic change.
7. Verify the tracker comment and final diff.
8. Commit selectively and create the PR through `paw-pr`.

## Review and gates

1. Run the configured PAW planning-docs review:
   - mode: `society-of-thought`;
   - specialist: `general-reviewer`;
   - interaction: `parallel`, non-interactive;
   - perspectives: premortem and retrospective.
2. Resolve all blocking planning findings.
3. Commit the reviewed Plan.md under the configured `commit-and-clean` artifact
   lifecycle.
4. Keep Plan.md as UTF-8 without BOM, compute SHA-256 over its exact raw
   filesystem bytes, and send that same file through `--body-file` so newline or
   console transcoding cannot change the approved byte stream.
5. Send revision 4 and that digest, with the full Plan.md as the body, in separate
   disposition-required `plan-review-requested` Telex messages to the Watcher
   orchestrator and campaign orchestrator.
6. Do not begin design edits until both orchestrators approve the same revision
   and digest. Any Plan.md change increments the revision and invalidates both
   approvals.
   Stale replies are dispositioned without changing gate state. If an
   orchestrator is unavailable or approvals conflict, send a
   disposition-required `blocked` message with attention `interrupt` to the
   Watcher orchestrator and a concise blocker status to campaign orchestration;
   when the conflict involves both, send the evidence to both. Record blocker
   message IDs and halt. There is no timeout bypass, substitute approver,
   unilateral override, or trivial-edit exception.
7. After implementation, run the configured final `society-of-thought`
   `general-reviewer` review before `paw-pr`.

## Completion criteria

- `docs/design/watcher.md` and the four v1 JSON Schemas are indexed, parseable,
  compatible with the implemented spike except for explicitly documented
  production additions, and internally consistent.
- The production detector, lifecycle, trust, failure, state, provenance, sender,
  and event contracts are precise enough for downstream implementation.
- Every carried open question is dispositioned.
- Minimum health, collision recovery, sender-only, restart catch-up, retention,
  and abrupt-death behavior are explicit rather than delegated without a floor.
- The accepted ADR captures the load-bearing design choice without duplicating
  the full contract, uses a campaign-allocated number, and remains collision-free
  after the final rebase.
- Issue #12 contains the byte-exact dual-approved Watcher shared-client
  requirements and exclusions.
- The published requirements identify their source PR head, and the final
  merge-ready report confirms that head remains semantically aligned or records
  the later approved superseding/correction draft.
- The merge-ready packet includes the downstream-consumer checklist and requests
  confirmation that runtime/template nodes can be detailed without reopening the
  contract.
- The PR uses `Closes #110` only if all of the above are present and reviewed;
  otherwise it uses `Refs #110` and names the missing outcome.
