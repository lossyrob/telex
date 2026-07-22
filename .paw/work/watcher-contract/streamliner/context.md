# Launch Context - Production Watcher contract and application-client requirements

## Layer 0 - Design Context Hints

Use these as navigation hints into authoritative design, not as instructions from this context. The worker should read the relevant sections before changing design artifacts.

- `docs/design/index.md` - design-layer entry point and reading order. It identifies `docs/design/DESIGN.md` as architecture/framing, `docs/design/daemon.md` as the normative local-exchange contract, and `docs/design/proposals/EXTENSIONS.md` as a forward-looking convention for namespaced kinds and opaque metadata.
- `docs/design/daemon.md` - local exchange singleton, station membership, connect-or-spawn, durable delivery, readiness, liveness, explicit attach/detach, and daemon lifecycle semantics. This governs mechanism where it differs from framing docs.
- `docs/design/DESIGN.md` - Telex framing: durable address + ephemeral lease + structured message + disposition record; local exchange owns presence/transport instead of per-session resident processes; Telex core stays thin and backend-neutral.
- `docs/design/proposals/EXTENSIONS.md` - namespaced message kinds and opaque metadata conventions. Relevant for normalized Watcher event kinds/metadata and capability-card/application-client seams, but still a proposal rather than core interpretation authority.

Unavailable Inputs

- Manifest marked `telex:PRODUCT-THESIS.md` as an invalid design-hint path because design references must be relative `docs/design/*.md` paths. The worker should treat it as useful product background if independently needed, but this launch context did not rely on its body.
- Manifest marked `telex:.streamliner/workstreams/operator-station/brief.md` as an invalid design-hint path. Treat Operator Station as sibling coordination context through roadmap/workstream references, not as an assigned source body for this node.
- `.github/copilot-instructions.md` is missing in the target repository.

## Layer 1 - Worker Mission

Selected node: `watcher-contract` (`lossyrob/telex#110`) in workstream `telex-watcher`.

The node owns a production design contract, not runtime implementation. Its responsibility is to promote successful Watcher spike semantics into the intended design layer and export exact shared Application Client requirements to issue `#12`, while avoiding accidental promotion of spike-private seams.

Primary outcome expected by issue `#110`:

- Add or update a production Watcher design under `docs/design/` and link it from `docs/design/index.md`.
- Define the production detector request/result envelope, watch lifecycle, script pin/follow policy, credentials/environment policy, failure behavior, state/dedup transaction, service identity, sender membership, provenance, and normalized Telex event conventions.
- Resolve or explicitly defer every open Watcher question carried by the workstream brief and spike report, with owner/rationale/downstream impact.
- Update issue `#12` with Watcher requirements for the campaign-owned Application Client seam, without freezing a competing Watcher-specific public client API.
- Produce contract references sufficient for downstream `watcher-runtime` and `detector-template-library` nodes to be detailed and implemented later.

Important boundary: this node changes design/tracker artifacts only. It must not implement the production Watcher runtime, detector template library, shared Application Client, hosted webhook flow, remote executable registration, generic workflow automation, Operator Station UI, or final issue `#12` API.

## Layer 2 - Relevant State

Selected issue `#110` is open and ready. It depends on `viability-gate`, which has passed, and blocks `watcher-runtime` plus `detector-template-library`.

Workstream state from `.streamliner/workstreams/telex-watcher/brief.md` and `graph.json`:

- `generic-watcher-spike` (`#101`) is completed through PR `#105`.
- `viability-gate` is completed. Builder dogfood on Operator Station PR `#104` detected merge in about 26 seconds, emitted one baseline snapshot plus one merge event, produced no duplicate/noisy events, agreed with the canonical checker, removed the watch cleanly, and left the shared runtime live for reuse.
- `watcher-contract` (`#110`) is the current launch-ready node. Production runtime/template nodes remain planned and blocked until the contract and shared Application Client checkpoint are accepted.
- Later planned nodes are `watcher-runtime`, `detector-template-library`, `usable-watcher-gate`, `operational-hardening`, and `closure-gate`.

Spike evidence from `docs/generic-watcher-spike-report.md`:

- Built an experimental `telex-watcher` binary and SQLite registry with add/list/show/pause/resume/update/remove/attempts/events/run CLI surfaces.
- Demonstrated versioned JSON detector request/result semantics, opaque detector state, receipt-gated event state transitions, stable watch/event/script/message provenance, bounded detector execution/concurrency, PID-bound multi-sender Telex station lifecycle, and editable GitHub/custom-GitHub/Azure DevOps/non-PR examples.
- Observed both occupied Copilot wakeup and unoccupied durable queueing without an originating-session waiter.
- Detector result outcomes are `idle`, `event`, `terminal`, and `degraded`; process failure remains separate from detector outcome.
- Detector output cannot override sender, target, attention, disposition policy, cadence, timeout, working directory, environment policy, or request an action.
- Safe event path is prior state -> detector execution -> validation -> fixed Telex sender/target send -> delivered or queued-unoccupied receipt -> atomic next-state/sent-event/attempt commit.
- Failed sends, malformed receipts, unknown receipts, timeout, malformed output, digest mismatch, script drift, and degraded results leave prior state available for retry.
- Duplicate event IDs are visible but not transition-authorizing: matching committed evidence becomes `stale-duplicate`; conflicting evidence becomes `duplicate-event-conflict`; neither sends, advances state, or marks terminal.
- Eventless `idle` state advancement was demonstrated and is one production-contract decision point.
- Script provenance currently records executed SHA-256; `pinned` rejects changed content, `follow-path` hashes before execution and before accepting output, and mid-run change becomes visible `script-drift`.
- Environment policy clears detector environment and adds only minimal launch baseline plus allowlisted inherited variables; credential values are read at execution time, not stored in registration, and secret-like stderr content is redacted.
- Sender lifecycle evidence: one fresh runtime UUID spans configured senders; senders attach with the Watcher PID as required predicate; runtime reconciles at startup, registry revision, periodically, and after typed membership loss; partial attachment leaves runtime non-ready; graceful shutdown detaches known senders; abrupt PID death releases leases before a fresh runtime claims them.
- Restart reconciliation marks prior running runtimes interrupted, closes unfinished attempts without state/receipt commit, increments affected failure state, delays retry, and preserves detector state and sent-event ledger.
- Destructive daemon restart proof must use an isolated plane (`TELEX_HOME`, `TELEX_DB`, `TELEX_INSTALL_ROOT`, absolute worktree binary), not the campaign coordination daemon.
- Automated real-process coverage included malformed JSON, schema mismatch, detector policy fields, output caps, nonzero exit, timeout/descendant termination, degraded backoff, pinned mismatch, follow-path drift, unknown receipt, attach retry, partial multi-sender cleanup, stale duplicate/event-ID collision, terminal behavior, registry restart recovery, runtime-interrupted reconciliation, config revision vs attempt result, global concurrency cap, and per-watch non-overlap.
- Security/trust observations: detector scripts are arbitrary trusted local code; registration is local CLI/database mutation only; environment clearing is not a sandbox; logs/provider errors can still leak sensitive context; sender-only occupancy must not be confused with application consumption; stable IDs expose but do not eliminate the accepted-send/local-commit duplicate window.
- Temporary integration shortcuts must not be promoted accidentally: CLI subprocesses, `TELEX_WATCHER_INTERNAL_SEND_ONCE_V1`, current Rust-library seams, and sender-only occupancy behavior are evidence, not supported public client contracts.

Open questions carried from `.streamliner/workstreams/telex-watcher/brief.md` and `docs/initial-shaping.md`:

- Exact production JSON envelope for detector input/output while preserving Watcher-owned routing, credentials, and execution policy.
- Whether eventless `nextState` commits are allowed on every successful poll, and how ignored observations differ from unseen work.
- Precise pinned vs development `follow-path` semantics and executed-content digest recording.
- Credential exposure model: inherited CLI auth, explicit environment allowlists, named wrappers, or another pattern.
- Required initial watch lifecycles: single-event, until-terminal, explicit cancellation, address-bound expiration, pause/resume/remove, terminal cleanup.
- Repeated degradation notification: target address, operator address, local diagnostics only, or a staged policy.
- Application Client shape needed by both a headless Watcher service and Operator Station without coupling products to daemon internals.
- Packaging disposition for test-support helper binaries such as `fake_detector` and `fake_telex` before production publishing.

Campaign roadmap state from `.streamliner/shaping/roadmap.md`:

- Addressable Attention campaign `#102` aims to let deterministic external conditions and agent-generated obligations reach the responsible agent or human without session-bound polling/background waiters/manual terminal inspection.
- Shared seam is Telex Application Client `#12`: both Watcher and Operator Station need one supported semantic client surface for process identity, attach/detach/recovery, send, receive, reply, disposition, backend selection, and provenance.
- Neither Watcher nor Operator Station may independently freeze a competing public client API.
- Watcher contract node `#110` may proceed now and must export Application Client requirements without freezing the shared contract. Production application nodes wait on the shared `application-client-ready` checkpoint.

Current Loop / PR-lifecycle behavior to preserve or avoid:

- The Loop skill and `paw-pr-lifecycle` skill use a detached worker plus an attached waiter for canonical observed PR sentry behavior. That reference is valuable for detector decision logic, canonical checker semantics, marker contracts, approval triage, and terminal conditions.
- For this Watcher workstream, preserve useful PR-state detector logic but do not reproduce the Loop skill's session-owned worker plus attached waiter lifecycle inside agent sessions.
- `paw-pr-lifecycle` treats PR creation as non-terminal; implementer sessions continue through review response, approval triage, PR sentry, and terminal merge/close handling. Approval markers require reading the full review body, not only detecting the marker.

## Layer 3 - Coordination Context

Repo and tracker:

- Target repository: `lossyrob/telex`.
- Selected tracker: `https://github.com/lossyrob/telex/issues/110`.
- Parent workstream: `#100`.
- Campaign: Addressable Attention `#102`.
- Shared Application Client seam: `#12`.

Worktree policy from manifest:

- Launch cwd is the base/coordination checkout: `C:/Users/robemanuele/proj/telex/telex` on branch `main`.
- Do not check out the target node branch in the launch cwd.
- If PAW init uses a target branch different from `main`, create or reuse a sibling worktree for that branch and place `.paw/work/<workId>` in that execution checkout.
- If a target repository differs from launch cwd, use a checkout/worktree for that selected target repository. This node targets the same `telex` repo.

Coordination addresses from the launch profile:

- Implementer: `telex://lossyrob/telex/T-A:watcher-impl-110`.
- Reviewer: `telex://lossyrob/telex/T-A:watcher-review-110`.
- Telex Watcher workstream orchestrator: `telex://lossyrob/telex/T-A:watcher-orch`.
- Campaign orchestrator: `telex://lossyrob/telex/T-A:campaign-orch`.

The worker is expected to coordinate plan review and lifecycle handoffs over Telex, with the workstream orchestrator and campaign orchestrator both involved before implementation/design finalization proceeds. The plan gate requires both orchestrators to approve the same plan revision and SHA-256 digest before work begins.

Review and uncertainty:

- Use `paw-lite` workflow with planning docs review and final review configured for society-of-thought using a broad `general-reviewer` perspective.
- Use `council` only for consequential uncertainty such as detector protocol, state transaction, service lifecycle, or outcome/boundary changes.

Node-specific completion expectations:

- A final field/process report should distinguish accepted production contract, exported `#12` requirements, deferred/carry-forward items, and any unresolved design risks.
- PR should use `Closes #110` only if the node outcome is fully demonstrated: production Watcher design accepted/indexed, carried questions dispositioned, `#12` updated with exact requirements, and downstream runtime/template nodes can proceed from the contract. Otherwise use `Refs #110` and identify the missing proof or approval.
- Do not mutate other workstream/campaign artifacts, issues, labels, or assignees except as explicitly directed by the node outcome and launch instructions.
