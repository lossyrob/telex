# Initial shaping: local presence/transport daemon

> Support document. Captures the design conversation and the full decision ledger
> that produced this workstream's `brief.md` and `graph.json`. Background and
> rationale, not authoritative current state. If anything here changes intended
> project design, promote it into the telex design layer (`DESIGN.md` /
> `DECISIONS.md` / `PRODUCT-THESIS.md`).

> **Reconciliation (2026-06-24): superseded by the merged design.** `design-foundation`
> is merged (issue #34; PRs #35 + #37). The authoritative design now lives under
> **`docs/design/`** (`daemon.md` is the normative contract), relocated there by **ADR
> 0021**; the eight open questions are resolved as **ADRs 0014-0024**. The
> session/presence/delivery model was revised to a **minimal form** by **ADR 0023**
> (unique `session_id` + explicit-only membership via `Detach` + non-destructive
> presence + agent-acked delivery), superseding the "incarnation" machinery. This
> document is retained as **pre-implementation shaping history**; where its specifics
> differ from the ADRs (e.g. a `DeregisterSession` RPC, `attendance_last_confirmed_at`,
> the root-level design layout), **the ADRs in `docs/design/` win.**

## Origin

No prior Streamliner shaping candidate existed - this is telex's first workstream.
The shaping was a live design conversation that surveyed [issue #32](https://github.com/lossyrob/telex/issues/32)
and the surrounding open issues/PRs to fix the proper scope, then bottomed out the
strategic decisions before formation.

## Root cause being fixed

The holder (`telex attach`) conflates two unrelated jobs:

1. **Presence / liveness** - "address A is occupied and alive" (lease + TTL
   heartbeat). Intrinsically coupled to whether a live agent attends A.
2. **Delivery transport** - poll/LISTEN-NOTIFY, buffer, push one message to a waiter
   over local IPC. **Not** session-coupled at all.

Binding both to an ephemeral per-session background process is the source of the
fragility (orphaned holders, zombie `occupied` leases, holder/waiter races, dismiss
leaving a holder attached, and - field-reported by a Streamliner orchestrator via
the dbagent telex agent - a forever-listener task starving the session's turn loop so
scheduled prompts queue behind it). The fix is to relocate the irreducible
presence part to one supervised per-user daemon and delete the per-session holder.

## What the daemon does and does NOT fix

- **Does not fix (irreducible):** "is address A actually attended by a live agent?"
  Something must still answer this. The daemon relocates it to one place.
- **Does fix:** eliminates the per-session holder (whole class of bugs gone, not
  hardened); one supervised process instead of N ad-hoc tasks; single writer of
  liveness; cleaner crash signal; consolidated backend work; frees the session's
  turn loop (no forever task).

## Attendance model - the key correction

Issue #32's draft attendance model listed a "live local connection / socket held by a
per-session supervisor" as the strongest signal. That is a **holder-shaped
per-session process** - it contradicts the eliminate-the-holder thesis, and it does
not even catch dismiss (no process death). It was dropped from the CLI model. The
held socket is only genuinely free for the SDK case (#12), where the host is already
a long-lived process; reserve it there, not in the CLI.

**Final CLI liveness model:**

- **Healthy disconnect = `sessionEnd` hook.** Quit and dismiss both fire
  `session.ended`; the plugin hook tells the daemon to deregister that session's
  addresses, keyed by the opaque session id (`COPILOT_AGENT_SESSION_ID` for Copilot).
  Normal path. Builds on PR #31's
  `session_registry`.
- **Ungraceful backstop = daemon pid-watch of one or more generic watch PIDs.**
  telex core takes a repeatable `--watch-pid` and releases when the set is all dead;
  it names nothing harness-specific. The Copilot plugin supplies `COPILOT_LOADER_PID`
  (catching terminal-close + app-crash; the whole tree including the loader dies).
  Reuses `session_watch::process_alive`.
- **No idle-TTL teardown.** A days-idle-but-alive session stays occupied and
  instantly wakeable - the operator's explicit requirement. Minimal **stale-attendance**
  (last-confirmed + `occupied_stale` + operator takeover) is tracked without teardown
  (spar R1; see below).
- **Lease TTL survives only as the daemon-down backstop** (daemon dies -> its leases
  lapse after the window; auto-respawn re-claims from the durable registry).
- **Residual edge:** dismiss where the hook did not fire (plugin absent/failed) ->
  stale `occupied` until quit/resume; messages still queue durably and deliver on
  resume. Minimal stale-attendance flags it as `occupied_stale` and allows operator
  takeover so it is not a permanent zombie (spar R1).

### Harness-agnostic watch PIDs (portability) + why sourcing is reliable

telex core must outlive Copilot CLI, so it names nothing harness-specific: the
one-shot register takes an opaque session id (`$TELEX_SESSION_ID`) and one or more
generic `--watch-pid`s, and the daemon releases the address when the watched set is
all dead. The **Copilot plugin/skill is the only layer that knows the Copilot env
contract** and maps it onto those generic arguments: `COPILOT_AGENT_SESSION_ID ->
TELEX_SESSION_ID` (hook deregister) and `COPILOT_LOADER_PID -> --watch-pid` (plus any
distinct per-session PID Copilot exposes). A future harness supplies its own ids/PIDs
the same way.

This is reliable because Copilot sets those env vars explicitly for every spawned
command (verified in-session; ancestry observed: shell <- copilot.exe child <-
copilot.exe loader = `COPILOT_LOADER_PID`) - **no fragile ppid-walking** (what #5/#17
worried about). Caveat: `COPILOT_LOADER_PID` is app/root-level, so a single session's
child killed while the loader survives (no hook) is not caught - it degrades to the
benign unhooked-dismiss residual.

## Decision ledger

Status legend: ratified = builder-confirmed; proposed = strongly recommended, to be
confirmed at the design gate; open = an implementation-design detail owned by the
`design-foundation` node; deferred = intentionally out of this deliverable's policy.

### Topology

- **(proposed) Per-user daemon.** One auto-spawned, single-instance, supervised
  per-user daemon owns backend connection(s), poll/LISTEN-NOTIFY, durable buffer, IPC
  endpoints, the attendance registry, the lease heartbeat (single writer), and
  pid-watch. Per-user, not per-workspace.
- **(proposed) Zero persistent session processes.** `attach` = one-shot
  register+exit; `wait` = one-shot per-turn block+exit; `detach` = one-shot
  deregister+exit. Cleanest realization of eliminate-the-holder; replaces the held
  socket.

### Liveness

- **(ratified) Hook is the healthy disconnect path** (quit + dismiss).
- **(ratified) pid-watch is the ungraceful backstop only** (crash/kill/
  terminal-close; redundantly quit).
- **(ratified) Harness-agnostic watch PIDs:** telex core takes a generic repeatable
  `--watch-pid` + opaque `$TELEX_SESSION_ID`; the Copilot plugin maps
  `COPILOT_LOADER_PID` / `COPILOT_AGENT_SESSION_ID` onto them (no harness-specific
  names in core; explicit env vars, not ppid-walk).
- **(proposed) No idle-TTL teardown**; idle-but-alive stays wakeable.
- **(proposed) Lease TTL written only by the daemon**; daemon-down backstop.
- **(ratified) Dismiss handled by the hook**; residual unhooked-dismiss edge accepted.
- **(ratified) Non-binary status deferred-with-schema-room**; never drives teardown;
  needs a duration heuristic (between-turns also has no waiter).
- **(ratified) pid-reuse hardening deferred** (fd-over-IPC is awkward with a
  singleton daemon).

### Lifecycle

- **(proposed) Auto-spawn on first use**; single-instance via lockfile/named-pipe
  bind; same binary (`telex daemon` subcommand).
- **(proposed) Daemon-down recovery:** on respawn, re-read the durable registry,
  re-validate each session pid, resume heartbeat + re-claim live leases, drop dead.
- **(open) Postgres respawn reclaim race:** best-effort re-claim; graceful "held
  elsewhere". SQLite-local is a non-issue.
- **(open) Daemon handoff window:** new daemon re-claims before/overlapping old
  release so no TTL gap lapses a lease mid-handoff.
- **(ratified) #6 seamless upgrade is in this workstream.** The daemon model shrinks
  lock-on-upgrade to one process; handoff reuses respawn-recovery.

### Protocol

- **(proposed) Extend `ipc.rs`** into a versioned Layer-1 control protocol (Register,
  Deregister, Wait, Status); keep it stable so the #12 SDK can reuse it.
- **(deferred) Daemon subsuming `address list`/occupancy reads**; V1 reads the
  backend lease table.

### Phasing / scope

- **(ratified) One complete deliverable, both SQLite and Postgres.** SQLite spike is
  an internal step, not the boundary.
- **(ratified) Hard cutover fine for existing sessions**; the real migration concern
  is future upgrades (#6), folded in.
- **(ratified) Plugin scope:** sessionEnd hook (healthy deregister) + `telex skill`
  as a real plugin skill, one source serving both command and plugin skill.
- **(ratified) Product framing:** no-server -> auto-spawned local daemon; update
  `PRODUCT-THESIS.md`.

## Issue scope map

| Issue / PR | Disposition |
|---|---|
| #32 | This workstream's umbrella. |
| #23 / PR #31 (sessionEnd hook + `session_registry`) | **Fold in, RESHAPED** (council G): reuse the hook plumbing as the healthy-disconnect input, but the daemon's in-memory `session->addresses` map is the authority and the hook calls a daemon-native `DeregisterSession` - the filesystem `session_registry` is **not** the authority. |
| #5 / #17 (holder session-binding, `--session-pid` pid-watch) | **Relocate into the daemon** (not retired); reuse `session_watch.rs`. |
| #28 (`--session-fd` reuse-immune binding) | **Defer**; fd-over-IPC awkward with a singleton daemon. |
| #3 (binary relay / `wait --loop`) | **Moot** - the daemon is always up. |
| #33 (in-memory queue drops messages) | **Headline stale** vs current `main` (0011/0013 already replaced `cursor=max_id` with durable per-recipient delivery). Residue (stale-lease-reads-healthy, `mark_delivered` cap, non-actionable-backlog visibility) becomes daemon hardening. |
| #6 (versioned installs + launcher shim) | **Fold in, SPLIT** (council C): a minimal upgrade floor (shim + `daemon stop --drain` + next-call respawn + cutover rule) lands early in `daemon-core`; the full platform (rollback/gc/UX) is the `seamless-upgrade` node, last. |
| #26 / #27 / #24 (delivery-scan index; `mark_delivered` cap; registry GC) | **#26 ELEVATED** (council B) to a `design-foundation` prerequisite: the `seen` dedup invariant (unpruned because holders restart) must be redesigned for a long-lived daemon. #27 / #24 **carry** - still relevant. |
| #12 (embeddable SDK client) | **Separate solve**; share the stabilized Layer-1 IPC. |
| #2 (response windows), #25 (`store_key` helper) | **Orthogonal**. |

## Open questions carried into design-foundation

1. Epoch lifecycle (resolves the reclaim race + handoff window via fencing): when the
   epoch increments, higher-epoch claim on respawn/handoff, loser self-demote, and
   Postgres cross-machine reclaim expressed in epochs.
2. Stale-attendance threshold + operator takeover flow (no teardown).
3. Typed `--watch-pid` final shape (anchor/required + start-time guard) and whether
   Copilot exposes a distinct per-session PID beyond the loader.

## Spar round 1 outcomes

A different-model spar (gpt-5.5) pressure-tested the design at arm's length; the
builder confirmed folding these in:

- **Lease-epoch fencing token (accepted).** The lease row is keyed by `address` only
  with no owner generation, so on stall/crash/handoff/reclaim an old daemon can write
  a row it no longer owns (duplicate delivery, ownership flip-flop). Add a monotonic
  `lease_epoch` / `owner_instance_id`: claim increments it; heartbeat/release are
  epoch-guarded; the daemon self-demotes on a 0-row heartbeat; IPC frames carry the
  epoch so waiters drop superseded ones. This is the real single-writer guarantee and
  the spine of daemon-down recovery, upgrade handoff, and Postgres reclaim.
- **Minimal stale-attendance, no teardown (accepted).** Without idle-TTL teardown, a
  dead/dismissed session whose hook failed but whose loader survives would hold
  `occupied` forever - the #32 zombie with the daemon as owner. Track
  `attendance_last_confirmed_at`, expose `occupied_stale`, and allow informed operator
  takeover past a threshold. Keeps `occupied` (idle-alive stays wakeable), never
  tears down; the full attended/idle/free policy stays deferred.
- **Typed watch-pid (accepted).** A flat global all/any rule is the wrong
  abstraction; semantics are per-pid: anchor (any sufficient) vs required (all
  necessary), plus a pid+start-time reuse guard. Loader-only is weak/hook-dependent
  liveness. `design-foundation` finalizes the shape.
- **Daemon singleton identity (accepted).** Per-user must not mean globally user-wide:
  key the singleton by user SID + config root (`TELEX_HOME`) + protocol-major, and
  have clients pass backend/store identity explicitly, to avoid cross-profile/version
  collisions.
- **Fencing-first sequencing (accepted as sequencing, not a scope cut).**
  `design-foundation` locks the hard contracts first; Postgres is gated on fencing
  proven under competing daemons; seamless-upgrade lands last. Both backends and #6
  stay in the deliverable; daemon-core + plugin (SQLite) is the first
  operator-unblocking slice.

## Council review outcomes

Two independent multi-perspective panels (different rosters/models) reviewed the
baseline + spar design at arm's length and **converged on the same top catch**. The
builder folded these in; they sharpen (not replace) the spar items. The load-bearing
code claims were verified against the source.

- **A - Server-side epoch fence + a distinct `fencing-proof` gate (BOTH panels;
  verified).** Lease-row fencing is insufficient: the holder emits the `Message`
  frame *before* `mark_delivered` commits, and per-process `seen` resets across a
  handoff, so a graceful handoff/crash can double-deliver. The fence must wrap the
  delivery-emit->commit critical section **server-side**
  (`mark_delivered_if_current_owner`; no frame unless the daemon owns the epoch;
  self-demote closes waiters; ordered handoff). Add a distinct executable
  `fencing-proof` gate before Postgres/plugin/upgrade rely on it.
- **B - `seen`-dedup redesign (verified).** 0013 left `seen` unpruned *because holders
  restart*; a long-lived daemon voids that. Specify a bounded/durable tombstone before
  `daemon-core`. Elevates #26 from carry to a design prerequisite.
- **C - Minimal upgrade floor early.** Not all of #6 can be last - the first
  daemon-aware install hits the Windows binary-lock (hit live this workstream). A
  minimal floor (shim + `daemon stop --drain` + next-call respawn + a deterministic
  legacy/non-epoch cutover rule) moves into `daemon-core`; full rollback/gc/UX stays
  last. Adds the `sqlite-unblock-shipped` milestone.
- **D - Daemon lifecycle contract + Status (BOTH panels).** Make auto-spawn/
  single-instance normative: spawn-lock, connect-or-spawn, readiness ACK, `wait`
  reconnect-on-EOF grace, exit codes, a bounded Status surface, and four gating tests
  (concurrent first-use, crash-during-`wait`, competing daemons, handoff duplicates).
- **E - Takeover is load-bearing (BOTH panels).** Because loader-PID is weak,
  stale-attendance/takeover is the primary per-session recovery path, not edge: define
  an explicit attendance/takeover state algebra. Temper the typed watch-pid CLI: v1
  floor = loader anchor + start-time; expose anchor/required flags only with a real
  consumer. (Stays within minimal stale-attendance; the full idle policy is still
  deferred.)
- **F - Scoped, versioned, capability-authorized IPC (BOTH panels; verified
  unauthenticated Wait/Shutdown).** A daemon-scoped (not address-keyed) control
  endpoint; requests carry `store_key`/`address`/`session_id`; a Hello/HelloAck version
  handshake; a scoped-capability authorization model (one token v1; scope/rotation
  fields reserved).
- **G - Daemon-native `DeregisterSession`, not PR #31's filesystem registry.** The
  daemon already owns `session_id->addresses` in memory; expose idempotent
  `Register`/`Re-register` + `DeregisterSession(session_id, proof)`. Reuse the #31 hook
  plumbing; drop the registry storage; the hook is a thin mapper.
- **H - Verb + docs/SKILL cutover decided early.** Keep the verb names; update
  `SKILL.md` + plugin docs **with** `daemon-core` so instructions never describe the
  dead model mid-workstream; hide the daemon entrypoint; single-source skill mechanism.

**Preserved dissent (do not drop):** a held-stream `SessionConnect` liveness was
proposed and **conceded** (needs a resident session process - conflicts with
zero-persistent-session-process; matches our held-socket rejection); a verb **rename**
was **withdrawn** (keep verbs); stronger multi-tier capability **parked** (record
scope/rotation fields now, defer tiers). No member reopened daemon-vs-holder.

**Reopen conditions:** if a server-side ownership guard can't be implemented/tested for
both backends; if "no distinct per-session PID" resolves and takeover still can't give
a safe recovery contract; if the early upgrade floor still needs manual process hunts;
if the cutover can't make legacy holders/non-epoch leases deterministic; if the threat
model is same-user-trusted-only (simplify capability) or multi-user (expand it); if
Postgres competing-daemon tests fail under fencing.

## Spar posture

The design was pressure-tested with a different-model sparring partner, **at
arm's length**: critique informs the design, but pivots are surfaced for builder
confirmation and not auto-applied (the operator has seen spar shift things in local
attention holes without good justification).
