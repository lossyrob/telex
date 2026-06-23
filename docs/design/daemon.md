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

0. **Authenticate the server first.** On the connected endpoint handle, verify the server's
   process identity (client-side server-auth, [§7.2](#72-os-level-trust-boundary-mr5):
   `GetNamedPipeServerProcessId` + PID/start-time/SID/canonical-exe on Windows, connected-socket
   `SO_PEERCRED` + the same checks on Unix) **before sending `Hello` or any metadata**. A
   server that fails this check is rejected without disclosing `store_key`/`session_id`.
1. Try to connect to the singleton endpoint and complete the [Hello handshake](#6-ipc-protocol).
2. On success → use it.
3. On failure (no endpoint, or stale endpoint that fails Hello) → acquire the
   **spawn-lock**, then spawn the daemon and **retry connect-and-Hello** until `HelloAck`
   completes within the readiness window ([§2.3](#23-readiness-ack)) — this `HelloAck`
   **is** the readiness ACK; no out-of-band readiness signal exists. (The client-side
   server-auth of step 0 re-runs on each connect attempt, including against the
   just-spawned daemon.)

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
  flush in-flight EMIT→ACK→MARK, hand off owned epochs in order
  ([§11.4 owner-directed transfer](#114-ordered-handoff--owner-directed-atomic-transfer-sf3)), then exit.

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
session from inherited env (see [§14.4](#144-wait-and-session-scoped-re-register)), and (c) resume
blocking — returning exit `3` only if the grace window expires without a healthy
reconnect. This makes ordered handoff and crash-respawn invisible to the agent's turn
loop. **Scope of transparency (action-triggered, not universal):** this covers a session
that is *taking a telex action* across the restart — a blocked `wait`, or a `send`/`reply`/
`ack` that opportunistically re-registers ([§14.6](#146-resolvefrom-sendreply-recovery-and-presence-between-waits-mr6-mr7)). A fully **idle** session (no wait, no send) between a
drain and its next action is `suspect` until it next acts; that is acceptable (it is not
actively transacting) and is the precise qualification of the "occupied while handling"
claim in DESIGN.md.

### 3.4 Per-store isolation and schema-version (sf5)

One exchange serves multiple stores, so a fault in one store must not stall healthy ones,
and a multi-store, populated-Postgres deployment needs a schema contract:

- **Per-store loop isolation.** The `RECOVERING`/heartbeat/delivery loops are **per
  store**: a backend that is unreachable, slow, or in `RECOVERING` pauses **only its own**
  store's heartbeat/delivery; other stores keep serving. One bad backend never freezes the
  whole exchange. (`SPAWNING` still requires the *requested* store to be reachable for the
  triggering client; other stores attach lazily.)
- **Store schema-version — an *executable* barrier, not just a policy (M10).** Each store
  records a `telex_schema_version`. A daemon-aware binary, on open, gates closed a store
  whose schema is newer than it understands, and applies additive migrations for
  older-but-compatible schemas (`CREATE TABLE IF NOT EXISTS` + additive column adds,
  consistent with ADR 0013). The hard part is a **genuinely pre-epoch binary** that does
  not know to read `telex_schema_version` — a pure policy ("too-old fails closed") is not
  self-enforcing, since such a binary would open the store and run the legacy
  `claim_lease`/`heartbeat`/`release_lease` paths, writing non-epoch rows and corrupting
  the fence. The barrier is therefore made enforceable by an **external gate the old binary
  cannot bypass**:
  - **Mandatory (R3-S2): a store-level schema change that makes the legacy write paths
    hard-fail before they touch lease rows.** Because a launcher/shim is bypassable by
    invoking an old binary **directly**, and an additive-only migration (`CREATE TABLE IF NOT
    EXISTS` + additive columns) leaves the legacy `claim_lease`/`heartbeat`/`release_lease`
    paths usable, only a **store-level hard-fail** is non-bypassable. The migration therefore
    **renames/constrains** the legacy lease columns the pre-epoch paths write (or adds a
    `CHECK`/`NOT NULL` those writes violate) so a directly-invoked old binary **errors out**
    instead of silently writing a non-epoch row. This is **required** in v1, not optional.
  - **Additional defense: a launcher/store lock** (the versioned shim refuses to exec a
    binary older than the store's `telex_schema_version` before the binary ever opens the
    store). It hardens the common path but does **not** replace the mandatory store-level
    hard-fail, since direct invocation bypasses the shim.

  The migration that advances `telex_schema_version` is performed by the **first
  daemon-aware claimant** under a **per-store exclusive lock/transaction**, **before** any
  epoch-column writes, and is **crash-safe** (re-runnable; a partial migration is detected
  and completed/rolled back on the next open). A **downgrade/migration gate test on both
  backends** is acceptance, and it MUST exercise a pre-epoch binary invoked **directly**
  (not only via the shim) against a migrated store, asserting it cannot write a non-epoch
  row. The full migration/downgrade *framework* for a populated
  multi-store Postgres deployment stays `seamless-upgrade` scope; v1 freezes the version
  field, the executable barrier, and the migration's exclusivity/crash-safety.

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
  `session_seq` (the current life's seq), `occupant`, `attendance_last_confirmed_at`,
  `occupied_stale` (bool) + `watch_pids` (pid + role + **alive**) so a live-but-quiet station is
  distinguishable from an unhooked-dead one (R7-Sc), `backend`/`store_key`, `host`.
- **`backoff`** — current backend reconnect/backoff/crashloop state.
- **`recent_errors`** — a bounded ring of recent actionable errors (e.g. failed
  `sessionEnd`-hint vetoes, **`force`-takeover audit events** with prior occupant (R7-Sb),
  `NotOwner` self-demotions, `Conflict`/`NeedsEstablish`, backend disconnects), each with a
  timestamp.
- **`retention`** — per store, the **retired/tombstoned `leases` count and the `sessions`-row
  count**, each with a **warn flag** when it crosses the frozen v1 budget (R6-Sf/R7-Sd).
- **`stores`** — the set of stores this exchange currently serves.

## 5. Attendance model and record shape

The exchange maintains one **attendance record** per attended address. The **durable**
part lives in the backend lease row; the **in-memory** part is rebuilt on respawn (see
[§14.3](#143-crash-recovery-suspect--verified--lapsed-oq8-da-3)).

```text
AttendanceRecord {
  address:                      String,        // the telex number
  store_key:                    StoreKey,       // effective store identity (profiles::store_key); part of the authority key
  session_id:                   Option<String>, // opaque; the attending session
  session_incarnation:          Option<String>, // the session-life token (currency in `sessions`, see §14.1); denormalized here for the union
  occupant:                     String,         // human/host label of the occupant
  owner_instance_id:            Option<String>, // owning daemon instance, or NULL when released (epoch retained)
  lease_epoch:                  u64,            // monotonic, never-reused fence token (see §11)
  watch_pids:                   Vec<WatchPid>,  // liveness backstop (see §9)
  host:                         String,
  last_heartbeat:               i64,            // backend-clock ms; lease liveness proof (heartbeat-only, bound rows only)
  attendance_last_confirmed_at: Option<i64>,    // backend-clock ms; POSITIVE session-carrying presence only; NULL = never bound (pending-bind)
  state:                        Attendance,     // Suspect | Verified | Lapsed
  occupied_stale:               bool,           // DERIVED: owner set AND session_id set AND now - attendance > stale_after
}

WatchPid { pid: u32, start_time: u64, role: Anchor | Required }
Attendance = Suspect | Verified | Lapsed
```

`owner_instance_id IS NULL` marks a **released-but-epoch-retained** row (occupancy is
`owner_instance_id IS NOT NULL` and not stale, never row existence — see
[§11.2](#112-epoch-guarded-heartbeat-non-deleting-releaseownership-and-self-demotion-mr2-mr3)).

### 5.1 Durable lease-row columns (new)

The backend `leases` table — today keyed by `address` only with **no owner generation**
(verified: `src/registry.rs` `HolderRecord`, backend `claim_lease`/`heartbeat`/
`release_lease`) — gains:

- **`lease_epoch INTEGER`** — the monotonic, never-reused fence token (retained across
  release; see [§11.2](#112-epoch-guarded-heartbeat-non-deleting-releaseownership-and-self-demotion-mr2-mr3)).
- **`owner_instance_id TEXT`** — the owning daemon instance, `NULL` when released.
- **`last_heartbeat INTEGER`** — backend-clock ms lease-liveness proof (heartbeat-only).
- **`attendance_last_confirmed_at INTEGER`** — backend-clock ms of last positive
  session-carrying confirmation (never written by heartbeat).
- **`session_id TEXT`** — the attending session (so a respawned daemon can rebuild the
  `(store_key, session_id) → addresses` authority from durable rows as `suspect`).
- **`session_incarnation TEXT`** — the **session-life token** for `(store_key, session_id)`,
  denormalized from the `sessions` authority (below) onto each of the session's lease rows so
  the address-optional `ReRegister` union can filter by it. Its **currency** (is this token
  still the live one?) is decided by the `sessions` row, **not** by the lease rows — see
  [§14.1](#141-the-store_key-session_id--addresses-authority-and-the-sessions-table). (R3-6 /
  spar: replaces the former per-`(session_id, address)` `session_generation`, which falsely
  invalidated live waiters when a *sibling* address was registered.)
- **`tombstoned_at INTEGER`** — set (at the current incarnation) by the **station-removal**
  paths **`DeregisterSession`/`Detach`/`Takeover`/lapsed-TTL** — **not** by `ReleaseOwnership`
  (which is a non-removing ownership handoff, [§11.2](#112-epoch-guarded-heartbeat-non-deleting-releaseownership-and-self-demotion-mr2-mr3)).
  A non-NULL `tombstoned_at` on a row still at the session's current incarnation excludes that
  address from the `ReRegister` union (`Stale`). Durable so the guard survives a daemon crash.

The exchange also gains a durable **session authority** table, **`sessions`**, keyed by
`(store_key, session_id)` — the single source of truth for *incarnation currency* (R3-6 /
spar, [§14.1](#141-the-store_key-session_id--addresses-authority-and-the-sessions-table)).
It is **current-only** (R4-1): one row per `(store_key, session_id)` holding the **live**
incarnation; there is no superseded-history column, because a superseded token is simply
"`!= the current (session_seq, nonce)`" — the currency gate needs no separate retained-superseded fact:

```text
sessions {
  store_key:           String,
  session_id:          String,           // (store_key, session_id) PK
  session_seq:         i64,              // daemon-assigned, durable, monotonic per (store_key, session_id) (R6-2)
  nonce:               String,           // per-life uniqueness tie-breaker
  establish_nonce:     String,           // idempotency key of the establishing Register (R7-1)
  nonce_seq:           i64,              // the seq establish_nonce allocated; rotated on any non-Establish bump (R9-1)
  watch_pid_identity:  Option<WatchSet>, // the current life's published watch-pid set (canonical), R7-2
  updated_at:          i64,              // BackendClock ms (see §11.1)
}
// the live incarnation token = "<session_seq>.<nonce>"
// WatchSet = canonical-sorted list of { pid: u32, start_time: u64, role: Anchor|Required } (§9.1)
```

**`watch_pid_identity` contract (frozen, R7-2).** Because the `SessionEndHint`'s
liveness veto reads this column to decide death, its writer/format/empty-handling are
**frozen** so that "no liveness proof" can never become "proof of death":
- **Writer / when:** it is written **only** by `Establish` and by a verified `Continue`/`ReRegister`
  that carries the current token, **atomically in the same `sessions`-row-locked transaction**
  that sets/continues the seq — so the published identity always belongs to the current life.
- **Canonical format:** a deterministically **sorted** `WatchSet` of `(pid, start_time, role)`
  (the [§9.1](#91-typed-watch-pid-predicates-oq3) typed predicate inputs), so two writers of the
  same set produce byte-identical values and a probe is reproducible.
- **Absent / empty / stale ⇒ hint no-op:** if `watch_pid_identity` is **NULL/empty** (never
  published) or does **not** match the latched current life, the `SessionEndHint`
  **cannot prove death and is a no-op (veto)** — never a teardown. Liveness is proven only by a
  **positive** dead-result of the typed predicate over a **present** identity; absence is
  treated as "unknown," not "dead."

The **incarnation is `<session_seq>.<nonce>`**, where **`session_seq` is assigned by the
daemon** (not the loader's wall-clock — R6-2 replaces the round-5 `<mint_ms>.<nonce>`, which
mis-ordered equal-millisecond lives and had no backward-skew recovery). **`Establish` is a
prior-seq CAS** (`Register{mode: Establish{establish_nonce, expected_prior_seq}}`, R7-1/R8-1/R9):
under the `sessions`-row lock the daemon (1) returns the **same** seq if `establish_nonce`
matches the row **and** `nonce_seq == current_seq` (idempotent lost-`Registered` retry); else
(2) if `current_seq == expected_prior_seq` **and** the prior life is not attendance-fresh,
allocates `current_seq + 1` and records `(establish_nonce, nonce_seq = new seq)`; else (3)
rejects — `Conflict` (prior attendance-fresh) or **`Stale{current_seq}`** (seq moved on). It is
**never** an unconditional `current_seq + 1` bump. The client forms `expected_prior_seq` by the
**observe/retry contract** (first attempt `0` or last-known seq; on `Stale{current_seq}` retry
bounded with the same nonce and `expected_prior_seq = current_seq`; fresh nonce per new intent —
[§14.1](#141-the-store_key-session_id--addresses-authority-and-the-sessions-table), R9-2). The
client (loader + its env-propagated verbs) carries `(session_seq, nonce)`. A
`Register{mode: Continue}`/`ReRegister`/removal carries the token and is **conditional on
`== current`** (else `Stale`; missing/unknown token → `NeedsEstablish`, never a silent
establish), so a delayed old-life op (lower seq) is rejected and cannot
clobber a live newer life. A daemon crash/respawn of a *still-live* session does **not** change
the seq (crash-recovery keeps continuity); session-id reuse allocates a new seq via the CAS and
fails the old life closed. All four currency operations
(`Register`/`ReRegister`/`DeregisterSession`/`Detach`) **serialize on the `sessions` row**
(`SELECT … FOR UPDATE` / SQLite write transaction) before the currency check
([§14.1](#141-the-store_key-session_id--addresses-authority-and-the-sessions-table)). The
`leases` table is **indexed by `(store_key, session_id)`** — with a **partial index `… WHERE
tombstoned_at IS NULL`** (R5-Sd) so the live rebuild/union skips the accumulating retired
rows — and the `sessions` row is retained for the session-id's lifetime (no row deletion in v1
— [§11.2](#112-epoch-guarded-heartbeat-non-deleting-releaseownership-and-self-demotion-mr2-mr3)
no-delete invariant; bulk GC is issue #24). Both durable authorities use the **same
`store_key` physical-schema convention** (R4-Sc).

Greenfield: the new lease columns **and the `sessions` table** are created **together in one
schema-version migration** (R4-Sc) via `CREATE TABLE IF NOT EXISTS` / additive column add,
gated by the store schema-version under the per-store exclusive lock ([§3.4](#34-per-store-isolation-and-schema-version-sf5);
consistent with ADR 0013), so a partial migration cannot leave `sessions` and `leases`
inconsistent. A row whose `lease_epoch` column is `NULL` is a **legacy** row (see
[§12](#12-legacy-cutover-oq5-da-1)); `NULL` is never conflated with `0`.

The **occupant-null release** branch (`release_lease ... WHERE address=? AND (occupant=?
OR occupant IS NULL)`, verified in `sqlite.rs`/`postgres.rs`) is **removed**: release is
strictly epoch- and owner-guarded (see [§11.2](#112-epoch-guarded-heartbeat-non-deleting-releaseownership-and-self-demotion-mr2-mr3)).

## 6. IPC protocol

A **daemon-scoped**, versioned, length-or-line-framed control protocol. Serialization is
**JSON, one object per line** (`serde` / `serde_json`), extending the current
`src/ipc.rs` framing. This node freezes the **frame shapes, the handshake, and the
fail-closed capability policy**; the protocol **stabilizes for embeddable-SDK (#12) reuse
at `daemon-core`** (when the compatibility table below is filled and frozen) — it is not
claimed to be an already-frozen Layer-1 SDK surface in this design node.

Every request after the handshake carries the routing/identity fields it needs:
`store_key`, and where relevant `address` and `session_id`. Privileged requests
additionally carry an authorization `proof` (see [§7](#7-authorization-and-the-trust-boundary)).

### 6.1 Version handshake + capability negotiation (Hello / HelloAck) (sf2)

The **first** frame on every connection is a handshake, so an old daemon and a new
client (or vice versa) detect skew deterministically instead of mis-framing, and
**security-sensitive incompatibility fails closed** rather than silently degrading:

```text
→ Hello    { protocol_version, client_version, store_key, capabilities: [..],
             required_capabilities: [..] }
← HelloAck { protocol_version, daemon_version, auth_policy_version,
             accepted: bool, required_capabilities: [..], reason?: string }
```

- If `protocol_major` differs, the client and daemon belong to different singletons
  ([§2.1](#21-singleton-identity)) and the client connect-or-spawns the correct one.
- A compatible-**minor** skew is accepted **only if every `required_capabilities` entry on
  each side is satisfied by the other**; an unsatisfied required capability, an unknown
  **operation**, or an unknown **required** field is rejected with `Incompatible`
  (fail-closed), never silently treated as the weaker behavior. Unknown **optional** fields
  are ignored; unknown optional capabilities are simply not used.
- **Security-sensitive behavior is always required, never optional**: the `auth_policy_version`
  and the capabilities governing `admin_cap`/peer-credential/`per_session_cap` enforcement
  are `required_capabilities`, so an old or hostile client cannot negotiate them away (it
  gets `Incompatible`/`Unauthorized`, not a weaker path).

This **stabilizes** as a Layer-1 surface for the plugin and the #12 SDK **at `daemon-core`**
(it is **not** claimed as an already-frozen surface in this node — consistent with [§6](#6-ipc-protocol)
above). A normative **IPC compatibility table** — `protocol_version`, minimum daemon/client,
each capability's required-vs-optional status, unknown-field/unknown-op behavior, and the
downgrade error code — is **owned and frozen as part of `daemon-core` acceptance** (this node
freezes the frame *shapes* and the fail-closed capability *policy*; `daemon-core` fills and
freezes the version/min-version *table*), with **N/N-1 and N+1/N tests** for attach, wait
reconnect/ReRegister, Drain, DeregisterSession, and Status.

### 6.2 Request / response frames

Requests (Layer-1 operations). Privileged requests carry `proof = admin_cap`
([§7.1](#71-scoped-capability-model-v1-one-instance-admin-token)); all keys that identify
a station carry `store_key` because one exchange serves multiple stores:

| Request | Purpose | Privileged? |
|---|---|---|
| `Hello` | version + capability handshake | no |
| `Register { store_key, address, session_id, mode: Establish{establish_nonce, expected_prior_seq} \| Continue{session_incarnation}, occupant, description?, scope?, tags?, watch_pids[] }` | create/refresh a station (attach); **serialized on the `sessions` row, the sole currency-setter** (R5-1, R7-1, R8-1, R9) with a **positive `mode` discriminator** (never "absence of token"): **`Establish`** is the **prior-seq CAS frozen in [§14.1](#141-the-store_key-session_id--addresses-authority-and-the-sessions-table)** — (i) `establish_nonce == current row's establish_nonce` **AND `nonce_seq == current_seq`** → **idempotent**, return the current seq (lost-`Registered` retry; the **`nonce_seq == current_seq` clause is load-bearing — R9-1** — so a post-`force`-Takeover replay of a rotated/old nonce does **not** match and is not handed the new seq); (ii) else `current_seq == expected_prior_seq` AND the prior life is **not attendance-fresh** → allocate `current_seq + 1`; (iii) else → **`Conflict`** (prior life attendance-fresh) or **`Stale{current_seq}`** (a seq-mismatched/replayed establish — the typed payload carries `current_seq` for the observe/retry loop, R9-2). **`Continue{session_incarnation}`** (mid-life) is a no-op continuation iff current, else `Stale`; a lost token → **`NeedsEstablish`** (a live session can never self-supersede). The daemon assigns `(session_seq, nonce)` and returns it in `Registered` (R6-2). `establish_nonce` is **high-entropy, single-use** per establish *intent* (a *retry* reuses the same nonce; the bounded observe/retry path may re-attempt with an updated `expected_prior_seq` — [§14.1](#141-the-store_key-session_id--addresses-authority-and-the-sessions-table)) | no (same-trust) |
| `ReRegister { store_key, session_id, session_incarnation, address?, watch_pids[] }` | idempotent re-register (address optional = session-scoped; currency-gated on the carried `(session_seq, nonce)` then rebuilds the set from durable rows); **never assigns a seq**; a lost token → `NeedsEstablish`, never a silent establish | no |
| `SessionEndHint { store_key, session_id, admin_cap }` | **non-authoritative** sessionEnd-hook hint (carries **no incarnation** — the hook can't have one, R6-1); triggers a **latched, liveness-vetoed, double-checked** teardown of the exact proven-dead life only ([§14.2](#142-the-sessionend-hook-a-non-authoritative-liveness-hint-oq6-r6-1)) | **yes** (cap-authed, but not a removal authority) |
| `DeregisterSession { store_key, session_id, session_incarnation, proof }` | **explicit, seq-gated** session removal (a loader-spawned `detach`/operator that *holds* the current token): a non-current `(session_seq, nonce)` returns `Stale` no-op; tombstones its addresses | **yes** |
| `Detach { store_key, address, session_id, session_incarnation, proof }` | remove one station; **seq-gated** (R4-3); tombstones | **yes** |
| `Wait { store_key, address, attention?, timeout_ms }` | block for one delivery (sessionless; not session presence; **not deliverable against a pending-bind address** — [§11.3](#113-server-side-delivery-fence-mr1--at-least-once-preserving)) | no |
| `DeliveryAck { store_key, address, message_id, lease_epoch, delivery_nonce }` | the waiter's post-flush, correlated delivery ack ([§11.3](#113-server-side-delivery-fence-mr1--at-least-once-preserving)) | no |
| `Status { store_key?, detail?, proof? }` | Status surface (detail requires proof) | detail: **yes** |
| `Takeover { store_key, address, proof, force? }` | operator **fence + evict + tombstone** of a stale address (does **not** bind a new occupant — a follow-up `Register` does; [§10.2](#102-takeover-fence-then-register-da-5-r3-3)); **`force` = break-glass seq-bumping supersession** that bypasses the `occupied_stale` time proof (R6-3) | **yes** |
| `Drain { proof }` | quiesce + flush + ordered transfer/exit (upgrade/stop) | **yes** |

Responses:

| Response | Carries |
|---|---|
| `HelloAck` | protocol/daemon version, `auth_policy_version`, `required_capabilities`, accepted |
| `Registered` | `lease_epoch`, `owner_instance_id`, `session_incarnation` = the daemon-assigned `(session_seq, nonce)`, `state` (`suspect`/`verified`/`lapsed`) |
| `Message` | `id, thread_id, parent_id, from_addr, to_addr, kind, attention, requires_disposition, subject, body, sent_at_ms, buffered_at_ms, lease_epoch, delivery_nonce` |
| `Keepalive` | `heartbeat_age_ms` |
| `Timeout` | — (idle-timeout) |
| `StatusReport` | the [§4](#4-status-surface-the-frozen-contract-shape) fields |
| `TookOver` | `prior_occupant`, `last_confirmed`, `lease_epoch`, `forced: bool` — the typed `Takeover` response (R4-7) so the operator decides informedly (the prose in [§10.2](#102-takeover-fence-then-register-da-5-r3-3) refers to this row, not a generic `Ack`) |
| `Ack` | generic success for Register/ReRegister/Detach/Deregister/Drain/DeliveryAck |
| `Error` | `{ code, message, … }` — incl. `UnknownSession`, `NotOwner`, `Unauthorized`, `Incompatible`, `Ambiguous`, **`Stale{current_seq}`** (a non-current `(session_seq, nonce)` or a seq-mismatched `Establish` — carries the observed `current_seq` so the client can re-form `expected_prior_seq` and bounded-retry, R9-2; also = tombstoned address), **`NeedsEstablish`** (a `Continue`/`ReRegister` whose token is missing/unknown — never a silent establish, R7-1), **`Conflict`** (a second concurrently-live `Establish` for an attendance-fresh `session_id` — surfaced, not silently superseded; fast-restart retry per [§14.1](#141-the-store_key-session_id--addresses-authority-and-the-sessions-table), R7-Sa/R9-S1) |

The `Message` frame carries `lease_epoch` and a per-emit `delivery_nonce` (**R3-1**). The
`delivery_nonce` is **frozen in scope**: a fresh, unique value minted by the daemon for
*this* in-flight EMIT on *this* connection; the waiter MUST echo it verbatim in its
`DeliveryAck`; it is **invalidated** (no longer accepted) on the matching MARK, on
ACK-deadline timeout, or on connection close; and it is **not durable across a crash** (a
redelivery after restart mints a new nonce). It exists only to correlate one ACK to one
EMIT so a delayed/duplicate/wrong-connection ACK cannot commit a *different* in-flight
delivery ([§11.3](#113-server-side-delivery-fence-mr1--at-least-once-preserving)). **Ordering
note (M1):** the daemon EMITs the frame under a non-durable **in-memory** current-owner
precheck, and records the **durable** delivery mark **only after** the waiter's
`DeliveryAck` ([§11.3](#113-server-side-delivery-fence-mr1--at-least-once-preserving)). The
durable mark is therefore **after** emission, not before — emitting "only after a durable
fence authorizes it" would reintroduce the at-most-once loss window. ("Authorizes" here =
the in-memory precheck, explicitly an optimization, not the durable fence.)

## 7. Authorization and the trust boundary

### 7.0 v1 threat model (normative)

The v1 threat model is **same-user, user-private, single-user, pre-beta**:

- **Cross-user isolation is mandatory and enforced by the OS** ([§7.2](#72-os-level-trust-boundary-mr5)): a *different* OS user must not be able to connect to the endpoint, read the capability, `Wait` on an address (and read `Message` bodies), or claim a lease.
- **Intra-user isolation is explicitly NOT provided in v1** ([§7.3](#73-no-intra-user-isolation-in-v1-mr6)): every process of the *same* user is trusted. A same-user process may `Register`/`Wait`/`ReRegister` on any address, read its `Message` bodies, and refresh its presence. This is a deliberate, documented choice, not an omission; the reserved `per_session_cap` is the forward path to intra-user isolation.
- **Capacity/scale is single-user / pre-beta**: address counts, history sizes, and attendance sets are small. The multi-user / Streamliner performance and isolation concerns are explicit acceptance limits, not v1 requirements; revisit at beta / multi-user (see [§13](#13-delivery-and-the-seen-dedup-redesign-da-8) budgets).

Today `Wait`/`Shutdown` are **unauthenticated** (verified in `src/ipc.rs`); the model below replaces that.

### 7.1 Scoped capability model (v1: one instance-admin token)

- At spawn the daemon mints an **instance secret** (the `admin_cap`) and writes it to a
  **singleton-scoped, user-private file**: `<run_dir>/daemon-<H>.cap`, where
  `H = short_hash(user_SID, config_root, protocol_major)` — the **same singleton key** as
  the endpoint ([§2.1](#21-singleton-identity)). Scoping the cap path by `<H>` is required:
  a bare `<config_root>/daemon.cap` would be **shared by two protocol-major-parallel
  daemons** under one config root, and the last writer would invalidate the other
  instance's clients (its `DeregisterSession`/`Detach`/`Takeover`/`Drain` would start
  authenticating against the wrong instance). An **N / N+1 protocol-major acceptance test**
  asserts both daemons keep authenticating independently.
- **Single acquisition contract.** The cap is acquired **exactly one way**: read the
  current daemon instance's `daemon-<H>.cap`. The file is the sole source; "available in
  env" is not an alternate contract. The cap is rotated only when a new daemon instance
  mints a new file (a respawn/upgrade); clients re-read on `Unauthorized`.
- **Owner-only creation is part of readiness.** The cap file is created with
  owner-only permissions atomically (see [§7.2](#72-os-level-trust-boundary-mr5)); if it
  cannot be created owner-only, **startup fails** (the daemon does not serve without an
  enforceable cap).
- **Unprivileged** requests (`Hello`, `Register`, `ReRegister`, `Wait`) need no `admin_cap`
  (see [§7.3](#73-no-intra-user-isolation-in-v1-mr6)).
- **Privileged** requests (`DeregisterSession`, `Detach`, `Takeover`, `Drain`,
  `Status detail`) carry `proof = admin_cap`; the daemon verifies it equals its instance
  secret.
- The capability frame **reserves `scope`, `rotation`, and `per_session_cap: Option<Cap>`
  fields** (recorded now, unused in v1) for future intra-user / lateral-compromise defense
  — deferred with rationale: a per-session cap is zero-marginal-value under v1 same-trust,
  and is **not obtainable today** because the minting (`Register` child) process and the
  later `sessionEnd` hook process are different processes ([§14.2](#142-the-sessionend-hook-a-non-authoritative-liveness-hint-oq6-r6-1)).

### 7.2 OS-level trust boundary (mr5)

The "user-private" property is enforced by the OS, made **normative** here (a predictable
endpoint name + a `0600` file alone do not stop another local user from connecting, and
data-bearing ops like `Wait → Message` body are otherwise readable):

- **Endpoint owner-only AND single-instance (exclusivity primitive).** Windows: the named
  pipe is created with a **DACL granting the current user SID only** (no `Everyone`/
  `Authenticated Users`) **and** with **`FILE_FLAG_FIRST_PIPE_INSTANCE`** (or an equivalent
  owner-only named mutex) — because a bare named pipe is **not** an exclusivity primitive
  (multiple servers can co-bind), so "bind the endpoint as the spawn-lock"
  ([§2.2](#22-auto-spawn-connect-or-spawn-and-the-spawn-lock)) requires the first-instance
  flag to actually be the singleton lock and to refuse a hostile/second co-binder. Unix:
  the socket lives under an **owner-only `0700` run directory**, created via atomic
  bind-or-fail; a **stale socket** is unlinked-and-rebound only after confirming no live
  owner (the lockfile + `daemon-<H>.cap` ownership check), never blindly.
- **Canonical, owner-private paths.** `config_root` and `run_dir` are **canonicalized**
  (symlinks resolved) and **rejected at startup if not owner-private** (not owner-owned, or
  group/world-accessible).
- **Cap/lock file safety, as a readiness precondition (S3).** `daemon-<H>.cap` and the
  spawn-lock/lockfile are created with **`O_NOFOLLOW` + exclusive create + atomic
  write-then-rename** and owner-only mode, so a pre-planted symlink or hostile pre-existing
  file cannot redirect or capture them. **Owner-only cap creation is part of the readiness
  contract** ([§2.3](#23-readiness-ack)): if the cap cannot be created owner-only — `ENOSPC`,
  permission, partial write, symlink — **startup fails** (the daemon never serves without an
  enforceable cap). These failpoints are acceptance tests.
- **Peer authenticity — server AND client, both MUST, with the *correct directional
  primitive* (R3-7).** Before the server sends `admin_cap` or any data-bearing frame
  (`Message`) it MUST verify the connected peer is the same user; and **the client MUST
  verify the server's process identity BEFORE sending `Hello` (which carries `store_key`),
  before trusting `HelloAck`, and before sending `admin_cap` or any data-bearing frame**,
  failing closed otherwise — a bearer `admin_cap` is exfiltrated the instant it is sent to a
  hostile pre-bound server, and even `store_key`/`session_id` are metadata not to disclose to
  an unauthenticated server, so server authenticity cannot be a "should" and cannot wait for
  `HelloAck`. The two directions use **different OS primitives** (a common design error is to
  cite the server-side primitive for the client check):
  - **Server verifies client** (server-side): `ImpersonateNamedPipeClient` (Windows) /
    `SO_PEERCRED` on the accepted socket (Unix) — the server inspecting who connected.
  - **Client verifies server** (client-side): on the *connected* handle, **`GetNamedPipeServerProcessId`**
    (Windows) to obtain the server PID, then verify **server PID + process-start-time, the
    server token's owner SID, and the server binary's canonical-exe path/hash**; on Unix,
    `getsockopt(SO_PEERCRED)` on the *connected* socket to read the server's uid + pid, then
    the same start-time + canonical-exe verification. `ImpersonateNamedPipeClient` is a
    **server-side** call and does **not** let a client authenticate a server.

  Both credential checks MUST be **reuse-safe** (PID + process-start-time, the same reuse
  defense as watch-pids, [§9.1](#91-typed-watch-pid-predicates-oq3)) to close the TOCTOU where
  a PID is reused before its token is read. Connect-or-spawn ([§2.2](#22-auto-spawn-connect-or-spawn-and-the-spawn-lock))
  therefore runs the **client-side server-auth as step 0, before the Hello handshake**, and
  never trusts an arbitrary first endpoint binder; the daemon **spawns only the canonical
  executable** (a verified absolute path), never a relative/`PATH`-resolved name.
- **Capability redaction (S11).** `admin_cap`/`proof` are **bearer secrets** and MUST
  **never** appear in `Status.recent_errors`, `Error.message`, logs, or traces (redact to a
  fixed placeholder). Acceptance asserts no cap material in any diagnostic surface.
- **Negative tests** (acceptance): a second OS principal cannot `Hello`/`Register`/`Wait`;
  a symlinked cap/lock is rejected; a **hostile pre-bound server is rejected client-side
  BEFORE any metadata disclosure** (before the client sends `Hello`/`store_key`, not merely
  before `admin_cap`); a **second server instance is refused** by the exclusivity
  primitive; a **PID-reuse race** does not authenticate the wrong process.

### 7.3 No intra-user isolation in v1 (mr6)

`Wait`/`Register`/`ReRegister`/`ResolveFrom` carry no per-session proof, so **any same-user
process** can wait on an address (and read its `Message` bodies), register, or trigger a
re-register. Under [§7.0](#70-v1-threat-model-normative) this is **accepted** (same-user
trust). The consequences are made explicit so they are deliberate:

- A same-user `Wait` is **not** treated as session presence: it does **not** refresh
  `attendance_last_confirmed_at` (only session-carrying `Register`/verified `ReRegister`
  do — [§10.1](#101-last_confirmed-occupied_stale-and-the-hook-semantics-split-oq2-da-6)),
  so an unrelated waiter cannot keep a dead station fresh and block takeover.
- The session authority and its race-safety (tombstones, `store_key` keying) are specified
  in [§14](#14-daemon-native-session-ownership) and hold regardless of the trust model
  (they are correctness, not isolation).
- Intra-user isolation, when needed, is the reserved `per_session_cap` path (mint a
  per-`(store_key, session_id)` cap at `Register`, require it on `Wait`/`ReRegister`/
  `DeregisterSession`) — deferred.

This is the [OQ6 resolution](#open-question-resolutions): proof without an external
session→address registry — the hook presents the singleton-scoped instance secret in a
**non-authoritative** `SessionEndHint(store_key, session_id, admin_cap)` (no incarnation — the
separately-spawned hook cannot have one, R6-1); the daemon checks the secret and that
`(store_key, session_id)` is in its map, then performs a **latched, liveness-vetoed,
double-checked teardown** of the exact proven-dead life
([§14.2](#142-the-sessionend-hook-a-non-authoritative-liveness-hint-oq6-r6-1)).

## 8. (reserved)

*(Section intentionally folded into §5 and §14; numbering preserved for cross-refs.)*

## 9. Liveness model

Two paths, exactly as ratified (ADR-to-be 0017):

1. **Healthy disconnect = the sessionEnd hook (a non-authoritative hint).** Quit and dismiss
   both fire `session.ended`; the harness plugin sends a non-authoritative
   `SessionEndHint(store_key, session_id, admin_cap)` (no incarnation — R6-1). The exchange does
   not remove on the hint's say-so; it runs a **latched, liveness-vetoed, double-checked**
   teardown ([§14.2](#142-the-sessionend-hook-a-non-authoritative-liveness-hint-oq6-r6-1)) — an
   accelerator over the pid-watch/stale-attendance backstop for the case where the dismissed
   life's pids are actually dead. This is the normal (fast) path; it never removes a live life.
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
| 1 | **sessionEnd hook** (clean quit/dismiss) | non-authoritative `SessionEndHint(store_key, session_id, admin_cap)` → **latched, double-checked, liveness-vetoed** teardown of the exact proven-dead life (R6-1, [§14.2](#142-the-sessionend-hook-a-non-authoritative-liveness-hint-oq6-r6-1)) | immediate **iff** the latched current life's watch-pids are dead; else no-op (falls to row 2/3/4) |
| 2 | **watch-pid failure** — the typed predicate resolves dead per [§9.1](#91-typed-watch-pid-predicates-oq3) (no `anchor` pid survives, or any `required` pid is gone, or a start-time mismatch) | the daemon's local watcher issues an **internal `DeregisterSession`** for that session, **bypassing `occupied_stale`** | immediate |
| 3 | **operator takeover** | privileged `Takeover` (see [§10.2](#102-takeover-fence-then-register-da-5-r3-3)) | fence + evict + tombstone; rebind via follow-up `Register` |
| 4 | **daemon-down TTL** | lease lapses after the daemon-down window; respawn re-claims | backstop only |

`occupied_stale` is reserved for the **unobserved-death case only**: no hook fired *and*
no watch-pid signal is available (e.g. unhooked dismiss where the loader anchor survives).
That is the residual the next section governs.

## 10. Stale-attendance and takeover (no teardown)

### 10.1 `last_confirmed`, `occupied_stale`, and the hook-semantics split (OQ2, DA-6)

`attendance_last_confirmed_at` is refreshed by **positive, session-carrying presence
signals only**, and — critically (R6-1 / spar) — that confirmation must be **seq-fenced and
prove *logical* session presence, not mere process existence**: it is refreshed **only by a
`Register` / verified `ReRegister` carrying the *current* `(session_seq, nonce)`** (an active
telex action by the live agent), and any future positive resume/connect hook
(see [§16 OQ-γ](#open-question-resolutions)). It is **NOT** refreshed by the daemon's
own heartbeat (which updates `last_heartbeat` only —
[§11.2](#112-epoch-guarded-heartbeat-non-deleting-releaseownership-and-self-demotion-mr2-mr3)),
**NOT** by a bare `Wait` connect (sessionless and proofless —
[§7.3](#73-no-intra-user-isolation-in-v1-mr6)), and **NOT by a merely-surviving process**: a
loader/anchor pid that lingers after the agent was dismissed runs **no current-seq telex
verb**, so it cannot refresh attendance — and an **old-seq** action from a superseded life is
`Stale` and likewise cannot. This is what makes the **stale-attendance backstop reliable for
the unhooked-dead case** (the residual the demoted hint defers to, [§14.2](#142-the-sessionend-hook-a-non-authoritative-liveness-hint-oq6-r6-1)): a dismissed agent stops issuing current-seq actions, so its
`attendance` necessarily goes stale and `occupied_stale` fires — a lingering process cannot
hold the lease fresh indefinitely. (Without seq-fencing, a surviving old process could keep the
session looking present forever and the lease would leak — the precise hole the spar surfaced.)
Either omission would otherwise let a continuously-heartbeating daemon, an unrelated same-user
waiter, or a zombie process keep a dead-but-unhooked session permanently fresh — defeating
`occupied_stale`/takeover and reintroducing the zombie lease. **`SessionEndHint`
does NOT refresh** either — it is a *liveness hint*, never a presence confirmation.

`occupied_stale` is **derived**, not stored:
`owner_instance_id IS NOT NULL AND session_id IS NOT NULL AND attendance_last_confirmed_at IS
NOT NULL AND now - attendance_last_confirmed_at > stale_after`, where `stale_after` is
configurable (default a small multiple of the heartbeat/lease window; the exact default is
frozen in `daemon-core`, with a **frozen minimum `stale_after` floor (R7-Sc)** below which a
station is never declared stale — a guard against a misconfigured tiny window tearing down a
merely-quiet-but-live session). **Semantics (R7-Sc):** `occupied_stale` means precisely **"no
current-seq session-carrying action has confirmed presence in the last `stale_after`"** — it is
**not** an assertion of *death*; a live-but-quiet session and an unhooked-dead session share the
same signature, which is exactly why it **never triggers teardown** by itself and only **offers
takeover**. So the operator (and `TookOver`/`Status`) can distinguish the two, `Status` surfaces
**both `occupied_stale` and whether the watch-pids are still alive** (`watch_pids_alive`), and a
`TookOver` response reports the prior occupant + `last_confirmed`. The `session_id`/`attendance
NOT NULL` guards matter: a **pending-bind**
row (post-takeover, owner set but `session_id`/`attendance` NULL — [§10.2](#102-takeover-fence-then-register-da-5-r3-3))
is **not** `occupied_stale` (`now - NULL` is never `> stale_after`); it is reclaimed instead by
the **un-heartbeated-aging** path ([§11.2](#112-epoch-guarded-heartbeat-non-deleting-releaseownership-and-self-demotion-mr2-mr3)),
so the two recovery paths partition cleanly and neither wedges. Both `now` and
`attendance_last_confirmed_at` are read from the **single backend/database-server clock
domain** ([§11.5](#115-postgres-cross-machine-reclaim-in-epochs-not-timing)) — never one
machine's local time compared against another's — and the clock source is injectable so
skewed-clock and suspend/resume cases are testable. It is surfaced in
Status and `address list`. It **never triggers teardown** — an idle-but-alive session
stays `occupied` and instantly wakeable (the operator's explicit requirement).

### 10.2 Takeover (fence-then-register) (DA-5, R3-3)

Because the exchange is a singleton, the common takeover case is **intra-daemon** (the
stale station and the new claimant are served by the same daemon process). Backend epoch
fencing alone would leave **stale in-memory IPC waiters and `session_id → addresses`
mappings** inside that process. Takeover is therefore **atomic at the exchange** — in one
critical section it:

1. mints a new backend **`lease_epoch`** (fencing the prior owner at the backend),
2. **evicts** the prior `session_id → addresses` entry for the rotated address,
3. **closes** the IPC waiters bound under the prior occupant (their `wait` reads return a
   defined disconnect, not a silent hang),
4. **tombstones** the prior `(store_key, session_id, address)` binding.

Takeover does **not** bind a new occupant (R3-3 / spar): the `Takeover { store_key, address,
proof }` RPC is an **operator/recovery** action carrying no session identity, so it cannot
install one. After the CAS the row is `owner_instance_id = :me` (the daemon), a fresh
`lease_epoch`, `session_id = NULL`, `attendance_last_confirmed_at = NULL` — a **pending-bind**
reservation, which is **non-deliverable**: the [§11.3](#113-server-side-delivery-fence-mr1--at-least-once-preserving)
delivery-selection precondition requires a verified bound session, so no message is EMITted or
MARKed against a pending-bind address and a `Wait` racing ahead of `Register` cannot consume
one (R4-5). A **subsequent `Register`** (by whoever wants the address) binds a new occupant
against the fresh epoch via the pinned-epoch+owner CAS ([§11.1](#111-epoch-lifecycle-oq1)); if
the pending-bind row is **simultaneously** being reclaimed by stale-heartbeat aging, the two
serialize on that pinned CAS — exactly one wins and the loser re-reads and retries (R4-Sa), so
there is no torn bind. There is no window where the address is both old-owned and new-owned
(the prior owner is fenced atomically); the intermediate **pending-bind** state is
owned-but-session-less and **bounded** (below), not a silent hang. The typed **`TookOver`
response** (`prior_occupant`, `last_confirmed`, `lease_epoch` — [§6.2](#62-request--response-frames),
R4-7) reports the prior occupant so the operator decides informedly. There is **no idle
teardown** — takeover is explicit, the recovery path
for the weak-loader-liveness residual.

**Normative takeover backend CAS (M6, R3-3).** The load-bearing takeover case is *stale
attendance with a still-fresh heartbeat* (an unhooked-dead session whose daemon still
heartbeats the lease), so the normal stale-claim predicate
(`owner_instance_id IS NULL OR last_heartbeat < :stale_cutoff`,
[§11.1](#111-epoch-lifecycle-oq1)) **does not fire**. Takeover therefore has its own
privileged CAS, gated on the **`occupied_stale` attendance predicate** and on the row being
**occupied** (`owner_instance_id IS NOT NULL`, so it partitions cleanly from the normal
ownerless-claim path), pinning the current epoch+owner and incrementing in-SQL:

```sql
UPDATE leases
   SET owner_instance_id = :me,
       lease_epoch = lease_epoch + 1,
       last_heartbeat = :backend_now,
       session_id = NULL,                      -- pending-bind: no session until a follow-up Register
       attendance_last_confirmed_at = NULL
 WHERE address = :addr
   AND lease_epoch = :observed_epoch
   AND owner_instance_id IS NOT DISTINCT FROM :observed_owner
   AND owner_instance_id IS NOT NULL                                    -- occupied (not the ownerless-claim branch)
   AND session_id IS NOT NULL                                           -- a bound session to take over
   AND (:backend_now - attendance_last_confirmed_at) > :stale_after     -- occupied_stale, NOT stale heartbeat
   -- → rows: 0|1
```

It additionally **tombstones** the prior `(store_key, session_id, address)`
([§14.1](#141-the-store_key-session_id--addresses-authority-and-the-sessions-table)) in the
same transaction. Like every claim it writes ownership/fence columns only (not `occupant`/
`attendance_last_confirmed_at` — the new owner has no verified session until a `Register`).

**Bounded pending-bind (R3-3 / spar — closing the post-takeover wedge).** A pending-bind row
(`owner set, session_id = NULL`) is **not heartbeated** — the daemon heartbeats **only bound
rows** (`session_id IS NOT NULL`, [§11.2](#112-epoch-guarded-heartbeat-non-deleting-releaseownership-and-self-demotion-mr2-mr3)).
So if the follow-up `Register` never lands (it hangs/crashes), `last_heartbeat` simply ages
and the **normal claim CAS reclaims the row** once `last_heartbeat < :stale_cutoff` — the
reservation is bounded by `stale_cutoff` with **no extra column**. (Without this, a row left
`owner set, attendance = NULL` would be neither `occupied_stale` — `now - NULL` is SQL-NULL,
never `> stale_after` — nor normally claimable while the heartbeat looked fresh, i.e. a
permanent wedge.) This CAS is cross-referenced from [§11.1](#111-epoch-lifecycle-oq1) and
exercised by [§17](#17-gating-tests--per-backend-conformance-matrix-daemon-core-acceptance)
tests 5 and 8.

**Force-Takeover — break-glass seq-bumping supersession (R6-3 / spar).** The normal takeover
CAS above needs the `occupied_stale` *elapsed-time* proof. But the daemon-down-TTL recovery
path ([§14.5](#145-daemon-down-and-the-ttl-backstop)) can reach a state where that time proof
is **unavailable** (an untrustworthy/slept/backward-stepped respawn wall clock), which would
make the normal CAS unable to fire — a self-contradiction that would strand the address. So
`Takeover { …, force: true }` is a **privileged break-glass** action that, **under the
`sessions`-row + address locks**, **atomically bumps `session_seq` (to `current_seq + 1`,
invalidating *all* old-seq client ops and any in-flight hint), rotates the `sessions` row's
`establish_nonce` to a fresh daemon-minted value in a **reserved daemon-only sentinel
namespace** (a frozen prefix that **clients MUST NOT use and the daemon rejects on any client
`Establish`**, R10-S3 — so "no client holds the rotated nonce" is a *hard* guarantee, not merely
high-entropy probability), advancing `nonce_seq` to the new seq (**R9-1**, so a post-force replay
of the *old* `establish_nonce` cannot hit the case-(1) idempotency match and be handed the new
seq; it is `Stale`), and mints a new `lease_epoch`**,
**bypassing the `occupied_stale` time predicate** — the operator's explicit action *replaces*
the unprovable time proof. It is **explicitly defined as operator-authorized supersession, not
proof of death**: it **can** seize a still-live session (a false positive), which is the
accepted cost of break-glass. It is **not** a separate liveness truth — after it runs, the old
life's `Register`/`ReRegister`/removals are `Stale` exactly like any normal supersession, and a
**concurrently-establishing** new life simply serializes on the `sessions` row (whoever commits
first sets the seq; the later op evaluates against the new state). The non-`force` predicate
remains the only *unprivileged*/automatic path — ordinary `Register` never inherits the
time-proof bypass. **Force-Takeover inherits the *full* §10.2 takeover sequence (R7-Sb):** it
performs the same **evict prior `session_id → addresses` map entry, close the prior occupant's
IPC waiters (defined disconnect), tombstone the prior `(store_key, session_id, address)`, and
leave the address in the bounded non-deliverable `pending-bind` state** — the *only* difference
from the normal CAS is the bypassed time predicate. It is **audited**: each `force` takeover
emits an **operator-audit `recent_error`/event** and is surfaced in `Status` (with the prior
occupant + `forced: true` in the [`TookOver`](#62-request--response-frames) response), so a
break-glass seizure is never silent. A short **operator runbook** (when `force` is appropriate —
only after a daemon-down/clock-untrustworthy recovery cannot prove staleness — and how to
confirm the seized session is truly gone) is part of the `daemon-core` operator docs.

## 11. Lease-epoch fence (the spine)

The lease row is keyed by `address` with **no owner generation today**, so on
stall/crash/handoff/reclaim an old daemon can write a row it no longer owns (duplicate
delivery, ownership flip-flop). The fence is a **monotonic `lease_epoch` +
`owner_instance_id`**. This is the single-writer guarantee and the spine of daemon-down
recovery, upgrade handoff, and Postgres reclaim. The mechanism below is the same on
SQLite and Postgres; the per-backend conformance matrix is [§17](#17-gating-tests--per-backend-conformance-matrix-daemon-core-acceptance).

### 11.1 Epoch lifecycle (OQ1)

- **The epoch is a durable, monotonic, never-reused per-address high-water mark.** It
  only ever increases for an address — across claim, release, re-claim, handoff, crash,
  and reclaim. The waiter epoch-filter and "higher epoch wins" reclaim both depend on
  this, so it is a normative invariant, not a convention.
- **Claim is a compare-and-set that pins the observed epoch AND owner and increments the
  epoch in the backend** (not in the client), so two concurrent claimants cannot both
  win or skip a value. A claim sets only the **ownership/fence** columns
  (`owner_instance_id`, `lease_epoch`, `last_heartbeat`); it does **not** write `occupant`
  or `attendance_last_confirmed_at` — those are **session** fields written only by
  `Register`/verified `ReRegister` ([§10.1](#101-last_confirmed-occupied_stale-and-the-hook-semantics-split-oq2-da-6)), so a bare recovery/stale-claim/takeover never forges session
  presence. `:backend_now` is the **`BackendClock`** — a frozen contract with one
  backend-specific implementation each (**R3-Sb**), never a client-supplied local timestamp:
  on **Postgres** it is the true server clock (`now()`/`CURRENT_TIMESTAMP`, evaluated
  server-side so every writer across processes/machines shares one domain); on **SQLite**
  there is no server, so the single writer **is** the one daemon process — but `BackendClock`
  **MUST be durable across a daemon restart (R4-6)**, because the timestamps it stamps
  (`last_heartbeat`, `tombstoned_at`, `sessions.updated_at`) are **persisted** and then compared
  against a *later* daemon's "now" across exactly the restart that the daemon-down TTL
  ([§14.5](#145-daemon-down-and-the-ttl-backstop)) and retention span. A bare process-monotonic
  clock **rebases on restart** and makes those comparisons meaningless (TTL/stale-cutoff could
  fail open → resurrection, or fail closed). The SQLite `BackendClock` is therefore a **durable,
  persisted, monotonic high-water clock**: a `clock_hwm_ms` is kept in the store, and each read
  returns `max(wall_now_ms, clock_hwm_ms + 1)` and persists the new high-water in the same
  transaction — so it never moves backward (across restart, suspend/resume, or wall-clock skew)
  while still tracking real time, and a respawned daemon resumes from the persisted high-water.
  (It remains injectable for tests.) Both implementations satisfy the same invariant — the
  **persisted** `last_heartbeat`, `attendance_last_confirmed_at`, and `stale_cutoff` are
  read from **one** durable clock domain ([§11.5](#115-postgres-cross-machine-reclaim-in-epochs-not-timing)).
  (The **ACK deadline is *not* on this clock** — it is a short in-process monotonic elapsed
  timer that never persists or crosses a restart; see [§11.3](#113-server-side-delivery-fence-mr1--at-least-once-preserving),
  R5-Sa.) The normative claim statement, identical on both backends:

  ```sql
  UPDATE leases
     SET owner_instance_id = :me,
         lease_epoch = lease_epoch + 1,
         last_heartbeat = :backend_now
   WHERE address = :addr
     AND lease_epoch = :observed_epoch
     AND owner_instance_id IS NOT DISTINCT FROM :observed_owner
     AND (owner_instance_id IS NULL OR last_heartbeat < :stale_cutoff)   -- → rows: 0|1
  ```

  `0` rows = lost the race (re-read and retry, or report held-elsewhere). The increment
  is `lease_epoch + 1` evaluated by the backend, never a client-computed
  `:observed_epoch + 1`. (`Register` additionally sets `occupant` and
  `attendance_last_confirmed_at` in the same transaction as the claim.)
- **First-ever absent row** (the address has no `leases` row yet) is created by an
  **`INSERT INTO leases (address, lease_epoch, owner_instance_id, last_heartbeat) VALUES
  (:addr, 1, :me, :backend_now) ON CONFLICT(address) DO NOTHING`** — the insert both
  **creates the epoch at `1` AND claims ownership** for the inserter (fence and ownership
  columns set together, never a transient ownerless row at epoch 1). If the insert succeeds
  (1 row) the claimant **owns the address at `lease_epoch = 1`** (then `Register` adds
  `occupant`/`attendance` in the same transaction); if it conflicts (0 rows — a row appeared
  concurrently), the claimant falls through to the UPDATE CAS above (which will pin the
  now-observed epoch+owner). **Legacy rows** (a row whose `lease_epoch` column is `NULL`,
  the epoch column) take a **separate, explicit** path
  (`... WHERE address = :addr AND lease_epoch IS NULL`) that sets `lease_epoch = 1`.
  **`NULL` is never conflated with `0`** in the normal claim predicate (see
  [§12](#12-legacy-cutover-oq5-da-1)).
- The winner's `owner_instance_id` is its stable instance identity for the daemon's life;
  `occupant` is a **human/host label of the attending session** (informational), distinct
  from `owner_instance_id` (the daemon fencing identity) and never overwritten by a claim.

### 11.2 Epoch-guarded heartbeat, non-deleting ReleaseOwnership, and self-demotion (mr2, mr3)

**Heartbeat** is epoch+owner-guarded, returns a rowcount, updates **lease-liveness proof
only** (not `attendance_last_confirmed_at`, which is session-presence, refreshed only by
Register / verified Re-register — see [§10.1](#101-last_confirmed-occupied_stale-and-the-hook-semantics-split-oq2-da-6)),
and is **scoped to bound rows** (`session_id IS NOT NULL`). Conflating heartbeat with
attendance would let the daemon's own continuous heartbeat keep a dead-but-unhooked session
permanently fresh; heartbeating an **un**bound row (a post-takeover pending-bind reservation,
[§10.2](#102-takeover-fence-then-register-da-5-r3-3)) would keep that reservation permanently
fresh and wedge the address — so a pending-bind row is deliberately **not** heartbeated and
ages into reclaimability:

```sql
heartbeat: UPDATE leases SET last_heartbeat = :backend_now
            WHERE address=? AND lease_epoch=? AND owner_instance_id=?
              AND session_id IS NOT NULL                            -- bound rows only (R3-3)
            -- → rows: 0|1
```

**`ReleaseOwnership` does NOT delete the row, and does NOT tombstone (R3-2 / spar).**
Deleting would discard the only durable carrier of `lease_epoch` (a later claim would reset
the epoch 7 → 1, breaking monotonicity). `ReleaseOwnership` is the **daemon-stop / crash
handoff** path: it clears the **fencing identity only** (`owner_instance_id`), and
**preserves the session binding** (`session_id`, `session_incarnation`,
`attendance_last_confirmed_at`) with **`tombstoned_at` left NULL**, so the same session's
next-call `ReRegister` re-proves presence and re-claims under the **same incarnation**
(§16 upgrade continuity):

```sql
ReleaseOwnership: UPDATE leases SET owner_instance_id = NULL
                   WHERE address=? AND lease_epoch=? AND owner_instance_id=?   -- → rows: 0|1
```

This is a **reserved-continuity** state, not removal: the ownerless row is **immediately
re-claimable at the backend** (`owner_instance_id IS NULL`), but *who* may re-bind it is
decided by the **incarnation currency** in `sessions`
([§14.1](#141-the-store_key-session_id--addresses-authority-and-the-sessions-table)) — the
same session's current-incarnation `ReRegister` reclaims continuity, while any **other**
session that claims the address re-binds it under its own `session_id`/incarnation (which
removes the prior session from *that* address; the prior session's `ReRegister` then simply
does not union an address whose `session_id` is no longer its own). **Station-removal**
(`DeregisterSession`/`Detach`/`Takeover`/lapsed-TTL) is the **distinct** path that sets
`tombstoned_at` (at the current incarnation) — `ReleaseOwnership` is never a station-removal.

Occupancy is derived from `owner_instance_id IS NOT NULL` (and not stale), **never from row
existence**. **Normative no-delete invariant:** no code path — `ReleaseOwnership`, detach,
cleanup, test helper, or migration — may `DELETE` a lease row whose `lease_epoch` matters; all
of them null the owner and preserve the high-water epoch. **There is no v1 GC of lease rows at
all** (R4-4/R5-3): tombstoned rows are retained for the store's life (any future reclamation is
out-of-scope issue #24, below). (If true
row reclamation is ever needed, the high-water moves to a separate append-only
`address_epoch(address, epoch)` table; out of scope for v1, where unbounded retired-row
growth is acceptable at single-user scale — GC is issue #24.)

A **0-row heartbeat or `ReleaseOwnership`** means a higher epoch exists. The daemon
**self-demotes** for that address — stop emitting AND stop heartbeating (relinquish the
address), close its waiters, and drop the in-memory station. It must not keep heartbeating
(which would hold the lease fresh and starve a successor). (Today's `heartbeat` returns
`Result<()>` with no rowcount — verified `sqlite.rs:325-333` / `postgres.rs:313-320` — so the
rowcount-returning shape is a required backend-API change.)

### 11.3 Server-side delivery fence (mr1 — at-least-once preserving)

**The fence must preserve the ratified at-least-once contract (ADR 0011) — it must never
introduce message loss.** A delivery is durably recorded only **after** a waiter has
accepted it, never before:

**Delivery-selection precondition (R4-5 — pending-bind is non-deliverable).** Before the fence
runs, an address is **eligible for delivery only if it is bound to a verified session**:
`owner_instance_id IS NOT NULL AND session_id IS NOT NULL AND attendance_last_confirmed_at IS
NOT NULL AND state = Verified`. A **pending-bind** row (post-`Takeover`, owner set but
`session_id`/`attendance` NULL — [§10.2](#102-takeover-fence-then-register-da-5-r3-3)) and a
`suspect`/`lapsed` row ([§14.3](#143-crash-recovery-suspect--verified--lapsed-oq8-da-3)) are
**not** delivery-eligible, so the daemon never EMITs/MARKs for an address bound to no session —
a sessionless `Wait` against a pending-bind address **blocks (or returns a defined
`not-bound`)** until a follow-up `Register` binds and verifies it. This keeps the
fence-then-register lifecycle from being bypassed by a `Wait` racing ahead of `Register`.

```text
mark_delivered_if_current_owner(address, owner_instance_id, lease_epoch, message_id)
    -> Result<DeliveryOutcome>

DeliveryOutcome = Marked | AlreadyDelivered | NotOwner
```

The daemon, in a **per-address critical section**, for each undelivered message:

1. *(optimization only — not the fence)* if in-memory state already knows it is not the
   current owner, skip and self-demote.
2. **EMIT** `Frame::Message(M, lease_epoch, delivery_nonce)` to the waiter, carrying a
   per-emit `delivery_nonce` that identifies *this* in-flight delivery.
3. **AWAIT THE CORRELATED WAITER ACK, under a bounded deadline.** The one-shot
   `telex wait` client performs a **complete, all-or-error write+flush** of `M` to its
   stdout (a partial write, flush error, or truncated output is **not** an ACK — it closes
   the connection instead), then sends `DeliveryAck { store_key, address, message_id,
   lease_epoch, delivery_nonce }` **on the same connection**. The daemon accepts the ACK
   **only if it matches the in-flight `(connection, store_key, address, message_id,
   lease_epoch, delivery_nonce)` exactly** — a delayed, duplicate, wrong-connection, or
   stale-epoch ACK is ignored, so it can never commit a *different* in-flight delivery. The
   ACK — not the socket write — is the commit signal: **"delivered" means the wait client
   completely flushed `M` to its stdout boundary** (not that a frame entered the IPC
   buffer; end-to-end application-consumption would need a separate consumer ack, out of
   scope). The AWAIT is bounded by an **ACK deadline** with **frozen semantics (R3-S1)**:
   - **Source/default.** `ack_deadline` is a named config with a concrete `daemon-core`
     default derived from the heartbeat/lease window (a small multiple, so a healthy-slow
     stdout is not clipped); it is **not** open-ended.
   - **Clock.** Measured on a **monotonic in-process timer** (an elapsed-since-EMIT duration,
     never wall-clock and never compared across a restart — so it is distinct from the durable
     persisted `BackendClock` of [§11.1](#111-epoch-lifecycle-oq1) used for stored timestamps),
     so suspend/resume and skew are testable and a backward clock step cannot expire it early.
   - **Boundary.** The deadline is **exclusive**: the waiter is timed-out only when
     `elapsed > ack_deadline` (exactly-at-deadline is not yet a timeout).
   - **Timer-vs-ACK serialization.** The ACK arrival and the deadline timer race **inside the
     per-address critical section**; the **first to fire wins and the other is ignored** (an
     ACK that arrives after the timer has fired and closed the waiter is dropped — it cannot
     resurrect a MARK; a timer that fires after a matching ACK has been accepted is a no-op).
   - **Repeated-timeout backoff / quarantine.** After **K** consecutive ACK-deadline timeouts
     on the same `(address, connection)` the daemon **quarantines that connection** (marks the
     waiter unhealthy and stops re-selecting it for that address for a backoff window) so a
     persistently slow/blocked stdout cannot spin the per-address critical section into a
     duplicate-redelivery storm that starves later messages. `K` and the backoff are
     `daemon-core` config with frozen defaults.

   On timeout, EOF, or stdout backpressure the daemon **closes the waiter and releases the
   critical section WITHOUT a MARK** — `M` stays undelivered and simply redelivers
   (at-least-once). This keeps a wedged/backpressured/EOF-masked waiter from hanging the
   address (and from hanging `stop --drain`, which must flush in-flight critical sections).
4. **MARK** via `mark_delivered_if_current_owner(...)` only after a matching ACK. **The
   ownership check and the mark MUST be one atomic step (R3-5).** Under the stated Postgres
   `READ COMMITTED` autocommit model a two-step *read owner → mark* races a transfer/takeover
   that rotates ownership between the read and the mark (the mark would then commit as a
   non-current owner). The frozen shape locks the lease row first, in **one transaction**:
   - **Postgres:** `SELECT owner_instance_id, lease_epoch FROM leases WHERE address=:addr FOR
     UPDATE` (row-lock), compare to the caller's `(owner_instance_id, lease_epoch)`, then mark
     the delivery, then `COMMIT`.
   - **SQLite:** the same sequence inside a **`BEGIN IMMEDIATE`** transaction. **Framing note
     (R4-S2):** `BEGIN IMMEDIATE` takes a **database-wide** write lock in SQLite, **not** a
     row-level lock — it briefly serializes **all** writers for the short
     lock→compare→mark→commit transaction (correctness is fine: the daemon is the lone writer
     and the tx is short, held only for the mark, never across the EMIT/AWAIT). The
     **per-address critical section** bounds only *in-process* concurrency; it does **not**
     buy cross-address writer concurrency on SQLite, so perf acceptance must not assume
     unrelated-address write parallelism there. (Postgres `FOR UPDATE` is genuinely row-level,
     so unrelated addresses proceed concurrently.)

   The **owner-directed transfer** and the **Takeover CAS** take the **same lease-row lock**,
   so they serialize against the mark — closing the rotate-between-check-and-mark race. The
   method returns one of, with **strict outcome precedence**:
   - **`NotOwner`** (precedence-winning, **fatal**): returned whenever the caller is **not
     the current `(address, owner_instance_id, lease_epoch)`** — **even if the message is
     already delivered**. The daemon **self-demotes immediately**
     ([§11.2](#112-epoch-guarded-heartbeat-non-deleting-releaseownership-and-self-demotion-mr2-mr3))
     and stops draining the rest of the backlog. (Without this precedence, a successor `S`
     that marks first would make a stale predecessor `P` see `AlreadyDelivered` and keep
     emitting stale-epoch frames — the exact race the fence exists to stop.)
   - **`AlreadyDelivered`** → returned **only after** current ownership is confirmed;
     **success** (idempotent), continue draining. *Not* fatal.
   - **`Marked`** → success; continue draining.

**Why this is at-least-once with no loss window:** any crash, pipe break, lost/late ACK,
ACK-deadline expiry, or ownership rotation **after EMIT but before a successful MARK**
leaves `M` undelivered in `deliveries`, so the current owner redelivers it → a
**duplicate**, never a loss. The **only** thing that prevents a superseded owner from
systematically re-delivering is the epoch-guarded MARK returning `NotOwner` (which now wins
over `AlreadyDelivered`) and forcing self-demotion (the in-memory check in step 1 is just
an optimization — it only proves the daemon has not yet *learned* it lost ownership, never
that it is still the owner). The at-least-once contract, stated normatively: **`M` is
delivered repeatedly until exactly one current-epoch owner records a successful MARK;
waiters/consumers dedupe by `message_id`.** The duplicate count is bounded by the number of
failed owners/handoffs, not "exactly one." The `lease_epoch` on the frame is a
**secondary** filter a waiter applies only **after** it has independently learned a newer
epoch (via reconnect/handshake); it is **not** a live defense against a stale daemon — that
defense is the server-side MARK plus self-demotion.

The corresponding [§17](#17-gating-tests--per-backend-conformance-matrix-daemon-core-acceptance) gating test asserts, across
crash-after-EMIT/before-ACK, crash-after-ACK/before-MARK, waiter-death-after-EMIT,
**slow/blocked-stdout (ACK-deadline) , EOF-masked, wrong-connection, and stale/duplicate
ACK**, and **ownership-rotation-after-EMIT plus successor-marks-then-predecessor-marks**,
that every message reaches a waiter **at least once** (never zero) and that a superseded
owner stops after one `NotOwner` (even when the message was already delivered by `S`).

**Performance contract (R3-Sf).** The EMIT→ACK→MARK round-trip and its lease-row lock now sit
on the **per-address hot path**, so the budget is frozen as acceptance, not left implicit:
`daemon-core` freezes a **p95/p99 single-delivery fence latency budget** (local IPC RTT +
one lease-row-locked mark) and a **numeric dedup resource contract** (the per-address
`message_id` dedup set's bounded memory/row footprint and its retention window). These are
**benchmarked** as part of the gating matrix; the fence is **not weakened** (e.g. dropping the
ACK or the lock) to meet them — if a budget cannot be met, it is renegotiated explicitly, the
correctness fence stays. The new `sessions` authority adds a **per-`Register` upsert** and a
**per-`ReRegister` currency lookup**; both are single-row by the `(store_key, session_id)`
primary key / index (R4-Sc), so they are O(1) and off the per-message delivery hot path —
bounded, and included in the benchmarked budget.

### 11.4 Ordered handoff = owner-directed atomic transfer (sf3)

A graceful handoff (coordinated upgrade/stop where a successor `S` exists) must not lapse
the lease, leave an ownerless window a third daemon could hijack, or double-deliver. The
predecessor `P` transfers ownership **directly to `S` in one guarded statement** — there
is no release-then-claim gap and no generic "claim from a live owner" path (either would
admit a hijack):

```text
prepare  → S is spawned and READY (endpoint bound, backend open, recovery pass done)
quiesce  → P stops accepting new Wait/Register for the address; stops new drains
flush    → P completes in-flight EMIT→ACK→MARK critical sections (bounded by the ACK deadline)
transfer → one atomic UPDATE: P@epoch E → S@epoch E+1
```

```sql
UPDATE leases
   SET owner_instance_id = :successor,
       lease_epoch = lease_epoch + 1,
       last_heartbeat = :backend_now
 WHERE address = :addr AND lease_epoch = :E AND owner_instance_id = :predecessor  -- → rows: 0|1
```

The transfer writes only ownership/fence columns — it does **not** refresh
`attendance_last_confirmed_at` or `occupant` (a daemon-to-daemon transfer is **not**
session-carrying presence; refreshing attendance here would re-create the round-1
zombie-lease class by making a dead-but-unhooked station look freshly confirmed across an
upgrade). `S` inherits the prior `attendance_last_confirmed_at`, so a station that was
`occupied_stale` before the transfer **stays** `occupied_stale` after it (asserted in
[§17](#17-gating-tests--per-backend-conformance-matrix-daemon-core-acceptance)).

Properties: no ownerless gap (`P@E → S@E+1` atomically); no third-party hijack (the owner
is never `NULL`/stale during the transfer, so a normal stale-claim cannot interpose);
monotonic (epoch increments once, in-SQL, `:backend_now` = server clock); concurrent
transfers serialize on the row; the transfer's **pinned-row write also serializes against the
delivery MARK's lease-row lock** (R3-5), so an in-flight mark sees either the pre- or
post-transfer owner atomically, never a torn read; `P`'s later heartbeat/release/mark at `E`
returns 0 rows so `P` self-demotes; any `P`-emitted-but-unmarked message stays undelivered and
`S` redelivers it. **Successor readiness is a precondition:** `P` performs the transfer **only
after `S` reports READY** (the `prepare` step); if `S` crashes **before** the transfer, the
row is unchanged and `P` keeps ownership (abort the handoff, retry or fall back to release);
if `S` crashes **after** the transfer, `S@E+1` is simply a crashed owner whose lease ages
out via stale-claim recovery like any other crash — no special case. **Crash-based
handoff** (no live `S`, `P` dead/stale) is not a transfer — the successor uses the normal
stale-claim CAS ([§11.1](#111-epoch-lifecycle-oq1)); the **minimal upgrade floor**
([§16](#16-minimal-upgrade-floor)) uses non-deleting release + next-call stale-claim,
whose brief ownerless window is acceptable single-user (no competing claimant; messages
queue durably). A **per-step handoff crash matrix** (kill/signal after prepare, quiesce,
flush, transfer — on **both** `P` and `S`) is part of [§17](#17-gating-tests--per-backend-conformance-matrix-daemon-core-acceptance) and must
recover with no duplicate-beyond-at-least-once and no loss.

### 11.5 Postgres cross-machine reclaim (in epochs, not timing)

On Postgres two daemons can race across machines. Reclaim is **expressed in epochs, not
wall-clock**: a reclaiming daemon wins via the pinned-epoch+owner CAS of
[§11.1](#111-epoch-lifecycle-oq1); the loser self-demotes on its next 0-row heartbeat. No
timing assumption decides ownership. The **stale precondition** that *gates* a reclaim
(`last_heartbeat < stale_cutoff`) is itself a clock decision, so it uses the **backend/
database-server clock as the single time domain** for `last_heartbeat`, the
`stale_cutoff`, and the stale/TTL predicates (see
[§10.1](#101-last_confirmed-occupied_stale-and-the-hook-semantics-split-oq2-da-6)) — never
one machine's local `now` compared against another's timestamp. SQLite-local is the
simple single-writer case (commit order == id order); `postgres-parity` proves the
competing-daemon behavior under MVCC. Correctness rests on READ COMMITTED autocommit reads
(the isolation precondition pinned by ADR 0013); the per-backend fault-injection and
isolation matrix is [§17](#17-gating-tests--per-backend-conformance-matrix-daemon-core-acceptance).

## 12. Legacy cutover (OQ5, DA-1)

The first daemon-aware rollout meets **legacy holders** (resident `attach` processes) and
**non-epoch lease rows** (`lease_epoch IS NULL`). **Occupant-rotation alone is
insufficient**: a legacy holder ships `Frame::Message` (`attach.rs:~477`) *before* its
post-emit `mark_delivered` (`~485`), and its `heartbeat` returns `Result<()>` with **no
rowcount** so it **cannot observe self-demotion**; if the daemon rebinds the address's
waiter endpoint, two endpoints emit independently regardless of any post-emit row fence.
The deterministic rule is therefore **two-phase, and Phase 1 requires positive proof that
the legacy waiter is unbound — not merely that its heartbeat has aged out**:

- **Phase 1 — prove-unbound (drain).** Before binding its own waiter, the daemon-aware
  claimant MUST establish that **no legacy waiter endpoint is bound** for the address, by
  one of:
  1. an **address-keyed IPC probe** to the legacy endpoint (the legacy endpoint name is
     still derivable) carrying a **quit/handover** signal, and observing the endpoint
     **gone/closed**; or
  2. **terminating/quiescing** the legacy holder process (it is the same user's process).

  A **bounded stale-window wait alone is NOT sufficient and is removed as a stand-alone
  branch**: a stale heartbeat does not prove the endpoint is unbound — a `SIGSTOP`/paused
  process, a partitioned backend connection, a long GC, host sleep/suspend, or clock skew
  can age the heartbeat out while the legacy endpoint stays bound and later resumes
  emitting (and the legacy holder cannot self-demote). A stale-window MAY be used only as
  a *secondary* timeout after a probe has already shown the endpoint gone.
- **Phase 2 — claim.** Only after Phase 1 proves unbound, claim under the legacy path:

  | column | before (legacy) | after (daemon) |
  |---|---|---|
  | `lease_epoch` | `NULL` | `1` |
  | `owner_instance_id` | `NULL` (absent) | the daemon instance |
  | `occupant` | the legacy occupant | unchanged (informational; `suspect` until a `Register`) |

  via the explicit legacy CAS
  (`UPDATE ... SET lease_epoch=1, owner_instance_id=:me WHERE address=:addr
  AND lease_epoch IS NULL`) — ownership/fence columns only, **not** `occupant` (S10) or
  `attendance_last_confirmed_at`. `NULL` is **never** treated as `0` in the normal claim
  predicate ([§11.1](#111-epoch-lifecycle-oq1)); the legacy row gets its first epoch (`1`)
  exactly once. Thereafter the rowcount-returning epoch-guarded heartbeat/release and the
  non-deleting release apply.

**Cutover gating assertion (frozen):** *no legacy (non-epoch) holder **emits** a new
`Frame::Message` after the daemon's waiter binds.* This is exercised by a **dedicated
sixth gating test that starts a real legacy holder / non-epoch lease on both backends**
([§17](#17-gating-tests--per-backend-conformance-matrix-daemon-core-acceptance) test 6), since the prior five do not. Hard
cutover of existing sessions is acceptable (ratified).

> **In-flight legacy frame (M9).** Phase-1 prove-unbound proves the legacy *endpoint* is
> closed, not that **zero frames are in flight**: a legacy holder may have already written
> a frame to its own wait client *before* the endpoint closed, and that client can flush to
> the recipient *after* the daemon binds. This is why the assertion is "no legacy holder
> **emits** after the barrier" rather than "no frame reaches a recipient": an
> already-in-flight legacy frame is bounded by **at-least-once + `message_id` dedupe** (the
> recipient dedupes it against the daemon's redelivery of the same `message_id`), so it is a
> deduped duplicate, never loss or a correctness break.
>
> Preserved minority + reopen (design-foundation council, sharpened at the design-gate):
> one reviewer held occupant-rotation alone is the cutover; adopted prove-unbound instead
> (the legacy heartbeat cannot self-demote, and a stale heartbeat is not proof the endpoint
> is unbound). The **strong alternative** — a real drain barrier (quiesced + zero in-flight
> legacy `Wait` handlers + endpoint closed) — needs a **new legacy IPC verb** (today's
> legacy IPC exposes only `Shutdown`), which **trips the reopen condition**; `daemon-core`/
> the builder may adopt it to make the assertion "no frame reaches a recipient." This node
> takes the in-place at-least-once-dedupe resolution and flags the stronger option.

## 13. Delivery and the `seen`-dedup redesign (DA-8)

The exchange reuses the **durable per-recipient delivery buffer** of ADRs 0011/0013 (the
`deliveries(message_id, recipient)` table, `UNIQUE(message_id, recipient)`,
`fetch_undelivered`) unchanged as the **cross-epoch / cross-restart dedup authority**.
The live drain remains "deliver the undelivered set, authoritative on delivery state,
never on id ordering" (ADR 0013), now fenced by [§11.3](#113-server-side-delivery-fence-mr1--at-least-once-preserving).

The in-memory `seen` set must be **redesigned for a long-lived daemon.** Today `seen` is
an **unbounded `Mutex<HashSet<i64>>` that is never pruned** *because holders restart*
(verified `attach.rs:32-41,67-83`; rationalized in ADR 0013) — a long-lived daemon voids
that assumption (unbounded growth; stale identity across epochs). Redesign:

- **Durable `deliveries` is the authority** for "has this been delivered?" — no
  behavioral change to 0011/0013.
- **In-memory dedup is a bounded fast-path** keyed by **`(recipient, message_id,
  lease_epoch)`** (in-flight identity, scoped to the current epoch).
- **Seed** the fast-path from `fetch_undelivered` on claim.
- **Evict** an entry on: a durable mark (`mark_delivered_if_current_owner → Marked`),
  a terminal disposition, or an epoch transition.
- **Reset/drop** the entire fast-path on epoch loss (self-demote, takeover) — its
  identity is epoch-scoped, so it must not survive a fence.

This keeps dedup bounded and correct without relying on process restart, and elevates
issue #26 from a carry to a satisfied design prerequisite. (#27 `mark_delivered` cap and
#24 registry GC remain carries.)

### 13.1 Capacity and latency budgets (sf6, c1, c2)

Frozen budgets so the fence and dedup do not silently degrade, **without weakening the
ordering or the fence**:

- **Dedup fast-path resource contract.** The in-memory fast-path is bounded by a
  configurable `max_in_flight_entries` per store (and an aggregate byte cap); on pressure
  it sheds **oldest in-flight identities** (the durable `deliveries` table remains the
  authority, so shedding a fast-path entry costs at most a redundant `fetch_undelivered`
  comparison, never correctness). Default caps frozen in `daemon-core`.
- **Poll-drain / heartbeat cadence.** The poll-drain interval and heartbeat cadence have
  frozen defaults sized to agent-turn scale (seconds), with the heartbeat strictly shorter
  than `stale_after` / the lease window.
- **Fence round-trip budget.** The added `mark_delivered_if_current_owner` round-trip
  carries a **p95/p99 latency budget** (benchmarked; the transaction shape is optimized —
  e.g. single round-trip CAS-style upsert) — but the budget is a target to *optimize the
  shape toward*, **never** a license to weaken the EMIT→ACK→MARK ordering for latency.
- These budgets are single-user / pre-beta acceptance limits
  ([§7.0](#70-v1-threat-model-normative)); multi-user / hot-address scaling is revisited at
  beta.

## 14. Daemon-native session ownership

### 14.1 The `(store_key, session_id) → addresses` authority and the `sessions` table

The exchange owns an **in-memory** authority map keyed by **`(store_key, session_id)` →
`{addresses}`** for which addresses a session attends. The `store_key` is part of the key
because **one exchange serves multiple stores**: a `session_id` that recurs across stores
must not let one store's hook drop another store's addresses, nor let `ResolveFrom`
misattribute a `from`. This **reshapes #23 / PR #31**: the hook plumbing
is reused, but the filesystem `session_registry` (verified on
`feature/copilot-session-end-plugin`: per-session JSON files) is **dropped as the
authority**. The Copilot hook becomes a **thin mapper**
(`COPILOT_AGENT_SESSION_ID → TELEX_SESSION_ID`), and Copilot JSON parsing never becomes a
core protocol dependency (it lives in the plugin layer).

**Session incarnation + tombstones (durable, fail-closed, frozen — M3, R3-6 / spar).** The
anti-resurrection guard has **two distinct durable facts**, because one per-address column
cannot do both jobs (the spar showed a single per-`(session_id, address)` generation either
falsely rejects live sibling-address waiters *or*, without a session-keyed authority, lets a
GC'd-tombstone or a same-`session_id` respawn resurrect a removed address):

1. **Incarnation currency — the `sessions` table** ([§5.1](#51-durable-lease-row-columns-new)),
   keyed by `(store_key, session_id)`, is the **single source of truth** for "is incarnation
   `I` still the live one?" The incarnation is **`<session_seq>.<nonce>` with a daemon-assigned,
   durable, monotonic `session_seq`** (R6-2 — *not* a loader wall-clock; equal-millisecond and
   backward-skew failure modes are gone), carried in the session env (so every loader-spawned
   client of that life — `wait`/`send`/`reply`/`ack` — holds the same token; the separately
   spawned `sessionEnd` hook does **not** carry it, see below). The mint/bump boundary is
   unambiguous under the `sessions`-row lock, and uses a **positive `mode` discriminator, never
   "absence of a token" (R7-1):**
   - **`Register{mode: Establish{establish_nonce, expected_prior_seq}}`** (the loader's
     session-start) is a **prior-seq CAS** under the `sessions`-row lock that **closes the
     idempotency horizon (R8-1)** — a previously-used `establish_nonce` can **never** allocate a
     new seq, so a delayed/replayed establish cannot silently supersede a later quiet-but-live
     life. The row stores `establish_nonce` **paired with the seq it allocated**
     (`(establish_nonce, nonce_seq)`):
     1. **`establish_nonce == the row's recorded `establish_nonce` AND `nonce_seq == current_seq`**
        → **idempotent**: return the *current* seq (a lost-`Registered` retry — same nonce, same
        allocation, no double-bump), independent of `expected_prior_seq`. The **`nonce_seq ==
        current_seq` clause is load-bearing (R9-1):** any **non-`Establish` seq bump** —
        `Takeover{force:true}` ([§10.2](#102-takeover-fence-then-register-da-5-r3-3)) — **rotates
        `establish_nonce` to a fresh daemon-minted sentinel and advances `nonce_seq`**, so a
        post-force replay of the old nonce **cannot** match case (i) and be handed the new seq;
        it falls to (3) and is `Stale`.
     2. else **`current_seq == expected_prior_seq`** (the establisher observed the world it is
        superseding) **AND the prior life is *not* attendance-fresh** → allocate
        `current_seq + 1`, record the new `(establish_nonce, nonce_seq = current_seq + 1)` (a
        genuine new life over a gone/quiet predecessor — `expected_prior_seq = 0` for a
        first-ever establish).
     3. else → **reject, never allocate**: if the prior life **is attendance-fresh** →
        **`Conflict`** (a concurrent live same-`session_id` life, R7-Sa/R9-S1); if
        `current_seq != expected_prior_seq` → **`Stale{current_seq}`** (a typed error carrying
        the observed `current_seq`, R9-2) — a stale/replayed establish whose observed world has
        moved on (e.g. a delayed L1 replay after L2 established, or a post-force retry).

     **Forming `expected_prior_seq` — the observe/retry contract (R9-2).** A client does **not**
     guess: its **first** `Establish` carries `expected_prior_seq = 0` (or its last-known seq
     from a prior `Registered`, when re-establishing). On a **seq-mismatch** the daemon returns
     the typed **`Stale{current_seq}`**; the client then **retries — bounded by a small frozen
     cap, with the *same* `establish_nonce`** (same establish intent) and `expected_prior_seq =
     current_seq`. **Cap-exhaustion is a terminal, actionable outcome (R10-S1):** after the
     frozen retry cap the client stops and surfaces a terminal `Stale`/`Conflict` (it indicates
     sustained contention or a genuine still-live predecessor — the unsupported concurrent-lives
     case), exactly like the persistent-`Conflict` terminal of [R9-S1](#141-the-store_key-session_id--addresses-authority-and-the-sessions-table)
     above — it never loops unbounded. A genuinely **new** establish intent uses a **fresh**
     nonce; an **idempotent retry** reuses the same nonce (so case (1) absorbs a lost-`Registered`).
     The **first-ever absent row** is created by an `INSERT … ON CONFLICT DO NOTHING` (seq 1); a
     concurrent absent-row establish that loses the insert gets `Stale{current_seq = 1}` and
     retries. The **manual / no-loader** path is identical: a manual verb `Establish`es with a
     fresh nonce + `expected_prior_seq = 0` and retries on `Stale`. `establish_nonce` is
     **high-entropy and single-use** per intent. **Wording precision (R10-S1):** "a previously-used
     nonce never allocates a *new* seq" means **a transport-level replay** (the *same*
     `(establish_nonce, expected_prior_seq)` pair re-arriving) is terminal — it only ever
     re-hits case (1) idempotency or case (3) `Stale`; it is **not** the same as the legitimate
     **observe/retry**, which reuses the nonce but with a **freshly-observed, advanced
     `expected_prior_seq`** and so *can* reach case (2) and allocate. The prior-seq CAS (not an
     unbounded used-nonce set) is what makes a *stale-`expected_prior_seq`* nonce non-allocating.
   - **`Register{mode: Continue{(session_seq, nonce)}}`** (a mid-life address-add or re-prove)
     is a no-op continuation iff the carried token is current; a **non-current/older** token is
     **`Stale`**; a token that is **missing/unknown** returns **`NeedsEstablish`** — it is
     **never** silently promoted to an establish. This is what stops a **live session from
     self-superseding** (R7-1): a manual multi-verb session, a lost-`Registered` retry, or a
     verb that lost its env token cannot accidentally bump the seq and turn its own live verbs
     `Stale`; it either retries `Establish` idempotently (same `establish_nonce`) or fails
     actionably.
   - **`ReRegister` / removals never assign a seq** — they validate the carried token under the
     same lock (`NeedsEstablish` on a lost token).

   A daemon crash/respawn of a *still-live* session re-presents the **same** env token, so
   currency is preserved; a reused `session_id` gets `current_seq + 1`, fencing the old life
   closed. **Manual-session continuity (R7-Se):** a manually-driven (non-plugin) session keeps
   continuity **only if it propagates `TELEX_SESSION_INCARNATION` across its own `telex`
   invocations** (the env is the carrier — there is no token-file); a manual verb that cannot
   inherit it must `Establish` a fresh life. This is a **documented, user-facing v1 limitation**
   (plugin-managed sessions are unaffected — the plugin propagates the env). **Concurrent
   same-`session_id` contract (R6-Sc / R7-Sa):** the current-only `sessions` row holds exactly
   **one** live incarnation, so it **cannot represent two concurrently-live lives** sharing a
   `session_id`. The design **depends on the harness guaranteeing sequential reuse** (true on
   Copilot CLI — `session_id` is unique per session-life). If that guarantee were **violated**
   (two live lives, same `session_id`), the behavior is **defined and surfaced, not silent
   corruption**: a second `Establish` against a `session_id` whose current life is still
   attendance-fresh returns a **`Conflict` `Error`** (frozen in [§6.2](#62-request--response-frames),
   surfaced in `Status`, exercised by a gating test) rather than silently superseding; v1 does
   **not** support genuine concurrent same-id lives (out of scope, like intra-user isolation).
   **Fast-restart recovery (R9-S1):** a *legitimate sequential* restart whose successor starts
   **before the ended predecessor's attendance has aged to `occupied_stale`** also transiently
   hits `Conflict` (the predecessor still looks attendance-fresh). v1 resolves this as a
   **bounded transient `Establish` retry**: the successor re-attempts (with the **same**
   `establish_nonce`) until the predecessor's attendance crosses `stale_after` (a dead
   predecessor stops refreshing, so this is bounded by `stale_after`), after which branch (2)
   allocates; an operator may instead `Takeover{force:true}` to skip the wait. A successor that
   keeps `Conflict`ing past the bound surfaces the `Conflict` actionably (it indicates a genuine
   still-live predecessor — the unsupported concurrent-lives case). The retry/`force` choice and
   the `stale_after` floor are frozen in `daemon-core`.
2. **Per-address membership — `tombstoned_at`** on the lease row, set by the **station-removal**
   paths `DeregisterSession`/`Detach`/`Takeover`/lapsed-TTL (**not** `ReleaseOwnership`),
   atomically with the removal.

The rules, now frozen:

- `ReRegister` **MUST** carry a `session_incarnation`, and is validated in **two gates, in
  order**: (a) **currency** — the carried `(session_seq, nonce)` must equal the `sessions`
  row's current `(session_seq, nonce)` (current-only,
  R4-1), else `Stale` (this holds **even if a lease row is absent** — the `sessions` row, not
  the lease row, is the authority); then (b) **membership** — union the session's lease rows
  where `session_incarnation = I AND tombstoned_at IS NULL`; a tombstoned or foreign-`session_id`
  row is excluded.
- This closes every resurrection path, **because tombstoned lease rows are never deleted in v1**
  (R4-4, consistent with the [§11.2](#112-epoch-guarded-heartbeat-non-deleting-releaseownership-and-self-demotion-mr2-mr3)
  no-delete invariant): a **same-incarnation `Detach`ed** address keeps its (undeleted)
  tombstoned row and is excluded by the membership gate **for the life of the session** — the
  round-3 "GC'd-tombstone sibling resurrection" cannot occur because the tombstone is not
  collected; a **rebound** address carries another session's id/incarnation; a
  **session-id-reuse** new life takes a higher `session_seq`, so the old token fails gate
  (a).
- **`UnknownSession` is not authority to resurrect, and auto-recovery never recreates (R4-4).**
  A `ReRegister`/`Wait` that finds **no lease row at all** (a genuinely never-registered address,
  not a tombstoned one) returns `UnknownSession`. **Automatic** `wait`/`ReRegister` recovery MUST
  treat `UnknownSession` as terminal-for-that-address (surface it; do **not** recreate the
  binding) — only a **user-initiated fresh `Register`** (a new `attach`/explicit re-bind) may
  create a new binding, and it does so under the current incarnation, validated against
  `sessions`. So a stale auto-waiter can never silently re-materialise a removed address.
- **Per-session serialization, not just atomicity (R5-1).** "One transaction" gives
  crash-atomicity but **not isolation** under the stated Postgres `READ COMMITTED`
  ([§17](#17-gating-tests--per-backend-conformance-matrix-daemon-core-acceptance)): a
  read-check-then-mutate can interleave (a delayed old-life explicit `DeregisterSession(seq1)`
  reads the current `(session_seq, nonce)` = seq1, a concurrent establish `Register(seq2)`
  commits the bump, then the old removal's tombstone proceeds against the **new** life's rows). So **all four currency
  operations — `Register` / `ReRegister` / `DeregisterSession` / `Detach` — MUST serialize on
  the `sessions` row**, using the **same lease-row-lock pattern that closed the R3-5 mark
  race**: take a `SELECT … FOR UPDATE` on the `sessions` row (Postgres) / the `BEGIN
  IMMEDIATE` write transaction (SQLite) **before** the currency check, and make every
  dependent lease mutation **conditional** (`… WHERE session_incarnation = :I`). Under that
  lock:
  - **The incarnation order is daemon-assigned (R6-2).** The token is `<session_seq>.<nonce>`
    with **`session_seq` assigned by the daemon** under the lock at establish (`current_seq + 1`,
    or `1` if absent) — *not* a loader wall-clock, so equal-millisecond sequential lives and
    backward-skew "cannot start" are both gone, and the order is a durable monotonic integer.
  - **`Register` is the sole currency-setter, and its bump rejects a non-current/older seq.**
    Under the lock, an **establish** `Register` takes the next seq and supersedes; a **mid-life**
    `Register` carrying the current token is a no-op continuation; a `Register` carrying a
    **non-current / older** seq is rejected `Stale` and changes nothing — so a delayed old-life
    `Register` can **not** overwrite a live newer current.
  - **`ReRegister` / `DeregisterSession` / `Detach` never set currency**; under the lock they
    act **only if** the carried `(session_seq, nonce) == current` (exact match), else `Stale`
    no-op — closing the delayed-stale-removal race (R4-3) at the isolation level, not just the
    atomicity level.
  This is exercised by concurrent **old-hook-vs-new-`Register`** and **old-`Register`-vs-current**
  cases in tests 13/17.
- **Two-authority atomicity (R4-S1).** Within that per-session serialized transaction,
  `Register` commits its `sessions` upsert **and** the lease-row bind
  (`owner`/`session_id`/`session_incarnation`/`occupant`/`attendance`/clear-`tombstoned_at`)
  together; the incarnation-gated removals commit the `sessions` check and the tombstone writes
  together. A crash between the two writes is impossible; recovery always re-derives a consistent
  `sessions`⇄`leases` state.
- The incarnation `(session_seq, nonce)` is **assigned by the daemon** at establish and
  propagated to loader-spawned clients via the `Registered` response + the session env
  ([§14.6](#146-resolvefrom-sendreply-recovery-and-presence-between-waits-mr6-mr7)) so
  `wait`/`send`/`reply` re-register or gate with the correct token. (There is **no** token-file
  and the `sessionEnd` hook carries **no** incarnation — R6-1,
  [§14.2](#142-the-sessionend-hook-a-non-authoritative-liveness-hint-oq6-r6-1).)
- **Same-incarnation re-bind (R5-Sc).** A `Detach`ed address stays excluded from the
  membership union **for the rest of that incarnation's life via automatic recovery** (auto
  `wait`/`ReRegister` never recreates it — above). A **user-initiated `Register`** of that
  address *may* re-bind it (clearing `tombstoned_at`) — that is an intentional new attach, not
  a resurrection; the rule is **auto-recovery never recreates, explicit `Register` may**.
- **Retention (frozen, R4-4):** tombstoned lease rows and `sessions` rows are **not GC'd in
  v1** (single-user scale; bulk GC is issue #24), so the currency/membership facts a
  legitimately-slow reconnect needs are always present — no v1 GC, no tension with the
  no-delete invariant. To keep the accumulating retired rows off the hot path, the live
  union/`ReRegister` queries use a **partial index** (e.g. `(store_key, session_id) WHERE
  tombstoned_at IS NULL`, R5-Sd). **A numeric retained-row budget is surfaced (R6-Sf, R7-Sd):**
  the daemon tracks **both** the retired/tombstoned `leases` count **and** the never-deleted
  `sessions`-row count per store, and raises a **`Status` warning** when either crosses a
  `daemon-core`-frozen threshold (the single-user v1 budget), so unbounded growth of *either*
  durable authority is *observable* before it matters and the issue-#24 GC has a concrete
  trigger. The authority semantics are **frozen** here (no "`daemon-core`
  MAY choose an alternative" escape); the merge rule is the two-gate currency-then-union above.

**Cross-address operation serialization + global lock order (S4, R6-Sa).** Per-address critical
sections ([§11.3](#113-server-side-delivery-fence-mr1--at-least-once-preserving)) protect
single-address delivery, but `DeregisterSession`/`ReRegister`/`Takeover`/`Drain` touch **many**
addresses. To avoid partial removal, one-address resurrection, and deadlock, a cross-address
operation **acquires per-address sections in a fixed total order** (e.g. sorted by `address`)
and is **atomic at the durable layer** (the tombstone writes for all of a session's addresses
commit in one backend transaction); a crash mid-operation leaves a consistent durable state
that recovery re-derives. **One global lock order is frozen (R6-Sa)** to prevent deadlock
between the new `sessions`-row lock and the per-address sections: **`sessions`-row lock
(outermost) → per-address sections in `address` order (inner)**; no path ever acquires a
`sessions` lock while holding a per-address section. No operation holds two per-address sections
except in that fixed order.

Operations (idempotent): `Register`, `ReRegister`, `DeregisterSession(store_key,
session_id, session_incarnation, proof)`, all keyed by `(store_key, session_id)` and
**serialized on the `sessions` row** (R5-1).

### 14.2 The sessionEnd hook: a non-authoritative liveness hint (OQ6, R6-1)

The sessionEnd hook runs as a **separately spawned process** (verified
`integrations/copilot-cli/hooks.json` runs `telex session-end`; `session_end.rs` reads
**only a `session_id`**). This is an **information-theoretic limit (R6-1):** because a
`session_id` **recurs** across sequential lives and the hook is given *only* that recurring
id (no inherited env, no per-life secret, and — today — no Copilot API to inject one), the
hook **cannot identify *its own* life**. Any per-life token it could read by `session_id`
(a shared file, a "current" pointer, an "ask the daemon for the current seq" lookup) is, by
construction, the value of *whichever* life is current at read time — so a delayed old-life
hook would read and present the **new** life's token. The round-5 mandatory token-file is
therefore **removed**: it could not solve the problem and reintroduced the very race it was
meant to close.

So the hook is **demoted to a non-authoritative liveness hint.** It presents
`SessionEndHint(store_key, session_id, admin_cap)` — **no incarnation.** The daemon does
**not** remove anything on its say-so; the hint only **accelerates** a teardown that the
existing seq/epoch-fenced machinery would perform anyway, via a **latched, double-checked
conditional teardown** that can only ever affect the exact life it observed dead:

1. Under the `sessions`-row lock, **latch** the current life's `(session_seq, nonce)` and its
   **published `watch_pid_identity`** (the canonical `WatchSet`, [§5.1](#51-durable-lease-row-columns-new)/[§9.1](#91-typed-watch-pid-predicates-oq3)). **If `watch_pid_identity` is absent/empty (never published), the hint is a no-op** — absence is "unknown," not "dead" (R7-2). Else release the lock.
2. **Probe** those watch-pids *outside* the lock (the OS probe must not be held under the row
   lock).
3. **Re-acquire** the `sessions`-row lock and **tear down only if the current life still equals
   the latched `(session_seq, nonce, watch_pid_identity)` AND the probe *positively* proved it
   dead**; otherwise **no-op** (a newer life established in between, or no positive death proof →
   the hint is harmless).

The frozen invariant: **a `SessionEndHint` may only accelerate teardown of the exact life
whose published watch-identity it latched and proved dead — never "the current life at commit
time."** Liveness is thus a **veto, never authorization**: pids alive → no-op; pids dead →
the *normal* [§9.3](#93-dismissal-path-matrix-the-four-disjoint-cases) teardown removes that
exact (latched, still-current, proven-dead) life. The hint needs no incarnation because it
never acts on a life other than the one it observed.

**What the demotion costs:** *no safety* (the old incarnation-gated hook never had a real
life discriminator, so it never bought safety the backstop does not) — only **latency** for
the **unhooked-dead** case (the agent logically ended but a watched loader pid survives): the
hint vetoes (pids look alive), so cleanup waits for **stale-attendance → `occupied_stale` →
takeover** ([§10](#10-stale-attendance-and-takeover-no-teardown)), which is the load-bearing
recovery path for exactly that residual and is **seq-fenced** ([§10.1](#101-last_confirmed-occupied_stale-and-the-hook-semantics-split-oq2-da-6)).
The **reopen condition:** if a Copilot API ever lets the harness inject the current
`session_incarnation` into the `sessionEnd` hook env, the hook can carry its life's token and
become an **authoritative** seq-gated `DeregisterSession` again (immediate logical-end cleanup
even when pids survive).

**Explicit, seq-gated removals still exist.** A client/operator that *does* hold the current
token (a loader-spawned `telex detach`, or an operator action) uses the **seq-gated**
`DeregisterSession(store_key, session_id, session_incarnation, admin_cap)` /
`Detach(…, session_incarnation, …)` of [§6.2](#62-request--response-frames): the daemon
verifies the `admin_cap`, that `(store_key, session_id)` is in its map, **and** that the carried
`(session_seq, nonce)` equals current ([§14.1](#141-the-store_key-session_id--addresses-authority-and-the-sessions-table)),
tombstoning in **one serialized transaction**; a non-current token is a `Stale` no-op. **No
external session→address registry**, no per-session cap minted at Register in v1 (the
`per_session_cap` field is reserved for a future intra-user-isolation threat model).

**Threat-model note (R5-Sf).** The `session_seq` token is **same-user-readable/forgeable** (it
is not a secret — only `admin_cap` is). This is **acceptable under the explicit v1
same-user-trust / no-intra-user-isolation model** ([§7.3](#73-no-intra-user-isolation-in-v1-mr6)):
the seq gate defends against **accidental** stale-op races, not a **malicious** same-user
process (which already holds `admin_cap`). A future intra-user-isolation model would pair the
seq with the reserved `per_session_cap`. Stated so the gate is not mistaken for a security
boundary.

### 14.3 Crash recovery: `suspect` / `verified` / `lapsed` (OQ8, DA-3)

A respawned daemon **must not freshen recovered rows as live attendance without proof.**
Recovery is a three-state machine over attendance records:

- **`suspect`** — every row recovered from durable storage on respawn starts here. The
  daemon **MUST NOT heartbeat or deliver** for a `suspect` row (it has no proof the
  session is still alive).
- **`verified`** — promoted by a successful `Register` or `ReRegister`. A `Wait`
  reconnect promotes only **indirectly** — via the auto-`ReRegister` triggered on
  `UnknownSession` (see [§14.4](#144-wait-and-session-scoped-re-register)); the `Wait` IPC frame itself
  remains sessionless ([§6.2](#62-request--response-frames)). Promotion claims a **new
  epoch** ([§11.1](#111-epoch-lifecycle-oq1)), refreshes `last_confirmed`, and rebuilds
  the `watch_pids` set.
- **`lapsed`** — a `suspect` row that ages out via the daemon-down TTL window or
  stale-attendance/takeover with no proof. Its lease is released/fenced; it is not a
  permanent zombie.

**Durable vs rebuilt:** durable (recovered) = the lease rows — `address`, `occupant`,
`lease_epoch`, `owner_instance_id`, `last_heartbeat`, `attendance_last_confirmed_at`,
**`session_id`, `session_incarnation`, `tombstoned_at`** — **plus the `sessions` authority
row** ([§5.1](#51-durable-lease-row-columns-new)) and the durable delivery buffer. On respawn
the daemon **re-derives** the `(store_key, session_id) → addresses` authority map from those
durable rows as `suspect` (the `sessions` currency + per-address tombstones intact, so the
anti-resurrection guard survives the crash **without** bumping the incarnation — a still-live
session keeps its token). Rebuilt-by-client (lost on crash, re-established by
re-register/verify) = promotion to `verified`, the live `watch_pids` set, and IPC waiter
registrations.

### 14.4 `wait` and session-scoped re-register

`wait` is the **only long-lived client** able to re-prove a running session after a
respawn (the loader's `attach` is one-shot and already exited). On reconnect-on-EOF,
`wait` MUST **auto-`ReRegister`** (carrying its `store_key`, `session_id`,
`session_incarnation`, address, and watch-pids) **before** failing; a `Wait` that returns
`UnknownSession` triggers the same `ReRegister` then retries. `ReRegister` is
**unprivileged** — no `admin_cap` is required (per [§7.1](#71-scoped-capability-model-v1-one-instance-admin-token)); the cap's **only** acquisition path is the
`daemon-<H>.cap` file (it is **not** carried in env — [§7.1](#71-scoped-capability-model-v1-one-instance-admin-token)).
`ReRegister` is **session-scoped**: its `address` is **optional** — with an address it
re-verifies that one station; **without** an address it re-verifies **all** of the
session's durable-recovered addresses for that store, by the **two-gate currency-then-union**
rule of [§14.1](#141-the-store_key-session_id--addresses-authority-and-the-sessions-table):
first the carried `(session_seq, nonce)` MUST match the `sessions` row's current
`(session_seq, nonce)` (else
`Stale`, decided by the `sessions` row alone — independent of any lease row), then the set is
the **union of the session's lease rows where `session_incarnation = I AND tombstoned_at IS
NULL`**. It is **idempotent**:
concurrent waits for one `(store_key, session_id)` converge to a single map entry by that
union; a tombstoned/foreign/superseded address is **not** unioned back in. This
currency-then-union rule is **frozen** (no `daemon-core` alternative).

### 14.5 Daemon-down and the TTL backstop

If the daemon is down, its leases lapse after the **daemon-down TTL window** (the one
surviving role of the old TTL heartbeat) and/or are fenced by the respawned daemon's
higher epoch. A session that **ends while the daemon is down**: its `sessionEnd` hook
no-ops against a down daemon (recorded as a transient on the harness side, not fatal),
and the address is recovered on respawn via `suspect`→`lapsed` (TTL) or operator
takeover — **no permanent zombie**.

**Wall-clock dependence and the fail-closed path (R5-Sb).** The daemon-down TTL is the one
predicate that **inherently depends on real elapsed wall time**: the durable high-water clock
([§11.1](#111-epoch-lifecycle-oq1)) guarantees monotonicity but **cannot advance while the
daemon is down**, so "has the TTL elapsed?" is only observable if the **respawn wall clock has
actually advanced past** the persisted `last_heartbeat + ttl`. If the host **slept**, the wall
clock was **stepped backward**, or `wall_now` is otherwise still near the old high-water, a
respawn may not be able to *prove* the downtime — the high-water advances one tick but the TTL
predicate cannot fire. This is resolved **fail-closed, not fail-open**: a lease whose TTL
cannot be *proved* elapsed is **left occupied** (never auto-lapsed on an untrustworthy clock —
auto-lapse-on-doubt would risk delivering a live session's address to another), and the
**no-permanent-zombie guarantee is routed through operator `Takeover { force: true }`**
([§10.2](#102-takeover-fence-then-register-da-5-r3-3)) — the **break-glass seq-bumping**
supersession that bypasses the unavailable `occupied_stale` time proof (R6-3), closing the
otherwise self-contradicting recovery (the normal CAS would *also* need the time proof the TTL
says is unavailable). Test 18 covers a **slept / backward-wall-clock restart whose real
downtime exceeds the TTL**, asserting no fail-open lapse and that **force-Takeover** still
recovers the address.

### 14.6 `ResolveFrom`, send/reply recovery, and presence between waits (mr6, mr7)

`from`-resolution and the continuity of presence depend on the authority map, which is
in-memory and rebuilt by re-register — so the design must say what happens when a send or
reply needs it while the map is empty/`suspect`:

- **`ResolveFrom(store_key, session_id)`** resolves `from` against *that* session's
  registered addresses for *that* store only (never across sessions or stores): exactly
  one → succeed; multiple → `Ambiguous`; none → the recovery below.
- **send/reply on an empty/`suspect` map** (e.g. a crash mid-turn with no active `wait` to
  have re-registered) MUST first issue a **session-scoped `ReRegister`** (no address —
  [§14.4](#144-wait-and-session-scoped-re-register)) from inherited env (`store_key`,
  `TELEX_SESSION_ID`, `session_incarnation`), which currency-gates then rebuilds the session's
  address set from the durable rows, then retry `ResolveFrom`. The address-less session-scoped form is what
  makes this work: a foreground `reply` does **not** know its address (that is exactly what
  `ResolveFrom` discovers), so an address-scoped `ReRegister` could not be formed — the
  session-scoped form re-derives the whole set. If it still resolves nothing, the send
  **fails actionably** (`refused-unrepliable`, as ADR 0010) — never a silent `from = None`.
  This fully closes the reintroduced ADR 0010 foot-gun (acceptance test: register, no
  blocked wait, kill+respawn the daemon mid-turn, `telex reply` without `--from` → the
  documented outcome).
- **The one-shot verbs inherit a frozen station env contract, and it carries the incarnation
  (R4-2 / R6-2).** `TELEX_SESSION_ID`, `store_key`, and **`TELEX_SESSION_INCARNATION`** (the
  daemon-assigned `(session_seq, nonce)` token) are present in the env of every
  `send`/`reply`/`wait`/`ack` the **loader/plugin** spawns, so the session-scoped re-register can
  always be formed. **Propagation authority:** the **loader's establish `Register` obtains the
  `(session_seq, nonce)` from the daemon** ([§14.1](#141-the-store_key-session_id--addresses-authority-and-the-sessions-table), in the `Registered` response — the daemon, not the loader, assigns
  the order) and the loader **injects it into the env of every one-shot verb it spawns**; a
  later verb re-presents it (mid-life, currency-gated). **There is no token-file** — the round-5
  mandatory token-file is **removed** (R6-1): it only ever needed to serve the separately-spawned
  `sessionEnd` hook, and that hook is now **non-authoritative and carries no incarnation**
  ([§14.2](#142-the-sessionend-hook-a-non-authoritative-liveness-hint-oq6-r6-1)), so no
  cross-process file channel is required. **No-loader / manual sessions:** a `telex` verb run
  with **no** inherited `TELEX_SESSION_INCARNATION` performs an **`Establish`** (fresh
  `establish_nonce` + `expected_prior_seq = 0`, the **prior-seq CAS** of
  [§14.1](#141-the-store_key-session_id--addresses-authority-and-the-sessions-table) — observing
  `Stale{current_seq}` and bounded-retrying if a prior life exists; the daemon assigns
  `(session_seq, nonce)` and returns it) and exports it for the rest of that manual invocation
  chain; a later manual verb that cannot inherit it `Establish`es again under the same CAS
  (which, via `expected_prior_seq`/attendance-fresh, keeps it from clobbering a newer current —
  it gets `Conflict`/`Stale` rather than an unconditional bump).
- **Presence between `wait` calls (mr7).** Only a blocked `wait` continuously re-proves a
  session; a foreground agent mid-task (between waits) during a `drain`/respawn would
  otherwise lose verified attendance and default-`from` until its next wait. To bound this,
  **`send`/`reply`/`ack` also opportunistically session-`ReRegister`** (cheap, idempotent,
  currency-guarded), so any agent action re-proves presence — not only `wait`. The
  continuous-occupancy claim is therefore **action-triggered, not universal** (aligned in
  [§3.3](#33-wait-reconnect-on-eof-grace) and DESIGN.md): presence is continuous for a
  session that takes *any* telex action across the handoff; a fully idle session (no wait,
  no send) between drain and its next action is `suspect` until it next acts, which is
  acceptable (it is not actively transacting).

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
swap). **v1 cutover is forward-only** (a new daemon supersedes an old one; there is no
supported rollback to a pre-epoch holder while `lease_epoch >= 1` rows exist — see the
downgrade note below):

- **Versioned install + launcher shim.** A stable `telex` shim resolves to a versioned
  binary (`telex-<version>`), so an upgrade writes a new versioned binary without
  overwriting the locked one.
- **`telex daemon stop --drain`.** Quiesce + flush in-flight EMIT→ACK→MARK + hand off via
  the owner-directed transfer where a successor exists, else non-deleting release, in order
  ([§11.4](#114-ordered-handoff--owner-directed-atomic-transfer-sf3)), then exit — freeing
  the binary lock.
- **Next-call respawn.** The next client connect-or-spawn starts the new version (handoff
  reuses the transfer / stale-claim + crash-recovery). Presence across the respawn for a
  mid-task agent is covered by opportunistic re-register on any action
  ([§14.6](#146-resolvefrom-sendreply-recovery-and-presence-between-waits-mr6-mr7)).
- **Legacy / non-epoch cutover** = the prove-unbound rule of [§12](#12-legacy-cutover-oq5-da-1).
- **Token-file orphan cleanup (R7-Sf).** Earlier builds wrote a per-session token-file (the
  round-5 mechanism, now removed — R6-1); on upgrade the new binary **best-effort deletes any
  pre-existing `<run_dir>/sessions/*.token` orphans** (they are inert — nothing reads them — but
  cleaning them avoids confusion and disk residue). Failure to delete is non-fatal.
- **Downgrade (forward-only v1).** Once rows carry `lease_epoch >= 1`, an **old pre-epoch
  holder must not run** against the store (it would write non-epoch rows and reset the
  fence). v1 states this as a constraint, not a supported path: a true downgrade contract
  (epoch-aware old binary, or an epoch-preserving rollback) is **out of scope for
  `daemon-core`** and belongs to `seamless-upgrade`. The store schema-version
  ([§3.4](#34-per-store-isolation-and-schema-version-sf5)) gates a too-old binary closed.
- **Protocol-major bump and already-blocked waits (c4).** A protocol-major bump runs a
  **separate singleton** ([§2.1](#21-singleton-identity)); a `wait` already blocked on the
  old-major daemon is not silently migrated. On reconnect-on-EOF it re-handshakes, sees the
  new major, and connect-or-spawns the new-major daemon (re-registering there). For
  `daemon-core`, cross-major live migration of a blocked wait is **out of scope**; the
  blocked wait simply re-targets the correct singleton on its next reconnect.

## 17. Gating tests + per-backend conformance matrix (daemon-core acceptance)

The executable gating tests below are **frozen as `daemon-core` acceptance**, each with the
**observable assertions** (OQ7 — the assertions are part of the frozen contract; rendering
is not). Because the fence's whole point is **cross-backend single-writer correctness**,
each is annotated with its **per-backend** requirement; "N/A" is justified inline. The
isolation precondition for all Postgres concurrency tests is **READ COMMITTED autocommit**
(ADR 0013). The single clock domain is the backend/db-server clock
([§11.5](#115-postgres-cross-machine-reclaim-in-epochs-not-timing)).

| # | Test | SQLite | Postgres | Key assertion |
|---|---|---|---|---|
| 1 | **Concurrent first-use** (thundering-herd auto-spawn) | required (multi-process) | required | exactly one daemon bound; losers connect; no duplicate/orphan |
| 2 | **Crash-during-`wait`** (+ suspect-row invariant) | required | required | `wait` reconnects+auto-`ReRegister`s, no spurious exit 3; recovered rows are `suspect` and **not delivered** until `verified` |
| 3 | **Competing daemons** | N/A (single-writer; commit order == id order) — assert single-writer holds | **required** (cross-process/cross-machine, fault injection) | higher epoch wins; loser self-demotes on 0-row heartbeat; no flip-flop; no delivery from demoted owner |
| 4 | **Delivery at-least-once failpoints** (mr1) — crash-after-EMIT/before-ACK, crash-after-ACK/before-MARK, waiter-death-after-EMIT, ownership-rotation-after-EMIT, **ownership-rotation BETWEEN the mark's ownership-check and the mark** (R3-5) | required | required | every message reaches a waiter **>=1** time (never 0); the atomic lease-row-locked mark returns `NotOwner` on the between-check-and-mark rotation (precedence over `AlreadyDelivered`) → superseded owner stops after one; dedupe by `message_id` |
| 5 | **Intra-daemon takeover local-eviction + bounded pending-bind** (R3-3, R4-5) | required | required | old-occupant `wait` gets a defined disconnect (not a hang); post-takeover the row is **pending-bind** (`owner=daemon, session_id=NULL`) and **non-deliverable** — a `Wait`+inject **before** any follow-up `Register` yields **no `Message`, no MARK** (R4-5); a follow-up `Register` binds the new occupant; **if `Register` never lands the un-heartbeated row ages and is reclaimed (no permanent wedge)**; an **aging-reclaim vs simultaneous follow-up `Register`** serializes on the pinned CAS (exactly one wins, loser retries, no torn bind); no both-owned intermediate state; the typed `TookOver` response carries `prior_occupant`/`last_confirmed` |
| 6 | **Real legacy holder / non-epoch cutover** (mr4) — start an actual legacy holder + `lease_epoch IS NULL` row | required | required | Phase-1 prove-unbound holds; **no NEW `Frame::Message` is emitted by the non-epoch holder after the daemon binds** (a pre-barrier in-flight frame may still arrive and is **at-least-once duplicate-deduped by `message_id`**, never lost — per [§12](#12-legacy-cutover-oq5-da-1) M9); legacy row advances `NULL → 1` |
| 7 | **Epoch monotonicity across release/cleanup/re-claim** (mr2) | required | required | after `ReleaseOwnership` at epoch E and a cleanup pass, the next claim is **E+1 (never 1)**; no row deletion of an epoch-bearing address |
| 8 | **Unhooked dismiss + loader survives + takeover CAS boundary** (mr3, R3-3) | required | required | the daemon's own heartbeat does **not** refresh `attendance_last_confirmed_at`; `occupied_stale` becomes true **with a still-fresh heartbeat** and the **takeover CAS fires on the occupied-stale *attendance* predicate** (the boundary: stale-heartbeat-alone would NOT, ownerless would take the claim path); the live-but-idle session is **not** torn down |
| 9 | **Ordered-handoff crash matrix + successor-readiness** (sf3) — kill after prepare / quiesce / flush / transfer, on **both** P and S | required | required | bounded idempotent recovery; no loss; no duplicate beyond at-least-once; no ownerless hijack window; **S-crash-before-transfer aborts the handoff (P keeps ownership), S-crash-after-transfer recovers via stale-claim**; a dead station stays `occupied_stale` across `stop --drain` + transfer (no attendance refresh) |
| 10 | **OS trust boundary negatives** (mr5, R3-7) | required (Unix 0700 socket / symlink) | required | a second OS principal cannot `Hello`/`Register`/`Wait`; symlinked cap/lock rejected; **a pre-bound hostile server is rejected client-side BEFORE any metadata disclosure** (before `Hello`/`store_key`, via `GetNamedPipeServerProcessId`/connected-`SO_PEERCRED`); **a second server instance is refused** by the exclusivity primitive; a **PID-reuse race** does not authenticate the wrong process; **`admin_cap` never appears** in `Status`/`Error`/logs/traces; non-owner-private `config_root`/`run_dir` rejected at startup |
| 11 | **IPC version/capability compatibility** (sf2, R6-Sb, R8-S1, R10-S2) — N/N-1 and N+1/N | required | required | security-sensitive `required_capabilities` mismatch fails closed (`Incompatible`/`Unauthorized`); attach/wait-reconnect/Drain/Deregister/Status behave per the **`daemon-core`-owned IPC compatibility table** ([§6.1](#61-version-handshake--capability-negotiation-hello--helloack-sf2)); the **`(session_seq, nonce)` token, the `Register.mode` enum (`Establish{establish_nonce, expected_prior_seq}`/`Continue`), the typed **`Stale{current_seq}`** payload + `nonce_seq` semantics, the `NeedsEstablish`/`Conflict` errors, and the `SessionEndHint`/`force`-Takeover frames are all part of that versioned surface** (R6-Sb, R8-S1, R10-S2) — N/N-1 cases assert an N-1 client/daemon either negotiates each or **fails closed**, **never silently degrades `Stale{current_seq}` to a bare `Stale`** (which would break the observe/retry loop) and never drops the seq gate or the `mode` discriminator (this node freezes the frame shapes + fail-closed policy, `daemon-core` fills/freezes the version table) |
| 12 | **N / N+1 protocol-major parallel** (mr8) | required | required | two protocol-major-parallel daemons under one config root each authenticate against their own `daemon-<H>.cap`; neither clobbers the other |
| 13 | **Session-seq currency / establish CAS / serialization** (M3, S13, R3-6, R4-1/4/S1, R5-1, R6-2, R7-1/Sa/Sg, R8-1, R9) | required | required | **live-sibling**: a `Continue` for sibling B does **NOT** falsely `Stale` A's waiter; **transition**: `Establish` seq=1, then `Establish(expected_prior_seq=1)` seq=2 → `ReRegister(seq1)=Stale`, `ReRegister(seq2)` passes; **idempotency (R7-1)**: a retried `Establish` (same `establish_nonce`, `nonce_seq==current`) returns the **same** seq; **idempotency horizon (R8-1)**: a replayed `Establish` whose nonce allocated an older seq, or whose `expected_prior_seq` no longer matches, **does NOT allocate** (`Stale{current_seq}`) and does **NOT** supersede a quiet-but-live newer life; **observe/retry (R9-2)**: a new life that `Establish`es with a stale `expected_prior_seq` gets **`Stale{current_seq}`** and a **bounded retry** with `expected_prior_seq=current` (same nonce) then allocates; **first-ever absent-row** insert: one of two concurrent establishers wins, the other gets `Stale{current_seq=1}` and retries; **post-force (R9-1)**: after `Takeover{force}` rotates `establish_nonce`, a replay of the **old** nonce is **`Stale`**, NOT handed the new seq; **no self-supersession (R7-1)**: a `Continue`/`ReRegister` that lost its token → **`NeedsEstablish`**, no seq bump; **`Conflict` (R7-Sa/R9-S1)**: a 2nd `Establish` vs an attendance-fresh life → `Conflict`; a **fast sequential restart** retries until the predecessor ages to `stale_after` then allocates; **concurrent serialization**: old `DeregisterSession(seq1)` vs new `Establish(seq2)` does not tombstone seq2; **state oracle (R6-Sd/R7-Sg):** assert post-state seq + leases intact (not response-code only); `sessions`+lease writes commit in **one serialized transaction** |
| 14 | **Schema-version downgrade gate + new-table migration** (M10, R3-S2, R4-Sc) | required | required | the per-store exclusive migration creates **both** the `sessions` table **and** the new lease columns **atomically** under one schema-version bump; a pre-epoch binary invoked **directly** (not via the shim) is refused by the **mandatory store-level legacy-write hard-fail** **before** it writes a non-epoch row (launcher lock asserted as additional defense); mid-migration crash recovers under the per-store exclusive lock |
| 15 | **ACK boundary + correlation + deadline failpoints** (M2, S1, R3-S1) | required | required | partial/errored stdout flush is not an ACK; slow/blocked-stdout hits the ACK deadline → no MARK, redeliver, address not wedged, `stop --drain` does not hang; **ACK exactly-at-deadline is NOT a timeout, one-tick-late IS**; **repeated timeouts quarantine the slow connection** (no duplicate-storm starvation); wrong-connection/stale/duplicate ACK never marks a different in-flight delivery |
| 16 | **Cross-address operation atomicity** (S4, R3-Sc) | required | required | `DeregisterSession`/`Drain` **and `ReRegister`/`Takeover`** over many addresses are atomic at the durable layer and fixed-order-acquired; a mid-operation crash leaves a consistent state (no partial removal, no one-address resurrection, no deadlock) |
| 17 | **Non-authoritative sessionEnd hint** (R6-1, R8-2) | required | required | a `SessionEndHint` whose **latched current life's watch-pids are alive** is a **no-op** (veto — never tombstones a live life), incl. the **reuse-startup** case (an old-life hint arriving the instant after a new life established → no-op, **state oracle:** new life's rows intact); a hint whose **latched life is still current AND its pids are proven dead** tears down that exact life; a hint that **latches life A then life B establishes before the probe commits** does **NOT** tear down B (double-checked on the latched `(session_seq, nonce, watch_pid_identity)`); **same-life WatchSet replacement (R8-2):** a hint **latches W1**, then a **same-seq current-token `Continue`/`ReRegister` publishes W2** (W1 dead, W2 alive) → the recheck (which includes `watch_pid_identity`, not just `(seq, nonce)`) **no-ops** — a live life is **not** torn down; the OS pid probe is **not** held under the `sessions`-row lock |
| 18 | **Durable BackendClock + daemon-down TTL + force-Takeover** (R4-6, R5-Sb, R6-3, R9-1) | required (SQLite high-water) | required (PG server clock) | persisted `last_heartbeat`/`tombstoned_at`/`sessions.updated_at` stamped before a restart compare correctly against the respawned daemon's clock; the SQLite high-water never moves backward across restart/suspend/skew; a **slept / backward-wall-clock restart whose real downtime exceeds the TTL** does **not** fail open (no auto-lapse of a live address on an untrustworthy clock); recovery is via **`Takeover{force:true}`**, the break-glass **seq-bumping** supersession (bypasses the unavailable `occupied_stale` time proof; **state oracle:** post-state `session_seq` is bumped, **`establish_nonce` is rotated / `nonce_seq` advanced** so a post-force old-nonce `Establish` replay is `Stale` not handed the new seq (R9-1), and old-seq ops are `Stale`), so no permanent zombie; a concurrent establish vs force-Takeover serializes on the `sessions` row (first commit wins) |
| 19 | **Seq-fenced attendance / unhooked-dead reclaim / hint no-op on absent identity** (R6-1, R7-2/Sg) | required | required | a dismissed session whose **loader/anchor pid survives** but issues **no current-seq telex action** has its `attendance_last_confirmed_at` go stale → `occupied_stale` fires → takeover reclaims (**state oracle:** post-state address is reclaimable, no leak); an **old-seq** `Register`/`ReRegister` (a superseded life's surviving process) **cannot** refresh attendance (`Stale`); heartbeat/bare-`Wait`/`SessionEndHint` never refresh attendance; **a `SessionEndHint` against a life with an absent/empty `watch_pid_identity` is a NO-OP** (absence ≠ death, R7-2) |

Tests 1–5 are the original five (4 and 5 strengthened); 6–19 are added by the review
rounds (6–12 round 1, 13–16 round 2, 17–18 rounds 3–5, **19 + the R6 reworks of 13/17/18**
round 6). `fencing-proof` owns 3/4/6/7/9/15 on Postgres; `postgres-parity` owns the
cross-machine axis of 3. Test 4 additionally asserts the **mark/transfer/takeover
lease-row-lock serialization is deadlock-free on both backends**, and the **global lock order**
([§14.1](#141-the-store_key-session_id--addresses-authority-and-the-sessions-table) R6-Sa:
`sessions`-row lock outermost → per-address sections inner) is asserted deadlock-free (the
SQLite db-wide `BEGIN IMMEDIATE` tx is short and never nested under the per-address section).

## Open-question resolutions

The eight open questions carried into `design-foundation`, resolved with implementable
specifics (cross-referenced to the sections above):

| OQ | Question | Resolution | Where |
|---|---|---|---|
| **1** | Epoch lifecycle | Monotonic, never-reused `lease_epoch` + `owner_instance_id`; pinned-epoch+owner **CAS that increments in-SQL** (NULL≠0); **non-deleting release** retains the epoch high-water (no-delete invariant); rowcount-guarded heartbeat (lease-liveness only) + self-demote that **stops heartbeating**; server-side delivery fence = **EMIT → waiter-ACK → epoch-guarded MARK** (at-least-once preserving, `NotOwner` fatal, `AlreadyDelivered` success); ordered handoff = **owner-directed atomic transfer**; Postgres reclaim in epochs with a single backend clock domain. | [§11](#11-lease-epoch-fence-the-spine) |
| **2** | Stale-attendance threshold + takeover (no teardown) | `last_confirmed` refreshed by positive **session-carrying** presence only (NOT heartbeat, NOT bare Wait, NOT sessionEnd); `occupied_stale` derived from `stale_after` on a single backend clock domain; never tears down; takeover = **fence + evict + close-waiters + tombstone** (mint epoch, leave **pending-bind** = owner-but-session-less, follow-up `Register` binds), allowed once `occupied_stale`; pending-bind bounded by un-heartbeated aging. | [§10](#10-stale-attendance-and-takeover-no-teardown) |
| **3** | Typed `--watch-pid` shape | `anchor` (any-sufficient) vs `required` (all-necessary) + pid+start-time reuse guard; v1 floor = loader anchor + start-time; expose flags only with a real consumer; dismissal-path matrix routes positive death to immediate teardown. | [§9.1](#91-typed-watch-pid-predicates-oq3), [§9.3](#93-dismissal-path-matrix-the-four-disjoint-cases) |
| **4** | Distinct per-session PID? | **No usable one** on Copilot CLI today (empirically grounded: inner worker pid not env-exposed and lazily spawned; ppid-walk rejected). Loader anchor + start-time is the sole env-sourced backstop; hook + stale-attendance/takeover carry the rest. | [§9.2](#92-per-session-pid-on-copilot-cli-oq4--resolved-none-usable) |
| **5** | Legacy / non-epoch cutover | **Two-phase, prove-unbound**: Phase 1 proves the legacy waiter endpoint is unbound (address-keyed IPC probe + quit, or process quiesce) — a stale-window alone is insufficient; Phase 2 claims `NULL → 1` via the explicit legacy CAS. Frozen cutover assertion + a dedicated legacy-holder gating test on both backends. | [§12](#12-legacy-cutover-oq5-da-1) |
| **6** | sessionEnd removal proof (no external registry) | Instance `admin_cap` from the **singleton-scoped** user-private `daemon-<H>.cap`; OS owner-only enforcement + peer-credential check ([§7.2](#72-os-level-trust-boundary-mr5)). The harness `sessionEnd` hook is **non-authoritative** (R6-1): it sends `SessionEndHint(store_key, session_id, admin_cap)` — **no incarnation** (a recurring `session_id`-only hook cannot identify its life) — and the daemon runs a **latched, liveness-vetoed, double-checked** teardown of the exact proven-dead life. **Explicit** removals (`DeregisterSession`/`Detach`) that *hold* the current `(session_seq, nonce)` are **seq-gated**. v1 = same-user trust, **no intra-user isolation**; per-session cap reserved. | [§7](#7-authorization-and-the-trust-boundary), [§14.2](#142-the-sessionend-hook-a-non-authoritative-liveness-hint-oq6-r6-1) |
| **7** | Status freeze line | Freeze the **field set + meaning** + the gating tests' observable assertions; `daemon-core` owns rendering/format/verbosity. | [§4](#4-status-surface-the-frozen-contract-shape), [§17](#17-gating-tests--per-backend-conformance-matrix-daemon-core-acceptance) |
| **8** | Attendance durability across daemon crash | Durable = lease rows (incl. epoch/owner/last_confirmed/`session_incarnation`/tombstone) **+ the `sessions` incarnation-currency authority** + delivery buffer; rebuilt-by-client = in-memory `(store_key, session_id)` map + watch-pids + IPC waiters. `suspect`/`verified`/`lapsed` recovery; `wait` (and send/reply/ack) auto-Re-register; **two-gate currency-then-union** anti-resurrection (incarnation currency in `sessions` decides validity from the `sessions` row alone, independent of any lease row); daemon-down TTL backstop; no permanent zombie. | [§14.1](#141-the-store_key-session_id--addresses-authority-and-the-sessions-table), [§14.3](#143-crash-recovery-suspect--verified--lapsed-oq8-da-3), [§14.4](#144-wait-and-session-scoped-re-register) |

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
| **0010** (local holder registry as `from`-default source) | **Superseded** as the `from` source: daemon-era `ResolveFrom(store_key, session_id)` against the session's registered addresses for *that store only* ([§14.1](#141-the-store_key-session_id--addresses-authority-and-the-sessions-table); DA-9). Never infer across sessions **or stores**; harness propagates `store_key` + `TELEX_SESSION_ID` to `send`/`reply`. |
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

- The cutover **drain** ([§12](#12-legacy-cutover-oq5-da-1)) cannot be realized via the
  address-keyed IPC probe + bounded stale-wait (i.e. it needs a *new* IPC verb) — would
  make a fix architectural rather than in-place.
- A Copilot plugin API appears that lets the plugin pre-populate the sessionEnd hook's
  env from a value captured at `attach` — then a **per-session cap** becomes the v1 path
  and [§7.1](#71-scoped-capability-model-v1-one-instance-admin-token) should re-tighten
  (not loosen).
- The `wait` auto-Re-register path ([§14.4](#144-wait-and-session-scoped-re-register)) cannot be
  implemented because the chosen IPC transport masks socket-EOF — would force a
  positive-presence heartbeat from `wait`.
- The single-source SKILL mechanism ([§15.2](#152-single-source-skill--plugin-skill-mechanism-oq-for-deliverable-7-da-10))
  hits a harness constraint (manifest cannot point outside the plugin dir **and** `exec`
  is rejected) — would force a code-touching deviation.
