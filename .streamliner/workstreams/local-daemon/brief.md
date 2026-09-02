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
**design-foundation** (a research node, written and spar-pressure-tested) locked the
hard contracts up front - the daemon-scoped capability/version-handshake IPC, the
**server-side lease-epoch fence**, the **seen-dedup redesign**, explicit membership
+ non-destructive liveness, negative-only watched-process evidence, the daemon
**singleton identity** + **lifecycle contract**, and daemon-native session RPCs -
behind a builder **design-gate**. Then the **daemon core**
(the centerpiece: daemon process, durable buffer, one-shot verbs, server-side
epoch-fenced delivery, the lifecycle contract, and a minimal upgrade floor) on
SQLite, which with the **Copilot plugin** is the first slice that can unblock the
operator (reached when the plugin lands on SQLite). A distinct **fencing-proof** gate
(epoch-guarded emission + ordered handoff, proven on SQLite) then blocks downstream
work. **Postgres parity** extends the core under that proof and adds the cross-machine
reclaim (competing daemons); **seamless upgrade** (#6) lands
**last**, after Postgres and the plugin, so the full upgrade platform never blocks
the unblock. The original large validation-harness and AKS-scale shape was later
replaced by a practical **release-confidence-validation** node, which is complete.
Issue #106 / PR #138 is the active hardening repair discovered after that
validation. The operator accepted persistent OS-lock containment and a truthful
degraded-enumeration contract for that PR. The mandatory downstream
**station-intent-transactional-authority** node closes the accepted gap before
the final **closure gate**, without blocking PR #138 or the builder
**hardening gate**. Nodes are coarse and PAW-sized; the completeness split is
justified by a transactional migration boundary and an independently useful,
safe PR #138 outcome.

The richer design rationale and the full decision ledger that led here live in
[`docs/initial-shaping.md`](docs/initial-shaping.md). The brief stays current and
distilled.

## Design References

The authoritative design layer (merged from `design-foundation`) lives under
`docs/design/`:

- `telex:docs/design/daemon.md` - the **normative daemon contract** the implementation
  nodes build against (17 sections + the sec.17 gating tests).
- [`design/current-design.md`](design/current-design.md) - the canonical integrated
  workstream design. It summarizes merged authority and keeps accepted issue #106 /
  PR #138 direction explicitly behind a product-promotion boundary.
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
  capability + version-handshake IPC**; the ADR 0023 liveness model
  (authoritative, non-destructive `sessionEnd`, negative-only watched-process
  evidence with start-time, and a non-destructive idle backstop); explicit-only
  membership removed only by explicit **`Detach`**; the Copilot CLI plugin as the
  harness adapter and one shared source for `telex skill`; the **minimal upgrade
  floor** (versioned shim + `daemon stop
  --drain` + next-call respawn + legacy/non-epoch cutover rule) in `daemon-core` with
  full seamless upgrade (#6) last; retiring superseded mechanisms (#3 relay, pid-watch
  as a per-session holder, the re-arm dance) and updating the docs **with**
  `daemon-core`, not at closure; desired station-intent recovery with bounded
  OS-lock safety and a degraded partial-scan contract; and the downstream
  transactional-authority closure.
- **Out of scope:** the embeddable SDK client (#12) - it shares the
  collapse-into-one-process theme and should reuse the stabilized Layer-1 IPC, but
  is a separate solve; response windows / TTL deadlines (#2); the `store_key` helper
  (#25).
- **Deferred:** a richer non-binary occupant status policy beyond the accepted
  non-destructive liveness states; the pid-reuse-immune fd-over-IPC backstop
  (#28-flavored), awkward with a singleton daemon (the accepted process evidence
  uses PID + start-time); and the daemon subsuming directory/occupancy reads
  (`address list`).

## Current State

The design foundation, daemon core, fencing proof, Postgres parity, Copilot plugin and
push bridge, lifecycle hardening, versioned/release upgrade paths, public release, and
release-confidence validation are merged and recorded complete. The normative design
remains `docs/design/daemon.md`, with ADR 0023 governing explicit-only membership and
non-destructive liveness; merged PR #139 additionally defines the Copilot App
turn-idle and bridge-host lifecycle behavior.

Dogfooding then exposed issue #106: daemon replacement can preserve durable messages
while losing a still-live bridge's desired push registration. Existing PR #138 is the
adopted `station-intent-reconciliation` repair. The operator selected persistent
owner-private OS advisory locking to prevent stale pathname mutation and accepted a
degraded contract for bounded partial directory scans. PR #138 may resume after this
Tier B authority lands, but it remains in progress and pending design promotion,
review repair, exact-head CI, and both-backend proof. The **hardening gate is not
ready** until that narrowed repair is merged and presented with isolated
restart/drain/upgrade and push-recovery evidence.

Unconditional transactional generation authority, seekable fair discovery and
garbage collection, exact counts, and exact over-cap recovery belong to the planned
XL `station-intent-transactional-authority` node. That node follows PR #138 and
blocks the final **closure gate**, not PR #138 or the hardening gate.

Workstream and design-steward branches are proposal/integration workspaces, not
silent authority. Streamliner artifact changes become durable only through the
campaign's sole artifact reconciler applying the reviewed, operator-authorized
packet directly to `main`.

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
- **Project design authority lives under `docs/design/`:** `daemon.md` is normative,
  `DESIGN.md` supplies architecture, and `DECISIONS.md` is the append-only ADR log.
  The workstream's canonical integrated summary lives at
  `design/current-design.md`; pending proposals are not project authority.
- **Spar at arm's length:** critique informs the design but pivots are surfaced for
  builder confirmation, not auto-applied.
- **Lease-epoch fencing is the spine (from spar):** daemon-down recovery, upgrade
  handoff, and Postgres reclaim are all made safe by one monotonic
  `lease_epoch`/`owner_instance_id` rather than by timing. `design-foundation` owns
  the epoch lifecycle.
- **Fencing-first sequencing (from spar):** lock the hard contracts (fencing,
  explicit membership, non-destructive liveness, watched-process identity) in
  `design-foundation`; gate Postgres on fencing proven under competing daemons;
  land seamless-upgrade last. Keeps both backends + #6 in the deliverable while
  limiting blast radius.
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
- **Daemon-native session ownership (revised by ADR 0023):** the daemon's
  in-memory `session->addresses` map is the authority. Reuse the hook plumbing as
  a non-destructive liveness input; explicit detach, not sessionEnd, owns
  membership removal.
- **Station-intent safety now, transactional convergence downstream:** PR #138
  replaces age-stealable intent locks with persistent owner-private OS advisory
  locks and exposes partial-scan degradation under the four-second response
  contract. A connected XL node restores unconditional transactional generation
  authority and fair maintenance before workstream closure. This preserves one
  complete, useful PR #138 outcome while keeping the accepted gap durable.
- **Docs/SKILL cutover with `daemon-core` (council):** keep the verb names; update
  `SKILL.md` + plugin docs when behavior changes, not at closure, so instructions
  never describe a dead holder/waiter model mid-workstream.

## Resolved questions and superseded validation shape

All eight design-foundation questions are **resolved** as ADRs 0014-0024 (see
`docs/design/DECISIONS.md` and `daemon.md`): epoch lifecycle (0015), session
presence/reaping + crash durability (0017/0023), watched-process evidence (0017),
legacy cutover (0020/0024), explicit membership and agent acknowledgement
(0019/0023), and the Status freeze line (0018).

The earlier `validation-harness`, Entra multi-host campaign, and AKS scale-rig
concepts were superseded by the completed `release-confidence-validation` node
(issue #78). The operator resolved the PR #138 M3/M5 fork on 2026-09-02:
PR #138 owns safe desired push restoration with persistent OS locking and an
explicit degraded-enumeration contract; the downstream transactional node owns
unconditional generation, discovery, GC, and over-cap authority.

## Imports and Exports

### Imports

- **PR #31 / issue #23 (sessionEnd hook plumbing):** the plugin reuses the hook
  wiring, but not its filesystem `session_registry` as attendance authority. The
  daemon owns `session_id->addresses` in memory; under ADR 0023 the hook supplies
  non-destructive liveness input while explicit detach owns membership removal.
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
