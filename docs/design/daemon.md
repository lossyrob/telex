# Telex Daemon — Normative Contract (the local exchange)

## Status

**Normative design specification.** This document is the contract that `daemon-core`
and the downstream local-daemon nodes implement against. It is the authority for the
local-daemon architecture: the IPC/attendance protocol, the authorization model, the
server-side lease-epoch fence, the lifecycle contract and Status surface, daemon-native
session ownership, the liveness model, and the minimal upgrade floor. Where this
document and prose in [DESIGN.md](DESIGN.md) differ on a mechanism, **this document
governs the mechanism** and DESIGN.md governs the framing.

It was produced by the `local-daemon / design-foundation` node and is the design-gate
artifact. The decisions it records are in [DECISIONS.md](DECISIONS.md) as ADRs
0014–0021. The consolidated resolutions of the eight design-foundation open questions
are in the [Open-question resolutions](#open-question-resolutions) section, and the
explicit relocate/supersede/defer accounting is in the
[Relocations, supersessions, deferrals](#relocations-supersessions-deferrals) map.

This is a **design** document. It specifies contracts and invariants; it does not ship
code. Concrete struct/SQL/wire shapes below are normative *shapes* (names, fields,
types, ordering invariants), not final source.

## 1. The local exchange

Telex's presence and transport were previously bound to a **per-session resident
holder** (`telex attach` blocking for the session's lifetime; see superseded ADR 0004).
That coupled two unrelated jobs — *presence/liveness* ("address A is attended by a live
agent") and *delivery transport* (poll/buffer/push one message to a waiter) — to an
ephemeral per-session process, which is the root cause of telex's recurring staleness
(orphaned holders, zombie `occupied` leases, holder/waiter races, a forever-listener
starving a session's turn loop).

The local-daemon architecture relocates the irreducible presence part to **one
auto-spawned, single-instance, per-user supervised daemon — the local exchange** — and
deletes the per-session holder. In the telex metaphor, the exchange is the historical
switching center that connected **stations** to **telex numbers (addresses)**: it owns
the backend connection(s), the poll/LISTEN-NOTIFY loop, the durable delivery buffer, the
attendance registry, the lease heartbeat (single writer), the IPC endpoint, and
pid-watch. Sessions no longer run a resident process; they perform **one-shot** verbs
against the exchange.

A **station** is no longer a resident holder + waiter pair. A station is now **a
registration in the local exchange**: the durable lease row plus the in-exchange
attendance record that says "this session attends this address." `attach` creates the
registration; `wait` blocks for one delivery against the exchange; `detach` removes the
registration. (See [Verbs](#15-verbs-cli-mapping-and-the-single-source-skill).)

```text
  session A (one-shot verbs)        session B (one-shot verbs)
        |  attach / wait / detach          |
        +-------------------+--------------+
                            |  local IPC (daemon-scoped endpoint)
                   ┌────────▼─────────┐
                   │  local exchange   │  one per (user SID, config root, protocol-major)
                   │   (telex daemon)  │  presence + transport, single lease writer
                   └────────┬─────────┘
                            │ backend driver (single writer of liveness)
                   ┌────────▼─────────┐
                   │ SQLite / Postgres │  durable leases + deliveries
                   └──────────────────┘
```

What the exchange does **not** fix (irreducible): "is address A *actually* attended by a
live agent?" Something must still answer this; the exchange relocates it to one place
and makes the answer recoverable (hook + watch-pid + stale-attendance/takeover), but it
does not make it free.

## 2. Daemon singleton identity and auto-spawn

### 2.1 Singleton identity

The exchange is a singleton keyed by **`(user SID, config root, protocol-major)`**:

- **user SID** — the OS security principal (Windows SID; uid on Unix). Per-user, never
  globally user-wide-across-accounts.
- **config root** — the effective `TELEX_HOME` / config directory in force. Distinct
  config roots (e.g. test vs real) get distinct exchanges.
- **protocol-major** — the major version of this IPC protocol. A protocol-major bump
  runs a *separate* singleton so an old client and a new daemon never share an endpoint
  with incompatible framing.

Backend/store identity is **not** part of the singleton key. One exchange serves
**multiple stores** (multiple SQLite files and/or Postgres backends); clients pass store
identity explicitly on every request (`store_key`, see [§6](#6-ipc-protocol)). This is
why the endpoint is daemon-scoped, not address-keyed (superseding the address-keyed
`ipc.rs` endpoint).

The endpoint name embeds a hash of the singleton key:

- Windows: `\\.\pipe\telex-daemon-<H>` where `H = short_hash(user_SID, config_root, protocol_major)`.
- Unix: `<run_dir>/telex-daemon-<H>.sock`.

### 2.2 Auto-spawn (connect-or-spawn) and the spawn-lock

A client performs **connect-or-spawn**:

1. Try to connect to the singleton endpoint and complete the [Hello handshake](#6-ipc-protocol).
2. On success → use it.
3. On failure (no endpoint, or stale endpoint that fails Hello) → acquire the
   **spawn-lock**, then spawn the daemon and **retry connect-and-Hello** until `HelloAck`
   completes within the readiness window ([§2.3](#23-readiness-ack)) — this `HelloAck`
   **is** the readiness ACK; no out-of-band readiness signal exists.

The **spawn-lock** is an OS-level mutual exclusion that prevents a thundering herd (N
sessions first-using the exchange at once spawning N daemons): the canonical mechanism is
**bind-the-endpoint-as-the-lock** — exactly one process can create/bind the named pipe /
unix socket; the winner becomes the daemon, losers fail the bind and **retry connect**.
A lockfile (`<config_root>/daemon.lock`, advisory + pid + start-time) MAY supplement it
for spawn bookkeeping, but the endpoint bind is the authority. A stale endpoint (bind
succeeds because the prior daemon died) is the normal respawn path.

### 2.3 Readiness ACK

The spawning client must not race the daemon's startup. The daemon signals **readiness**
only after: endpoint bound, backend(s) reachable for the requested store, durable
recovery pass complete (see [§14.3](#143-crash-recovery-suspect--verified--lapsed-oq8-da-3)), and the
accept loop running. A connecting client treats "endpoint exists but Hello does not
complete within the readiness window" as not-ready and retries within backoff.

## 3. Lifecycle contract

A normative state machine for the daemon process:

```text
  SPAWNING ──readiness ACK──▶ SERVING ──drain/stop──▶ DRAINING ──▶ EXIT
     │                          │  ▲                      │
     │ bind/backend fail        │  │ recover               │ flush pending mark_delivered,
     ▼                          ▼  │                       │ release epochs cleanly
  EXIT(spawn-fail)          RECOVERING (on backend reconnect)
```

- **SPAWNING** — acquire endpoint bind (spawn-lock), open backend(s), run the recovery
  pass, then emit readiness. Bind contention → exit cleanly (a peer won; the client
  reconnects). Backend-unreachable → bounded retry, then exit `spawn-fail`.
- **SERVING** — accept connections, serve the protocol, write lease heartbeats (single
  writer), run delivery drains, watch pids.
- **RECOVERING** — a transient backend disconnect; the daemon pauses delivery/heartbeat,
  reconnects with backoff, re-validates owned epochs, and returns to SERVING. It does
  **not** exit on a transient blip.
- **DRAINING** — on `telex daemon stop --drain` or an upgrade handoff: quiesce new work,
  flush pending `mark_delivered`, release owned epochs in order
  ([§11.4 ordered handoff](#114-ordered-handoff)), then exit.

### 3.1 Retry / backoff / crashloop

A client's connect-or-spawn uses bounded exponential backoff (jittered) and a
**crashloop guard**: if spawn→readiness fails more than `crashloop_max` times within
`crashloop_window`, the client stops respawning and surfaces a daemon-down error
(below) rather than fork-bombing. The window/threshold are configurable; defaults frozen
in `daemon-core` acceptance.

### 3.2 Exit codes (client-observable)

`telex wait` keeps its existing contract (grounded in `src/commands/wait.rs`), extended
for the daemon:

| Code | Meaning |
|---|---|
| `0` | message delivered (printed as JSON on stdout) |
| `2` | idle timeout (no message within `--timeout-ms`) |
| `3` | daemon gone (connect/read failed or EOF) **after** the reconnect-on-EOF grace expired |
| `4` | daemon hung (no frame within the hang window, or heartbeat observed stale) |

One-shot verbs (`attach`/`detach`/`send`/`reply`/`status`) return `0` on success and a
documented non-zero on a daemon-down or protocol error; the exact non-zero set is frozen
in `daemon-core` acceptance.

### 3.3 `wait` reconnect-on-EOF grace

A daemon **restart or handoff is not a turn failure.** When `wait` is blocked and the
connection drops (EOF / broken pipe), `wait` MUST, within a short **reconnect grace
window**, (a) connect-or-spawn the (possibly new) daemon, (b) **auto-Re-register** the
session from inherited env (see [§14.4](#144-wait-auto-re-register)), and (c) resume
blocking — returning exit `3` only if the grace window expires without a healthy
reconnect. This makes ordered handoff and crash-respawn invisible to the agent's turn
loop.

## 4. Status surface (the frozen contract shape)

`telex daemon status` (and a per-store `telex status` projection) exposes a **bounded,
actionable** surface. **`design-foundation` freezes the field set and meaning** (below);
`daemon-core` acceptance owns the exact rendering/format, verbosity, and any extra
diagnostics. This is the [OQ7 freeze line](#open-question-resolutions): *frozen = the
fields + their meaning + the gating tests' per-test observable assertions; not frozen =
wire format, ordering, additional diagnostics.*

Frozen Status fields:

- **`protocol_version`**, **`daemon_version`**, **`instance_id`** (the
  `owner_instance_id` this daemon uses), **`singleton_key`** (user/config-root/proto-major,
  redacted as needed).
- **`epoch_by_address`** — for each owned address: `lease_epoch`, `owner_instance_id`,
  `state` (`suspect|verified|lapsed`), `occupied_stale` (bool).
- **`attendees`** — for each attendance record: `address`, `session_id` (opaque),
  `occupant`, `attendance_last_confirmed_at`, `watch_pids` (pid + role + alive),
  `backend`/`store_key`, `host`.
- **`backoff`** — current backend reconnect/backoff/crashloop state.
- **`recent_errors`** — a bounded ring of recent actionable errors (e.g. failed
  `sessionEnd`, `NotOwner` self-demotions, backend disconnects), each with a timestamp.
- **`stores`** — the set of stores this exchange currently serves.

## 5. Attendance model and record shape

The exchange maintains one **attendance record** per attended address. The **durable**
part lives in the backend lease row; the **in-memory** part is rebuilt on respawn (see
[§14.3](#143-crash-recovery-suspect--verified--lapsed-oq8-da-3)).

```text
AttendanceRecord {
  address:                      String,        // the telex number
  store_key:                    StoreKey,       // effective store identity (profiles::store_key)
  session_id:                   Option<String>, // opaque; the attending session
  occupant:                     String,         // human/host label of the occupant
  owner_instance_id:            String,         // the owning daemon instance (fencing identity)
  lease_epoch:                  u64,            // monotonic fence token (see §11)
  watch_pids:                   Vec<WatchPid>,  // liveness backstop (see §9)
  host:                         String,
  attendance_last_confirmed_at: i64,            // epoch ms; refreshed by POSITIVE presence only
  state:                        Attendance,     // Suspect | Verified | Lapsed
  occupied_stale:               bool,           // DERIVED: now - last_confirmed > stale_after
}

WatchPid { pid: u32, start_time: u64, role: Anchor | Required }
Attendance = Suspect | Verified | Lapsed
```

### 5.1 Durable lease-row columns (new)

The backend `leases` table — today keyed by `address` only with **no owner generation**
(verified: `src/registry.rs` `HolderRecord`, backend `claim_lease`/`heartbeat`/
`release_lease`) — gains:

- **`lease_epoch INTEGER`** — the monotonic fence token.
- **`owner_instance_id TEXT`** — the owning daemon instance.
- **`attendance_last_confirmed_at INTEGER`** — epoch ms of last positive confirmation.

Greenfield: added via `CREATE TABLE IF NOT EXISTS` / additive column add (no migration
machinery; pre-first-non-beta, single-user — consistent with ADR 0013). A row with
`lease_epoch IS NULL` is a **legacy** row (see [§12](#12-legacy-cutover-two-phase-oq5-da-1)).

The **occupant-null release** branch (`release_lease ... WHERE address=? AND (occupant=?
OR occupant IS NULL)`, verified in `sqlite.rs`/`postgres.rs`) is **removed**: release is
strictly epoch- and owner-guarded (see [§11.2](#112-epoch-guarded-heartbeat-and-release)).

## 6. IPC protocol

A **daemon-scoped**, versioned, length-or-line-framed control protocol. Serialization is
**JSON, one object per line** (`serde` / `serde_json`), extending the current
`src/ipc.rs` framing. The protocol is intended to be reusable by the embeddable SDK
client (#12) — it is a stable Layer-1 surface.

Every request after the handshake carries the routing/identity fields it needs:
`store_key`, and where relevant `address` and `session_id`. Privileged requests
additionally carry an authorization `proof` (see [§7](#7-authorization)).

### 6.1 Version handshake (Hello / HelloAck)

The **first** frame on every connection is a handshake, so an old daemon and a new
client (or vice versa) detect skew deterministically instead of mis-framing:

```text
→ Hello    { protocol_version, client_version, store_key, capabilities: [..] }
← HelloAck { protocol_version, daemon_version, accepted: bool, reason?: string }
```

If `protocol_major` differs, the client and daemon belong to different singletons
([§2.1](#21-singleton-identity)) and the client connect-or-spawns the correct one. A
compatible-minor skew is accepted; `capabilities` lets each side gate optional behavior.

### 6.2 Request / response frames

Requests (Layer-1 operations):

| Request | Purpose | Privileged? |
|---|---|---|
| `Hello` | version handshake | no |
| `Register { store_key, address, session_id, occupant, description?, scope?, tags?, watch_pids[] }` | create/refresh a station (attach) | no (same-trust) |
| `ReRegister { store_key, address, session_id, watch_pids[] }` | idempotent re-register after respawn | no |
| `DeregisterSession { session_id, proof }` | drop all of a session's addresses (healthy disconnect) | **yes** |
| `Detach { store_key, address, session_id, proof }` | remove one station | **yes** |
| `Wait { store_key, address, attention?, timeout_ms }` | block for one delivery | no (session-scoped) |
| `Status { detail?, proof? }` | Status surface (detail requires proof) | detail: **yes** |
| `Takeover { store_key, address, proof }` | operator takeover of a stale address | **yes** |
| `Drain { proof }` | quiesce + flush + exit (upgrade/stop) | **yes** |

Responses:

| Response | Carries |
|---|---|
| `HelloAck` | protocol/daemon version, accepted |
| `Registered` | `lease_epoch`, `owner_instance_id`, `state` |
| `Message` | `id, thread_id, parent_id, from_addr, to_addr, kind, attention, requires_disposition, subject, body, sent_at_ms, buffered_at_ms, lease_epoch` |
| `Keepalive` | `heartbeat_age_ms` |
| `Timeout` | — (idle-timeout) |
| `StatusReport` | the [§4](#4-status-surface-the-frozen-contract-shape) fields |
| `Ack` | generic success for Register/ReRegister/Detach/Deregister/Takeover/Drain |
| `Error` | `{ code, message }` — incl. `UnknownSession`, `NotOwner`, `Unauthorized`, `Incompatible`, `Ambiguous` |

The `Message` frame **carries the `lease_epoch`** so a waiter can drop a frame from a
superseded epoch. Crucially, the daemon emits a `Message` frame **only after** the
server-side delivery fence authorizes it (see [§11.3](#113-server-side-delivery-fence-da-7)).

## 7. Authorization

Today `Wait`/`Shutdown` are **unauthenticated** (verified in `src/ipc.rs`). The exchange
serves multiple sessions/stores for one user, so privileged operations need a proof. The
threat model in v1 is **same-user, user-private** (one human's local processes); the
model is built to extend, not to enforce cross-user isolation yet.

### 7.1 Scoped capability model (v1: one instance-admin token)

- At spawn the daemon mints an **instance secret** (the `admin_cap`) and writes it to a
  **user-private file** in the config root (`<config_root>/daemon.cap`, mode `0600` /
  Windows ACL: owner-only). Being same-user, a legitimate client can read it; a
  different user cannot.
- **Unprivileged** requests (`Hello`, `Register`, `ReRegister`, `Wait`) need no proof:
  any same-user local process may register a station or wait on one. (Registering is a
  presence claim under lease exclusivity, not a privileged action.)
- **Privileged** requests (`DeregisterSession`, `Detach`, `Takeover`, `Drain`,
  `Status detail`) carry `proof = admin_cap`. The daemon verifies `proof` equals its
  instance secret.
- The capability frame **reserves `scope` and `rotation` fields** (recorded now, unused
  in v1) and reserves a **`per_session_cap: Option<Cap>`** field for future
  lateral-compromise defense — both **deferred with rationale**: a per-session
  capability is zero-marginal-value over the admin cap under the same-trust user-private
  model, and (critically) is **not obtainable today** because the minting process and the
  later hook process are different processes ([§14.2](#142-deregistersession-proof-oq6)).

This is the [OQ6 resolution](#open-question-resolutions): proof without an external
session→address registry — the hook reads the user-private instance secret and presents
`(session_id, admin_cap)`; the daemon checks the secret and that `session_id` is in its
in-memory map.

## 8. (reserved)

*(Section intentionally folded into §5 and §14; numbering preserved for cross-refs.)*

## 9. Liveness model

Two paths, exactly as ratified (ADR-to-be 0017):

1. **Healthy disconnect = the sessionEnd hook.** Quit and dismiss both fire
   `session.ended`; the harness plugin calls `DeregisterSession(session_id, admin_cap)`,
   and the exchange drops that session's addresses. This is the normal path.
2. **Ungraceful backstop = daemon pid-watch of typed watch-pids.** The exchange watches
   each station's `watch_pids` and tears the station down when the predicate resolves to
   dead. telex core names nothing harness-specific; the Copilot plugin maps
   `COPILOT_LOADER_PID` onto a generic `--watch-pid` and
   `COPILOT_AGENT_SESSION_ID` onto `$TELEX_SESSION_ID`.

There is **no idle-TTL teardown** — but the precise statement is **"no time-based
dismissal of a *live* session; positive death evidence triggers immediate teardown"**
(see the dismissal-path matrix below). Lease TTL survives in exactly one role: the
**daemon-down backstop** ([§14.5](#145-daemon-down-and-the-ttl-backstop)).

### 9.1 Typed watch-pid predicates (OQ3)

The singular `--session-pid` (issues #5/#17) generalizes to **typed predicates**:

- **`anchor`** — the station is alive if **any** anchor pid survives. (A loader/root pid
  is an anchor: if it is gone, the whole tree is gone.)
- **`required`** — the station is alive only if **all** required pids survive. (A
  specific per-session pid, where one exists, is required.)
- A **pid + start-time reuse guard** accompanies every watched pid: a pid is "alive" only
  if it is alive **and** its process start-time matches the captured start-time. This
  closes the pid-reuse hole in today's `session_watch::process_alive` (verified pid-only,
  no start-time).

**v1 floor = a single loader `anchor` + start-time.** The `required`/`anchor` flag
surface is exposed **only where a real consumer/test exists** (council E discipline): in
v1 the only populated predicate is the loader anchor.

### 9.2 Per-session PID on Copilot CLI (OQ4 — resolved: none usable)

Empirically grounded (live probe, Copilot CLI 1.0.64-1, Windows): the harness exposes
`COPILOT_AGENT_SESSION_ID` and `COPILOT_LOADER_PID`. `copilot.exe` is a **supervisor that
re-execs an identical-argv inner worker**; the inner worker's PID is **not** exposed as
an env var **and spawns lazily** (a freshly launched, idle session is loader-only — two
probe sessions stayed loader-only with no inner child). Therefore the inner pid is **not
reliably capturable at register time**, and discovering it would require the **ppid-walk
the design rejects** (superseded ADR 0012: a reparented background launch makes ppid
unsound). 

Resolution: the **loader anchor + start-time is the sole env-sourced backstop** in v1.
The `required` per-session predicate's "additional per-session pid" slot is documented as
**not reliably sourceable on Copilot CLI today**. This makes the sessionEnd hook the
necessary healthy-dismiss path and **stale-attendance/takeover the load-bearing
unhooked-dismiss recovery** (council E), since loader-only liveness is weak (a single
session's inner process dying while the loader survives is not caught by pid-watch).

### 9.3 Dismissal-path matrix (the four disjoint cases)

The exchange dismisses a station via exactly one of four disjoint paths. Positive death
evidence is categorically different from "idle" and must never be routed through
`occupied_stale`:

| # | Trigger | Mechanism | Teardown |
|---|---|---|---|
| 1 | **sessionEnd hook** (clean quit/dismiss) | `DeregisterSession(session_id, admin_cap)` | immediate, addresses released |
| 2 | **watch-pid failure** — the typed predicate resolves dead per [§9.1](#91-typed-watch-pid-predicates-oq3) (no `anchor` pid survives, or any `required` pid is gone, or a start-time mismatch) | the daemon's local watcher issues an **internal `DeregisterSession`** for that session, **bypassing `occupied_stale`** | immediate |
| 3 | **operator takeover** | privileged `Takeover` (see [§10.2](#102-takeover-atomic-at-the-exchange-da-5)) | atomic re-bind |
| 4 | **daemon-down TTL** | lease lapses after the daemon-down window; respawn re-claims | backstop only |

`occupied_stale` is reserved for the **unobserved-death case only**: no hook fired *and*
no watch-pid signal is available (e.g. unhooked dismiss where the loader anchor survives).
That is the residual the next section governs.

## 10. Stale-attendance and takeover (no teardown)

### 10.1 `last_confirmed`, `occupied_stale`, and the hook-semantics split (OQ2, DA-6)

`attendance_last_confirmed_at` is refreshed by **positive-presence signals only**:
`Register`, each authenticated `Wait` connect, and any future positive resume/connect
hook (see [§16 OQ-γ](#open-question-resolutions)). **`sessionEnd` does NOT refresh** — it
is a *removal* signal: it releases addresses / drops session membership. A **failed**
`sessionEnd` records a `recent_error` and **leaves `last_confirmed` unchanged** — it must
not "refresh-then-fail-before-remove," which would artificially reanimate a dying
session.

`occupied_stale` is **derived**, not stored: `now - attendance_last_confirmed_at >
stale_after`, where `stale_after` is configurable (default a small multiple of the
heartbeat/lease window; the exact default is frozen in `daemon-core`). It is surfaced in
Status and `address list`. It **never triggers teardown** — an idle-but-alive session
stays `occupied` and instantly wakeable (the operator's explicit requirement).

### 10.2 Takeover (atomic at the exchange) (DA-5)

Because the exchange is a singleton, the common takeover case is **intra-daemon** (the
stale station and the new claimant are served by the same daemon process). Backend epoch
fencing alone would leave **stale in-memory IPC waiters and `session_id → addresses`
mappings** inside that process. Takeover is therefore **atomic at the exchange** — in one
critical section it:

1. mints a new backend **`lease_epoch`** (fencing the prior owner at the backend),
2. **evicts** the prior `session_id → addresses` entry for the rotated address,
3. **closes** the IPC waiters bound under the prior occupant (their `wait` reads return a
   defined disconnect, not a silent hang),
4. **binds** the address under the new occupant.

No observable intermediate state (no window where the address is both old-owned and
new-owned). Takeover is a **privileged** `Takeover` RPC, allowed once the address is
`occupied_stale`, and the response reports the prior occupant + `last_confirmed` so the
operator decides informedly. There is **no idle teardown** — takeover is an explicit
operator action, the recovery path for the weak-loader-liveness residual.

## 11. Lease-epoch fence (the spine)

The lease row is keyed by `address` with **no owner generation today**, so on
stall/crash/handoff/reclaim an old daemon can write a row it no longer owns (duplicate
delivery, ownership flip-flop). The fence is a **monotonic `lease_epoch` +
`owner_instance_id`**. This is the single-writer guarantee and the spine of daemon-down
recovery, upgrade handoff, and Postgres reclaim.

### 11.1 Epoch lifecycle (OQ1)

- **Claim / takeover increments the epoch.** A daemon claims an address by writing
  `lease_epoch = observed_epoch + 1, owner_instance_id = self` **conditioned on the
  observed row** (compare-and-set on `(address, observed_epoch, observed_owner)`). A
  `NULL`/absent epoch is treated as epoch `0` (legacy; see [§12](#12-legacy-cutover-two-phase-oq5-da-1)).
- **Monotonic, never reused.** Epochs only increase per address.
- The winner's `owner_instance_id` is its stable instance identity for the daemon's life.

### 11.2 Epoch-guarded heartbeat and release

Heartbeat and release are **conditioned on ownership** and **must return a rowcount**:

```text
heartbeat: UPDATE leases SET last_heartbeat=?, attendance_last_confirmed_at=?
           WHERE address=? AND lease_epoch=? AND owner_instance_id=?   → rows: 0|1
release:   DELETE FROM leases
           WHERE address=? AND lease_epoch=? AND owner_instance_id=?   → rows: 0|1
```

A **0-row heartbeat** means a higher epoch exists (someone else claimed). The daemon
**self-demotes** for that address: it stops heartbeating, **closes its waiters** for the
address, drops the in-memory station, and emits no further frames for it. (Today's
`heartbeat` returns `Result<()>` with no rowcount — verified in `sqlite.rs:325-333` /
`postgres.rs:313-320` — so this rowcount-returning shape is a required backend-API
change.) The occupant-null release branch is removed.

### 11.3 Server-side delivery fence (DA-7)

**Lease-row fencing alone is insufficient for delivery.** The verified hazard
(`src/commands/attach.rs:477` vs `:485`): the holder writes the `Frame::Message` to the
waiter **before** `mark_delivered` commits, and per-process `seen` resets across a
handoff — so a graceful handoff or crash can double-deliver. The fence must wrap the
delivery-emit→commit critical section **server-side**, via a typed backend method:

```text
mark_delivered_if_current_owner(address, owner_instance_id, lease_epoch, message_id)
    -> Result<DeliveryOutcome>

DeliveryOutcome = Delivered | NotOwner | AlreadyDelivered
```

**Ordering invariant:** the daemon MUST receive a **non-`NotOwner`** result **before it
emits any `Frame::Message`** for that message:

- `Delivered` → permit emission of the `Message` frame.
- `AlreadyDelivered` → no-op (idempotent; another path already recorded it). **No frame.**
- `NotOwner` → the daemon has lost the epoch; **self-demote** (as for a 0-row heartbeat).
  **No frame.**

No `Frame::Message` is ever emitted after `NotOwner` or `AlreadyDelivered`. This closes
the pre-commit emission hazard at the server.

### 11.4 Ordered handoff

A graceful handoff (upgrade/stop) must not lapse a lease mid-flight or double-deliver:

```text
quiesce  → stop accepting new Wait/Register for the address; stop new drains
flush    → complete pending mark_delivered_if_current_owner calls
unbind   → release the epoch-guarded lease row (rowcount-checked)
claim    → the successor claims epoch+1 under the row condition (overlapping/after, no TTL gap)
```

The successor's higher epoch fences the predecessor; the predecessor's next heartbeat
0-rows and it self-demotes. Waiters see EOF and reconnect-on-EOF
([§3.3](#33-wait-reconnect-on-eof-grace)).

### 11.5 Postgres cross-machine reclaim (in epochs, not timing)

On Postgres two daemons can race across machines. Reclaim is **expressed in epochs, not
wall-clock**: a reclaiming daemon wins by the same compare-and-set claim
(`epoch+1` conditioned on the observed row); the loser self-demotes on its next 0-row
heartbeat. No timing assumption decides ownership. SQLite-local is the simple
single-writer case (commit order == id order); `postgres-parity` proves the competing
-daemon behavior under MVCC. (Correctness rests on READ COMMITTED autocommit reads, the
isolation precondition already pinned by ADR 0013.)

## 12. Legacy cutover (two-phase) (OQ5, DA-1)

The first daemon-aware rollout meets **legacy holders** (resident `attach` processes) and
**non-epoch lease rows** (`lease_epoch IS NULL`). **Occupant-rotation alone is
insufficient**: a legacy holder ships `Frame::Message` (`attach.rs:~477`) *before* its
post-emit `mark_delivered` (`~485`), and its `heartbeat` returns `Result<()>` with **no
rowcount** so it **cannot observe self-demotion**; if the daemon rebinds the address's
waiter endpoint, two endpoints emit independently regardless of any post-emit row fence.
The deterministic rule is therefore **two-phase**:

- **Phase 1 — drain.** The daemon-aware claimant detects the non-epoch row and, **before
  binding its own waiter**, confirms **no legacy holder is actively bound** — either via
  an address-keyed IPC probe carrying a quit/handover signal (the legacy
  endpoint name is still derivable), **or** by waiting a bounded stale-window for the
  legacy heartbeat to age out. There must be **no live overlap of two waiter-binds** for
  one address.
- **Phase 2 — claim.** Only after drain, claim `lease_epoch = 1` (NULL→0→1) under the row
  condition and **atomically rotate `occupant → owner_instance_id`**, using the
  rowcount-returning epoch-guarded heartbeat/release thereafter. Remove the occupant-null
  release branch.

**Cutover gating assertion (frozen):** *no `Frame::Message` from a non-epoch holder
reaches a recipient after the daemon's waiter binds.* Hard cutover of existing sessions
is acceptable (ratified).

> Preserved minority (design-foundation council): one reviewer held that occupant
> rotation alone is the cutover (the legacy heartbeat would naturally 0-row and trigger
> legacy shutdown). Adopted the two-phase rule instead, because the legacy heartbeat does
> **not** return rowcount and the wire-level `Frame::Message` emission is not covered by a
> post-emit row fence. Reopen if a legacy holder's local heartbeat is shown to
> self-terminate on 0-row, or cannot bind its waiter once occupant is rotated.

## 13. Delivery and the `seen`-dedup redesign (DA-8)

The exchange reuses the **durable per-recipient delivery buffer** of ADRs 0011/0013 (the
`deliveries(message_id, recipient)` table, `UNIQUE(message_id, recipient)`,
`fetch_undelivered`) unchanged as the **cross-epoch / cross-restart dedup authority**.
The live drain remains "deliver the undelivered set, authoritative on delivery state,
never on id ordering" (ADR 0013), now fenced by [§11.3](#113-server-side-delivery-fence-da-7).

The in-memory `seen` set must be **redesigned for a long-lived daemon.** Today `seen` is
an **unbounded `Mutex<HashSet<i64>>` that is never pruned** *because holders restart*
(verified `attach.rs:32-41,67-83`; rationalized in ADR 0013) — a long-lived daemon voids
that assumption (unbounded growth; stale identity across epochs). Redesign:

- **Durable `deliveries` is the authority** for "has this been delivered?" — no
  behavioral change to 0011/0013.
- **In-memory dedup is a bounded fast-path** keyed by **`(recipient, message_id,
  lease_epoch)`** (in-flight identity, scoped to the current epoch).
- **Seed** the fast-path from `fetch_undelivered` on claim.
- **Evict** an entry on: a durable mark (`mark_delivered_if_current_owner → Delivered`),
  a terminal disposition, or an epoch transition.
- **Reset/drop** the entire fast-path on epoch loss (self-demote, takeover) — its
  identity is epoch-scoped, so it must not survive a fence.

This keeps dedup bounded and correct without relying on process restart, and elevates
issue #26 from a carry to a satisfied design prerequisite. (#27 `mark_delivered` cap and
#24 registry GC remain carries.)

## 14. Daemon-native session ownership

### 14.1 The in-memory `session_id → addresses` authority

The exchange owns an **in-memory** `session_id → {addresses}` map as the **authority**
for which addresses a session attends. This **reshapes #23 / PR #31**: the hook plumbing
is reused, but the filesystem `session_registry` (verified on
`feature/copilot-session-end-plugin`: per-session JSON files) is **dropped as the
authority**. The Copilot hook becomes a **thin mapper**
(`COPILOT_AGENT_SESSION_ID → TELEX_SESSION_ID`), and Copilot JSON parsing never becomes a
core protocol dependency (it lives in the plugin layer).

Operations (idempotent): `Register`, `ReRegister`, `DeregisterSession(session_id,
proof)`.

### 14.2 `DeregisterSession` proof (OQ6)

The sessionEnd hook runs as a **separately spawned process** (verified
`integrations/copilot-cli/hooks.json` runs `telex session-end`; `session_end.rs` reads
only a session id). It cannot inherit a secret minted in the earlier `attach`/loader
process's memory. So the proof in v1 is the **instance `admin_cap`**, read from the
user-private `<config_root>/daemon.cap` ([§7.1](#71-scoped-capability-model-v1-one-instance-admin-token)):
the hook presents `DeregisterSession(session_id, admin_cap)`; the daemon verifies the
secret and that `session_id` is in its map, then drops the addresses. **No external
session→address registry**, no per-session cap minted at Register in v1 (the
`per_session_cap` field is reserved for a future lateral-compromise threat model).

### 14.3 Crash recovery: `suspect` / `verified` / `lapsed` (OQ8, DA-3)

A respawned daemon **must not freshen recovered rows as live attendance without proof.**
Recovery is a three-state machine over attendance records:

- **`suspect`** — every row recovered from durable storage on respawn starts here. The
  daemon **MUST NOT heartbeat or deliver** for a `suspect` row (it has no proof the
  session is still alive).
- **`verified`** — promoted by a successful `Register` or `ReRegister`. A `Wait`
  reconnect promotes only **indirectly** — via the auto-`ReRegister` triggered on
  `UnknownSession` (see [§14.4](#144-wait-auto-re-register)); the `Wait` IPC frame itself
  remains sessionless ([§6.2](#62-request--response-frames)). Promotion claims a **new
  epoch** ([§11.1](#111-epoch-lifecycle-oq1)), refreshes `last_confirmed`, and rebuilds
  the `watch_pids` set.
- **`lapsed`** — a `suspect` row that ages out via the daemon-down TTL window or
  stale-attendance/takeover with no proof. Its lease is released/fenced; it is not a
  permanent zombie.

**Durable vs rebuilt:** durable (recovered) = the lease rows (`address`, `occupant`,
`lease_epoch`, `owner_instance_id`, `last_confirmed`) + the durable delivery buffer.
Rebuilt-by-client = the in-memory `session_id → addresses` map, the live `watch_pids`
set, and IPC waiter registrations.

### 14.4 `wait` auto-Re-register

`wait` is the **only long-lived client** able to re-prove a running session after a
respawn (the loader's `attach` is one-shot and already exited). On reconnect-on-EOF,
`wait` MUST **auto-`ReRegister`** from inherited env (`TELEX_SESSION_ID` and the
watch-pids) **before** failing. `ReRegister` is unprivileged — no `admin_cap` is required
(per [§7.1](#71-scoped-capability-model-v1-one-instance-admin-token)); the `admin_cap`
remains available in env for any privileged follow-up. A `Wait` that returns
`UnknownSession` triggers the same `ReRegister` then retries. `ReRegister` is
**idempotent**: concurrent waits for one `session_id` converge to a single map entry by
**union** of their address sets (so a multi-address session is never narrowed by one
re-register; `daemon-core` MAY freeze an alternative single rule, but union is the
default).

### 14.5 Daemon-down and the TTL backstop

If the daemon is down, its leases lapse after the **daemon-down TTL window** (the one
surviving role of the old TTL heartbeat) and/or are fenced by the respawned daemon's
higher epoch. A session that **ends while the daemon is down**: its `sessionEnd` hook
no-ops against a down daemon (recorded as a transient on the harness side, not fatal),
and the address is recovered on respawn via `suspect`→`lapsed` (TTL) or operator
takeover — **no permanent zombie**.

## 15. Verbs, CLI mapping, and the single-source SKILL

### 15.1 Verb mapping (no renames — preserved dissent)

The **CLI verbs are unchanged**: `attach` / `detach` / `wait` (and `send`/`reply`/etc.).
They become **one-shot** against the exchange instead of resident:

| CLI verb | Was | Now (against the exchange) | IPC operation |
|---|---|---|---|
| `attach` | block as resident holder | one-shot: register a station, exit | `Register` |
| `detach` | stop the holder | one-shot: remove the station | `Detach` |
| `wait` | block on the local holder | block on the exchange for one delivery, exit | `Wait` |

`Register` / `ReRegister` / `DeregisterSession` are **protocol/IPC operations**, not CLI
renames. The held-stream `SessionConnect` liveness is **not** adopted (preserved
dissent). The `telex daemon` entrypoint (and `telex daemon stop --drain`,
`telex daemon status`) is **hidden** from normal user help — the exchange is implicit and
zero-config, like `rust-analyzer`/`gopls`.

### 15.2 Single-source SKILL / plugin-skill mechanism (OQ for deliverable 7, DA-10)

One source serves both the CLI command and the plugin skill:

- **Canonical file:** root `SKILL.md` (unchanged; stays at the repo root).
- **CLI consumer:** `telex skill` prints the embedded `SKILL.md`
  (`include_str!` in `src/commands/skill.rs`, unchanged) — add a `--raw` form for
  machine consumption.
- **Plugin-skill consumer:** a plugin manifest pointer if the harness supports pointing
  at a file, otherwise a thin wrapper that `exec`s `telex skill --raw`.
- **Invariant:** **no generated divergent copy** — both consumers resolve to the same
  `SKILL.md`. The holder/waiter → exchange narrative cutover in `SKILL.md` lands **with
  `daemon-core`**, not in this node and never mid-workstream describing a dead model.

## 16. Minimal upgrade floor

The full seamless-upgrade platform (rollback / gc / UX) is the `seamless-upgrade` node
(last). A **minimal floor** lands in `daemon-core`, because the first daemon-aware
install hits the Windows binary-lock (a running `telex` process locks the binary during
swap):

- **Versioned install + launcher shim.** A stable `telex` shim resolves to a versioned
  binary (`telex-<version>`), so an upgrade writes a new versioned binary without
  overwriting the locked one.
- **`telex daemon stop --drain`.** Quiesce + flush pending `mark_delivered` + release
  epochs in order ([§11.4](#114-ordered-handoff)), then exit — freeing the binary lock.
- **Next-call respawn.** The next client connect-or-spawn starts the new version
  (handoff reuses ordered-handoff + crash-recovery).
- **Legacy / non-epoch cutover rule** = the two-phase rule of [§12](#12-legacy-cutover-two-phase-oq5-da-1).

## 17. Gating tests (daemon-core acceptance)

Five executable gating tests are **frozen as `daemon-core` acceptance**, each with the
**observable assertions** below (OQ7 — the assertions are part of the frozen contract;
rendering is not). They run on **both backends** unless noted.

1. **Concurrent first-use (thundering-herd auto-spawn).** N clients first-use the
   exchange simultaneously. *Assert:* exactly one daemon ends up bound; all losers
   connect to it; no duplicate exchange; no orphaned spawn.
2. **Crash-during-`wait`.** Kill the daemon while a client is blocked in `wait`. *Assert:*
   `wait` reconnects within the grace window, auto-`ReRegister`s from env, and resumes
   (no spurious exit 3); recovered rows are `suspect` and are **not delivered** until
   promoted to `verified`.
3. **Competing daemons** (Postgres-focused; cross-machine). Two daemons race a claim for
   one address. *Assert:* the higher epoch wins; the loser self-demotes on its next 0-row
   heartbeat; no flip-flop; no delivery from the demoted owner.
4. **Handoff duplicates + ownership-loss-around-delivery.** Force an ordered handoff and,
   separately, rotate ownership *between* a `mark_delivered_if_current_owner` call and its
   response. *Assert:* no message is delivered twice; the racing call returns `NotOwner`;
   the losing daemon self-demotes; **no `Frame::Message` after `NotOwner`/`AlreadyDelivered`.**
5. **Intra-daemon takeover local-eviction.** Operator `Takeover` of an `occupied_stale`
   address served by the same daemon. *Assert:* the old-occupant `wait` receives a defined
   error/disconnect (not a silent hang); a new-occupant `wait` receives subsequent
   messages; no observable both-owned intermediate state.

## Open-question resolutions

The eight open questions carried into `design-foundation`, resolved with implementable
specifics (cross-referenced to the sections above):

| OQ | Question | Resolution | Where |
|---|---|---|---|
| **1** | Epoch lifecycle | Monotonic `lease_epoch` + `owner_instance_id`; claim/takeover = `epoch+1` CAS on the observed row; rowcount-returning epoch-guarded heartbeat/release; 0-row → self-demote; server-side `mark_delivered_if_current_owner` typed fence with the non-`NotOwner`-before-frame ordering invariant; ordered handoff; Postgres reclaim in epochs not timing. | [§11](#11-lease-epoch-fence-the-spine) |
| **2** | Stale-attendance threshold + takeover (no teardown) | `last_confirmed` refreshed by positive presence only (sessionEnd does not refresh); `occupied_stale` derived from `stale_after`; never tears down; takeover atomic at the exchange (mint epoch + evict map + close waiters + bind), allowed once stale. | [§10](#10-stale-attendance-and-takeover-no-teardown) |
| **3** | Typed `--watch-pid` shape | `anchor` (any-sufficient) vs `required` (all-necessary) + pid+start-time reuse guard; v1 floor = loader anchor + start-time; expose flags only with a real consumer; dismissal-path matrix routes positive death to immediate teardown. | [§9.1](#91-typed-watch-pid-predicates-oq3), [§9.3](#93-dismissal-path-matrix-the-four-disjoint-cases) |
| **4** | Distinct per-session PID? | **No usable one** on Copilot CLI today (empirically grounded: inner worker pid not env-exposed and lazily spawned; ppid-walk rejected). Loader anchor + start-time is the sole env-sourced backstop; hook + stale-attendance/takeover carry the rest. | [§9.2](#92-per-session-pid-on-copilot-cli-oq4--resolved-none-usable) |
| **5** | Legacy / non-epoch cutover | **Two-phase**: drain (confirm no legacy waiter bound) THEN claim `epoch=1` + occupant rotation; rotation-alone insufficient (wire-level pre-commit emission proof). Cutover gating assertion frozen. | [§12](#12-legacy-cutover-two-phase-oq5-da-1) |
| **6** | DeregisterSession proof (no external registry) | Instance `admin_cap` from a user-private `daemon.cap` file; hook presents `(session_id, admin_cap)`; daemon verifies secret + map membership. Per-session cap reserved/deferred (not obtainable across the hook process boundary in v1). | [§7](#7-authorization), [§14.2](#142-deregistersession-proof-oq6) |
| **7** | Status freeze line | Freeze the **field set + meaning** + the five gating tests' observable assertions; `daemon-core` owns rendering/format/verbosity. | [§4](#4-status-surface-the-frozen-contract-shape), [§17](#17-gating-tests-daemon-core-acceptance) |
| **8** | Attendance durability across daemon crash | Durable = lease rows (incl. epoch/owner/last_confirmed) + delivery buffer; rebuilt-by-client = in-memory map + watch-pids + IPC waiters. `suspect`/`verified`/`lapsed` recovery; `wait` auto-Re-register; daemon-down TTL backstop; no permanent zombie. | [§14.3](#143-crash-recovery-suspect--verified--lapsed-oq8-da-3), [§14.4](#144-wait-auto-re-register) |

**OQ-γ (adjacent, design-foundation council).** *sessionResume / positive-presence hook
scope:* if a positive-presence resume/connect hook is added later, it **joins** the
`last_confirmed` refresh path ([§10.1](#101-last_confirmed-occupied_stale-and-the-hook-semantics-split-oq2-da-6));
`design-foundation` does **not** require it in v1. Stated so `daemon-core` is not
stranded if such a hook lands.

## Relocations, supersessions, deferrals

What the exchange relocates, supersedes, or defers across the related issues/PRs and the
existing decision log. Deferred items are explicit so they are not silently dropped.

### Supersedes / amends (decision log)

| Prior | Disposition |
|---|---|
| **0004** (resident holder + ephemeral client) | **Superseded** by the exchange: zero persistent session processes; one-shot verbs. |
| **0009** (station = holder + waiter) | **Recast**: station = a registration in the local exchange (lease row + attendance record), not a resident pair. |
| **0010** (local holder registry as `from`-default source) | **Superseded** as the `from` source: daemon-era `ResolveFrom(TELEX_SESSION_ID)` against the session's registered addresses ([§14.1](#141-the-in-memory-session_id--addresses-authority); DA-9). Never infer across sessions; harness propagates `TELEX_SESSION_ID` to `send`/`reply`. |
| **0012** (holder self-binds via pid-watch; ppid declined) | **Relocated** into the exchange's typed watch-pid; ppid-walk stays rejected; reaffirmed by the OQ4 probe. |
| **0013** (live drain on the undelivered set; `seen` never pruned) | **Drain retained**; the **never-prune `seen`** rationale is **superseded** (holder-restart assumption voided) by the bounded epoch-keyed fast-path ([§13](#13-delivery-and-the-seen-dedup-redesign-da-8)). |
| **0005** (TTL-heartbeat liveness) | **Narrowed**: TTL survives only as the **daemon-down backstop** ([§14.5](#145-daemon-down-and-the-ttl-backstop)); it no longer governs live-session liveness (hook + watch-pid + stale-attendance do). |

### Issue / PR relocations

| Issue / PR | Disposition |
|---|---|
| **#32** | Workstream umbrella. |
| **#23 / PR #31** (sessionEnd hook + filesystem `session_registry`) | **Reshaped**: reuse hook plumbing; drop the filesystem registry as authority; daemon-native `DeregisterSession` + in-memory map. |
| **#5 / #17** (`--session-pid` pid-watch) | **Relocated** into the exchange as typed watch-pid (reuses `session_watch::process_alive`, now start-time-guarded). |
| **#3** (binary relay / `wait --loop`) | **Moot** — the exchange is always up. |
| **#26** (delivery-scan / `seen` invariant) | **Elevated and satisfied** as the `seen`-redesign prerequisite. |
| **#6** (versioned installs + launcher shim) | **Split**: minimal upgrade floor in `daemon-core` ([§16](#16-minimal-upgrade-floor)); full platform in `seamless-upgrade`. |

### Deferred (explicit — not dropped)

- **Full non-binary occupant status policy** (attended/idle/free) — the minimal
  stale-attendance signal (`last_confirmed`/`occupied_stale`/takeover) is in scope; the
  full state machine and any idle policy are deferred and **never drive teardown**.
- **fd-over-IPC pid-reuse-immune backstop** (#28-flavored) — awkward with a singleton
  daemon; the lighter pid+start-time guard is in scope; the fd path is deferred.
- **Daemon subsuming directory/occupancy reads** (`address list`) — V1 reads the backend
  lease table; the daemon does not own directory reads yet.
- **`per_session_cap` / multi-tier capability** — fields reserved now; tiers deferred
  (same-trust user-private threat model in v1).
- **#27** (`mark_delivered` cap) and **#24** (registry GC) — carry; still relevant.
- **#12** (embeddable SDK client) — separate solve; reuses this stabilized Layer-1 IPC.

### Reopen conditions (carried from the design-foundation council)

- The cutover **drain** ([§12](#12-legacy-cutover-two-phase-oq5-da-1)) cannot be realized via the
  address-keyed IPC probe + bounded stale-wait (i.e. it needs a *new* IPC verb) — would
  make a fix architectural rather than in-place.
- A Copilot plugin API appears that lets the plugin pre-populate the sessionEnd hook's
  env from a value captured at `attach` — then a **per-session cap** becomes the v1 path
  and [§7.1](#71-scoped-capability-model-v1-one-instance-admin-token) should re-tighten
  (not loosen).
- The `wait` auto-Re-register path ([§14.4](#144-wait-auto-re-register)) cannot be
  implemented because the chosen IPC transport masks socket-EOF — would force a
  positive-presence heartbeat from `wait`.
- The single-source SKILL mechanism ([§15.2](#152-single-source-skill--plugin-skill-mechanism-oq-for-deliverable-7-da-10))
  hits a harness constraint (manifest cannot point outside the plugin dir **and** `exec`
  is rejected) — would force a code-touching deviation.
