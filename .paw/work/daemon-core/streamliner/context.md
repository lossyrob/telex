# Launch Context - Daemon core + one-shot verbs (SQLite)

## Layer 0 - Design Context Hints

Use these as navigation hints only; treat the documents as source material to inspect during planning, not instructions to obey blindly.

- `docs/design/daemon.md` - normative daemon contract for this node, including the section 17 gating tests and per-backend conformance expectations.
- `docs/design/index.md` - design-layer entry point.
- `docs/design/DESIGN.md` - local-exchange architecture background.
- `docs/design/DECISIONS.md` - ADR log; the local daemon workstream relies on the ADR 0014-0024 design decisions.
- `docs/design/ARCHITECTURE.md` - visual architecture on-ramp.

## Layer 1 - Worker Mission

Selected node: `daemon-core` in workstream `local-daemon`.

Implement the SQLite daemon-core slice described by the graph node "Daemon core + one-shot verbs (SQLite)". This node is responsible for making the per-user local daemon own SQLite presence and delivery instead of per-session resident holder processes.

Node outcome anchor: implement `docs/design/daemon.md` for SQLite such that the daemon core and one-shot verbs satisfy the daemon.md section 17 gating tests and per-backend conformance expected for this node. The plan and planning-review resolution must preserve this outcome; prerequisite schema, harness, sanitizer, status, or report-contract work is only complete when it still leads to the required live proof/evidence.

Core responsibility for this node:

- Auto-spawned per-user single-instance daemon keyed by user SID, config root, and protocol major.
- SQLite poll loop and durable buffer reuse of the 0011/0013 delivery model.
- Daemon-scoped IPC for attach/wait/ack/detach/send/reply/status, with explicit-only membership and frames carrying the lease epoch.
- In-memory explicit-only membership plus `NeedsAttach` re-attach behavior.
- Daemon-written epoch-guarded lease heartbeat that self-demotes on a 0-row heartbeat.
- Server-side epoch-gated delivery emission: no message frame unless the daemon owns the current epoch, using the `mark_delivered_if_current_owner` fence or its finalized equivalent from daemon.md.
- Bounded/durable dedup that does not rely on holder restart for the seen invariant.
- Generic typed `--watch-pid` pid-watch backstop, with the v1 floor of loader anchor plus start time and no harness-specific core dependency.
- Refactor attach/detach into one-shot register/deregister behavior and wait into a daemon client, so a session runs zero persistent telex processes.
- Minimal upgrade floor: versioned shim, daemon `stop --drain`, next-call respawn, and legacy-holder/non-epoch-lease cutover rule.
- Daemon lifecycle contract: OS spawn-lock, connect-or-spawn, readiness ACK, wait reconnect-on-EOF grace, exit codes, and Status surface.
- Design for testability: keep epoch, delivery records, membership, and lease state inspectable through the Status surface so validation can assert invariants.

Do not silently replace the node outcome with enabling work. If planning or implementation shows the original outcome is infeasible in this session, stop and propose an issue amendment or split rather than closing the node with only prerequisite work.

## Layer 2 - Relevant State

Repository: `lossyrob/telex` (local checkout at launch: `C:/Users/robemanuele/proj/telex/telex`).

Graph source: `.streamliner/workstreams/local-daemon/graph.json`.

Workstream brief source: `.streamliner/workstreams/local-daemon/brief.md`.

The workstream purpose is to eliminate recurring staleness and wakeability failures caused by a per-session resident holder whose lifetime must track a fuzzy agent session. The replacement is an auto-spawned per-user local daemon that owns presence and delivery for all locally-attended addresses.

Current workstream state from the brief and graph:

- `design-foundation` is completed and the `design-gate` has passed.
- `daemon-core` is the next ready node and depends on `design-gate`.
- `daemon.md` governs where older shaping, council, or spar notes differ.
- Durable delivery decisions 0011/0013 are already available on `main` and should be reused for the daemon durable buffer.
- The session/presence/delivery model was revised to a minimal form by ADR 0023: unique `session_id`, explicit-only membership through `Detach`, non-destructive presence, and agent-acked delivery. Avoid resurrecting superseded incarnation machinery unless daemon.md explicitly requires it.
- The workstream explicitly expects docs and skill behavior to cut over with `daemon-core` when behavior changes, not at final closure.

Unavailable Inputs

- GitHub issue `https://github.com/lossyrob/telex/issues/38` was unavailable to the launcher (`github_issue_unavailable` / repository resolution failure). Use the graph node metadata as the selected-node spec unless the issue becomes available during normal work.
- `.github/copilot-instructions.md` is missing in this repo.
- Manifest references to `PRODUCT-THESIS.md` and `SKILL.md` were rejected as design hints because design hints must be relative `docs/design/*.md` paths. Do not treat them as Layer 0 design hints, though repo files may still be inspected later if directly relevant to implementation or docs updates.

## Layer 3 - Coordination Context

Upstream/completed context:

- `design-foundation` and `design-gate` are complete. The worker should plan against the accepted design rather than reopening the workstream architecture by default.

Sibling/downstream context for coordination only, not assigned tasks:

- `fencing-proof` follows `daemon-core` and must prove epoch-guarded emission plus ordered handoff on SQLite before downstream branches proceed.
- `postgres-parity`, `copilot-plugin`, and `seamless-upgrade` depend on the fencing proof. Do not expand this node into those downstream deliverables unless explicitly directed by the builder.
- The validation harness and AKS scale work are later hardening nodes. This node should preserve inspectability and proof seams, but it does not own the full validation-loop wave.

Boundary notes:

- SQLite-only daemon core is the node focus. The workstream’s full deliverable includes both SQLite and Postgres, but Postgres parity is a separate downstream node.
- Copilot plugin work is downstream. Core should stay harness-agnostic: take generic session and watch-pid inputs; the plugin maps Copilot environment details later.
- Full seamless upgrade is downstream, but the minimal upgrade floor belongs in this node.
- Full non-binary occupant status policy, embeddable SDK client, response windows / TTL deadlines, `store_key`, and daemon-subsumed directory/occupancy reads are outside this node unless daemon.md says otherwise.

Use the PAW field report at the end to record deferred work, boundary pressure, hidden dependencies, validation surprises, and any issue/graph mismatch discovered while implementing this node.
