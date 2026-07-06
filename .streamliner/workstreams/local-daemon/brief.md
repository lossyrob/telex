# Local presence/transport daemon (eliminate the per-session holder)

## Purpose

Most of telex's recurring staleness (orphaned holders, zombie `occupied` leases,
holder/waiter startup races, dismiss leaving a holder attached, a forever-listener
starving an orchestrator's turn loop) traces to one structural choice: a
**per-session resident holder** whose lifetime must track a fuzzy agent session.
This workstream eliminates that holder by introducing an **auto-spawned per-user
local daemon** that owns presence and delivery for all locally-attended addresses,
and ships the surrounding pieces (Copilot plugin, seamless upgrade) needed for a
real, end-to-end unblock so idle long-lived sessions stay wakeable and stations
stop going stale. It resolves [issue #32](https://github.com/lossyrob/telex/issues/32).

## Approach

The work is a single complete deliverable across **both SQLite and Postgres** (the
operator runs both and has stations idle waiting on this), not a thin V1 slice. A
SQLite-only spike is an internal step inside the daemon-core node, not the shippable
boundary.

Formation orders the work as confidence transitions expressed through the node DAG.
**design-foundation** (a research node, written and spar-pressure-tested) locks the
hard contracts up front - the daemon-scoped capability/version-handshake IPC, the
**server-side lease-epoch fence**, the **seen-dedup redesign**, minimal
**stale-attendance**/takeover, **typed watch-pid**, the daemon **singleton identity**
+ **lifecycle contract**, and daemon-native session RPCs - behind a builder
**design-gate**. Then the **daemon core**
(the centerpiece: daemon process, durable buffer, one-shot verbs, server-side
epoch-fenced delivery, the lifecycle contract, and a minimal upgrade floor) on
SQLite, which with the **Copilot plugin** is the first slice that can unblock the
operator (reached when the plugin lands on SQLite). A distinct **fencing-proof** gate
(epoch-guarded emission + ordered handoff, proven on SQLite) then blocks downstream
work. **Postgres parity** extends the core under that proof and adds the cross-machine
reclaim (competing daemons); **seamless upgrade** (#6) lands
**last**, after Postgres and the plugin, so the full upgrade platform never blocks
the unblock. A **validation-loop hardening wave** then exercises the complete system -
Tier 1/2 chaos + Entra-PG multi-host (parallel with an **AKS large-network spike**),
then the **AKS scale rig + stress loop** last - and a **hardening gate** must pass
before the final **closure gate** validates the real-world unblock and retires
superseded mechanisms. Nodes are coarse and PAW-sized; the completeness split is
justified by distinct expertise, independent validation, and parallelism.

The richer design rationale and the full decision ledger that led here live in
[`docs/initial-shaping.md`](docs/initial-shaping.md). The brief stays current and
distilled.

## Design References

The authoritative design layer (merged from `design-foundation`) lives under
`docs/design/`:

- `telex:docs/design/daemon.md` - the **normative daemon contract** the implementation
  nodes build against (17 sections + the sec.17 gating tests).
- `telex:docs/design/DESIGN.md` - the local-exchange architecture.
- `telex:docs/design/DECISIONS.md` - the ADR log; **0014-0024** are this workstream's
  decisions (0023 = the minimal session/presence/delivery model; 0021 = the
  `docs/design/` relocation).
- `telex:docs/design/index.md` / `docs/design/ARCHITECTURE.md` - the entry point and the
  5-diagram visual on-ramp.
- `telex:PRODUCT-THESIS.md` (root) - the "no server" -> "auto-spawned local exchange"
  framing.

## Boundaries

- **In scope:** the per-user daemon (presence + transport) for SQLite **and**
  Postgres; one-shot `attach`/`detach`/`wait` against the daemon; durable buffer
  (reuse 0011/0013) with the **seen-dedup redesign** for a long-lived daemon; the
  **lease-epoch fencing token** with a **server-side fence on delivery emission +
  ordered handoff** (`mark_delivered_if_current_owner`) proven by a distinct
  **fencing-proof** gate; the daemon **singleton identity** (user SID + config root +
  protocol-major) and **lifecycle contract** (spawn-lock, connect-or-spawn, readiness
  ACK, `wait` reconnect-on-EOF grace, exit codes, Status surface); the **daemon-scoped
  capability + version-handshake IPC**; the liveness model (sessionEnd hook
  healthy-path + a **typed** `--watch-pid` backstop, v1 floor loader anchor +
  start-time; no idle-TTL teardown, but **stale-attendance/takeover** as a
  load-bearing recovery path - last-confirmed + `occupied_stale` + takeover); the
  Copilot CLI plugin (sessionEnd hook -> explicit **`Detach`** per daemon.md sec.14.2, not
  PR #31's filesystem registry) and moving `telex skill` into a real plugin skill with
  one shared source; the **minimal upgrade floor** (versioned shim + `daemon stop
  --drain` + next-call respawn + legacy/non-epoch cutover rule) in `daemon-core` with
  full seamless upgrade (#6) last; retiring superseded mechanisms (#3 relay, pid-watch
  as a per-session holder, the re-arm dance) and updating the docs **with**
  `daemon-core`, not at closure.
- **Out of scope:** the embeddable SDK client (#12) - it shares the
  collapse-into-one-process theme and should reuse the stabilized Layer-1 IPC, but
  is a separate solve; response windows / TTL deadlines (#2); the `store_key` helper
  (#25).
- **Deferred:** the **full** non-binary occupant status policy (attended/idle/free) -
  the **minimal** stale-attendance signal (last-confirmed + `occupied_stale` +
  takeover) is now in scope, but the full state machine and any idle policy stay
  deferred and never drive teardown; the pid-reuse-immune fd-over-IPC backstop
  (#28-flavored), awkward with a singleton daemon (a lighter pid+start-time guard IS
  in scope); the daemon subsuming directory/occupancy reads (`address list`).

## Current State

**design-foundation is merged** (issue #34; PRs #35 + #37) after a 10-round
`design-gate` review, so the **design-gate has passed**. The authoritative design now
lives under `docs/design/` (`daemon.md` is the normative contract), relocated there by
ADR 0021; the eight open questions are resolved as **ADRs 0014-0024**. The
session/presence/delivery model was revised to a **minimal form** by **ADR 0023**
(unique `session_id` + explicit-only membership via `Detach` + non-destructive presence
+ agent-acked delivery), superseding the earlier "incarnation" machinery - so some
council/spar specifics (e.g. a `DeregisterSession` RPC, `attendance_last_confirmed_at`)
are realized differently; **`daemon.md` governs** where the shaping differs.

**`daemon-core` is the next ready node** - implement `docs/design/daemon.md` on SQLite
(acceptance = its sec.17 gating tests). The graph also adds a **validation-loop
hardening wave** (harness + Entra-PG multi-host + AKS scale) before closure. Workstream
artifacts are edited in the dedicated `telex-streamliner` worktree (branch
`streamliner`) that pushes to `main`, keeping the primary checkout clear.

## Decisions

- **One complete deliverable, both backends:** SQLite and Postgres ship together;
  the SQLite spike is an internal step, not the boundary. Rationale: the operator
  runs both with stations idle waiting; a partial cutover does not unblock them.
- **Coarse, PAW-sized nodes (~one per confidence transition):** bias to fewer,
  heavier nodes; the three completeness tracks are the one deliberate split for
  parallelism + distinct expertise.
- **Local-spec-first tracking:** node specs live under `tasks/`; promote to GitHub
  issues at wave promotion. The umbrella issue #32 is the workstream's parent
  tracker.
- **Design layer stays at the telex repo root** (`DESIGN.md`, `DECISIONS.md`,
  `PRODUCT-THESIS.md`) rather than being restructured into `docs/design/`; ADRs
  extend the existing numbered `DECISIONS.md` series.
- **Spar at arm's length:** critique informs the design but pivots are surfaced for
  builder confirmation, not auto-applied.
- **Lease-epoch fencing is the spine (from spar):** daemon-down recovery, upgrade
  handoff, and Postgres reclaim are all made safe by one monotonic
  `lease_epoch`/`owner_instance_id` rather than by timing. `design-foundation` owns
  the epoch lifecycle.
- **Fencing-first sequencing (from spar):** lock the hard contracts (fencing,
  stale-attendance, typed watch-pid, identity) in `design-foundation`; gate Postgres
  on fencing proven under competing daemons; land seamless-upgrade last. Keeps both
  backends + #6 in the deliverable while limiting blast radius.
- **Server-side epoch fence + a distinct `fencing-proof` gate (council):** lease-row
  fencing alone is insufficient - delivery emission is fenced server-side
  (`mark_delivered_if_current_owner`; no frame unless the daemon owns the epoch) and
  handoff is ordered; a distinct executable `fencing-proof` gate blocks
  Postgres/plugin/upgrade until proven. Verified: the holder emits the frame *before*
  `mark_delivered` commits, and per-process `seen` resets across a handoff.
- **Minimal upgrade floor early (council):** a versioned shim + `daemon stop --drain`
  + next-call respawn + a legacy-holder/non-epoch-lease cutover rule land in
  `daemon-core` (the first daemon-aware install hits the Windows binary-lock); full
  rollback/gc/UX stays last.
- **Daemon-native session ownership (council):** the hook calls a daemon-native
  `DeregisterSession`; the daemon's in-memory `session->addresses` map is the
  authority, reshaping #23/#31 (reuse the hook plumbing, drop the filesystem
  registry).
- **Docs/SKILL cutover with `daemon-core` (council):** keep the verb names; update
  `SKILL.md` + plugin docs when behavior changes, not at closure, so instructions
  never describe a dead holder/waiter model mid-workstream.

## Open Questions

All eight design-foundation open questions are **resolved** as ADRs 0014-0024 (see
`docs/design/DECISIONS.md` and `daemon.md`): epoch lifecycle (0015), session
presence/reaping + crash durability (0017/0023), typed watch-pid + per-session PID
(0017), legacy cutover (0020/0024), `Detach`/`Ack` removal proof (0019/0023), and the
Status freeze line (0018). Remaining open items are deliberately deferred to execution:

- The validation-loop **invariant suite + observability hooks** are derived during
  `validation-harness` against the implemented reality, not pre-specified.
- The **AKS large-network approach** (orchestration, cost, oracle pipeline) is proven
  in `aks-scale-spike`.

## Imports and Exports

### Imports

- **PR #31 / issue #23 (sessionEnd hook plumbing):** the hook wiring the plugin
  reuses. Its filesystem `session_registry` is **not** the attendance authority
  (council G) - the daemon owns `session_id->addresses` in memory and the hook calls
  explicit `Detach` (ADR 0019/0023, daemon.md sec.14.2); the hook is a thin mapper. Provider: branch
  `feature/copilot-session-end-plugin`. Available now.
- **Decisions 0011/0013 durable delivery (`deliveries` table, `fetch_undelivered`):**
  reused as the daemon's durable buffer. Available in `main`.
- **Harness env contract (consumed only by the plugin layer):**
  `COPILOT_AGENT_SESSION_ID` and `COPILOT_LOADER_PID`, verified present and reliable
  (explicit env vars, not ppid-walk). telex core stays harness-agnostic - it takes an
  opaque `$TELEX_SESSION_ID` and one or more generic `--watch-pid`s; the Copilot
  plugin maps these env vars onto them.

### Exports

- **Stabilized Layer-1 IPC/attendance protocol:** the daemon's documented control
  protocol, intended for reuse by the embeddable SDK client (#12).
- **Seamless-upgrade install layout + launcher shim:** the versioned-install
  mechanism (#6), reusable for any future telex distribution.

### External Dependencies

- None outside telex itself. Building/installing from source on Windows is locked by
  running `telex` processes during the binary swap - the very pain #6 fixes - so
  validating `seamless-upgrade` requires care during dogfooding.

## Closeout Observations

(parking lot - populated during execution)
