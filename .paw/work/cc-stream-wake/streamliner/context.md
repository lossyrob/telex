# Launch Context - CC / stream-wake (deliberative-table visibility)

## Layer 0 - Design Context Hints

Use these manifest-provided design hints as navigation pointers only; verify exact behavior in code and tests before changing implementation:

- `docs/design/index.md` - design index for local daemon/transport documents.
- `docs/design/daemon.md` - local exchange/daemon contract. Relevant baseline: message frames carry delivery context and `lease_epoch`; primary deliveries are ack-required and CC deliveries are currently visibility-only/auto-seen for transport.
- `docs/design/DESIGN.md` - product/system model and address/occupancy concepts.
- `docs/design/DECISIONS.md` - accepted decisions. Relevant baseline decisions include delivery-role metadata for primary vs CC recipients, CC auto-seen/visibility-only semantics, current-recipient workflow dispositions, and `reply --cc` visibility.
- `docs/design/ARCHITECTURE.md` - architecture-level context for local daemon integration.

## Layer 1 - Worker Mission

Selected node: `cc-stream-wake` / `CC / stream-wake (deliberative-table visibility)`.

Node outcome anchor: implement a daemon delivery-semantics change that lets a seat explicitly opt in to being woken by the stream/table it belongs to, so deliberative-table/CC observer traffic can wake that seat without hand-rolled DB polling. Preserve durable, ack-aware, zero-loss delivery semantics and keep the solution backend-agnostic.

Scope boundary:

- Do implement an explicit opt-in mechanism such as stream/table subscription, wake-on-stream, or a per-address wake-attention setting after a small design pass.
- Do preserve existing primary `--to` ack-required delivery behavior.
- Do preserve current CC observer visibility in `inbox --all` / `read`.
- Do not make all CC traffic wake by default; the node explicitly rejects `CC-always-wakes` because it would reintroduce notification churn.
- Do not replace the node outcome with prerequisite-only design, schema, or harness work. Prerequisite hardening is allowed only if the plan still ends with live proof that the opt-in stream/table wake behavior works.

Expected worker behavior:

- Use `paw-lite` with planning docs review and final review enabled.
- Use the node outcome anchor during planning and while resolving planning-review findings.
- Use a gated council only for consequential uncertain decisions; treat council output like planning-review input that can sharpen or resequence the path but cannot replace the node outcome without builder/orchestrator agreement.
- Read the PAW PR lifecycle guidance before PR lifecycle work, create lifecycle TODOs, and after creating the PR immediately enter Review Response mode rather than handing off at PR creation.
- Keep lightweight field notes outside committed repo artifacts and synthesize them into the final issue field report after merge.

## Layer 2 - Relevant State

Repository/workstream:

- Target repo id: `telex` at launch cwd `C:/Users/robemanuele/proj/telex/telex`.
- Workstream: `local-daemon` - local presence/transport daemon replacing the per-session holder.
- Graph path: `.streamliner/workstreams/local-daemon/graph.json`.
- Selected tracker: `https://github.com/lossyrob/telex/issues/40`.
- The tracker issue was unavailable through the launch manifest and local `gh` lookup, so the graph node summary is the authoritative selected-node source for launch context until issue access is restored.

Current baseline discovered from repository context:

- `docs/design/daemon.md` states `Message` frames carry `delivered_to`, `primary_to`, `cc`, `delivery_role`, attention, disposition flags, and `lease_epoch`; explicit agent ack records durable epoch-guarded consumption for primary deliveries.
- `docs/design/daemon.md` currently says CC recipients are visibility-only: they remain visible in `inbox --all` / `read` with `delivery_role: "cc"`, do not wake `wait`, and do not require manual `ack`.
- `docs/design/DECISIONS.md` decisions 0032-0035 record the accepted CC baseline: delivery-role metadata, CC auto-seen/visibility-only semantics, current-recipient dispositions, and `reply --cc`.
- Existing tests include coverage that CC observers can read/inbox observer messages but are not woken/wedged by visibility-only delivery (`tests/daemon_process_sqlite.rs`) and per-recipient fanout semantics (`tests/conformance.rs`). Expect to update or add tests to prove the explicit opt-in behavior while preserving default no-wake behavior.

Unavailable Inputs:

- `telex:PRODUCT-THESIS.md` and `telex:SKILL.md` appeared in source references but were rejected by the manifest as design inputs because design references must be relative `docs/design/*.md` paths.
- `.github/copilot-instructions.md` is missing in the selected repo.
- GitHub issue `https://github.com/lossyrob/telex/issues/40` was unavailable during manifest generation and also failed via local `gh` with repository resolution errors.

## Layer 3 - Coordination Context

Dependency/background context:

- Upstream dependency `fencing-proof` is completed. Its role was proving epoch-guarded emission and ordered handoff, which this node must not regress.
- Downstream `closure-gate` depends on this node plus hardening/liveness work; this node gates final end-to-end closure but is not responsible for unrelated closure-gate cleanup.
- Related coordination theme: `Coordination hardening (stream-wake + liveness visibility)` groups this node with liveness visibility, but this worker owns only `cc-stream-wake` unless the builder explicitly expands scope.

PR and lifecycle constraints:

- Final PR title format must start with the workstream name in square brackets and include the issue number at the end, e.g. `[local-daemon] ... (#40)`.
- Use `Closes #40` only if the node outcome anchor is satisfied with live proof/evidence. Use `Refs #40` and mark partial/blocked if the PR lands prerequisite or blocker-documentation work without completing stream/table wake opt-in.
- Final PR body must begin with a collapsible `<details>` section whose `<summary>` is `Docs.md` and whose contents follow the PAW Docs.md template.
- After PR creation, enter PAW lifecycle Review Response mode and run the canonical Windows review-response checker loop for the derived repo and PR number, or handle any immediate checker event already present.
- After merge, post a concise field report comment to issue #40 for workstream reconciliation, including outcome, whether the PR closed the issue, key design/implementation decisions, changed assumptions, hidden dependencies, boundary pressure, deferred work, risks, and any builder/orchestrator attention needed.

Authority:

- The implementer may change code in its worktree, create/update its PR, and reply on its own issue/PR.
- Do not create new durable workstream state such as new issues, labels, graph edits, or brief amendments; record those as field-report recommendations for the orchestrator.
