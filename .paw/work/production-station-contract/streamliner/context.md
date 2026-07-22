# Launch Context - Production Station and operator-loop contract

## Layer 0 - Design Context Hints

Use these manifest-provided design hints as navigation anchors for the design layer. Treat them as authoritative design sources only after reading the current files in the execution checkout; do not rely on this context as a substitute for the documents.

- `docs/design/index.md` - design-layer entry point and reading order. Link any new Operator Station design from here.
- `docs/design/daemon.md` - normative local-exchange contract: station membership, delivery/ack, restart, liveness, sender identity, and status semantics.
- `docs/design/proposals/EXTENSIONS.md` - forward-looking convention space for namespaced message kinds, opaque metadata, extension IDs, capability cards, and the guardrail that Telex carries extension data without interpreting it.
- `docs/design/proposals/DISPATCH.md` - forward-looking reasoning-receptionist/dispatch model and the boundary that semantic judgment remains in agents or orchestrators, not Telex core.

## Layer 1 - Worker Mission

You own exactly the selected Streamliner node `station-contract`, tracked by `lossyrob/telex#114`: **Production Station and operator-loop contract**.

The mission is design/contract work, not another desktop implementation spike. Produce an accepted production Operator Station and mediated-attention contract in the Telex design layer that lets downstream workers detail `station-app` and `operator-broker` without accidentally promoting unsupported spike seams. The contract must cover:

- human-attended Station responsibilities and address occupancy semantics;
- direct, assisted, and quiet routing modes, including safe transitions without ambiguous or competing occupancy;
- operator-agent authority to resolve, clarify, aggregate, escalate, route back, and disposition, while keeping judgment outside Telex core;
- source-message/thread provenance, non-impersonation, recommendations, and source availability across restart/backend boundaries;
- production message kinds and opaque metadata, including explicit disposition of the experimental `operator-station-spike` namespace and campaign-local `campaignAttention` convention;
- notification policy by attention, kind, disposition requirement, source, quiet-hours/Focus Assist posture, suppression, and aggregation;
- feed/thread behavior, unresolved-history semantics, delivery/ack health, and human-visible operator/station health;
- reply/disposition semantics, especially the dogfood finding that replying must not silently leave a stale human obligation;
- operator-agent replacement, rehydration, duplicate delivery, delayed replies, stale-origin handling, and route-back recovery;
- sender/principal presentation, source trust, safe links, and the boundary against arbitrary command execution;
- exact Operator Station requirements to publish to shared Application Client issue `#12`, without freezing a competing public client API.

Expected artifact shape from issue `#114`: create or update a production Operator Station design under `docs/design/`, link it from `docs/design/index.md`, update `docs/design/DECISIONS.md` only if a choice meets ADR threshold, and update issue `#12` with precise shared-client requirements. Do not implement the production desktop Station, reusable operator-agent package, shared Application Client API, packaging/autostart/signing, cross-platform support, hardening, or later usability/closure gates in this node.

Completion anchor: the node is complete only when the design is reviewable and internally consistent, open questions and dogfood carry-forward items are resolved or explicitly deferred with owners/rationale/downstream impact, Station/operator-specific behavior is separated from Telex core and Application Client requirements, `#12` has the shared requirements, and `station-app` plus `operator-broker` can be promoted from the accepted contract.

## Layer 2 - Relevant State

Selected node metadata from `.streamliner/workstreams/operator-station/graph.json`:

- Node ID: `station-contract`
- Type: `research`
- Status: `ready`
- Depends on: `viability-gate` (completed)
- Blocks: `station-app`, `operator-broker`, and the `usable-station` checkpoint
- Tracker: `https://github.com/lossyrob/telex/issues/114`
- Summary: use spike evidence to establish the accepted product boundary for human-attended address semantics, provenance, message kinds/metadata, notification policy, reply/disposition, routing modes, operator recovery, identity, and safety. Contribute Application Client requirements to `#12`; do not own a desktop-only client contract.

Workstream and campaign state:

- Parent workstream `operator-station` is active and focused under `lossyrob/telex#92`.
- The Addressable Attention campaign is tracked by `#102`. Its goal is to let external conditions and agent obligations reach the right agent or human without manual tab polling or session-owned background waiters.
- The Operator Station Wave 1 spike is complete through issue `#93` and PR `#104` at `fc2ec2cbf0d23ebdb6064564f64c62c89efe5508`.
- The builder has passed the mediated-attention viability gate after guided dogfood. Production Station contract work is now ready; production application work still waits on this node and the shared Application Client seam.
- Shared seam `#12` owns the common long-lived application client contract consumed by Operator Station and Telex Watcher. Operator Station may publish requirements there but must not freeze its own competing public API.
- Sibling Watcher contract `#110` is a separate design-contract node and independent input to `#12`; it is coordination background, not this worker's task.

Spike evidence to read before planning:

- `docs/notes/operator-loop-spike-report.md` - live evidence, dogfood observations, temporary seams, UX gaps, deferred items, and initial `#12` requirements.
- `spike/operator-station/README.md` - spike-only Station runtime shape and validation commands.
- `spike/operator-station/OPERATOR-AGENT.md` - experimental mediator assignment, filtering policy, source envelope, ack/disposition ordering, and route-back recovery behavior.
- `spike/operator-station/WALKTHROUGH.md` - real Windows builder walkthrough for `worker -> attention:rob -> operator agent -> operator:rob Station -> human reply -> operator agent -> worker`.
- `spike/operator-station/evidence/` - demo transcript, screenshots, Action Center record, stress evidence, and return-path recovery evidence.
- `.streamliner/workstreams/operator-station/reconciliation-note.md` - reconciled lessons: honest attendance required wait/read/ingest/ack, route-back recovery and restart projection were hardened, and reply/disposition clarity was promoted to `#114`.

Key accepted learning from the spike and gate:

- The mediated loop was valuable enough to productionize: selective human escalation, routine resolution, evidence-seeking clarification, route-back, restart continuity, and Windows notification publication all mattered.
- Separate raw and mediated threads made provenance and route-back behavior clearer than a single conversation model.
- Application attendance must prove delivery and consumption, not just database visibility. The spike moved from inbox polling to an application-owned one-shot wait courier with ack after ingestion.
- Delivery/ack health and operator-agent occupancy are human-visible product state; unattended operator status should not look like quiet success.
- Reply plus separate disposition was usable in the spike but confusing in dogfood. The production contract must decide whether common flows need an explicit or atomic `Reply & Handle` operation.
- Station-authored human replies were disposition-required; the operator assignment routed back to the worker before terminally handling the human reply to preserve recovery.
- The default coordination Telex plane is campaign infrastructure. Destructive daemon, upgrade, crash, handoff, or branch-binary tests must use isolated `TELEX_HOME`, `TELEX_DB`, and `TELEX_INSTALL_ROOT` values and an absolute worktree binary.

Temporary spike seams that must not be promoted accidentally:

- CLI subprocess courier for every application operation.
- Repeated one-shot waiter supervision as the live delivery strategy.
- Full-history JSONL export for unresolved startup recovery.
- Store path fingerprint as the source identity boundary.
- Local SQLite and Windows-only deployment assumptions.
- Development Tauri launch and HKCU AUMID registration behavior.
- Local app-data session UUID/high-water marker as client state.
- `operator-station-spike.*` message kinds and `urn:telex:experimental:operator-station-spike:v1` extension identity.
- Campaign-local `attention.*` kinds and `campaignAttention` metadata.
- Current spike UI layout and source rendering semantics.

Application Client requirements already surfaced by the spike report; use them as inputs, refine them for `#12`, and keep Station-specific UX separate:

- supported application station identity and attach/detach/recovery lifecycle;
- stable store identity without exposing credentials or raw paths;
- streaming/callback/async receive API yielding message, delivery-role context, metadata, and ack capability;
- ack-after-ingest, duplicate/redelivery identity, and observable ack-pending/deaf states;
- unresolved-obligation plus bounded recent-history cursor/query without full-store materialization;
- typed send, reply, read-thread, and disposition operations;
- service/application identity and backend selection for SQLite and credentialed Postgres;
- safe source-reference/store identity conventions;
- reply/disposition atomicity and recovery behavior when a human answers after source context changes;
- delta-oriented application events rather than full-feed serialization;
- reply attention selection, richer operator notes, receipt identity checks, retry throttling, and local-scope discovery/cleanup.

Streamliner Desktop reference inspection:

- Reference checkout: `C:\Users\robemanuele\proj\streamliner\streamliner-pr-122\desktop`
- Treat it as read-only reference material. Do not modify that checkout.
- It is a Tauri v2 Windows tray/feed shell over a local Streamliner API, with React 19/Vite/Tauri, notification feed components, read state, SSE client, tray/toast/deeplink/config code, and Windows icon/bundle assets.
- Its README states Phase 4a deliberately excluded toasts and uses API backfill plus SSE with in-memory `Last-Event-ID`. Use it for UI/tray/feed/notification implementation precedent only; it is not a Telex runtime dependency or contract source.

Other relevant references outside Layer 0 hints:

- `PRODUCT-THESIS.md` - Telex is a durable, addressable message fabric for responsibilities; delivery is distinct from disposition; messages coordinate work but artifacts remain authoritative; Streamliner is a reference profile, not a dependency.
- `telex-console/README.md` - existing separate read-only operator console precedent: feed, addresses, thread, reader, delivery vs disposition presentation, bounded cursor polling, and read-only backend opening. It remains read-only and should not be conflated with the writable Station.
- `.streamliner/shaping/roadmap.md` - campaign staging, shared `application-client-ready` seam, and boundary rules.
- `.streamliner/workstreams/operator-station/brief.md` and `docs/initial-shaping.md` - purpose, boundaries, operating loop, routing modes, source/thread model, notification posture, risks, and current state.

Unavailable Inputs:

- Manifest marks `telex:PRODUCT-THESIS.md` and `telex:telex-console/README.md` as invalid Layer 0 design paths because design references must be relative `docs/design/*.md` paths. They remain useful source references and were inspected separately, but they should not be treated as manifest design hints.
- Manifest marks `.github/copilot-instructions.md` missing for the target repo. Use repository conventions and explicit launch instructions instead.

## Layer 3 - Coordination Context

Addresses for this implementer launch:

- Implementer: `telex://lossyrob/telex/T-A:operator-impl-114`
- Reviewer: `telex://lossyrob/telex/T-A:operator-review-114`
- Operator Station workstream orchestrator: `telex://lossyrob/telex/T-A:operator-station-orch`
- Campaign orchestrator: `telex://lossyrob/telex/T-A:campaign-orch`

Before planning or editing, load the `telex` skill, run `telex copilot skill`, attach the implementer address on backend `local` with the Copilot bridge and a description containing issue `#114` and implementer role, call `extensions_reload`, and send `session-online` Telex messages to both orchestrators with attention `background` and `--session $env:COPILOT_AGENT_SESSION_ID`. Do not start `telex wait` or a background Telex waiter.

Use the GitHub public config for this personal repository before `gh` commands:

```powershell
$env:GH_CONFIG_DIR = "$env:APPDATA\gh-pub"
```

Do not add or modify PR assignees unless explicitly instructed, and specifically do not assign `mmcfarland_microsoft`.

Coordination gates:

- Follow PAW Lite planning with planning-docs review first.
- Before implementation/design editing, send the reviewed `.paw/work/<work-id>/Plan.md` body to both orchestrators as `plan-review-requested` with attention `next-checkpoint`, disposition required, issue/repo/workstream/revision/path metadata, and SHA-256 digest of the exact bytes sent.
- Implementation may begin only after both orchestrators approve the same plan revision and digest. Any Plan.md byte change invalidates prior approvals and requires a new revision, digest, and approvals.
- Do not silently choose between conflicting orchestrator feedback; send `decision-needed` to both and hold.

PR and review coordination:

- Use the reviewer session over Telex for review handshakes; do not use PAW PR lifecycle review-comment polling as the discovery mechanism for reviewer comments.
- Before `review-ready` or `rereview-requested`, ensure the PR head is pushed, CI is green with no pending/failing checks, and there is no merge conflict.
- The reviewer replies with `review-posted` or `review-approved`; read the actual GitHub review body and address or explicitly defer all blocking, nit, optional, and follow-up notes.
- Do not merge the PR yourself.

Watcher-backed PR sentry is a later post-approval dogfood step for this launch. Prefer shared-runtime Telex Watcher only after reviewer approval is fully triaged and the required preflight passes; otherwise fall back to canonical PAW PR lifecycle Loop sentry and report `watcher-fallback`. Do not start a Watcher runtime of your own, do not stop the shared runtime, and clean up only this run's exact registrations on terminal stand-down.

Merge-ready control:

- On first ready state for approved head, send a full field report as disposition-required `merge-ready` to the Operator Station workstream orchestrator with attention `interrupt`, and send concise `node-merge-ready` status to the campaign orchestrator.
- Keep PR sentry healthy until the workstream orchestrator sends terminal stand-down or a directed obligation.
- After verified merge, send exactly one `reconciliation-requested` message per `(issue, PR, merge commit)` to the workstream orchestrator and concise `node-merged` to the campaign orchestrator, including final outcome/evidence/deferred items and a request to reconcile workstream artifacts.

Field report expectations for this node:

- outcome: complete, partial, blocked, or escalated;
- whether the PR closes `#114` and why;
- design contract delivered and evidence it satisfies the node;
- key decisions, pivots, accepted/deferred/rejected spike conventions;
- source provenance, notification, routing, reply/disposition, recovery, identity, safety, and #12 seam impact;
- stale/missing context or workstream mismatches;
- boundary pressure and hidden dependencies;
- discrete deferred/carry-forward items for `station-app`, `operator-broker`, `#12`, or hardening;
- risks, known defects, and incomplete validation;
- items for workstream orchestrator, campaign orchestrator, and builder.

Hard blockers should be reported as disposition-required `blocked` to the workstream orchestrator with attention `interrupt` plus concise campaign status. Include evidence, attempts, and recommended amendment or decision, then remain attached and wait for directed resolution or terminal stand-down.
