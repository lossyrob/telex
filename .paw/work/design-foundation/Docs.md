# Local-daemon Design Foundation

## Overview

This work establishes the **design layer** for the `local-daemon` workstream: the
architecture, normative contracts, and decision record for replacing telex's
per-session resident **holder** with an auto-spawned per-user **local exchange** (a
daemon) that owns presence and delivery for all locally-attended addresses. It is the
output of the `design-foundation` research/design node and serves as the **design-gate**
the rest of the workstream (`daemon-core`, `fencing-proof`, `postgres-parity`,
`copilot-plugin`, `seamless-upgrade`) implements against.

It is **design/writing only** — no production code. The problem it solves is structural:
binding presence + delivery transport to an ephemeral per-session process is the root
cause of telex's recurring staleness (orphaned holders, zombie `occupied` leases,
holder/waiter startup races, dismiss leaving a holder attached, a forever-listener
starving a session's turn loop). The design relocates the irreducible presence concern to
one supervised per-user daemon and deletes the per-session holder.

## Architecture and Design

### High-Level Architecture

The design introduces a per-user **local exchange** (the historical telex switching
center, mapped onto a daemon) sitting between the one-shot CLI verbs and the backend
driver:

```
CLI (one-shot: attach / wait / detach / send / ...)
  -> Telex core library (semantic model)
    -> Local exchange (per-user daemon)   = presence + transport, single lease writer
       owns attendance, durable buffer, lease heartbeat + epoch, IPC, pid-watch
      -> Backend driver -> SQLite / Postgres
```

The exchange is a singleton keyed by `(user SID, config root, protocol-major)`, serves
multiple stores, auto-spawns on first use, and is implicit/zero-config (like
`rust-analyzer`/`gopls`). Sessions run **one-shot** verbs against it; there is no resident
per-session process. The full normative contract is `docs/design/daemon.md`.

### Design Decisions

The load-bearing decisions are recorded as ADRs 0014–0021 in `docs/design/DECISIONS.md`:

- **0014** — Per-user local exchange; zero persistent session processes (supersedes 0004;
  recasts the "station" of 0009 as a registration in the exchange).
- **0015** — Server-side **lease-epoch fence**: a monotonic `lease_epoch` +
  `owner_instance_id`, epoch-guarded rowcount-returning heartbeat/release, self-demote on
  0-row, and a **typed `mark_delivered_if_current_owner` delivery fence** that closes the
  verified pre-commit double-delivery hazard (the holder ships the message frame before
  `mark_delivered` commits).
- **0016** — `seen`-dedup redesign for a long-lived daemon (durable `deliveries` as
  cross-epoch authority + a bounded, epoch-keyed in-memory fast-path).
- **0017** — Liveness: sessionEnd hook + typed `--watch-pid`; minimal
  stale-attendance/takeover; **no idle-TTL teardown** but immediate teardown on positive
  death evidence (a four-case dismissal-path matrix). Grounded in an empirical finding
  (OQ4): Copilot CLI exposes no reliable per-session PID beyond the loader.
- **0018** — Daemon singleton identity + lifecycle contract (spawn-lock, connect-or-spawn,
  readiness ACK, `wait` reconnect-on-EOF grace, exit codes) + a frozen Status surface.
- **0019** — Daemon-scoped capability/version-handshake IPC (instance-`admin_cap` auth in
  v1) + daemon-native session ownership (`Register`/`Re-register`/`DeregisterSession`, the
  `from`-default rule, suspect/verified/lapsed crash recovery).
- **0020** — Minimal upgrade floor + a **two-phase** legacy/non-epoch-lease cutover rule.
- **0021** — Verb + docs/SKILL cutover (keep verbs; hide the daemon entrypoint;
  single-source SKILL) + the relocation of the design layer to `docs/design/`.

Two builder-directed framing decisions: the **"local exchange"** telex metaphor, and a
**full rewrite** of `DESIGN.md` to the daemon end-state (acceptable because the repo is
private and the workstream ships before it opens).

### Integration Points

- **`daemon-core`** implements the `daemon.md` contracts (IPC protocol, epoch fence,
  lifecycle, session ownership, minimal upgrade floor) and must pass the **five gating
  tests** specified there.
- **`fencing-proof`** must prove epoch-guarded delivery emission + ordered handoff
  (including the ownership-loss-around-delivery scenario) before Postgres/plugin/upgrade
  rely on it.
- **`copilot-plugin`** maps `COPILOT_AGENT_SESSION_ID -> TELEX_SESSION_ID` and
  `COPILOT_LOADER_PID -> --watch-pid`, and calls the daemon-native `DeregisterSession`
  (reshaping PR #31's filesystem session registry out as the authority).
- **Durable buffer** reuses decisions 0011/0013 (`deliveries` table) unchanged.

## User Guide

### Prerequisites

The "users" of this layer are the builder (validating it at the design-gate) and
downstream node-worker sessions (implementing against it). To engage with it you only
need to read the `docs/design/` layer; no build or runtime is involved.

### Basic Usage

1. Start at `docs/design/index.md` — the design-layer entry point.
2. Read `docs/design/DESIGN.md` for the architecture and the local-exchange framing.
3. Read `docs/design/daemon.md` for the precise contracts (this is what `daemon-core`
   implements against). Its "Open-question resolutions" table maps each of the eight
   open questions to its specification.
4. Read `docs/design/DECISIONS.md` ADRs 0014–0021 for the decisions and their
   supersessions.

### Advanced Usage

Reviewing as the design-gate: validate that the daemon architecture, liveness model,
protocol, durable-buffer reuse, and upgrade/handoff design are explicit, consistent, and
unblock the idle-session/stale-station problems motivating issue #32. Each open-question
resolution and each gating test is written to be implementable without re-deciding
architecture.

## API Reference

### Key Components (the normative contracts in `docs/design/daemon.md`)

- **IPC protocol** — daemon-scoped endpoint; Hello/HelloAck version handshake; request/
  response frames; per-request `store_key`/`address`/`session_id`.
- **Authorization** — instance `admin_cap` (user-private secret file) for privileged RPCs;
  `scope`/`rotation`/`per_session_cap` reserved.
- **Lease-epoch fence** — `lease_epoch` + `owner_instance_id`; CAS claim; epoch-guarded
  heartbeat/release; `mark_delivered_if_current_owner(address, owner_instance_id,
  lease_epoch, message_id) -> {Delivered | NotOwner | AlreadyDelivered}` with the
  non-`NotOwner`-before-frame ordering invariant; ordered handoff; epoch-based Postgres
  reclaim.
- **Attendance record** — address, session_id, occupant, owner_instance_id, lease_epoch,
  typed watch-pids, last_confirmed, state (suspect/verified/lapsed), derived
  occupied_stale.
- **Session ownership** — in-memory `session_id -> addresses` authority;
  `Register`/`Re-register`/`DeregisterSession`; `wait` auto-Re-register.
- **Lifecycle + Status** — the state machine, exit codes, and the frozen Status field set.

### Configuration Options

Frozen at this layer: the Status field set and meaning; the gating-test observable
assertions. Not frozen (owned by `daemon-core`): Status rendering/format, backoff/
crashloop thresholds, `stale_after` default, and the Re-register address-merge policy
(default union).

## Testing

### How to Test

This layer is validated by **builder review at the design-gate** (the PR to `main`). The
acceptance bar: the daemon architecture, liveness model, protocol, durable-buffer reuse,
and upgrade/handoff design are explicit, internally consistent, and agreed to unblock the
idle-session and stale-station problems. The design also specifies **five executable
gating tests** as `daemon-core` acceptance (not run here): concurrent first-use;
crash-during-`wait`; competing daemons; handoff duplicates + ownership-loss-around-
delivery; intra-daemon takeover local-eviction.

### Edge Cases

The design explicitly addresses: unhooked dismiss (loader survives) -> `occupied_stale` +
operator takeover; daemon crash mid-`wait` -> reconnect-on-EOF + suspect/verified/lapsed
recovery; competing cross-machine daemons -> epoch-based reclaim; graceful handoff and
ownership-loss-around-delivery -> server-side fence; legacy holders / non-epoch lease rows
-> two-phase drain-then-claim cutover; session ends while the daemon is down -> TTL
backstop + higher-epoch fence (no permanent zombie).

## Limitations and Future Work

- **Design only.** No code ships in this node; `daemon-core` implements the contracts.
- **Deferred (explicit in `daemon.md`):** the full non-binary occupant-status policy;
  the fd-over-IPC pid-reuse-immune backstop; daemon-owned directory/occupancy reads;
  per-session capability tiers (fields reserved); the full seamless-upgrade platform
  (only the minimal floor is in `daemon-core`).
- **Deferred doc cutover:** `README.md` and `SKILL.md` intentionally still describe the
  shipped holder/waiter model; their narrative cutover lands **with** `daemon-core` (so
  the docs never describe a dead model mid-workstream).
- **Reopen conditions (carried for the orchestrator):** if the legacy-cutover drain needs
  a new IPC verb (not an address-keyed probe + bounded stale-wait); if a Copilot plugin
  API to pre-populate the sessionEnd hook env appears (would make per-session caps the v1
  path); if `wait` Re-register is blocked by IPC transport masking socket-EOF; if the
  single-source SKILL mechanism hits a harness constraint.
- **Process deviation flagged:** relocating the design layer to `docs/design/` deviates
  from issue #34's "keep the design layer at the repo root" — builder-directed during
  shaping; the workstream brief/issue text update is an orchestrator action.
