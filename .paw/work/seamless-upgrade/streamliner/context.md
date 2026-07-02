# Launch Context - Seamless upgrade (#6)

## Layer 0 - Design Context Hints

Use these as navigation pointers, not as instructions to obey blindly. The selected node should be implemented against the current repository state and the durable PAW plan/review process.

- `docs/design/index.md` - design-layer entry point and reading order. It identifies `daemon.md` as the normative daemon contract and `copilot-bridge-push.md` as the push-delivery design of record.
- `docs/design/daemon.md` - normative local-exchange daemon contract. For this node, start at:
  - `daemon.md` sec. 3.4, per-store isolation and schema-version.
  - `daemon.md` sec. 6.1, version handshake and capability negotiation. Security-sensitive or required capability mismatch must fail closed.
  - `daemon.md` sec. 11.4, ordered handoff and backend-specific transfer/release semantics.
  - `daemon.md` sec. 16, minimal upgrade floor. This explicitly says the full seamless-upgrade platform, including rollback/gc/UX, belongs to this `seamless-upgrade` node.
  - `daemon.md` sec. 17, gating tests and per-backend conformance matrix, especially IPC compatibility, protocol-major parallelism, ordered handoff, schema downgrade/legacy behavior, and daemon process lifecycle cases.
- `docs/design/DECISIONS.md` - ADR log. Relevant anchors:
  - ADR 0015: server-side lease-epoch fence, ordered handoff, and epoch lifecycle.
  - ADR 0020: minimal upgrade floor and two-phase legacy/non-epoch cutover. The minimal floor landed earlier; full rollback/gc/UX and epoch-aware downgrade framework are deferred to this node.
  - ADR 0024: legacy-holder cutover.
  - ADR 0039: push delivery via generic on-deliver exec and Copilot session bridge. It calls out deferred bridge hardening: stale-exe guard, bridge protocol negotiation/enforcement, and `telex copilot gc` for orphaned endpoints.
  - ADR 0040: Copilot skill is binary-owned; plugin skill is only a bootstrap. Version-matched instructions come from `telex copilot skill`.
- `docs/design/ARCHITECTURE.md` - visual on-ramp for local exchange, delivery, restart/re-attach, liveness, epoch fence, and authorization. Non-normative; `daemon.md` governs.
- `docs/design/copilot-bridge-push.md` - push-delivery design of record. Useful sections: lifecycle load/unload, where this lives in code, post-review hardening, compatibility gate, and deferred bridge/protocol cleanup.

## Layer 1 - Worker Mission

Selected node: `seamless-upgrade` in workstream `local-daemon`.

Node outcome anchor: deliver the full seamless-upgrade platform for telex issue #6, not just prerequisite hardening. Completion means versioned install plus stable launcher shim, `telex upgrade` / rollback / gc / version UX, daemon `stop --drain` / drain-restart, backend-specific handoff behavior, schema/protocol compatibility gates, direct old-binary fail-closed behavior, and the Copilot bridge upgrade story are implemented and observable so upgrades do not require tearing down stations. The minimal upgrade floor already exists; this node adds the full platform and mixed-version observability.

Keep the plan and implementation aligned to that outcome. Schema, importer, sanitizer, harness, report-contract, bridge GC, compatibility metadata, or installer scaffolding are prerequisite work unless the plan still ends in live proof/evidence that the seamless-upgrade outcome is satisfied. If the final PR cannot satisfy the node outcome, use `Refs #6` rather than `Closes #6`, mark the state partial or blocked, and preserve the original completion condition for reconciliation.

Primary expected implementation areas based on current repo/design state:

- CLI and command routing: `src/cli.rs`, `src/commands/daemon.rs`, `src/commands/skill.rs`, `src/commands/copilot.rs`, and any new `upgrade`/`rollback`/`gc` command modules needed.
- Daemon lifecycle/IPC: `src/daemon.rs`, `src/daemon_ipc.rs`, `src/commands/daemon.rs`, and daemon process integration tests.
- Backend compatibility and schema gates: `src/backend/sqlite.rs`, `src/backend/postgres.rs`, `tests/conformance.rs`, and backend-specific daemon tests.
- Copilot bridge/versioning: `src/commands/copilot.rs`, `copilot-bridge/`, `COPILOT.md`, `skills/telex/SKILL.md`, and plugin/bridge tests.
- Installer/versioned binary/shim surfaces: inspect existing install scripts and packaging (`install.ps1`, `install.sh`, `Cargo.toml`, plugin packaging) before adding new layout assumptions.

Required behavioral emphasis:

- Upgrades must preserve station wakeability and durable delivery invariants.
- SQLite upgrade handoff should use non-deleting release plus next-call respawn; Postgres may use live transfer where possible.
- Direct invocation of too-old/pre-epoch binaries must fail closed through store/schema/protocol gates, not only through a bypassable launcher shim.
- Protocol-major or required-capability skew must be deterministic and fail closed where required.
- Copilot bridge bytes and skill text must be versioned with telex; stale bridge directories/registries/endpoints should be discoverable and garbage-collectable.
- Mixed-version state should be visible enough for operators and tests to diagnose which daemon/client/plugin/bridge/protocol versions are involved.

## Layer 2 - Relevant State

Workstream state from `.streamliner/workstreams/local-daemon/graph.json` and `brief.md`:

- Design foundation, daemon core, fencing proof, Postgres parity, Copilot plugin, liveness visibility, and push delivery are completed. This node is marked `ready` and depends on `postgres-parity`, `copilot-plugin`, and `push-delivery`.
- The workstream goal is eliminating per-session resident holders by introducing an auto-spawned per-user local daemon that owns presence and delivery for all locally-attended addresses across SQLite and Postgres.
- The design foundation is merged and builder-validated. The authoritative design layer is under `docs/design/`; `daemon.md` is normative where older shaping/brief text differs.
- `daemon-core` included the minimal upgrade floor: versioned shim, daemon `stop --drain`, next-call respawn, and legacy/non-epoch cutover rule. This node is explicitly the full seamless-upgrade platform.
- `push-delivery` changed Copilot delivery to bind -> load bridge -> receive pushed turns -> disposition, while retaining generic `telex wait` for pull users. Do not reintroduce the agent-managed waiter/re-arm loop as the Copilot primary path.

Observed code state from quick source scan:

- `src/cli.rs` already exposes daemon subcommands `serve`, `status`, `version`, `reset`, `session-end`, and `stop --drain`.
- `src/commands/daemon.rs` routes daemon `Status`, `Version`, `Reset`, `SessionEnd`, and `Stop`.
- `src/commands/copilot.rs` already contains `CopilotCmd::Push`, bridge registry/request/response types, bridge push handling, `telex copilot skill` rendering with version/compatibility header, bridge provisioning rollback paths, and tests around protocol/version rendering.
- `src/commands/skill.rs` embeds the generic skill and currently mentions teardown/upgrade guidance for station stop.
- Existing tests with likely overlap include `tests/daemon_core_sqlite.rs`, `tests/daemon_process_sqlite.rs`, `tests/copilot_plugin.rs`, and `tests/conformance.rs`.

Unavailable Inputs

- GitHub issue `https://github.com/lossyrob/telex/issues/6` was unavailable through the launch manifest (`gh issue view` repository resolution failure) and through the GitHub MCP API (404). Treat the graph node summary and repository design docs as the accessible issue/spec source unless access is restored later.
- Manifest entries for `PRODUCT-THESIS.md` and `SKILL.md` as design refs were marked invalid because design refs are restricted to `docs/design/*.md`. Do not retry those as design documents for Layer 0. They may still be ordinary repo files if implementation needs them.
- Repo custom instructions `.github/copilot-instructions.md` are missing.

## Layer 3 - Coordination Context

Upstream/completed dependencies provide context, not new tasks for this worker:

- `postgres-parity` (#42) is completed and should make both backend behaviors available for upgrade/handoff validation.
- `copilot-plugin` (#41) is completed but was later reworked by push delivery; use the current push-delivery path as the operative Copilot model.
- `push-delivery` (#53) is completed and establishes the bridge/on-deliver model. Its deferred bridge hardening items are relevant only insofar as they are part of the seamless-upgrade story: stale bridge guard, bridge protocol negotiation/enforcement, and bridge endpoint/registry GC.

Downstream nodes are validation and scale work, not this worker's assignment:

- `validation-harness` depends on this node and will derive invariant suites and chaos/multi-host validation against the implemented reality.
- `aks-scale-spike`, `scale-rig-and-loop`, `hardening-gate`, and `closure-gate` come later. Do not quietly expand this PR into those nodes; capture deferred validation/scale work in the field report.

PR/lifecycle coordination constraints for the worker:

- Workstream ID: `local-daemon`; selected node ID: `seamless-upgrade`; issue number: `6`; GitHub loop user: `lossyrob`.
- Final PR title must begin with the workstream name in square brackets and include the issue number at the end, e.g. `[local-daemon] ... (#6)`, with the workstream id and issue number represented.
- Use `Closes #6` only if the node outcome anchor is actually satisfied. Otherwise use `Refs #6` and make partial/blocked state explicit.
- The PR body must begin with a collapsible `<details>` / `<summary>Docs.md</summary>` section containing a completed Docs.md per `paw-docs-guidance`.
- After PR creation, immediately enter PAW PR lifecycle Review Response mode and run the canonical Windows review-response loop for the derived repo/PR until the lifecycle reaches the required handoff point.
- Keep field notes during the session and post a concise field report to the node issue after the PR is merged, if issue access is available.
