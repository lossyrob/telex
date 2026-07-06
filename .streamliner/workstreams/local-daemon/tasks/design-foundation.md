# Node: design-foundation

- **Workstream:** local-daemon
- **Type:** research (design)
- **Status:** ready
- **Depends on:** (none)
- **Blocks:** design-gate -> daemon-core
- **Tracker:** local (this file); promote to a GitHub issue under lossyrob/telex at
  wave promotion if GitHub-backed execution is desired. Umbrella issue: #32.

## Purpose

Make the local-daemon architecture explicit and builder-validated **before any
implementation**. This node produces the design layer the rest of the workstream
executes against: the decision record(s), the updated living design docs, the
IPC/attendance protocol, the liveness model, the durable-buffer reuse, and the
upgrade/handoff design. It also resolves the two open implementation-design
questions so `daemon-core` and the completeness tracks have a stable contract.

This is a design/writing node. It does **not** implement production code.

## Context and design references

Read first:

- [`docs/initial-shaping.md`](../docs/initial-shaping.md) - the full decision ledger
  and rationale behind this workstream (the authoritative input for this node).
- `telex:DESIGN.md` - "Station" section and "Architecture overview"; the
  holder/waiter model being restructured.
- `telex:DECISIONS.md` - 0004 (holder/waiter split, the decision being revisited),
  0005 (TTL heartbeat), 0011/0013 (durable per-recipient delivery, reused as the
  daemon buffer).
- `telex:PRODUCT-THESIS.md` - the "no server at all" framing being shifted.
- `telex:SKILL.md` - the holder/waiter re-arm guidance the daemon supersedes.
- Code seams to ground the design: `src/commands/attach.rs` (holder loop, `drain`,
  `State`), `src/commands/wait.rs` (exit-code contract), `src/ipc.rs` (current
  framing), `src/session_watch.rs` (`process_alive`, `resolve_session_pid`),
  `src/registry.rs` (holder records), and on branch
  `feature/copilot-session-end-plugin`: `src/session_registry.rs`,
  `src/commands/session_end.rs`, `integrations/copilot-cli/`.

## Deliverables

1. **Decision record(s) in `DECISIONS.md`** (extend the existing numbered series,
   currently through 0013). At minimum one ADR for the daemon presence/transport
   split that captures: the per-user auto-spawned daemon; zero persistent session
   processes (one-shot register/wait/release); the liveness model (hook healthy-path
   + a typed `--watch-pid` backstop, v1 floor loader anchor + start-time; no idle-TTL
   teardown; lease TTL as the daemon-down backstop) plus the **stale-attendance/
   takeover state algebra** (attended / occupied_stale / takeover-eligible;
   last-confirmed + safe operator takeover; no teardown); the **server-side
   lease-epoch fence** (epoch-guarded heartbeat/release AND delivery emission via
   `mark_delivered_if_current_owner` - no Message frame unless the daemon owns the
   epoch; self-demote closes waiters; ordered handoff: quiesce -> flush pending
   mark_delivered -> unbind -> claim new epoch; remove occupant-null release); the
   **seen-dedup redesign** for a long-lived daemon (bounded/durable tombstone, no
   reliance on holder restart); the **daemon singleton identity** (user SID + config
   root + protocol-major); durable buffer reuse of 0011/0013; and
   how this supersedes/relocates 0004, #5/#17, #3. Record the deferred items (full
   non-binary status policy, fd-over-IPC, directory reads) explicitly so they are not
   silently dropped.
2. **`DESIGN.md` update** - the station model moves from "holder + waiter resident in
   the session" to "per-user daemon + one-shot client verbs"; keep the telex/station
   vocabulary coherent.
3. **`PRODUCT-THESIS.md` update** - "one small binary, no server" -> "one small
   binary + an auto-spawned local daemon"; frame the daemon as zero-config and
   implicit (like `rust-analyzer`/`gopls`).
4. **IPC / attendance protocol + authorization** - a **daemon-scoped** control
   endpoint (not address-keyed, since one daemon serves multiple stores); a versioned
   Layer-1 protocol (Register, Re-register, Deregister, DeregisterSession, Wait,
   Status; frames carry the lease epoch) where requests carry `store_key`/`address`/
   `session_id`; a **Hello/HelloAck version handshake** for old-daemon/new-client
   skew; an explicit **scoped-capability authorization** model (Register mints an
   unforgeable daemon-instance/epoch/session-bound capability; privileged RPCs - Wait,
   Deregister, DeregisterSession, Status detail, handoff/upgrade - present proof; one
   token in v1, scope/rotation fields reserved). The attendance record shape (address,
   opaque session id, watch-PIDs with role + start-time, occupant, backend, host,
   `attendance_last_confirmed_at`, lease epoch), and the auto-spawn + single-instance
   mechanism: the daemon singleton is keyed by **user SID + config root +
   protocol-major** and clients pass backend/store identity explicitly. telex core
   names nothing harness-specific; the Copilot mapping
   (`COPILOT_AGENT_SESSION_ID`/`COPILOT_LOADER_PID` -> `TELEX_SESSION_ID`/
   `--watch-pid`) lives in the `copilot-plugin` node, and the core `COPILOT_*` fallback
   is removed. The protocol is intended to be reusable by the #12 SDK client.
5. **Daemon lifecycle contract + Status surface** - a normative state machine: OS
   spawn-lock (thundering-herd auto-spawn), connect-or-spawn election, singleton
   stale-lock takeover, readiness ACK, `wait` short **reconnect-on-EOF** grace (a
   restart/handoff is not a turn failure), retry/backoff/crashloop behavior,
   daemon-down **exit codes**, and a bounded, actionable **Status** surface (epoch,
   instance, attendees, stale state, backoff, recent errors). Specify the four gating
   tests (concurrent first-use, crash-during-`wait`, competing daemons, handoff
   duplicates) as `daemon-core` acceptance.
6. **Daemon-native session ownership** - the daemon's **in-memory**
   `session_id -> addresses` map is the authority; clients re-register after a restart;
   expose idempotent `Register`/`Re-register` + `DeregisterSession(session_id, proof)`.
   This **reshapes** #23/#31: reuse the hook plumbing, drop the filesystem
   `session_registry` as authority; the Copilot hook is a thin mapper, and Copilot
   JSON parsing must not become a core protocol dependency.
7. **Verb + docs/SKILL cutover decision** - keep the verb names
   (`register`/`deregister`/`wait`; no rename/deprecation debt); require `SKILL.md` +
   plugin docs to update **with** `daemon-core` (never describe the dead holder/waiter
   model mid-workstream); hide the daemon entrypoint from normal user help; specify the
   single-source `SKILL.md` / plugin-skill mechanism (one source serves both the CLI
   command and the plugin skill).
8. **Minimal upgrade floor + cutover rule** - specify the minimal floor that lands in
   `daemon-core` (versioned shim + `daemon stop --drain` + next-call respawn) and the
   deterministic **legacy-holder / non-epoch-lease cutover rule** for the first
   daemon-aware rollout; the full upgrade platform (rollback/gc/UX) stays in
   `seamless-upgrade`.
9. **Resolutions for the open questions** (below).

## Open questions this node must resolve

1. **Epoch lifecycle (resolves the reclaim race + handoff window).** Both are
   resolved in approach by the lease-epoch fencing token. Specify the epoch
   lifecycle: when it increments, how a new daemon claims a higher epoch on
   respawn/handoff, how the loser self-demotes on a 0-row heartbeat, and how Postgres
   cross-machine reclaim is expressed in epochs (not timing). (SQLite-local is the
   simple case; `postgres-parity` must prove it under competing daemons.)
2. **Stale-attendance threshold + takeover.** Define how `attendance_last_confirmed_at`
   is updated (register/wait/hook), the `occupied_stale` threshold, and the operator
   takeover flow - without any idle-TTL teardown.
3. **Typed `--watch-pid` semantics.** Generalize the singular `--session-pid`
   (#5/#17) into typed predicates: **anchor** (alive if any sufficient pid survives)
   vs **required** (alive only if all necessary survive), plus a pid+start-time reuse
   guard. v1 floor = loader anchor + start-time; expose required/anchor flags only
   where a real consumer/test exists. Keep names harness-agnostic; loader-only is
   weak/hook-dependent liveness, not strong pid-backed attendance.
4. **Distinct per-session PID?** Determine whether Copilot exposes a per-session PID
   beyond `COPILOT_LOADER_PID`; if so the plugin can pass it as an additional
   `--watch-pid` for finer-grained release, otherwise the loader PID is the sole
   backstop anchor. (Owned jointly with `copilot-plugin`.)
5. **Cutover rule.** The deterministic rule handling legacy holders / non-epoch lease
   rows during the first daemon-aware rollout.
6. **DeregisterSession proof.** How the sessionEnd hook obtains/presents proof for
   `DeregisterSession` without reintroducing an external registry (instance admin
   capability, session capability in plugin env, or another user-private mechanism).
7. **Status freeze line.** How much diagnostic/Status surface must freeze in
   `design-foundation` vs `daemon-core` acceptance.
8. **Attendance durability across a daemon crash.** What attendance state is durable
   vs intentionally rebuilt by client re-register (behavior for a session that ends
   while the daemon is down).

## Boundaries

- **In scope:** design docs, decision record(s), protocol/attendance sketch, the
  open-question resolutions, and an explicit list of what the daemon relocates,
  supersedes, or defers across the related issues.
- **Out of scope:** production code, the daemon implementation, the plugin
  implementation, the upgrade implementation (those are later nodes). Do not
  restructure telex's design docs into `docs/design/` - keep the root-level layer.

## Expected output / definition of done

- `DECISIONS.md`, `DESIGN.md`, `PRODUCT-THESIS.md` updated and internally consistent.
- The IPC/attendance protocol and auto-spawn/single-instance mechanism are described
  precisely enough that `daemon-core` can implement against them without re-deciding
  architecture.
- Both open questions have a recorded resolution.
- Spar critique has been incorporated or explicitly dispositioned (arm's length: the
  builder confirms pivots).
- Ready for the **design-gate** (builder validation) before `daemon-core` starts.

## Validation expectations

Builder review at the design-gate. The design is accepted when the daemon
architecture, liveness model, protocol, durable-buffer reuse, and upgrade/handoff
design are explicit, consistent, and the builder agrees they unblock the idle-session
and stale-station problems that motivated #32.
