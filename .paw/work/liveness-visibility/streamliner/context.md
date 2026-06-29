# Launch Context - Liveness & visibility hardening (deaf-evidence, terminal-exit-status, foreign-lease view)

## Layer 0 - Design Context Hints

Use these manifest-provided design hints as navigation anchors, not as copied requirements. Treat `docs/design/daemon.md` as the normative daemon contract when design docs disagree.

- `docs/design/index.md` - design entry point for the local-daemon workstream.
- `docs/design/daemon.md` - normative contract for the local exchange daemon, including the Status surface, liveness model, waiter outcome artifacts, membership model, and gating tests.
- `docs/design/DESIGN.md` - local-exchange architecture framing.
- `docs/design/DECISIONS.md` - ADR log; ADRs 0014-0024 cover the local-daemon design foundation.
- `docs/design/ARCHITECTURE.md` - visual/structural on-ramp.

Especially relevant daemon.md sections:

- Section 3.2.1: `telex wait --out-dir` outcome artifacts. `status.json` and `exit.code` are intended to be written for every wait outcome, with `exit.code` written last as completion marker.
- Section 4 / status surface: members include station health, pending unconsumed count, live waiter state, recent errors, store visibility, and related operator-facing diagnostics.
- Sections 9-10: liveness is non-destructive; reaping releases waiters and marks stations idle without destroying durable station state or buffered messages.
- Sections 14.3-14.6: daemon restart/unknown membership is recovered through explicit re-attach via `NeedsAttach`; never resurrect membership from durable history.
- Section 17 gating tests: tests 8-11 cover non-destructive reaping and visibility, while tests 3-5 cover wait/re-attach and outcome safety. This node is an observability seam that validation/harness work can assert through.

## Layer 1 - Worker Mission

Selected node: `liveness-visibility`.

Node outcome anchor: daemon-side observability must make deaf stations, abnormal waiter exits, and ghost/foreign leases self-evident without requiring the operator or peer sessions to infer the state manually. The completion condition is not merely prerequisite cleanup; it should end with demonstrable CLI/status visibility and tests/proof for these three visibility surfaces.

The node responsibility is specifically:

1. Make the already-computed `unattended_with_backlog` / `UnattendedWithBacklog` grade visible across the right operator surfaces and proactively surfaced past a threshold, so a station with queued work and no live waiter is unmistakable.
2. Author a consistent terminal waiter status for every waiter exit path, including kill/no-op/lost-race/non-delivery paths, and ensure the terminal status is decoupled from the waiter itself being able to write an out-dir file.
3. Add an all-sessions / foreign-lease visibility view so a session-scoped status command no longer hides ghost leases or live activity owned by another session/store context.

Keep this distinct from broader local-daemon implementation work. Do not silently expand into full daemon-core, plugin heartbeat/guard implementation, validation-loop harness work, Postgres parity, or seamless upgrade unless the builder explicitly redirects scope.

Current code seams to inspect first:

- `src/daemon.rs` currently computes station health in `MemberRecord::station_health`; `UnattendedWithBacklog` already exists when `pending_unconsumed_count > 0` and there are no live waiters.
- `src/daemon_ipc.rs` defines `MemberStatus`, `LiveWaiterStatus`, and `StationHealth` including `UnattendedWithBacklog`.
- `src/commands/status.rs` enriches address-scoped `telex status` output with daemon members, live waiters, station health, pending counts, and an `also_active_on` warning for alternate backend activity.
- `src/commands/address.rs` includes station health and pending waiter counts in `address list` / `address show` JSON, but text output is sparse.
- `src/commands/station.rs` exposes session-scoped `station status` and currently filters members by the current session id.
- `src/commands/wait.rs` implements `--out-dir` artifacts (`message.json`, `delivery.json`, `status.json`, `exit.code`, `wait.pid`) and has tests for normal delivery, timeout, daemon-gone, error, stale message clearing, and owner-only artifact permissions.
- `src/cli.rs` defines `WaitArgs`, `StationStatusArgs`, `AddressListArgs`, and related CLI surfaces. There is no obvious existing `--all-sessions` option on `station status` or `address list`.

Suggested proof direction:

- Add focused unit/integration coverage around the exact visibility contract, not only internal enum computation.
- Test JSON output shapes where downstream harnesses will assert them.
- Test text output only where operator visibility is part of the contract.
- Include a lost-race/no-op waiter exit proof if the implementation adds a daemon-authored terminal waiter status path.
- If a full live proof cannot be completed in this node, preserve the outcome anchor and use `Refs #46` rather than `Closes #46` in the final PR.

## Layer 2 - Relevant State

Workstream: `local-daemon` / "Local presence/transport daemon (eliminate the per-session holder)".

Workstream framing: telex is moving from a per-session resident holder to an auto-spawned per-user local daemon that owns presence and delivery for all locally attended addresses. The daemon owns backend connections, durable delivery buffering, attendance, lease heartbeat, IPC, pid-watch, and status visibility.

Selected graph node metadata:

- Node ID: `liveness-visibility`
- Type: task
- Status: ready
- Attention: watch
- Tracker: `https://github.com/lossyrob/telex/issues/46`
- Depends on: `fencing-proof`
- Repo: `telex`

Upstream dependency context:

- `fencing-proof` is marked completed. It proves epoch-guarded emission and ordered handoff: delivery emission is server-side fenced, graceful handoff/crash cannot double-deliver or flip ownership, and downstream branches can rely on that proof.

Downstream coordination context:

- `closure-gate` depends on `liveness-visibility` along with `hardening-gate` and `cc-stream-wake`. It validates the full end-to-end unblock and retires superseded mechanisms. Treat this node's observability surfaces as evidence the closure/hardening work can consume, not as the closure gate itself.

Relevant design facts to preserve:

- Liveness is a non-destructive UX dial, not a correctness gate. Reaping completes blocked waits and marks stations idle, but never destroys membership/durable buffers or loses messages.
- `UnattendedWithBacklog` means pending unconsumed messages exist while no live waiter is armed. This should become obvious to a peer/operator and should not be hidden by a session-scoped status view.
- `telex wait --out-dir` outcome artifacts are transport/visibility only. `exit.code` is the completion marker; stdout flush or file writing is not the consumed mark.
- Waiter exit status must be observable even when the waiter cannot be trusted to write the outcome itself.
- Session-scoped status is useful but insufficient for ghost/foreign leases; this node should expose an intentional all-sessions/foreign view instead of requiring manual backend inspection.

Unavailable Inputs

- The manifest marks the GitHub tracker issue body unavailable (`github_issue_unavailable`), and a direct `gh issue view 46 --repo lossyrob/telex` also failed with repository resolution. Use the graph node metadata as the authoritative node outcome anchor unless the issue becomes available later.
- The manifest rejected root-level `PRODUCT-THESIS.md` and `SKILL.md` as design inputs because design references must be relative `docs/design/*.md` paths. Do not treat those as Layer 0 design hints for this launch unless separately needed during implementation.
- `.github/copilot-instructions.md` is missing in the selected repo; follow repository-local conventions from the code and the PAW/Streamliner instructions instead.

## Layer 3 - Coordination Context

PAW launch configuration requested by the builder is paw-lite with planning docs review enabled and both planning/final reviews using a contained society-of-thought review by the ad hoc `general-reviewer` persona on `claude-opus-4.7-high`. The durable PAW fields should encode that; launch-time kickoff instructions remain owned by Streamliner.

Node outcome preservation is important: planning review may add prerequisites, hardening, safety gates, or sequencing, but it must not replace the node outcome with enabling work. If the final PR only lands prerequisites or blocker documentation, it should use `Refs #46`, make the partial/blocking state explicit, and preserve the original completion condition for reconciliation.

PR coordination requirements for the worker:

- Role: implementer.
- Issue: `#46`.
- Workstream ID: `local-daemon`.
- GitHub loop user: `robemanuele_microsoft`.
- Final PR title must start with the workstream name in square brackets and include the issue number at the end.
- The PR description should use `Closes #46` only if the node outcome anchor is actually satisfied; otherwise use `Refs #46` and clearly state the partial/blocking condition.
- The PR description should include a top collapsible `<details>` section with `<summary>Docs.md</summary>` containing a completed Docs.md following PAW docs guidance.

Lifecycle coordination:

- After PR creation, PR creation is not the terminal handoff. Enter lifecycle Review Response mode and use the canonical implementer review-response loop/checker for the derived repo and PR number.
- Stop only for serious blockers or if the GitHub issue needs amendment. Do not create or mutate shared workstream state beyond this node's issue/PR authority.
- Keep lightweight field notes during execution for final reconciliation. After merge, post a concise field report comment on issue #46 covering outcome, partial/blocked status if any, important decisions, stale context, hidden dependencies, boundary pressure, deferred work, and orchestrator/builder attention items.

Worktree coordination:

- Launch cwd is the base/coordination checkout on `main`: `C:\Users\robemanuele\proj\telex\telex`.
- Work should run in a sibling worktree for the target node branch, not by checking out the node branch in the launch cwd.
- Keep `.paw/work/<workId>` in the execution checkout/worktree.
