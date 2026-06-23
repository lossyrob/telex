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
session from inherited env (see [§14.4](#144-wait-auto-re-register)), and (c) resume
blocking — returning exit `3` only if the grace window expires without a healthy
reconnect. This makes ordered handoff and crash-respawn invisible to the agent's turn
loop.

### 3.4 Per-store isolation and schema-version (sf5)

One exchange serves multiple stores, so a fault in one store must not stall healthy ones,
and a multi-store, populated-Postgres deployment needs a schema contract:

- **Per-store loop isolation.** The `RECOVERING`/heartbeat/delivery loops are **per
  store**: a backend that is unreachable, slow, or in `RECOVERING` pauses **only its own**
  store's heartbeat/delivery; other stores keep serving. One bad backend never freezes the
  whole exchange. (`SPAWNING` still requires the *requested* store to be reachable for the
  triggering client; other stores attach lazily.)
- **Store schema-version.** Each store records a `telex_schema_version`. On open, the
  daemon **gates closed** a store whose schema is newer than the binary understands
  (forward-incompatible) and applies additive migrations for older-but-compatible schemas
  (`CREATE TABLE IF NOT EXISTS` + additive column adds, consistent with ADR 0013). A
  too-old binary against a newer store fails closed (this is the downgrade gate referenced
  in [§16](#16-minimal-upgrade-floor)). A full migration/downgrade framework for a
  populated multi-store Postgres deployment is `seamless-upgrade` scope; v1 freezes only
  the version field + the fail-closed gate.

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
  store_key:                    StoreKey,       // effective store identity (profiles::store_key); part of the authority key
  session_id:                   Option<String>, // opaque; the attending session
  session_generation:           u64,            // monotonic per (store_key, session_id); tombstone guard (see §14.1)
  occupant:                     String,         // human/host label of the occupant
  owner_instance_id:            Option<String>, // owning daemon instance, or NULL when released (epoch retained)
  lease_epoch:                  u64,            // monotonic, never-reused fence token (see §11)
  watch_pids:                   Vec<WatchPid>,  // liveness backstop (see §9)
  host:                         String,
  last_heartbeat:               i64,            // backend-clock ms; lease liveness proof (heartbeat-only)
  attendance_last_confirmed_at: i64,            // backend-clock ms; refreshed by POSITIVE session-carrying presence only (NOT heartbeat, NOT bare Wait)
  state:                        Attendance,     // Suspect | Verified | Lapsed
  occupied_stale:               bool,           // DERIVED: now - last_confirmed > stale_after, single clock domain
}

WatchPid { pid: u32, start_time: u64, role: Anchor | Required }
Attendance = Suspect | Verified | Lapsed
```

`owner_instance_id IS NULL` marks a **released-but-epoch-retained** row (occupancy is
`owner_instance_id IS NOT NULL` and not stale, never row existence — see
[§11.2](#112-epoch-guarded-heartbeat-non-deleting-release-and-self-demotion-mr2-mr3)).

### 5.1 Durable lease-row columns (new)

The backend `leases` table — today keyed by `address` only with **no owner generation**
(verified: `src/registry.rs` `HolderRecord`, backend `claim_lease`/`heartbeat`/
`release_lease`) — gains:

- **`lease_epoch INTEGER`** — the monotonic, never-reused fence token (retained across
  release; see [§11.2](#112-epoch-guarded-heartbeat-non-deleting-release-and-self-demotion-mr2-mr3)).
- **`owner_instance_id TEXT`** — the owning daemon instance, `NULL` when released.
- **`last_heartbeat INTEGER`** — backend-clock ms lease-liveness proof (heartbeat-only).
- **`attendance_last_confirmed_at INTEGER`** — backend-clock ms of last positive
  session-carrying confirmation (never written by heartbeat).

Greenfield: added via `CREATE TABLE IF NOT EXISTS` / additive column add gated by the
store schema-version ([§3.4](#34-per-store-isolation-and-schema-version-sf5); consistent
with ADR 0013). A row whose `lease_epoch` column is `NULL` is a **legacy** row (see
[§12](#12-legacy-cutover-oq5-da-1)); `NULL` is never conflated with `0`.

The **occupant-null release** branch (`release_lease ... WHERE address=? AND (occupant=?
OR occupant IS NULL)`, verified in `sqlite.rs`/`postgres.rs`) is **removed**: release is
strictly epoch- and owner-guarded (see [§11.2](#112-epoch-guarded-heartbeat-non-deleting-release-and-self-demotion-mr2-mr3)).

## 6. IPC protocol

A **daemon-scoped**, versioned, length-or-line-framed control protocol. Serialization is
**JSON, one object per line** (`serde` / `serde_json`), extending the current
`src/ipc.rs` framing. The protocol is intended to be reusable by the embeddable SDK
client (#12) — it is a stable Layer-1 surface.

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

This is a stable Layer-1 surface for the plugin and the #12 SDK. A normative **IPC
compatibility table** — `protocol_version`, minimum daemon/client, each capability's
required-vs-optional status, unknown-field/unknown-op behavior, and the downgrade error
code — is frozen as part of `daemon-core` acceptance, with **N/N-1 and N+1/N tests** for
attach, wait reconnect/ReRegister, Drain, DeregisterSession, and Status.

### 6.2 Request / response frames

Requests (Layer-1 operations). Privileged requests carry `proof = admin_cap`
([§7.1](#71-scoped-capability-model-v1-one-instance-admin-token)); all keys that identify
a station carry `store_key` because one exchange serves multiple stores:

| Request | Purpose | Privileged? |
|---|---|---|
| `Hello` | version + capability handshake | no |
| `Register { store_key, address, session_id, occupant, description?, scope?, tags?, watch_pids[] }` | create/refresh a station (attach); establishes the session generation | no (same-trust) |
| `ReRegister { store_key, address, session_id, session_generation?, watch_pids[] }` | idempotent re-register after respawn/reconnect | no |
| `DeregisterSession { store_key, session_id, proof }` | drop a session's addresses for that store (healthy disconnect); tombstones the generation | **yes** |
| `Detach { store_key, address, session_id, proof }` | remove one station; tombstones | **yes** |
| `Wait { store_key, address, attention?, timeout_ms }` | block for one delivery (sessionless; not session presence) | no |
| `DeliveryAck { store_key, message_id }` | the waiter's post-flush delivery ack ([§11.3](#113-server-side-delivery-fence-mr1--at-least-once-preserving)) | no |
| `Status { store_key?, detail?, proof? }` | Status surface (detail requires proof) | detail: **yes** |
| `Takeover { store_key, address, proof }` | operator takeover of a stale address; tombstones prior | **yes** |
| `Drain { proof }` | quiesce + flush + ordered transfer/exit (upgrade/stop) | **yes** |

Responses:

| Response | Carries |
|---|---|
| `HelloAck` | protocol/daemon version, `auth_policy_version`, `required_capabilities`, accepted |
| `Registered` | `lease_epoch`, `owner_instance_id`, `session_generation`, `state` (`suspect`/`verified`/`lapsed`) |
| `Message` | `id, thread_id, parent_id, from_addr, to_addr, kind, attention, requires_disposition, subject, body, sent_at_ms, buffered_at_ms, lease_epoch` |
| `Keepalive` | `heartbeat_age_ms` |
| `Timeout` | — (idle-timeout) |
| `StatusReport` | the [§4](#4-status-surface-the-frozen-contract-shape) fields |
| `Ack` | generic success for Register/ReRegister/Detach/Deregister/Takeover/Drain/DeliveryAck |
| `Error` | `{ code, message }` — incl. `UnknownSession`, `NotOwner`, `Unauthorized`, `Incompatible`, `Ambiguous`, `Stale` (tombstoned generation) |

The `Message` frame **carries the `lease_epoch`** so a waiter can drop a frame from a
superseded epoch. Crucially, the daemon emits a `Message` frame **only after** the
server-side delivery fence authorizes it (see [§11.3](#113-server-side-delivery-fence-mr1--at-least-once-preserving)).

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
  later `sessionEnd` hook process are different processes ([§14.2](#142-deregistersession-proof-oq6)).

### 7.2 OS-level trust boundary (mr5)

The "user-private" property is enforced by the OS, made **normative** here (a predictable
endpoint name + a `0600` file alone do not stop another local user from connecting, and
data-bearing ops like `Wait → Message` body are otherwise readable):

- **Endpoint owner-only.** Windows: the named pipe is created with a **DACL granting the
  current user SID only** (no `Everyone`/`Authenticated Users`). Unix: the socket lives
  under an **owner-only `0700` run directory**.
- **Canonical, owner-private paths.** `config_root` and `run_dir` are **canonicalized**
  (symlinks resolved) and **rejected at startup if not owner-private** (not owner-owned, or
  group/world-accessible).
- **Cap/lock file safety.** `daemon-<H>.cap` and the spawn-lock/lockfile are created with
  **`O_NOFOLLOW` + exclusive create + atomic write-then-rename** and owner-only mode, so a
  pre-planted symlink or a hostile pre-existing file cannot redirect or capture them.
- **Peer authenticity.** Before sending `admin_cap` or any data-bearing frame
  (`Message`), the server verifies the **peer credential** (peer uid on Unix /
  `GetNamedPipeClientProcessId` → token SID on Windows) is the **same user**; the client
  likewise should verify the server peer. Connect-or-spawn ([§2.2](#22-auto-spawn-connect-or-spawn-and-the-spawn-lock)) must **not** trust an arbitrary first endpoint binder: a hostile pre-bind that wins the endpoint is rejected by the peer-credential check, and the daemon **spawns only the canonical executable** (a verified absolute path), never a relative/`PATH`-resolved name.
- **Negative tests** (acceptance): a second OS principal cannot `Hello`/`Register`/`Wait`;
  a symlinked cap/lock is rejected; a pre-bound hostile server is rejected by peer check.

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
session→address registry — the hook reads the singleton-scoped instance secret and
presents `(store_key, session_id, admin_cap)`; the daemon checks the secret and that
`(store_key, session_id)` is in its in-memory map.

## 8. (reserved)

*(Section intentionally folded into §5 and §14; numbering preserved for cross-refs.)*

## 9. Liveness model

Two paths, exactly as ratified (ADR-to-be 0017):

1. **Healthy disconnect = the sessionEnd hook.** Quit and dismiss both fire
   `session.ended`; the harness plugin calls `DeregisterSession(store_key, session_id, admin_cap)`,
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
| 1 | **sessionEnd hook** (clean quit/dismiss) | `DeregisterSession(store_key, session_id, admin_cap)` | immediate, addresses released |
| 2 | **watch-pid failure** — the typed predicate resolves dead per [§9.1](#91-typed-watch-pid-predicates-oq3) (no `anchor` pid survives, or any `required` pid is gone, or a start-time mismatch) | the daemon's local watcher issues an **internal `DeregisterSession`** for that session, **bypassing `occupied_stale`** | immediate |
| 3 | **operator takeover** | privileged `Takeover` (see [§10.2](#102-takeover-atomic-at-the-exchange-da-5)) | atomic re-bind |
| 4 | **daemon-down TTL** | lease lapses after the daemon-down window; respawn re-claims | backstop only |

`occupied_stale` is reserved for the **unobserved-death case only**: no hook fired *and*
no watch-pid signal is available (e.g. unhooked dismiss where the loader anchor survives).
That is the residual the next section governs.

## 10. Stale-attendance and takeover (no teardown)

### 10.1 `last_confirmed`, `occupied_stale`, and the hook-semantics split (OQ2, DA-6)

`attendance_last_confirmed_at` is refreshed by **positive, session-carrying presence
signals only**: `Register`, verified `ReRegister`, and any future positive resume/connect
hook (see [§16 OQ-γ](#open-question-resolutions)). It is **NOT** refreshed by the daemon's
own heartbeat (which updates `last_heartbeat` only —
[§11.2](#112-epoch-guarded-heartbeat-non-deleting-release-and-self-demotion-mr2-mr3)) and
**NOT** by a bare `Wait` connect (which is sessionless and proofless —
[§7.3](#73-no-intra-user-isolation-in-v1-mr6)). Either would let a continuously-heartbeating
daemon, or an unrelated same-user waiter, keep a dead-but-unhooked session permanently
fresh — defeating `occupied_stale`/takeover and reintroducing the zombie lease. **`sessionEnd`
does NOT refresh** either — it is a *removal* signal; a **failed** `sessionEnd` records a
`recent_error` and **leaves `last_confirmed` unchanged** (no refresh-then-fail reanimation).

`occupied_stale` is **derived**, not stored: `now - attendance_last_confirmed_at >
stale_after`, where `stale_after` is configurable (default a small multiple of the
heartbeat/lease window; the exact default is frozen in `daemon-core`). Both `now` and
`attendance_last_confirmed_at` are read from the **single backend/database-server clock
domain** ([§11.5](#115-postgres-cross-machine-reclaim-in-epochs-not-timing)) — never one
machine's local time compared against another's — and the clock source is injectable so
skewed-clock and suspend/resume cases are testable. It is surfaced in
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
recovery, upgrade handoff, and Postgres reclaim. The mechanism below is the same on
SQLite and Postgres; the per-backend conformance matrix is [§17](#17-gating-tests--per-backend-conformance-matrix-daemon-core-acceptance).

### 11.1 Epoch lifecycle (OQ1)

- **The epoch is a durable, monotonic, never-reused per-address high-water mark.** It
  only ever increases for an address — across claim, release, re-claim, handoff, crash,
  and reclaim. The waiter epoch-filter and "higher epoch wins" reclaim both depend on
  this, so it is a normative invariant, not a convention.
- **Claim is a compare-and-set that pins the observed epoch AND owner and increments the
  epoch in the backend** (not in the client), so two concurrent claimants cannot both
  win or skip a value. The normative claim statement, identical on both backends:

  ```sql
  UPDATE leases
     SET owner_instance_id = :me, occupant = :me,
         lease_epoch = lease_epoch + 1,
         last_heartbeat = :backend_now,
         attendance_last_confirmed_at = :backend_now
   WHERE address = :addr
     AND lease_epoch = :observed_epoch
     AND owner_instance_id IS NOT DISTINCT FROM :observed_owner
     AND (owner_instance_id IS NULL OR last_heartbeat < :stale_cutoff)   -- → rows: 0|1
  ```

  `0` rows = lost the race (re-read and retry, or report held-elsewhere). The increment
  is `lease_epoch + 1` evaluated by the backend, never a client-computed
  `:observed_epoch + 1`.
- **First-ever / legacy rows** (a row whose `lease_epoch` column is `NULL`, predating the
  epoch column) take a **separate, explicit** path
  (`... WHERE address = :addr AND lease_epoch IS NULL`) that sets `lease_epoch = 1`.
  **`NULL` is never conflated with `0`** in the normal claim predicate (see
  [§12](#12-legacy-cutover-oq5-da-1)).
- The winner's `owner_instance_id` is its stable instance identity for the daemon's life.

### 11.2 Epoch-guarded heartbeat, non-deleting release, and self-demotion (mr2, mr3)

**Heartbeat** is epoch+owner-guarded, returns a rowcount, and updates **lease-liveness
proof only** — it does **not** touch `attendance_last_confirmed_at` (which is
session-presence, refreshed only by Register / verified Re-register — see
[§10.1](#101-last_confirmed-occupied_stale-and-the-hook-semantics-split-oq2-da-6)).
Conflating the two would let the daemon's own continuous heartbeat keep a dead-but-
unhooked session permanently fresh, defeating `occupied_stale`/takeover and
reintroducing the zombie lease:

```sql
heartbeat: UPDATE leases SET last_heartbeat = :backend_now
            WHERE address=? AND lease_epoch=? AND owner_instance_id=?    -- → rows: 0|1
```

**Release does NOT delete the row.** Deleting would discard the only durable carrier of
`lease_epoch`, so a later claim would see no row and reset the epoch (7 → 1), breaking
monotonicity. Release clears ownership but **retains the epoch high-water**:

```sql
release:   UPDATE leases SET owner_instance_id = NULL, occupant = NULL
            WHERE address=? AND lease_epoch=? AND owner_instance_id=?    -- → rows: 0|1
```

Occupancy is derived from `owner_instance_id IS NOT NULL` (and not stale), **never from
row existence**. **Normative no-delete invariant:** no code path — release, detach,
cleanup, GC, test helper, or migration — may `DELETE` a lease row whose `lease_epoch`
matters; all of them tombstone (null the owner) and preserve the high-water epoch. (If
true row reclamation is ever needed, the high-water moves to a separate append-only
`address_epoch(address, epoch)` table; out of scope for v1, where unbounded retired-row
growth is acceptable at single-user scale — GC is issue #24.)

A **0-row heartbeat or release** means a higher epoch exists. The daemon **self-demotes**
for that address — and self-demotion means **stop emitting AND stop heartbeating
(relinquish the address)**, close its waiters, and drop the in-memory station. It must
not keep heartbeating (which would hold the lease fresh and starve a successor). (Today's
`heartbeat` returns `Result<()>` with no rowcount — verified `sqlite.rs:325-333` /
`postgres.rs:313-320` — so the rowcount-returning shape is a required backend-API change.)

### 11.3 Server-side delivery fence (mr1 — at-least-once preserving)

**The fence must preserve the ratified at-least-once contract (ADR 0011) — it must never
introduce message loss.** A delivery is durably recorded only **after** a waiter has
accepted it, never before:

```text
mark_delivered_if_current_owner(address, owner_instance_id, lease_epoch, message_id)
    -> Result<DeliveryOutcome>

DeliveryOutcome = Marked | AlreadyDelivered | NotOwner
```

The daemon, in a **per-address critical section**, for each undelivered message:

1. *(optimization only — not the fence)* if in-memory state already knows it is not the
   current owner, skip and self-demote.
2. **EMIT** `Frame::Message(M, lease_epoch)` to the waiter.
3. **AWAIT THE WAITER ACK.** The one-shot `telex wait` client reads the frame, prints and
   **flushes** `M` to its stdout, then sends a one-line `Ack(message_id)`. The ACK — not
   the socket write — is the commit signal. **"Delivered" means the wait client flushed
   `M` to its stdout boundary**, not merely that a frame entered the IPC buffer. (This
   strengthens ADR 0011, whose commit point was the bare frame-handoff; end-to-end
   application-consumption delivery would require a separate consumer-level ack and is
   out of scope.)
4. **MARK** via `mark_delivered_if_current_owner(...)` only after the ACK. Outcomes:
   - **`Marked`** → success; continue draining.
   - **`AlreadyDelivered`** → **success** (idempotent; another path/owner already recorded
     it); continue draining. *Not* fatal.
   - **`NotOwner`** → **fatal**: the daemon lost the epoch between emit and mark;
     **self-demote immediately** ([§11.2](#112-epoch-guarded-heartbeat-non-deleting-release-and-self-demotion-mr2-mr3)) and stop draining the rest of the backlog. `M` stays
     undelivered and the current owner redelivers it.

**Why this is at-least-once with no loss window:** any crash, pipe break, lost ACK, or
ownership rotation **after EMIT but before a successful MARK** leaves `M` undelivered in
`deliveries`, so the current owner redelivers it → a **duplicate**, never a loss. The
**only** thing that prevents a superseded owner from systematically re-delivering is the
epoch-guarded MARK returning `NotOwner` and forcing self-demotion (the in-memory check in
step 1 is just an optimization — it only proves the daemon has not yet *learned* it lost
ownership, never that it is still the owner). The at-least-once contract, stated
normatively: **`M` is delivered repeatedly until exactly one current-epoch owner records
a successful MARK; waiters/consumers dedupe by `message_id`.** The duplicate count is
bounded by the number of failed owners/handoffs, not "exactly one." The `lease_epoch` on
the frame is a **secondary** filter a waiter applies only **after** it has independently
learned a newer epoch (via reconnect/handshake); it is **not** a live defense against a
stale daemon — that defense is the server-side MARK plus self-demotion.

The corresponding [§17](#17-gating-tests--per-backend-conformance-matrix-daemon-core-acceptance) gating test asserts, across
crash-after-EMIT/before-ACK, crash-after-ACK/before-MARK, waiter-death-after-EMIT, and
ownership-rotation-after-EMIT, that every message reaches a waiter **at least once**
(never zero) and that a superseded owner stops after one `NotOwner`.

### 11.4 Ordered handoff = owner-directed atomic transfer (sf3)

A graceful handoff (coordinated upgrade/stop where a successor `S` exists) must not lapse
the lease, leave an ownerless window a third daemon could hijack, or double-deliver. The
predecessor `P` transfers ownership **directly to `S` in one guarded statement** — there
is no release-then-claim gap and no generic "claim from a live owner" path (either would
admit a hijack):

```text
quiesce  → P stops accepting new Wait/Register for the address; stops new drains
flush    → P completes in-flight EMIT→ACK→MARK critical sections
transfer → one atomic UPDATE: P@epoch E → S@epoch E+1
```

```sql
UPDATE leases
   SET owner_instance_id = :successor, occupant = :successor,
       lease_epoch = lease_epoch + 1,
       last_heartbeat = :backend_now, attendance_last_confirmed_at = :backend_now
 WHERE address = :addr AND lease_epoch = :E AND owner_instance_id = :predecessor  -- → rows: 0|1
```

Properties: no ownerless gap (`P@E → S@E+1` atomically); no third-party hijack (the owner
is never `NULL`/stale during the transfer, so a normal stale-claim cannot interpose);
monotonic (epoch increments once, in-SQL); concurrent transfers serialize on the row;
`P`'s later heartbeat/release/mark at `E` returns 0 rows so `P` self-demotes; any
`P`-emitted-but-unmarked message stays undelivered and `S` redelivers it. **Crash-based
handoff** (no live `S`, `P` dead/stale) is not a transfer — the successor uses the normal
stale-claim CAS ([§11.1](#111-epoch-lifecycle-oq1)); the **minimal upgrade floor**
([§16](#16-minimal-upgrade-floor)) uses non-deleting release + next-call stale-claim,
whose brief ownerless window is acceptable single-user (no competing claimant; messages
queue durably). A **per-step handoff crash matrix** (kill/signal after quiesce, after
flush, after transfer) is part of [§17](#17-gating-tests--per-backend-conformance-matrix-daemon-core-acceptance) and must
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
  | `occupant` | the legacy occupant | the daemon instance |

  via the explicit legacy CAS
  (`UPDATE ... SET lease_epoch=1, owner_instance_id=:me, occupant=:me WHERE address=:addr
  AND lease_epoch IS NULL`). `NULL` is **never** treated as `0` in the normal claim
  predicate ([§11.1](#111-epoch-lifecycle-oq1)); the legacy row gets its first epoch (`1`)
  exactly once. Thereafter the rowcount-returning epoch-guarded heartbeat/release and the
  non-deleting release apply.

**Cutover gating assertion (frozen):** *no `Frame::Message` from a non-epoch holder
reaches a recipient after the daemon's waiter binds.* This is exercised by a **dedicated
sixth gating test that starts a real legacy holder / non-epoch lease on both backends**
([§17](#17-gating-tests--per-backend-conformance-matrix-daemon-core-acceptance) test 6), since the prior five do not. Hard
cutover of existing sessions is acceptable (ratified).

> Preserved minority (design-foundation council): one reviewer held that occupant
> rotation alone is the cutover. Adopted the two-phase prove-unbound rule instead, because
> the legacy heartbeat does **not** return rowcount, cannot self-demote, and a stale
> heartbeat is not proof the waiter endpoint is unbound. Reopen if the Phase-1 prove-unbound
> step cannot be realized via the address-keyed IPC probe + process quiesce (i.e. it needs
> a new IPC verb), which would make this an architectural change rather than an in-place one.

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

### 14.1 The in-memory `(store_key, session_id) → addresses` authority

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

**Generations and tombstones (race-safety, independent of the trust model).** Each
`(store_key, session_id)` carries a monotonic **session generation**: `Register`
establishes/advances it; `DeregisterSession`/`Detach`/`Takeover` **tombstone** the removed
`(store_key, session_id, address)` at the current generation. A `ReRegister` carrying an
**older-or-tombstoned generation is rejected** with `Stale` and does **not** resurrect the
address — closing the race where a stale blocked `Wait`'s EOF-reconnect `ReRegister`
reorders around a legitimate removal and revives a dropped address. A genuinely new
session re-uses the address only by a fresh `Register` (new generation), which is the
intended path.

Operations (idempotent): `Register`, `ReRegister`, `DeregisterSession(store_key,
session_id, proof)`, all keyed by `(store_key, session_id)`.

### 14.2 `DeregisterSession` proof (OQ6)

The sessionEnd hook runs as a **separately spawned process** (verified
`integrations/copilot-cli/hooks.json` runs `telex session-end`; `session_end.rs` reads
only a session id). It cannot inherit a secret minted in the earlier `attach`/loader
process's memory. So the proof in v1 is the **instance `admin_cap`**, read from the
singleton-scoped user-private `<run_dir>/daemon-<H>.cap` ([§7.1](#71-scoped-capability-model-v1-one-instance-admin-token)):
the hook presents `DeregisterSession(store_key, session_id, admin_cap)`; the daemon verifies the
secret and that `(store_key, session_id)` is in its map, then drops the addresses. **No external
session→address registry**, no per-session cap minted at Register in v1 (the
`per_session_cap` field is reserved for a future intra-user-isolation threat model).

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
**idempotent**: concurrent waits for one `(store_key, session_id)` converge to a single
map entry by **union of their non-tombstoned address sets at the current generation** (so
a multi-address session is never narrowed by one re-register), while a tombstoned address
is **not** unioned back in (it returns `Stale` — [§14.1](#141-the-in-memory-store_key-session_id--addresses-authority)). `daemon-core` MAY freeze an alternative single rule, but
generation-guarded union is the default.

### 14.5 Daemon-down and the TTL backstop

If the daemon is down, its leases lapse after the **daemon-down TTL window** (the one
surviving role of the old TTL heartbeat) and/or are fenced by the respawned daemon's
higher epoch. A session that **ends while the daemon is down**: its `sessionEnd` hook
no-ops against a down daemon (recorded as a transient on the harness side, not fatal),
and the address is recovered on respawn via `suspect`→`lapsed` (TTL) or operator
takeover — **no permanent zombie**.

### 14.6 `ResolveFrom`, send/reply recovery, and presence between waits (mr6, mr7)

`from`-resolution and the continuity of presence depend on the authority map, which is
in-memory and rebuilt by re-register — so the design must say what happens when a send or
reply needs it while the map is empty/`suspect`:

- **`ResolveFrom(store_key, session_id)`** resolves `from` against *that* session's
  registered addresses for *that* store only (never across sessions or stores): exactly
  one → succeed; multiple → `Ambiguous`; none → the recovery below.
- **send/reply on an empty/`suspect` map** (e.g. a crash mid-turn with no active `wait` to
  have re-registered) MUST first **opportunistically `ReRegister`** from inherited env
  (`store_key`, `TELEX_SESSION_ID`, watch-pids) and retry `ResolveFrom`; if it still
  resolves nothing, it **fails actionably** (`refused-unrepliable`, as ADR 0010) — never a
  silent `from = None`. This closes the reintroduced ADR 0010 foot-gun.
- **Presence between `wait` calls (mr7).** Only a blocked `wait` continuously re-proves a
  session; a foreground agent mid-task (between waits) during a `drain`/respawn would
  otherwise lose verified attendance and default-`from` until its next wait. To bound this,
  **`send`/`reply`/`ack` also opportunistically `ReRegister`** (cheap, idempotent,
  generation-guarded), so any agent action re-proves presence — not only `wait`. The
  continuous-occupancy claim ([§3.3](#33-wait-reconnect-on-eof-grace), DESIGN.md) is
  qualified accordingly: presence is continuous for a session that takes *any* telex action
  across the handoff; a fully idle session (no wait, no send) between drain and its next
  action is `suspect` until it next acts, which is acceptable (it is not actively
  transacting).

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
| 4 | **Delivery at-least-once failpoints** (mr1) — crash-after-EMIT/before-ACK, crash-after-ACK/before-MARK, waiter-death-after-EMIT, ownership-rotation-after-EMIT | required | required | every message reaches a waiter **>=1** time (never 0); `NotOwner` makes the superseded owner stop after one; dedupe by `message_id` |
| 5 | **Intra-daemon takeover local-eviction** | required | required | old-occupant `wait` gets a defined disconnect (not a hang); new-occupant `wait` gets subsequent messages; no both-owned intermediate state |
| 6 | **Real legacy holder / non-epoch cutover** (mr4) — start an actual legacy holder + `lease_epoch IS NULL` row | required | required | Phase-1 prove-unbound holds; **no `Frame::Message` from the non-epoch holder reaches a recipient after the daemon binds**; legacy row advances `NULL → 1` |
| 7 | **Epoch monotonicity across release/cleanup/re-claim** (mr2) | required | required | after release at epoch E and a cleanup pass, the next claim is **E+1 (never 1)**; no row deletion of an epoch-bearing address |
| 8 | **Unhooked dismiss + loader survives** (mr3) | required | required | the daemon's own heartbeat does **not** refresh `attendance_last_confirmed_at`; `occupied_stale` becomes true and takeover is offered; the live-but-idle session is **not** torn down |
| 9 | **Ordered-handoff crash matrix** (sf3) — kill after quiesce / after flush / after transfer | required | required | bounded idempotent recovery; no loss; no duplicate beyond at-least-once; no ownerless hijack window |
| 10 | **OS trust boundary negatives** (mr5) | required (Unix 0700 socket / symlink) | required | a second OS principal cannot `Hello`/`Register`/`Wait`; symlinked cap/lock rejected; pre-bound hostile server rejected by peer check; non-owner-private `config_root`/`run_dir` rejected at startup |
| 11 | **IPC version/capability compatibility** (sf2) — N/N-1 and N+1/N | required | required | security-sensitive `required_capabilities` mismatch fails closed (`Incompatible`/`Unauthorized`); attach/wait-reconnect/Drain/Deregister/Status behave per the compatibility table |
| 12 | **N / N+1 protocol-major parallel** (mr8) | required | required | two protocol-major-parallel daemons under one config root each authenticate against their own `daemon-<H>.cap`; neither clobbers the other |

Tests 1–5 are the original five (4 and 5 strengthened); 6–12 are added by this review
round. `fencing-proof` owns 3/4/6/7/9 on Postgres; `postgres-parity` owns the cross-machine
axis of 3.

## Open-question resolutions

The eight open questions carried into `design-foundation`, resolved with implementable
specifics (cross-referenced to the sections above):

| OQ | Question | Resolution | Where |
|---|---|---|---|
| **1** | Epoch lifecycle | Monotonic, never-reused `lease_epoch` + `owner_instance_id`; pinned-epoch+owner **CAS that increments in-SQL** (NULL≠0); **non-deleting release** retains the epoch high-water (no-delete invariant); rowcount-guarded heartbeat (lease-liveness only) + self-demote that **stops heartbeating**; server-side delivery fence = **EMIT → waiter-ACK → epoch-guarded MARK** (at-least-once preserving, `NotOwner` fatal, `AlreadyDelivered` success); ordered handoff = **owner-directed atomic transfer**; Postgres reclaim in epochs with a single backend clock domain. | [§11](#11-lease-epoch-fence-the-spine) |
| **2** | Stale-attendance threshold + takeover (no teardown) | `last_confirmed` refreshed by positive **session-carrying** presence only (NOT heartbeat, NOT bare Wait, NOT sessionEnd); `occupied_stale` derived from `stale_after` on a single backend clock domain; never tears down; takeover atomic at the exchange (mint epoch + evict map + close waiters + bind), allowed once stale. | [§10](#10-stale-attendance-and-takeover-no-teardown) |
| **3** | Typed `--watch-pid` shape | `anchor` (any-sufficient) vs `required` (all-necessary) + pid+start-time reuse guard; v1 floor = loader anchor + start-time; expose flags only with a real consumer; dismissal-path matrix routes positive death to immediate teardown. | [§9.1](#91-typed-watch-pid-predicates-oq3), [§9.3](#93-dismissal-path-matrix-the-four-disjoint-cases) |
| **4** | Distinct per-session PID? | **No usable one** on Copilot CLI today (empirically grounded: inner worker pid not env-exposed and lazily spawned; ppid-walk rejected). Loader anchor + start-time is the sole env-sourced backstop; hook + stale-attendance/takeover carry the rest. | [§9.2](#92-per-session-pid-on-copilot-cli-oq4--resolved-none-usable) |
| **5** | Legacy / non-epoch cutover | **Two-phase, prove-unbound**: Phase 1 proves the legacy waiter endpoint is unbound (address-keyed IPC probe + quit, or process quiesce) — a stale-window alone is insufficient; Phase 2 claims `NULL → 1` via the explicit legacy CAS. Frozen cutover assertion + a dedicated legacy-holder gating test on both backends. | [§12](#12-legacy-cutover-oq5-da-1) |
| **6** | DeregisterSession proof (no external registry) | Instance `admin_cap` from the **singleton-scoped** user-private `daemon-<H>.cap`; OS owner-only enforcement + peer-credential check ([§7.2](#72-os-level-trust-boundary-mr5)); hook presents `(store_key, session_id, admin_cap)`; daemon verifies secret + `(store_key, session_id)` map membership. v1 = same-user trust, **no intra-user isolation** (documented); per-session cap reserved/deferred. | [§7](#7-authorization-and-the-trust-boundary), [§14.2](#142-deregistersession-proof-oq6) |
| **7** | Status freeze line | Freeze the **field set + meaning** + the gating tests' observable assertions; `daemon-core` owns rendering/format/verbosity. | [§4](#4-status-surface-the-frozen-contract-shape), [§17](#17-gating-tests--per-backend-conformance-matrix-daemon-core-acceptance) |
| **8** | Attendance durability across daemon crash | Durable = lease rows (incl. epoch/owner/last_confirmed) + delivery buffer; rebuilt-by-client = in-memory `(store_key, session_id)` map + watch-pids + IPC waiters. `suspect`/`verified`/`lapsed` recovery; `wait` (and send/reply/ack) auto-Re-register; generation/tombstone race-safety; daemon-down TTL backstop; no permanent zombie. | [§14.3](#143-crash-recovery-suspect--verified--lapsed-oq8-da-3), [§14.4](#144-wait-auto-re-register) |

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
| **0010** (local holder registry as `from`-default source) | **Superseded** as the `from` source: daemon-era `ResolveFrom(TELEX_SESSION_ID)` against the session's registered addresses ([§14.1](#141-the-in-memory-store_key-session_id--addresses-authority); DA-9). Never infer across sessions; harness propagates `TELEX_SESSION_ID` to `send`/`reply`. |
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
- The `wait` auto-Re-register path ([§14.4](#144-wait-auto-re-register)) cannot be
  implemented because the chosen IPC transport masks socket-EOF — would force a
  positive-presence heartbeat from `wait`.
- The single-source SKILL mechanism ([§15.2](#152-single-source-skill--plugin-skill-mechanism-oq-for-deliverable-7-da-10))
  hits a harness constraint (manifest cannot point outside the plugin dir **and** `exec`
  is rejected) — would force a code-touching deviation.
