# Launch Context - Copilot bridge idle drain (defer queued pushes until idle)

## Layer 0 - Design Context Hints

Use these as navigational hints, not as copied requirements. If design prose conflicts, prefer the normative documents called out below and the selected node outcome anchor.

- `docs/design/index.md` - design-layer entry point. It identifies `daemon.md` as the normative daemon contract and `copilot-bridge-push.md` as the push-delivery design narrative.
- `docs/design/daemon.md` - normative local-exchange contract. Relevant areas include daemon-owned delivery state, agent-acked consumption, harness-neutral on-deliver push, and the rule that transport handoff is not the consumed mark.
- `docs/design/copilot-bridge-push.md` - bridge-push design narrative for issue #53. Key invariant: daemon -> `telex copilot push` -> bridge -> Copilot turn is transport only; the agent still records durable Telex ack/disposition after seeing the turn.
- `docs/design/DESIGN.md` - architectural framing for the local exchange. Use for system context, not as the tie-breaker over `daemon.md`.
- `docs/design/DECISIONS.md` - ADR log. ADR 0039 and ADR 0040 are likely relevant to Copilot push delivery and binary-owned Copilot skill behavior.
- `docs/design/ARCHITECTURE.md` - visual on-ramp. Section 9 is referenced by `copilot-bridge-push.md` for push delivery sequence diagrams; `daemon.md` still governs.

## Layer 1 - Worker Mission

Selected node: `bridge-idle-drain` / issue #65, titled **Copilot bridge idle drain (defer queued pushes until idle)**.

Node outcome anchor: fix stale queued Copilot bridge turns. For non-`interrupt` Copilot bridge pushes, if the agent/session is busy, defer delivery until the session is idle / turn-stop, then revalidate durable Telex state before sending. Skip messages already acked or terminally dispositioned. `interrupt` / immediate messages must still push immediately. Durable Telex state and daemon push bookkeeping remain authoritative; bridge memory is not the source of truth.

Required behavior from the graph summary:

- Add bridge busy/idle status.
- Add a deferred on-deliver outcome or drain request path for non-interrupt pushes when the session is busy.
- Trigger drain when the session becomes idle / reaches turn-stop.
- Revalidate durable Telex state immediately before sending deferred messages.
- Preserve batching/ordering for deferred work.
- Cover the manual-read-and-ack-before-idle case with tests: if a queued message is manually read + acked before idle, idle drain must skip it rather than resurrecting it as a turn.

Scope boundaries:

- This node is specifically about busy enqueue -> idle drain for Copilot bridge pushes.
- Do not silently replace the outcome with prerequisite-only bridge plumbing. If prerequisite hardening is necessary, it must still lead to the live proof/evidence for the idle-drain behavior.
- Distinguish from sibling node `bridge-liveness-hardening` (#66), which covers liveness/self-stop/stale-deaf/backstop/status wording issues. Avoid absorbing #66 unless directly necessary for #65.
- Distinguish from `harness-skill-layout` (#61), which is about skill/plugin layout, not queued-turn delivery semantics.

Likely implementation seams discovered locally:

- `src\commands\copilot.rs` is the Copilot harness boundary. It owns `telex copilot attach/resume/push/detach`, embeds `copilot-bridge\extension.mjs`, derives bridge endpoints, builds pushed prompts, and maps attention to Copilot send modes.
- In `src\commands\copilot.rs`, `attention_to_send_mode` currently maps `interrupt` to `immediate` and every other attention level to `enqueue`. This is the selected node's main behavior surface.
- `telex copilot push` currently exits 0 when the bridge accepts a request and never acks Telex; nonzero leaves the message durably unacked for daemon retry. Preserve this durable-consumption invariant.
- `copilot-bridge\extension.mjs` currently calls `session.send(options)` and waits briefly for the SDK send promise. When that short timeout elapses, it returns `{ ok: true, accepted: "pending" }` and keeps observing the promise. The existing comments identify the duplicate/stale-turn risk when the agent is busy.
- `tests\copilot_plugin.rs` covers plugin/hook/bootstrap wiring but does not appear to cover idle-drain semantics. New focused tests will likely need to be added elsewhere around bridge push/daemon delivery behavior.

## Layer 2 - Relevant State

Repository: `telex` at launch cwd `C:\Users\robemanuele\proj\telex\telex`; launch cwd initial branch was `main` and must remain the base/coordination checkout.

Worktree policy from manifest: work in a sibling worktree for the target node branch; do not check out the target node branch in the launch cwd. Place `.paw/work/<workId>` in the execution checkout.

Workstream: `local-daemon` - **Local presence/transport daemon (eliminate the per-session holder)**. The workstream has already merged the design foundation and is building the per-user local exchange, Copilot plugin/push bridge, upgrade path, and validation hardening. This node is a downstream hardening/fix node in that stream, not the entire stream.

Upstream dependency: `push-delivery` is listed as this node's dependency. Treat the existing Copilot push design and implementation as the baseline, then harden it for busy-session queued pushes.

Sibling/downstream coordination background:

- `bridge-liveness-hardening` (#66) is ready/focus and explicitly distinct from #65. It covers live bridge status, self-stop, terminal/no-disposition re-push prevention, stale bridge state, and related #64/#62 overlap.
- `harness-skill-layout` (#61) depends on `push-delivery` and concerns skill/plugin layout.
- Validation-harness and broader hardening nodes are downstream background; do not take them on in this PR unless the builder explicitly expands scope.

Repository guidance found:

- `.github\copilot-instructions.md` is missing according to the manifest.
- Root `COPILOT.md` is the binary-owned Copilot-specific workflow text for push delivery. It says pushed bridge messages arrive as turns; agents should ack/handle by id; `interrupt` is delivered as Copilot `immediate`, other attention levels are `enqueue`; and normal receive path should not proactively drain unseen messages from `telex inbox` while the bridge is live.

Unavailable Inputs:

- `https://github.com/lossyrob/telex/issues/65` was unavailable from both the manifest diagnostics and this launch session's GitHub MCP read (`404 Not Found`). Use the manifest/graph-selected node summary as the selected-node spec unless the issue becomes available later.
- Manifest rejected root `PRODUCT-THESIS.md` and `SKILL.md` as design references because Streamliner design references must be relative `docs/design/*.md` paths. Do not retry them as design-doc sources for launch context.
- `.github\copilot-instructions.md` is missing.

## Layer 3 - Coordination Context

PAW launch configuration requested by the builder:

- Use `paw-lite`.
- Planning docs review is enabled and uses society-of-thought with the ad hoc `general-reviewer` specialist on `claude-opus-4.7-high`, parallel mode, non-interactive, with `premortem` and `retrospective` perspectives.
- Final review is enabled and uses the same society-of-thought configuration.
- Review policy is `final-pr-only`.
- Artifact lifecycle is `commit-and-clean`.
- Work in a worktree from an updated local source branch.

Operational expectations for the worker:

- Before planning, preserve the node outcome anchor: the PR should complete the idle-drain behavior and its evidence, not merely install prerequisites.
- Use the `council` skill only when its gates warrant a contained multi-model deliberation for a consequential uncertain decision. The plan before planning review is the canonical possible trigger; do not run councils reflexively.
- Use the PAW PR lifecycle guidance after PR creation. PR creation is not the terminal handoff; enter Review Response mode and run the canonical review-response loop.
- Final PR title format: begin with the workstream name in square brackets and include the issue number at the end, plus workstream id in the title. Use `Closes #65` only if the node outcome anchor is satisfied; otherwise use `Refs #65` and make partial/blocking state explicit.
- Final PR description must start with a collapsible `<details><summary>Docs.md</summary>` section containing a completed Docs.md following `paw-docs-guidance`.
- Keep field notes during the session for the final field report. After merge, post a concise field report on issue #65 if the issue is available and the lifecycle reaches that point.
