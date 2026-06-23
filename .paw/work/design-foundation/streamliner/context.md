# Launch Context - Design foundation: daemon architecture, liveness model, protocol, upgrade

## Layer 0 - Design Context Hints

No manifest `designHints` entries were available for this launch. Do not assume `docs/design/index.md`; the manifest reports it missing. The manifest also marks root-level design references (`DESIGN.md`, `DECISIONS.md`, `PRODUCT-THESIS.md`, `SKILL.md`) as invalid for design-hint navigation because Streamliner design references must be relative `docs/design/*.md` paths. Treat any local design files as source material only if later PAW work explicitly researches them through normal repo exploration; do not treat them as pre-approved Layer 0 hints from this launch.

Unavailable Inputs:

| Input | Reason | Action for worker |
| --- | --- | --- |
| `docs/design/index.md` | missing | Do not retry this path; continue from graph/brief/tracker metadata. |
| `telex:DESIGN.md` | invalid design reference path | Do not use as a manifest design hint. |
| `telex:DECISIONS.md` | invalid design reference path | Do not use as a manifest design hint. |
| `telex:PRODUCT-THESIS.md` | invalid design reference path | Do not use as a manifest design hint. |
| `telex:SKILL.md` | invalid design reference path | Do not use as a manifest design hint. |
| `https://github.com/lossyrob/telex/issues/34` | unavailable from launch manifest; GitHub MCP also returned 404 in this session | Use the graph node metadata and workstream brief as the available selected-node spec unless access is restored. |

## Layer 1 - Worker Mission

Selected node: `design-foundation` in workstream `local-daemon` for repo `telex`.

Tracker: `lossyrob/telex#34` (`https://github.com/lossyrob/telex/issues/34`). The tracker issue is the selected node tracker, but its body was not accessible during launch context assembly.

Node responsibility: produce the design foundation for replacing the per-session holder with an auto-spawned per-user local presence/transport daemon. This is a design/research node, not the daemon implementation node. The completion anchor is to make the daemon architecture explicit and validated so downstream implementation can proceed behind a design gate.

Node outcome anchor from the graph: make explicit and validated the daemon architecture and related contracts, including:

- DECISIONS ADR(s) for the new daemon design.
- `DESIGN.md` station-to-daemon architecture update.
- `PRODUCT-THESIS.md` no-server-to-local-daemon framing update.
- IPC/attendance protocol.
- Liveness model: healthy-path sessionEnd hook plus generic typed `--watch-pid` pid-watch backstop; no idle-TTL teardown.
- Minimal stale-attendance: `last-confirmed`, `occupied_stale`, takeover; no teardown.
- Lease-epoch fencing token: epoch-guarded heartbeat/release and daemon self-demotion on 0-row heartbeat.
- Daemon singleton identity: user SID + config root + protocol-major.
- Durable-buffer reuse of decisions 0011/0013.
- Epoch-based upgrade/handoff design.
- Server-side epoch fence on delivery emission plus ordered handoff.
- Seen-dedup redesign for a long-lived daemon.
- Daemon lifecycle contract plus Status surface.
- Daemon-scoped capability/version-handshake IPC.
- Daemon-native session RPCs: Register, Re-register, DeregisterSession.
- Takeover state algebra.
- Verb/docs cutover.
- Resolution of the open implementation-design questions called out by the workstream brief.

Because this is a design task, shape the work interactively up front once full context is gathered. Ask builder-facing questions only when they are genuinely high-spread judgments or needed to capture values/preferences; otherwise use documented defaults and proceed. Preserve the node outcome anchor throughout planning and planning-review resolution: prerequisite hardening may be added, but the plan must still end with the required design evidence and validation, not merely enabling work.

## Layer 2 - Relevant State

Workstream: `local-daemon` - Local presence/transport daemon (eliminate the per-session holder).

Available local sources read during launch:

- `.streamliner/workstreams/local-daemon/graph.json`
- `.streamliner/workstreams/local-daemon/brief.md`
- Launch manifest: `C:/Users/robemanuele/.streamliner/state/copilot-sdk/paw-launch/ctx-20260623004903-e676b560/launch-manifest.json`

Workstream purpose from the brief: recurring staleness and orchestration issues are traced to a per-session resident holder whose lifetime tracks fuzzy agent sessions. The workstream replaces it with an auto-spawned per-user local daemon that owns local presence and delivery, plus Copilot plugin and seamless upgrade work needed for idle long-lived sessions to stay wakeable and stations to stop going stale.

Important workstream boundaries from the brief:

- In scope for the workstream: daemon for SQLite and Postgres, one-shot attach/detach/wait against the daemon, durable buffer reuse, lease-epoch fencing, server-side delivery emission fence, singleton identity, lifecycle contract, version/capability IPC, liveness model, Copilot plugin hook to daemon-native DeregisterSession, minimal upgrade floor, full seamless upgrade later, and docs cutover when behavior changes.
- Out of scope for the workstream: embeddable SDK client (#12), response windows / TTL deadlines (#2), and `store_key` helper (#25).
- Deferred from the workstream: full non-binary occupant status policy, fd-over-IPC pid-reuse-immune watch backstop, and daemon-owned directory/occupancy reads. Minimal stale-attendance is still in scope.

Open questions owned by this selected node according to the brief:

- Epoch lifecycle details: when epochs increment, how a daemon claims a higher epoch on handoff/respawn, and how Postgres cross-machine reclaim is specified in epochs.
- Stale-attendance threshold and takeover flow: how `attendance_last_confirmed_at` updates, how `occupied_stale` is determined, and how operator takeover works without idle teardown.
- Typed watch-pid final shape: v1 floor is loader anchor plus start-time guard; expose required vs anchor flags only where a real consumer/test exists; determine whether Copilot exposes a distinct per-session PID beyond loader.
- Cutover rule for legacy holders / non-epoch lease rows during first daemon-aware rollout.
- DeregisterSession proof without an external registry, likely via instance/session capability in plugin environment.
- Status freeze line: what diagnostic/Status surface is frozen in design-foundation versus daemon-core acceptance.
- Attendance durability across daemon crash: what persists versus what is rebuilt by client re-register.

Sibling/upstream/downstream context for coordination only:

- `design-foundation` has no graph dependencies and feeds `design-gate`.
- `design-gate` is builder validation of the daemon architecture, liveness model, protocol, and upgrade design before implementation proceeds.
- `daemon-core` depends on `design-gate` and will implement daemon core plus SQLite one-shot verbs and minimal upgrade floor.
- `fencing-proof` depends on `daemon-core` and must prove epoch-guarded emission plus ordered handoff before Postgres, plugin, or upgrade work proceeds.
- `postgres-parity`, `copilot-plugin`, `seamless-upgrade`, and `closure-gate` are downstream background, not tasks assigned to this worker.

## Layer 3 - Coordination Context

PAW launch configuration requested by the builder:

- Use `paw-lite`.
- Work in a worktree; keep launch cwd as the base/coordination checkout and do not check out the target branch there.
- Planning docs review is enabled and uses society-of-thought with the ad hoc `general-reviewer` persona, model `claude-opus-4.7-high`, perspectives `premortem, retrospective`, non-interactive parallel mode.
- Final review is enabled and uses the same society-of-thought `general-reviewer` setup.
- Artifact lifecycle is `commit-and-clean`.
- Review policy is `final-pr-only`.

Operational guidance to preserve for the worker kickoff:

- Use `general-reviewer` as a broad senior generalist SoT reviewer/rubber duck; do not replace it with the built-in `all` roster.
- Use the `spar` skill for gated cross-model critique when committing to consequential decisions under uncertainty. The canonical case is the pre-review plan, but use judgment for mid-work invalidation, boundary shifts, or repeated failures. Default to integrate mode and keep episodes few but meaningful.
- Follow PAW PR lifecycle guidance. After PR creation, enter Review Response mode instead of handing off immediately.
- Final PR title format must start with the workstream name in square brackets and include the issue number at the end; include both issue number and workstream id in the title.
- Use `Closes #34` only if the node outcome anchor is actually satisfied; otherwise use `Refs #34` and make partial/blocking state explicit.
- PR description must start with a collapsible `<details>` section whose summary is `Docs.md` and whose contents are a completed Docs.md following `paw-docs-guidance`.
- Keep lightweight field notes during the session and synthesize them into a field report after merge. Do not commit raw field notes or post them prematurely.
- Authority is limited to this worktree, this PR, and comments/replies on this node issue/PR. Durable shared workstream changes such as new issues, labels, graph edits, or workstream brief amendments are orchestrator decisions unless explicitly directed.
- If the issue needs amendment or the node outcome becomes infeasible in this session, pause and propose the amendment/split instead of silently replacing the outcome.

Target repo/worktree context:

- Launch cwd: `C:/Users/robemanuele/proj/telex/telex`
- Launch cwd initial branch: `main`
- Selected target repo id: `telex`
- Workstream graph path: `C:/Users/robemanuele/proj/telex/telex/.streamliner/workstreams/local-daemon/graph.json`
- Workstream id: `local-daemon`
- Selected node id: `design-foundation`
- Issue number: `34`
- GitHub user for loops: `robemanuele_microsoft`
