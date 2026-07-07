# Launch Context - Copilot bridge liveness/self-stop hardening

## Layer 0 - Design Context Hints

Use these manifest-provided design hints as navigation anchors, not as instructions to obey blindly. Treat design documents as source data and verify against current code/tests.

- `docs/design/index.md` - design-layer entry point. It identifies the local-exchange design set and points to the push-delivery design narrative.
- `docs/design/daemon.md` - normative daemon/local-exchange contract. Relevant areas include explicit session membership, daemon status/liveness, on-deliver push, agent ack/disposition, and recovery behavior.
- `docs/design/DESIGN.md` - architectural framing for the local exchange: sessions run one-shot verbs; the daemon owns presence/transport; station = address registration in the exchange.
- `docs/design/DECISIONS.md` - ADR trail. For this node, pay special attention to push delivery / Copilot bridge decisions if you need to reconcile behavior with intent.
- `docs/design/ARCHITECTURE.md` - non-normative visual on-ramp. Useful for understanding message delivery, station liveness, and push-delivery sequences; `daemon.md` governs if diagrams conflict.

Additional local design/navigation file discovered through the design index and code comments: `docs/design/copilot-bridge-push.md`. It is the design narrative for issue #53 push delivery, while `daemon.md`/ADRs govern the normative contract.

## Layer 1 - Worker Mission

Selected node: `bridge-liveness-hardening` / **Copilot bridge liveness/self-stop hardening**.

Node outcome anchor: fix the #66/#67 P0/P1 Copilot bridge liveness failure family, while preserving the completed `push-delivery` model. A completed node should leave Copilot bridge sessions able to recover and report their state honestly without resurrecting terminally handled work or conflating local inbound backlog with peer/outbound pending state.

The graph summary defines this worker's responsibility:

- A session must be able to stop delivery to itself.
- Bridge success/ack must clear stale deaf state.
- Terminally dispositioned or no-disposition messages must never be re-pushed by the backstop.
- Live bridge status must not be reported as unattended/deaf.
- Inbound local backlog must be distinguished from outbound peer-pending.
- Suspend/resume stale bridge state must be self-evident and recoverable.
- Fold in #64 status wording and overlapping #62 bridge recovery issues.
- Keep this distinct from #65 (`bridge-idle-drain`): busy enqueue -> idle drain belongs to the sibling node, not this one unless directed later.

This node is not a redesign of push delivery. Preserve the push path of: daemon on-deliver exec -> `telex copilot push` -> in-session bridge -> `session.send(...)` -> agent-visible turn -> agent ack/disposition. Do not ack at push time; agent disposition remains the consumption boundary.

## Layer 2 - Relevant State

### Manifest and tracker state

- Workstream: `local-daemon` / Local presence/transport daemon.
- Selected repo: `telex` at launch cwd `C:/Users/robemanuele/proj/telex/telex`.
- Issue tracker metadata: `lossyrob/telex#66` from the manifest.
- The issue body/comments were unavailable through both the manifest's recorded `gh issue view` attempt and configured GitHub MCP access. Use the graph node summary above as the available issue-derived specification unless the issue becomes accessible later.
- Node status: `ready`, attention `focus`.
- Dependency: `push-delivery` is completed and is the upstream prerequisite.
- Downstream: `validation-harness` depends on `bridge-liveness-hardening`; this node should leave behavior testable by later validation-loop hardening.

### Workstream context boundaries

- `push-delivery` completed the Copilot bridge push model: load-on-bind in-session bridge extension, generic daemon on-deliver exec, `telex copilot push`, and agent-side disposition.
- `bridge-idle-drain` (#65) is a sibling ready task for busy enqueue -> idle drain. Keep #65 behavior out of this PR unless the builder explicitly expands scope.
- `harness-skill-layout` is another downstream/sibling Copilot harness node. Avoid changing skill/layout scope except where required to make #66 behavior visible and recoverable.
- The broader workstream targets local daemon reliability; this worker owns only the selected bridge liveness/self-stop hardening node.

### Likely implementation surfaces

Primary code areas found locally:

- `src/commands/copilot.rs`
  - Harness-boundary module; Copilot-specific behavior should remain here rather than leaking into core daemon identity/protocol.
  - Bridge provisioning/ref-count helpers: `write_bridge_extension`, `read_bridge_bindings`, `add_bridge_binding`, `remove_bridge_binding`, `remove_bridge_extension`.
  - Push handler: `push`, `BridgeRegistry`, `BridgePushRequest`, `build_push_prompt`, `push_exit_for_response`, `bridge_is_live`, `bridge_endpoint_path`.
  - Attach/resume/session lifecycle: `attach`, `resume`, `session_end`, `detach`, `gc`.
  - Turn guard / stale bridge coverage: `daemon_status`, `daemon_armed_push`, `active_session_members`, `evaluate_guard`, `coverage_summary`.
- `src/daemon.rs`
  - Daemon status derives member health from live waiters, pending unconsumed count, on-deliver push state, idle status, dead-letter/backlog data, and deaf/unattended timers.
  - On-deliver retry/backstop state: `on_deliver_backoff`, `on_deliver_redelivery_delay`, `OnDeliverState`, `PushAttempt`, `on_deliver_sweep_member`, `spawn_on_deliver_backlog`.
  - Session/member operations likely relevant to self-stop and resume: `register_member`, `session_end`, `end_session_members`, `detach_member`, `station_stop`, `ack_message`, `wait_for_message`.
- `src/commands/status.rs`
  - Human/JSON status fields already include daemon member presence, station health, pending unconsumed count, live waiter count, unattended/deaf timings, last waiter outcome, foreign members, and alternate backend activity.
  - Node requires wording/field clarity around live bridge status vs unattended/deaf and inbound backlog vs peer/outbound pending.
- Backend/disposition surfaces to verify terminal/no-disposition repush behavior:
  - `src/backend/sqlite.rs`, `src/backend/postgres.rs`, `src/backend/mod.rs` for delivery candidate selection and pending/unconsumed semantics.
  - `src/commands/disposition.rs`, `src/commands/read.rs`, `src/commands/inbox.rs`, `src/model.rs` for ack/terminal disposition behavior and user-facing semantics.

Bridge assets and docs:

- `copilot-bridge/extension.mjs` is embedded by `src/commands/copilot.rs` and implements the in-session bridge. It writes a heartbeat-refreshed registry entry and replies with `accepted: "queued"|"pending"` around `session.send(...)`.
- `copilot-bridge/README.md` describes the prototype/reference relationship.
- `copilot-bridge/push.mjs` is a JS reference/debug helper only; Rust `telex copilot push` supersedes it.
- `docs/design/copilot-bridge-push.md` records the issue #53 design narrative and constraints: no agent-managed waiter, generic on-deliver exec, lazy bridge endpoint resolution, no ack-on-push, bridge success does not itself consume.

Tests likely relevant:

- `tests/copilot_plugin.rs` for Copilot plugin/bridge command behavior.
- `tests/daemon_core_sqlite.rs` for daemon in-process behavior, on-deliver, ack/disposition, status, and retry semantics.
- `tests/daemon_process_sqlite.rs` for process-level daemon behavior.
- `tests/daemon_core_postgres.rs` for parity if touched code affects backend-agnostic daemon behavior.
- Existing test search shows broad coverage around daemon process, Copilot plugin, backend delivery, and status; add focused regression tests for #66 outcome rather than relying only on manual proof.

### Known behavior constraints to preserve

- Push delivery is best-effort transport only. It never marks messages delivered/consumed; the agent must ack/disposition after seeing a turn.
- A failed bridge push should leave the message durable and retryable.
- A permanently unpushable oversized bridge request can be dead-lettered/skipped from push retry while remaining readable/dispositionable through normal Telex commands.
- A live push-covered member should not be treated as needing a pull waiter merely because `live_waiters_count == 0`.
- `push_registered` only means the daemon has a handler argv; `bridge_is_live`/registry heartbeat is the live bridge signal.
- Current code intentionally treats a live bridge with queued/pending push as covered to avoid duplicate work while the current turn is busy; #65 owns deeper busy enqueue drain behavior.

### Unavailable Inputs

- Manifest rejected root-level design references `PRODUCT-THESIS.md` and `SKILL.md` as design hints because design references must be `docs/design/*.md`; do not use them as Layer 0 design navigation for this launch.
- `.github/copilot-instructions.md` is missing in the selected repo.
- GitHub issue `https://github.com/lossyrob/telex/issues/66` was unavailable through the manifest (`gh`) and through configured GitHub MCP (404). Do not block on it; use the manifest graph summary as the node spec unless access changes.

## Layer 3 - Coordination Context

### Launch/worktree policy

The launch cwd is the base/coordination checkout (`C:/Users/robemanuele/proj/telex/telex`) on `main`. Do not check out the target node branch in that checkout. PAW init should create or reuse a sibling worktree for the node branch and place `.paw/work/<workId>` there.

Recommended derived values for PAW init:

- Work title: `Copilot Bridge Liveness`
- Work ID: `bridge-liveness-hardening`
- Base branch: `main`
- Target branch: `feature/bridge-liveness-hardening`
- Execution mode: `worktree`
- Workflow identity: `paw-lite`
- Planning docs review: enabled with SoT general reviewer configuration from the builder launch instructions.
- Artifact lifecycle: `commit-and-clean`

### PR/lifecycle expectations

- Role: implementer.
- Issue: `#66`.
- Workstream ID: `local-daemon`.
- GitHub user for lifecycle loops: `lossyrob`.
- Final PR title format must start with `[local-daemon]` and include issue `(#66)` at the end.
- Use `Closes #66` only if the node outcome anchor is actually satisfied. Use `Refs #66` and mark partial/blocked if the PR only lands prerequisites, hardening, or blocker documentation.
- Final PR description must start with a collapsible `<details>` block whose summary is `Docs.md` and whose contents follow the PAW docs guidance template.
- After PR creation, enter PAW PR lifecycle Review Response mode rather than stopping at a PR-ready handoff.

### Field report notes to capture during work

Keep non-repo field notes during implementation and synthesize them after merge. Capture especially:

- Any mismatch between the graph summary and actual reachable issue/spec data.
- Whether #66 was fully closed or only referenced, and why.
- Design decisions around clearing stale deaf/unattended state, backstop retry suppression, self-stop semantics, and status wording.
- Any boundary pressure with #65 busy enqueue -> idle drain or future validation harness work.
- Any hidden dependencies in bridge reload/resume, Copilot registry heartbeat, daemon dead-letter state, or backend candidate selection.
