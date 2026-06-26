# Telex Daemon — Normative Contract (the local exchange)

## Status

**Normative design specification.** This document is the contract that `daemon-core`
and the downstream local-daemon nodes implement against. It is the authority for the
local-daemon architecture: the IPC/membership protocol, the authorization model, the
server-side lease-epoch fence, the lifecycle contract and Status surface, session
identity and explicit membership, the liveness model, and the minimal upgrade floor. Where this
document and prose in [DESIGN.md](DESIGN.md) differ on a mechanism, **this document
governs the mechanism** and DESIGN.md governs the framing.

The decisions behind this contract are recorded in [DECISIONS.md](DECISIONS.md) as ADRs.
Design assumptions and the conditions under which a decision should be revisited are in the
[Design assumptions and revisit conditions](#design-assumptions-and-revisit-conditions)
section.

This is a **design** document. It specifies contracts and invariants; it does not ship
code. Concrete struct/SQL/wire shapes below are normative *shapes* (names, fields,
types, ordering invariants), not final source.

## 1. The local exchange

The **local exchange** is one auto-spawned, single-instance, per-user supervised daemon that
owns presence and transport for a user's telex addresses. In the telex metaphor it is the
switching center that connects **stations** to **telex numbers (addresses)**: it owns the
backend connection(s), the poll/LISTEN-NOTIFY loop, the durable delivery buffer, the attendance
registry, the lease heartbeat (the single writer of liveness), the IPC endpoint, and pid-watch.
It keeps two distinct jobs — *presence/liveness* ("address A is attended by a live agent") and
*delivery transport* (poll/buffer/push one message to a waiter) — in one supervised place rather
than bound to an ephemeral per-session process.

Sessions do **not** run a resident process; they perform **one-shot** verbs against the
exchange. A **station** is **a registration in the local exchange**: the durable lease row plus
the in-exchange attendance record that says "this session attends this address." `attach`
creates the registration; `wait` blocks for one delivery against the exchange; `detach` removes
the registration. (See [Verbs](#15-verbs-cli-mapping-and-the-single-source-skill).)

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

Presence — "is address A *actually* attended by a live agent?" — is the one question the
exchange cannot answer for free; it concentrates the answer in one place and makes it
recoverable (the authoritative sessionEnd hook + a negative-only loader-pid watch + an
idle-TTL backstop). Crucially, liveness in this model is a **non-destructive UX dial** (release
waiters + mark a station idle), never a correctness gate: a station's membership and durable
message buffer persist regardless, so a wrong liveness call costs at most one waiter re-arm,
never data loss.

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

On Windows the endpoint is a named-pipe kernel object, not a path under `run_dir`; `run_dir`
stores authority artifacts such as `daemon-<H>.cap` but does not define the rendezvous. On Unix
the socket is under `run_dir`, so the Unix default is load-bearing for singleton rendezvous
compatibility and is intentionally not "aligned" with the Windows local-app-data default.

### 2.2 Auto-spawn (connect-or-spawn) and the spawn-lock

`attach` performs **connect-or-spawn**. Other one-shot verbs (`wait`, `send`, `reply`, `ack`,
`detach`, `station stop`) connect to an existing daemon and fail clearly if it is not running; the
agent then re-runs `attach`, which is the only normal respawn/recovery verb. This prevents a
detached waiter launched with the wrong environment/profile from silently creating a parallel
singleton. The attach path is:

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
recovery pass complete (see [§14.3](#143-crash-recovery-and-re-attach)), and the
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
  flush in-flight EMIT→ACK→MARK, hand off owned epochs in order (**Postgres**: live owner-directed
  transfer; **SQLite**: release + next-call respawn — [§11.4](#114-ordered-handoff--owner-directed-atomic-transfer-sf3)), then exit.

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
| `5` | **presence ended** — the exchange **reaped** this blocked `wait` (a `PresenceEnded` frame: sessionEnd hook, loader-pid death, **or the idle-TTL backstop** — [§9](#9-liveness-model)/[§10](#10-reaping-and-the-idle-ttl-backstop)). **Non-destructive**: the station persists; a still-live agent **re-attaches + re-waits** (handled like reconnect-on-EOF), and a new message still wakes it. |

One-shot verbs (`attach`/`detach`/`send`/`reply`/`status`) return `0` on success and a
documented non-zero on a daemon-down or protocol error; the exact non-zero set is frozen
in `daemon-core` acceptance.

#### 3.2.1 `--out-dir` outcome artifacts (detached delivery)

`telex wait --out-dir <DIR>` additionally persists the outcome to files in `<DIR>`, so a
**detached, variable-free** invocation can deliver both the message and the terminal outcome
to an agent that cannot capture the detached process's stdout or real exit code (e.g. Copilot
CLI on Windows, where the detached shell wrapper string-interpolates the command — stripping
`$variables` — and reports only the launcher/wrapper exit code). stdout/stderr behaviour is
unchanged; the files are purely additive:

| File | When | Contents |
|---|---|---|
| `message.json` | exit `0` only | the delivered message (same object printed to stdout) |
| `status.json` | always | `{ outcome, exit_code, detail, address, written_at_ms }` |
| `exit.code` | always | the integer exit code, written **last** as the completion marker |
| `wait.pid` | startup | the waiter process id, written before blocking |

`exit.code` is written after the other files (each via a sibling temp-file + rename), so a
reader that observes `exit.code` can treat all artifacts as fully written. On reuse of a `<DIR>`
across re-arms, a non-delivery outcome removes any prior `message.json` so a stale payload cannot
linger. Because `message.json` may contain the message body, artifacts are owner-only on Unix
(directory `0700`, files `0600`); Windows local app data / `%TEMP%` are already per-user. The
agent waits for
the detached completion notification, reads `exit.code` (then `message.json` on `0`), and
re-arms a fresh wait — it never trusts the runtime's reported detached exit code. This keeps
the stdout flush as pure transport: the file artifacts are likewise transport-only and are
**not** the consumed mark, which still fires only on the explicit agent `ack`
([§11.3](#113-server-side-delivery-fence-mr1--at-least-once-preserving)). See `SKILL.md`
("Copilot CLI detached waiter pattern") and ADR 0026.

#### 3.2.2 `--min-attention` threshold waits (two-phase attention loop)

`telex wait --min-attention <LEVEL>` delivers only messages whose attention is at least that
priority. Priority order is `interrupt` > `next-checkpoint` > `background` > `fyi`; the threshold is
inclusive (`next-checkpoint` wakes for `interrupt` and `next-checkpoint`). Bare `telex wait` remains
back-compatible and wakes on any pending message. Filtering changes eligibility only: among eligible
messages, delivery remains oldest-first by message id, and skipped lower-priority messages stay
durably buffered until a later checkpoint drain or a lower-threshold/bare wait.

The intended focused-work pattern is two-phase: arm a single detached wait with
`--min-attention interrupt` while doing foreground work, then at a safe checkpoint inspect and
ack/disposition the buffered lower-priority messages via `inbox`/`read`. Because only one live waiter
is accepted per station, an agent switches modes by letting the current waiter complete or using
`station stop` + `attach`, then arming either another interrupt-only waiter or a bare wait if idle.

### 3.3 `wait` reconnect-on-EOF grace

A daemon **restart or handoff is not a turn failure when a replacement daemon already exists**.
When `wait` is blocked and the connection drops (EOF / broken pipe), `wait` MUST, within a short
**reconnect grace window**, (a) connect to an **existing** daemon (it does not spawn), (b) re-issue
its `Wait`; if the exchange has no in-memory membership for the session/address (a restart cleared
it), the `Wait` returns **`NeedsAttach`** and `wait` **explicitly re-attaches** (`Register`) the
address it was waiting on from inherited env (see [§14.4](#144-wait-and-re-attach-on-needsattach)),
then (c) resumes blocking. If no daemon is running, `wait` returns exit `3`; the agent runs
`attach` (the spawning verb) and then re-arms `wait`. Any message that arrived while the daemon was
down was durably buffered and is delivered at-least-once once the session re-attaches and waits.
This keeps detached waiters from creating a daemon in the wrong environment/profile while preserving
transparent handoff when another action has already spawned the replacement daemon.

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
  consistent with ADR 0013). The hard part is a **binary too old to read
  `telex_schema_version`** — a pure policy ("too-old fails closed") is not
  self-enforcing, since such a binary would open the store and run its older lease write
  paths, writing incompatible (non-epoch) rows and corrupting the fence. The barrier is
  therefore made enforceable by an **external gate an incompatible binary cannot bypass**:
  - **Mandatory (R3-S2): a store-level schema change that makes incompatible write paths
    hard-fail before they touch lease rows.** Because a launcher/shim is bypassable by
    invoking an older binary **directly**, and an additive-only migration (`CREATE TABLE IF NOT
    EXISTS` + additive columns) leaves an older binary's lease write
    paths usable, only a **store-level hard-fail** is non-bypassable. The migration therefore
    **renames/constrains** the lease columns an older binary writes (or adds a
    `CHECK`/`NOT NULL` those writes violate) so a directly-invoked incompatible binary **errors out**
    instead of silently writing a non-epoch row. This is **required** in v1, not optional.
  - **Additional defense: a launcher/store lock** (the versioned shim refuses to exec a
    binary older than the store's `telex_schema_version` before the binary ever opens the
    store). It hardens the common path but does **not** replace the mandatory store-level
    hard-fail, since direct invocation bypasses the shim.

  The migration that advances `telex_schema_version` is performed by the **first
  daemon-aware claimant** under a **per-store exclusive lock/transaction**, **before** any
  epoch-column writes, and is **crash-safe** (re-runnable; a partial migration is detected
  and completed/rolled back on the next open). A **downgrade/migration gate test on both
  backends** is acceptance, and it MUST exercise an incompatible (older-schema) binary invoked **directly**
  (not only via the shim) against a migrated store, asserting it cannot write a non-epoch
  row. The full migration/downgrade *framework* for a populated
  multi-store Postgres deployment stays `seamless-upgrade` scope; v1 freezes the version
  field, the executable barrier, and the migration's exclusivity/crash-safety.

## 4. Status surface (the frozen contract shape)

`telex daemon status` (and a per-store `telex status` projection) exposes a **bounded,
actionable** surface. **`design-foundation` freezes the field set and meaning** (below);
`daemon-core` acceptance owns the exact rendering/format, verbosity, and any extra
diagnostics. This is the **Status freeze line**: *frozen = the
fields + their meaning + the gating tests' per-test observable assertions; not frozen =
wire format, ordering, additional diagnostics.*

Frozen Status fields:

- **`protocol_version`**, **`daemon_version`**, **`instance_id`** (the
  `owner_instance_id` this daemon uses), **`singleton_key`** (user/config-root/proto-major,
  redacted as needed).
- **`epoch_by_address`** — for each owned address: `lease_epoch`, `owner_instance_id`,
  `idle` (bool — no waiter currently attended/blocked).
- **`members`** — for each in-memory membership record: `address`, `session_id` (opaque),
  `occupant`, `waiters` (count of blocked waiters), `live_waiters` (pid/start-time/alive,
  attention, timeout), `pending_unconsumed_count`, `station_health`
  (`armed` / `recently_delivered` / `unattended` / `unattended_with_backlog` / `idle`), and
  `watch_pids` (pid + role + **alive**) so a live-but-quiet station is distinguishable from an
  unattended one with queued work,
  `backend`/`store_key`, `host`. (Membership is in-memory and explicit-only — see
  [§14.1](#141-identity-and-in-memory-membership) — so this set is empty for sessions that
  have not (re-)attached since the last daemon start.)
- **`live_waiters`** — the top-level live waiter registry, keyed by daemon-assigned
  `waiter_id`, including waiters that are in the small teardown interval between
  membership release and process exit. At most one live waiter is accepted for a
  `(store_key, session_id, address)` station; a concurrent second waiter returns
  `PresenceEnded` so it cannot receive a duplicate copy of the same unacked message.
- **`backoff`** — current backend reconnect/backoff/crashloop state.
- **`recent_errors`** — a bounded ring of recent actionable errors (e.g. `NeedsAttach`
  responses, operator-reset audit events with prior occupant, `NotOwner` self-demotions,
  idle-TTL reaps, backend disconnects), each with a timestamp.
- **`retention`** — per store, the durable **message/ack buffer row count**, with a
  **warn flag** when it crosses the frozen v1 budget.
- **`stores`** — the set of stores this exchange currently serves.

## 5. Membership model and record shapes

The exchange maintains one **in-memory membership record** (a *station*) per address a
session attends. Membership is **explicit-only** (established by `Register`/`attach`) and
**in-memory** — it is **not** rebuilt from durable history on respawn (see
[§14.1](#141-identity-and-in-memory-membership)). The **durable** layer holds only two
things: the per-address **lease-ownership row** (the epoch fence, [§11](#11-lease-epoch-fence-the-spine))
and the **message/ack buffer** ([§13](#13-delivery-and-seen-dedup)).

```text
MemberRecord {                  // in-memory only; lost on daemon restart, rebuilt by explicit re-attach
  address:                      String,        // the telex number
  store_key:                    StoreKey,       // effective store identity (profiles::store_key); part of the membership key
  session_id:                   String,         // opaque, unique, stable; the attending session (the identity — see §14.1)
  occupant:                     String,         // human/host label of the occupant
  waiters:                      Vec<Waiter>,   // detached blocked waiters for this address
  watch_pids:                   Vec<WatchPid>,  // negative-only liveness backstop (see §9)
  host:                         String,
}

WatchPid { pid: u32, start_time: u64, role: Anchor | Required }
```

`owner_instance_id IS NULL` marks a **released-but-epoch-retained** row (delivery ownership
is `owner_instance_id IS NOT NULL` and not stale, never row existence — see
[§11.2](#112-epoch-guarded-heartbeat-non-deleting-releaseownership-and-self-demotion-mr2-mr3)).
The `MemberRecord` is **never persisted**: it lives only in the serving daemon's memory and
is recreated solely by an explicit `Register` ([§14.1](#141-identity-and-in-memory-membership)).

### 5.1 Durable lease-row columns (new)

The durable layer holds **lease-ownership (the epoch fence) and the message/ack buffer
only** — never membership. The backend `leases` table is keyed by `address` and carries the
ownership/fence columns:

- **`lease_epoch INTEGER`** — the monotonic, never-reused fence token (retained across
  release; see [§11.2](#112-epoch-guarded-heartbeat-non-deleting-releaseownership-and-self-demotion-mr2-mr3)).
  It arbitrates **delivery ownership** and is the active fence on the multi-writer Postgres
  backend ([§11](#11-lease-epoch-fence-the-spine)).
- **`owner_instance_id TEXT`** — the owning daemon instance, `NULL` when released.
- **`last_heartbeat INTEGER`** — backend-clock ms lease-liveness proof (heartbeat-only).
- **`occupant`/`session_id`** — retained as the **lease-holder label** (which session, plus its
  human/host label, the lease is currently held for), written at `Register`. It is **only** a
  label: the exchange **never** uses it to rebuild the in-memory membership map after a restart
  (see below).

The durable **message/ack buffer** ([§13](#13-delivery-and-seen-dedup))
is the `deliveries(message_id, recipient, …)` table of ADRs 0011/0013, and **gains durable
per-message consumed state keyed by `(message_id, recipient)`** so the agent ack is
**idempotent**: an `Ack{address, message_id}` **updates the existing** `(message_id, recipient = address)` delivery row to consumed; a **missing** row returns a typed **`AckNoOp`** and **inserts nothing** (never fabricates a consumed row); a
replayed or duplicate ack is a no-op; an unacked message redelivers (at-least-once, dedup by
`message_id` — [§11.3](#113-server-side-delivery-fence-mr1--at-least-once-preserving)).
**Retention (S5, M1-r4):** the `deliveries(message_id, recipient)` rows are the **durable consumed
authority** and are **NOT pruned in v1** (consistent with ADR 0013's never-pruned `seen`): an
unacked row stays for redelivery, and an **acked (consumed) row is retained** — deleting it would
re-admit the message to the `fetch_undelivered` selector ([§13](#13-delivery-and-seen-dedup))
and **resurrect a consumed message**, so the safe per-recipient id-floor that would permit deletion
is the **deferred #24 GC** item. Only the **in-memory fast path** is bounded
(`max_in_flight_entries` / byte caps); durable buffer growth is the deferred #24. The latency
budget is a benchmark gate ([§17](#17-gating-tests--per-backend-conformance-matrix-daemon-core-acceptance) test 19).
**Fan-out (S1):** every `to`/`cc`/`watcher` recipient of a message expands into its own
`(message_id, recipient = address)` row, so delivery and the consumed mark are **strictly
per-recipient** — acking one recipient never consumes another.

**Membership is in-memory and explicit-only.** Although the lease row keeps an
`occupant`/`session_id` label, the **set of addresses a session attends** is the in-memory
`MemberRecord` set ([§5](#5-membership-model-and-record-shapes)) established only by `Register` —
the exchange **never** reverse-indexes the durable `session_id` to resurrect membership on
respawn. There is **no** durable `session_incarnation`, **no** `tombstoned_at`, **no** attendance
column on the lease row, and **no `sessions` currency table** — identity is the unique, stable
`session_id` ([§14.1](#141-identity-and-in-memory-membership)). When the exchange does not know a
session/address, the relevant op returns **`NeedsAttach`** and the agent explicitly re-attaches;
nothing is ever resurrected from durable history, so no tombstones are needed.

Greenfield: the new lease columns and the per-message consumed state are created **together
in one schema-version migration** via `CREATE TABLE IF NOT EXISTS` / additive column add,
gated by the store schema-version under the per-store exclusive lock
([§3.4](#34-per-store-isolation-and-schema-version-sf5); consistent with ADR 0013). A row
whose `lease_epoch` column is `NULL` is treated as an **unowned/foreign** row; **`NULL` is
never conflated with `0`**.

There is **no occupant-null release** branch (no `release_lease ... WHERE address=? AND
(occupant=? OR occupant IS NULL)`): release is
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
reconnect/re-attach, Drain, Detach, and Status.

### 6.2 Request / response frames

Requests (Layer-1 operations). Privileged requests carry `proof = admin_cap`
([§7.1](#71-scoped-capability-model-v1-one-instance-admin-token)); all keys that identify
a station carry `store_key` because one exchange serves multiple stores. Identity is the
unique, stable `session_id` ([§14.1](#141-identity-and-in-memory-membership)); there is **no
incarnation token**. Membership is **in-memory and explicit-only**: a `Register` (attach)
establishes it. When the exchange does not know a session/address (e.g. after a restart),
`Wait`/`Send`/`Reply`/`Ack` return a typed **`NeedsAttach`** error (terminal for that op) and
the agent explicitly re-attaches **the addresses it still wants** — a `NeedsAttach` after a
**deliberate `Detach`** is **terminal**: the agent does **not** auto-`Register` to resurrect a
station it intentionally dropped (for `Ack`, the message simply redelivers to a future attendee,
[§11.3](#113-server-side-delivery-fence-mr1--at-least-once-preserving)):

| Request | Purpose | Privileged? |
|---|---|---|
| `Hello` | version + capability handshake | no |
| `Register { store_key, address, session_id, occupant, description?, scope?, tags?, watch_pids[] }` | **explicit attach** — establishes the in-memory membership `(store_key, session_id) → address` and claims/renews the durable lease for the address. **Idempotent**: re-issuing it for an already-attended address is a no-op refresh. This is the **only** way membership is created; nothing implicit ever (re)creates it. | no (same-trust) |
| `Detach { store_key, session_id, address }` | drop one station — removes the in-memory membership entry and releases the address's waiters. Non-privileged (same-user trust): like every unprivileged op it carries **no per-session proof**, so **any same-user process can drop any same-user station** — the accepted v1 same-user-trust tradeoff ([§7.3](#73-no-intra-user-isolation-in-v1-mr6)), **not** a per-session authorization guarantee; nothing is tombstoned (a later explicit `Register` re-attaches if wanted). | no |
| `Wait { store_key, session_id, address, attention?, timeout_ms }` | block for one delivery against the address. If the exchange has no membership for `(store_key, session_id, address)`, returns **`NeedsAttach`** (the agent re-attaches then re-waits). Waiters are **detached** ([§9](#9-liveness-model)). | no |
| `Send { store_key, session_id, to_addr, … }` / `Reply { store_key, session_id, message_id, … }` | enqueue a message into the durable buffer. If the exchange does not know the sending session/address, returns **`NeedsAttach`** (the agent re-attaches its own address, then retries) — `from` is never silently `None` ([§14.6](#146-from-resolution-and-re-attach)). | no |
| `Ack { store_key, session_id, address, message_id }` | **explicit agent ack**, issued immediately on read — names the **`address` the message was delivered to** (the `recipient`), so the daemon validates the session attends `address` and writes the durable, epoch-guarded **MARK-consumed** for the exact `(message_id, recipient = address)` (idempotent). A session attends multiple addresses, so `address` is **required** to disambiguate per-recipient consumption (optionally carry the delivered `lease_epoch` as an audit input). The transport flush to stdout is **not** the consumed mark ([§11.3](#113-server-side-delivery-fence-mr1--at-least-once-preserving)). | no |
| `Status { store_key?, detail?, proof? }` | Status surface (detail requires proof) | detail: **yes** |
| `Reset { store_key, address, proof }` | operator **reset** of a station: release its waiters and mark it idle (non-destructive). Replaces the former force-takeover; it mints **no** epoch for session eviction and rotates **no** nonce — there is nothing to invalidate ([§10.2](#102-operator-reset)). | **yes** |
| `Drain { proof }` | quiesce + flush + ordered transfer/exit (upgrade/stop) | **yes** |

Responses:

| Response | Carries |
|---|---|
| `HelloAck` | protocol/daemon version, `auth_policy_version`, `required_capabilities`, accepted |
| `Registered` | `lease_epoch`, `owner_instance_id` (the attach succeeded; membership established) |
| `Message` | `id, thread_id, parent_id, from_addr, to_addr, delivered_to, primary_to, cc, delivery_role, kind, attention, requires_disposition, requires_disposition_for_current_recipient, subject, body, sent_at_ms, buffered_at_ms, lease_epoch` |
| `Keepalive` | `heartbeat_age_ms` |
| `Timeout` | — (idle-timeout) |
| `PresenceEnded` | the waiter-completion status the exchange writes when it reaps a blocked `Wait` (sessionEnd hook, loader-pid death, **or the idle-TTL backstop** — [§9](#9-liveness-model)/[§10](#10-reaping-and-the-idle-ttl-backstop)); non-destructive (the station survives and wakes on a new message) |
| `StatusReport` | the [§4](#4-status-surface-the-frozen-contract-shape) fields |
| `Ack` | generic success for `Register`/`Detach`/`Reset`/`Drain`; the **consume-`Ack`** carries the typed `DeliveryOutcome` (`Marked` / `AlreadyConsumed` / `AckNoOp` / `NotOwner`, [§11.3](#113-server-side-delivery-fence-mr1--at-least-once-preserving)) — a daemon that collapses the attended-but-never-delivered case to generic success fails tests 5/16 |
| `StationStopped` | station teardown summary: `store_key`, `session_id`, `address`, `detached`, `waiters_before`, `waiters_after`, remaining `live_waiters`, optional `message`/`lease_epoch` |
| `Error` | `{ code, message, … }` — incl. **`NeedsAttach`** (the exchange does not know this session/address — the agent must explicitly `Register` then retry; never an implicit rebuild), `NotOwner`, `Unauthorized`, `Incompatible`, `Ambiguous` |

The `Message` frame carries `lease_epoch` (the delivery-ownership fence —
[§11](#11-lease-epoch-fence-the-spine)). Delivery is **at-least-once**: the daemon EMITs the
frame, the waiter **prints** it to stdout (**transport only**), and the **agent** issues an
explicit `Ack{address, message_id}` ([§11.3](#113-server-side-delivery-fence-mr1--at-least-once-preserving))
which records the **durable, epoch-guarded MARK-consumed** for `(message_id, recipient)`. An
unacked message redelivers; consumers dedupe by `message_id`. There is **no** waiter
`DeliveryAck`, **no** per-emit `delivery_nonce`, and "delivered" is **not** the stdout flush —
the consumed commit is the explicit agent ack, decoupled from any EMIT-time connection.

CC recipients are visibility-only: their delivery rows are materialized as already seen/consumed for
transport, so they remain visible in `inbox --all` / `read` with `delivery_role: "cc"` but do not wake
`wait` and do not require manual `ack`. The primary `--to` recipient remains the actionable,
ack-required delivery.

## 7. Authorization and the trust boundary

### 7.0 v1 threat model (normative)

The v1 threat model is **same-user, user-private, single-user, pre-beta**:

- **Cross-user isolation is mandatory and enforced by the OS** ([§7.2](#72-os-level-trust-boundary-mr5)): a *different* OS user must not be able to connect to the endpoint, read the capability, `Wait` on an address (and read `Message` bodies), or claim a lease.
- **Intra-user isolation is explicitly NOT provided in v1** ([§7.3](#73-no-intra-user-isolation-in-v1-mr6)): every process of the *same* user is trusted. A same-user process may `Register`/`Wait`/`Ack` on any address, read its `Message` bodies, and attach. This is a deliberate, documented choice, not an omission; the reserved `per_session_cap` is the forward path to intra-user isolation.
- **Capacity/scale is single-user / pre-beta**: address counts, history sizes, and attendance sets are small. The multi-user / Streamliner performance and isolation concerns are explicit acceptance limits, not v1 requirements; revisit at beta / multi-user (see [§13](#13-delivery-and-seen-dedup) budgets).

Every IPC verb — including `Wait` and `Shutdown` — is authorized under the model below; none is unauthenticated.

### 7.1 Scoped capability model (v1: one instance-admin token)

- At spawn the daemon mints an **instance secret** (the `admin_cap`) and writes it to a
  **singleton-scoped, user-private file**: `<run_dir>/daemon-<H>.cap`, where
  `H = short_hash(user_SID, config_root, protocol_major)` — the **same singleton key** as
  the endpoint ([§2.1](#21-singleton-identity)). Scoping the cap path by `<H>` is required:
  a bare `<config_root>/daemon.cap` would be **shared by two protocol-major-parallel
  daemons** under one config root, and the last writer would invalidate the other
  instance's clients (its `Reset`/`Drain` would start
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
- **Unprivileged** requests — `Hello`, `Register`, `Detach`, `Wait`, `Send`, `Reply`, `Ack` —
  need **no** `admin_cap` (v1 same-user trust, no intra-user isolation,
  [§7.3](#73-no-intra-user-isolation-in-v1-mr6)): any same-user process may attend, drop a
  station, wait, send, or ack.
- **Privileged** requests — `Status { detail }`, `Reset`, `Drain` — carry `proof = admin_cap`;
  the daemon verifies it equals its instance secret. These are the operator/lifecycle actions
  (status introspection, station reset, drain/handoff), not per-session traffic.

**Authorization truth table (frozen — the single source, bound to
[§6.2](#62-request--response-frames), [§7.3](#73-no-intra-user-isolation-in-v1-mr6), and
[§17](#17-gating-tests--per-backend-conformance-matrix-daemon-core-acceptance) test 15):**

| IPC op | Auth |
|---|---|
| `Hello`, `Register`, `Detach`, `Wait`, `Send`, `Reply`, `Ack` | **unprivileged** (same-user) |
| `Status { detail }`, `Reset`, `Drain` | **privileged** (`admin_cap`) |

`Detach` is **unprivileged**: dropping your own station is a same-user action, identical in trust
terms to `Register`/`Wait` ([§7.3](#73-no-intra-user-isolation-in-v1-mr6)). Test 15 asserts the
with/without-`proof` compatibility for each row.
- The capability frame **reserves `scope`, `rotation`, and `per_session_cap: Option<Cap>`
  fields** (defined now, unused in v1) for future intra-user / lateral-compromise defense.
  A per-session cap is zero-marginal-value under the v1 same-trust model, and is **not
  feasible** while the minting (`Register` child) process and the later `sessionEnd` hook
  process are different processes ([§9](#9-liveness-model)).

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
- **Canonical path roles.** `config_root` is **identity-only** in v1: it is canonicalized
  and contributes to the singleton key, but it stores no `admin_cap`, socket, lock, or other
  authority material. If `config_root` ever stores authority material, it MUST become
  owner-private and fail closed like `run_dir`. `run_dir` is the **authority-bearing runtime
  directory** and is canonicalized (symlinks resolved) and **rejected at startup if not
  owner-private** (not owner-owned, or group/world-accessible).
- **Cap/lock file safety, as a readiness precondition (S3).** `daemon-<H>.cap` and the
  spawn-lock/lockfile are created with **`O_NOFOLLOW` + exclusive create + atomic
  write-then-rename** and owner-only mode, so a pre-planted symlink or hostile pre-existing
  file cannot redirect or capture them. **Owner-only cap creation is part of the readiness
  contract** ([§2.3](#23-readiness-ack)): if the cap cannot be created owner-only — `ENOSPC`,
  permission, partial write, symlink — **startup fails** (the daemon never serves without an
  enforceable cap). These failpoints are acceptance tests. *Where* these paths resolve from, and
  the **portability** of the owner-only requirement (the unattended environments that trip it, a
  recommended resolution policy, and a single-tenant opt-out), are `daemon-core` policy —
  [§7.4](#74-path-resolution-and-the-portability-of-fail-closed-startup-deferred-to-daemon-core).
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

`Wait`/`Register`/`Ack`/`Detach` carry no per-session proof, so **any same-user
process** can wait on an address (and read its `Message` bodies), attach, or ack. Under
[§7.0](#70-v1-threat-model-normative) this is **accepted** (same-user
trust). The consequences are made explicit so they are deliberate:

- A same-user `Wait` is **not** treated as session presence: it is pure transport and does
  **not** establish or refresh membership, so an unrelated waiter cannot mark a station present.
  Presence is the authoritative `sessionEnd` hook + the loader-pid negative signal
  ([§9](#9-liveness-model)); membership is established only by an explicit `Register`.
- Explicit-only membership and its race-safety (`store_key` keying) are specified
  in [§14](#14-session-identity-and-explicit-membership) and hold regardless of the trust model
  (they are correctness, not isolation).
- Intra-user isolation, when needed, is the reserved `per_session_cap` path (mint a
  per-`(store_key, session_id)` cap at `Register`, require it on `Wait`/`Ack`/
  `Detach`) — deferred.

This is the **sessionEnd-removal proof**: proof without an external
session→address registry — the hook presents the singleton-scoped instance secret in an
**authoritative but non-destructive** `SessionEndHook(store_key, session_id, admin_cap)` (no
incarnation — identity is the unique, stable `session_id`); the daemon checks the secret and that
`(store_key, session_id)` is in its map, then **releases that session's blocked waiters + marks
its stations IDLE** — it never destroys a station, and a late/spurious hook self-heals on the
next `Register` ([§9](#9-liveness-model)).

### 7.4 Path resolution and the portability of fail-closed startup (deferred to `daemon-core`)

[§7.2](#72-os-level-trust-boundary-mr5) freezes a **requirement** (`run_dir`
owner-private; the cap creatable owner-only; `config_root` identity-only unless it later stores
authority material) and a **behavior** (startup **fails closed** otherwise, as readiness
acceptance tests). It deliberately does **not** freeze *where* those paths resolve from, nor
*what to do on a filesystem that cannot represent owner-only permissions* — those are
**`daemon-core` policy**. This subsection records why the gap matters and the recommended
direction, so `daemon-core` starts from it rather than rediscovering it.

**Owner-only is an EFFECTIVE-permission postcondition, not just an explicit `0700`/DACL write.**
Creating the cap/dir with an explicit owner-only mode is **necessary but not sufficient** as the
final proof: on Windows DACL + inherited ACEs, POSIX-or-NFSv4 ACLs, SMB/9p, WSL/DrvFs, and other
translated filesystems, mode bits can round-trip while the **effective** access is broader or
**unknowable**. The readiness check is therefore an **effective permission / ACL / DACL
postcondition**: confirm the artifact is owner-only *in effect*, and classify any **ambiguous or
inconclusive** representation as **cannot-enforce → fail closed** (unless the explicit
single-tenant opt-out below is set).

**The failure surface is two independent trip-wires** ([§7.2](#72-os-level-trust-boundary-mr5)):
*(a)* the directory is **not owned by the running uid**, and *(b)* it is **(effectively)
group/world-accessible**. On a normal interactive install (local disk, `~/.config`/
`%LOCALAPPDATA%`, telex creating its own owner-only dir) neither fires and fail-closed startup is
invisible. The concern is that the **environments where an agent runs unattended are exactly the
ones that trip these**, and there the failure is **total** (telex will not run) and often
**unwatched** (no human reads the error):

- **Arbitrary-uid / non-root containers** (OpenShift, K8s `runAsNonRoot` random uid, uid-remapped
  mounts): the mounted dir is not owned by the running uid → trips *(a)*. A mainstream enterprise
  pattern, not exotic.
- **Network / remapped filesystems** (NFS root-squash, SMB/CIFS, **9p on WSL2 and Docker
  Desktop**, some CSI volumes): ownership/permission/ACL bits are not faithfully represented, so
  owner-only **cannot be set or verified** — distinct from *(a)*/*(b)*: the check is not *false*,
  it is *unenforceable / inconclusive*.
- **`$HOME`/`$XDG_RUNTIME_DIR` unset or shared** (cron, systemd units without `User=`, minimal CI
  shells, distroless): resolution can fall back to a world-writable location (`/tmp`, mode `1777`)
  → trips *(b)*.
- **Redirected / roaming profiles on Windows** (`%APPDATA%`/`%LOCALAPPDATA%` redirected to a
  network home drive): network-FS semantics + non-local DACLs can trip the owner-only check.
- **The umask footgun (an implementation-correctness item, not exotic).** If `daemon-core`
  creates its `run_dir` with `0755 & ~umask` rather than an **explicit `0700`**, a *normal* user
  with `umask 022` gets a world-readable dir that the §7.2 check rejects. Create with an explicit
  owner-only mode, independent of umask.

**Recommended `daemon-core` policy (direction, not frozen here):**

1. **Deterministic, documented, PLATFORM-SCOPED path resolution with an explicit override.** Do
   **not** copy a Unix order literally onto Windows (that is what strands redirected-profile
   users). Unix: an explicit `TELEX_RUN_DIR` / `--run-dir` → `$XDG_RUNTIME_DIR` → a private subtree
   under `$HOME` (e.g. `$HOME/.local/state/telex`). Windows: an explicit `TELEX_RUN_DIR` /
   `--run-dir` → a **local** `%LOCALAPPDATA%\telex\run` (never a redirected/roaming profile path by
   default). Each runtime directory is created/repaired **explicit owner-only** (`0700` /
   current-SID-only protected DACL), with a
   refuse-to-run error that **names the configured override** so the operator has an immediate
   remedy.
2. **Distinguish "cannot enforce owner-only" from "permission denied"** with a specific,
   **actionable** message (e.g. "`run_dir` is on a filesystem that cannot represent owner-only
   permissions — set `TELEX_RUN_DIR` to a local owner-private path or tmpfs") rather than a
   generic opaque failure. **Fail-closed actionability is part of the operability contract** even
   though the message text is `daemon-core`'s.
3. **Prefer `$XDG_RUNTIME_DIR` / tmpfs (Unix) or local `%LOCALAPPDATA%` (Windows) for the runtime
   artifacts** (socket, lockfile, cap) where available — owner-private and local by construction —
   which sidesteps the network-FS class for everything except the durable store. (The "lockfile"
   here is the per-config-root OS-singleton **spawn** lock; the **canonical-store** single-writer
   lock of [§11.3](#113-server-side-delivery-fence-mr1--at-least-once-preserving) deliberately does
   **not** live under the config-root-overridable `run_dir` — it uses a config-root-invariant
   per-user namespace keyed by the store file-id, so all aliases of one physical store map to one
   lock target.)
4. **The remedy is path-shaped first; the trust opt-out is a narrow last resort.** Steer an
   operator in a tripped environment to **`TELEX_RUN_DIR` / tmpfs on local owner-private storage
   FIRST**. Only if that is genuinely impossible, an **explicit, logged single-tenant opt-out**
   (e.g. `TELEX_TRUST_ENV=single-tenant`) relaxes the owner-only-*file* requirement — but it is
   **narrowly defined** (it asserts **no shared or host-mounted `run_dir`, socket, lock, or cap**,
   **not** a blanket "inside a container/VM": sidecars, shared volumes, hostPath/bind mounts, and
   WSL/Docker-Desktop shares can still expose the bearer-`admin_cap` path to another principal),
   **opt-in and audited, never a silent fallback**, and it **voids**
   [§7.0](#70-v1-threat-model-normative)/[§7.2](#72-os-level-trust-boundary-mr5) protections by
   design — a **builder/operator policy call**, not a default this design takes.

**Acceptance.** The runtime owner-private-rejection failpoint is already gated
([§17](#17-gating-tests--per-backend-conformance-matrix-daemon-core-acceptance) test 14); the
**cannot-enforce-owner-only** filesystem case and the **actionable-error** contract join it as
`daemon-core` acceptance, where the actionable error **names the configured run-dir override**
(`TELEX_RUN_DIR` is the recommended example, **not** a frozen knob — the resolution order and the
opt-out are `daemon-core`'s to fix). Recorded as **ADR 0022**, with the directory role taxonomy
and Windows local runtime default accepted in **ADR 0025**.

## 8. (reserved)

*(Section intentionally folded into §5 and §14; numbering preserved for cross-refs.)*

## 9. Liveness model

Liveness is a **non-destructive UX dial**, not a correctness gate. A station's membership and
its durable message buffer persist regardless of any liveness signal; all liveness does is
**reap blocked waiters** (complete a blocked `Wait` with a `PresenceEnded` status) and **mark
the station idle**. It **never destroys a station** — a session may idle for days and still
wake on a new message. Two signals drive reaping:

1. **The sessionEnd hook — AUTHORITATIVE but NON-DESTRUCTIVE.** Quit fires `session.ended`, and
   **dismiss is assumed to also fire it** (a clean-dismiss behavior to be confirmed by a spike —
   see [revisit conditions](#design-assumptions-and-revisit-conditions); if the
   spike shows dismiss does **not** fire the hook, the **idle-TTL becomes the primary dismiss
   bound** rather than a backstop, and the safety model is unchanged because reaping is
   non-destructive either way). The harness plugin sends
   `SessionEndHook(store_key, session_id, admin_cap)`.
   On receipt the exchange **releases that session's blocked waiters and marks its stations
   IDLE** — it **never** destroys a station. The hook is authoritative because identity is the
   unique, stable `session_id` ([§14.1](#141-identity-and-in-memory-membership)): there is no
   "which life?" ambiguity to resolve. Because the action is non-destructive and the
   `session_id` is unique, a **late or spurious hook is harmless** — it costs at most one waiter
   re-arm (the agent re-attaches and re-waits), never data loss. There is no latched,
   double-checked, or liveness-vetoed teardown, and the hook carries **no incarnation token**.
   The Copilot plugin is a thin mapper: `COPILOT_AGENT_SESSION_ID → $TELEX_SESSION_ID`.
2. **The watched LOADER pid — a NEGATIVE-only signal.** The exchange watches each station's
   `watch_pids` (the loader anchor + start-time, [§9.1](#91-typed-watch-pid-predicates-oq3)).
   **Loader death** releases that session's waiters and marks its stations idle (the same
   non-destructive reap). Loader-**alive** is **never** positive presence — a lingering loader
   after a dismiss does not keep a station "live"; it only means the negative signal has not
   fired. telex core names nothing harness-specific; the Copilot plugin maps
   `COPILOT_LOADER_PID` onto a generic `--watch-pid`.

A single **idle-TTL >= 1 day** is a non-destructive backstop ([§10](#10-reaping-and-the-idle-ttl-backstop))
for the one residual case (an unhooked dismiss whose loader pid survives). A waiter blocked past
the TTL with no delivery and no fresh agent action is **observationally identical** to a
live-idle session, so the backstop **MAY release a live-idle waiter** — but that is harmless: it
releases only the *waiter* (a `PresenceEnded`, exit `5`), the **station and buffer persist**, the
`wait` loop **re-arms** on `PresenceEnded`, and a new message still wakes the station. So the TTL
is a **non-destructive max-blocked-wait boundary**, not a cap on legitimate idle, and **no message
is lost** (at-least-once). There is **no** time-based **destruction** of a station, ever.

### 9.1 Typed watch-pid predicates (OQ3)

The singular `--session-pid` (issues #5/#17) generalizes to **typed predicates**:

- **`anchor`** — the station is alive if **any** anchor pid survives. (A loader/root pid
  is an anchor: if it is gone, the whole tree is gone.)
- **`required`** — the station is alive only if **all** required pids survive. (A
  specific per-session pid, where one exists, is required.)
- A **pid + start-time reuse guard** accompanies every watched pid: a pid is "alive" only
  if it is alive **and** its process start-time matches the captured start-time. A pid-only
  liveness check is insufficient — it admits a pid-reuse false-positive — so the start-time
  guard is required.

**v1 floor = a single loader `anchor` + start-time.** The `required`/`anchor` flag
surface is exposed **only where a real consumer/test exists**: in v1 the only populated
predicate is the loader anchor.

### 9.2 Loader-pid: the sufficient negative signal (OQ4)

Empirically grounded (live probe, Copilot CLI 1.0.64-1, Windows): the harness exposes
`COPILOT_AGENT_SESSION_ID` and `COPILOT_LOADER_PID`. `copilot.exe` is a **supervisor that
re-execs an identical-argv inner worker**; the inner worker's PID is **not** exposed as an
env var and spawns lazily. We **do not need** a per-session inner pid: liveness here is a
**non-destructive UX dial, not a correctness gate**, so a coarse cohort signal is exactly
right. The watched **loader pid (anchor + start-time)** is precisely the right and sufficient
**negative** signal — when the loader is gone the whole session tree is gone, so the exchange
reaps that session's waiters and marks its stations idle. Loader-**alive** is never positive
presence (a lingering loader after a dismiss is not "live"); the negative-only reading is what
makes loader-death a safe reap trigger and a surviving loader merely a "not-yet-fired" state.

The residual case — an **unhooked dismiss where the loader pid survives** — is bounded by the
idle-TTL backstop ([§10](#10-reaping-and-the-idle-ttl-backstop)), which releases only that
station's presumed-dead waiters (non-destructively). The ppid-walk is still rejected
(superseded ADR 0012: a reparented background launch makes ppid unsound).

### 9.3 Reaping-path matrix (the four non-destructive cases)

The exchange reaps a station's blocked waiters via exactly one of four paths. **Every path is
NON-DESTRUCTIVE**: it releases waiters and marks the station idle; it never destroys the
station or its durable buffer. The station survives and wakes on the next message.

| # | Trigger | Mechanism | Action |
|---|---|---|---|
| 1 | **sessionEnd hook** (clean quit; **dismiss pending the [§9](#9-liveness-model) spike**) | authoritative `SessionEndHook(store_key, session_id, admin_cap)` ([§9](#9-liveness-model)) | release that session's waiters + mark its stations idle (never destroy) |
| 2 | **loader-pid death** — the negative-only watch-pid predicate resolves dead per [§9.1](#91-typed-watch-pid-predicates-oq3) (no anchor pid survives, or a start-time mismatch) | the daemon's local watcher reaps that session | release waiters + mark idle |
| 3 | **idle-TTL backstop** (>= 1 day) | the unhooked-dismiss + loader-alive residual ([§10](#10-reaping-and-the-idle-ttl-backstop)) | release **presumed-dead** waiters + mark idle |
| 4 | **operator reset** | privileged `Reset` (see [§10.2](#102-operator-reset)) | release waiters + mark idle |

A reaped station is **idle, not gone**: its membership and durable message buffer remain, so a
new message wakes it (the next `Wait` re-arms a waiter). The next section governs reaping and
the idle-TTL backstop.

## 10. Reaping and the idle-TTL backstop

Reaping is **non-destructive**: on a definite liveness signal the exchange **releases a
session's blocked waiters and marks its stations IDLE** — it never destroys a station, never
evicts membership, and never touches the durable message buffer. A station's membership (the
in-memory `MemberRecord`, [§5](#5-membership-model-and-record-shapes)) and the durable buffer
persist **indefinitely**: a session may idle for days and still wake on a new message. There is
no `occupied_stale`/attendance-staleness machinery and no force-takeover — those existed only to
adjudicate weak-liveness and to invalidate incarnation tokens, both of which are gone.

**Idle-station resource budget (C1).** Because reaping is non-destructive, idle stations retain
their (in-memory) membership and their durable buffer indefinitely, so a long-lived daemon can
accumulate idle entries. `daemon-core` exposes a `Status` counter + a warning threshold for
accumulated idle stations and stress-tests synthetic idle stations before beta-scale expansion;
modest at v1 single-user scale.

### 10.1 Non-destructive reaping

A blocked `Wait` is reaped (completed with a `PresenceEnded` status —
[§6.2](#62-request--response-frames)) on a **definite signal**, and the station is marked idle:

- the **authoritative sessionEnd hook** (release the session's waiters + mark idle), or
- **loader-pid death** (the negative-only watch signal — [§9](#9-liveness-model)).

Reaping changes nothing durable: the lease row keeps its epoch high-water, the membership entry
remains, and any buffered-but-unacked message is still there. A later message wakes the station
(its next `Wait` re-arms a waiter, then prints and the agent acks). Because reaping is
non-destructive and `session_id` is unique, a **late or spurious signal is harmless** — at worst
one waiter re-arms.

**The idle-TTL >= 1 day backstop.** Exactly one residual escapes both definite signals: an
**unhooked dismiss whose loader pid survives** (no hook fired, loader-alive gives no negative
signal). A single **idle-TTL of at least one day** bounds it: a waiter that has been blocked with
**no delivery and no fresh agent action** for longer than the TTL is treated as **presumed dead**
and released (the station marked idle). The TTL is:

- **non-destructive** — it releases only a (presumed-dead) waiter as a `PresenceEnded` and marks
  the station idle; it never removes membership or the buffer;
- **a max-blocked-wait boundary, not a cap on legitimate idle** — because a live-idle waiter and
  an unhooked-dead waiter are **observationally identical**, the TTL **MAY** release a *live-idle*
  waiter; that is harmless because the `wait` loop treats `PresenceEnded` (exit `5`) as a
  **re-arm**, the station + buffer persist, and a new message still wakes the station (no loss,
  at-least-once). The TTL never *destroys* a station;
- a **UX/latency dial**, not a correctness gate — at-least-once delivery + explicit ack
  ([§11.3](#113-server-side-delivery-fence-mr1--at-least-once-preserving)) guarantee no message
  is lost regardless of when a waiter is released.

### 10.2 Operator reset

`Reset { store_key, address, proof }` ([§6.2](#62-request--response-frames)) is the operator
break-glass: it **releases the station's waiters and marks it idle** — the same non-destructive
action a definite signal would take, available on demand. It mints **no** new epoch for session
eviction and rotates **no** nonce: there is no incarnation token to invalidate, so there is
nothing to fence. (The lease-epoch still increments only on the genuine delivery-ownership
transitions of [§11](#11-lease-epoch-fence-the-spine) — claim/handoff/reclaim — never as a
session-eviction device.) Reset is **audited**: it emits an operator-audit `recent_error`/event
surfaced in `Status` with the prior occupant, so an operator action is never silent. A short
operator runbook (when reset is appropriate, and how to confirm the session is truly gone) is
part of the `daemon-core` operator docs.

## 11. Lease-epoch fence (the spine)

Keyed by `address` alone with no owner generation, a lease row could be written on
stall/crash/handoff/reclaim by a daemon that no longer owns it (duplicate delivery,
ownership flip-flop). The fence that prevents this is a **monotonic `lease_epoch` +
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
  (`owner_instance_id`, `lease_epoch`, `last_heartbeat`); it does **not** write `occupant`,
  which is set only by `Register`, so a bare recovery/stale-claim never forges occupancy.
  `:backend_now` is the **`BackendClock`** — a frozen contract with one
  backend-specific implementation each (**R3-Sb**), never a client-supplied local timestamp:
  on **Postgres** it is the true server clock (`now()`/`CURRENT_TIMESTAMP`, evaluated
  server-side so every writer across processes/machines shares one domain); on **SQLite**
  there is no server, so the single writer **is** the one daemon process — but `BackendClock`
  **MUST be durable across a daemon restart (R4-6)**, because the timestamp it stamps
  (`last_heartbeat`) is **persisted** and then compared
  against a *later* daemon's "now" across exactly the restart that the daemon-down TTL
  ([§14.5](#145-daemon-down-and-the-ttl-backstop)) and retention span. A bare process-monotonic
  clock **rebases on restart** and makes those comparisons meaningless (TTL/stale-cutoff could
  fail open → resurrection, or fail closed). The SQLite `BackendClock` is therefore a **durable,
  persisted, monotonic high-water clock**: a `clock_hwm_ms` is kept in the store, and each read
  returns `max(wall_now_ms, clock_hwm_ms + 1)` and persists the new high-water in the same
  transaction — so it never moves backward (across restart, suspend/resume, or wall-clock skew)
  while still tracking real time, and a respawned daemon resumes from the persisted high-water.
  (It remains injectable for tests.) Both implementations satisfy the same invariant — the
  **persisted** `last_heartbeat` and `stale_cutoff` are
  read from **one** durable clock domain ([§11.5](#115-postgres-cross-machine-reclaim-in-epochs-not-timing)).
  The normative claim statement, identical on both backends:

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
  `:observed_epoch + 1`. (`Register` additionally sets `occupant` in the same transaction
  as the claim.)
- **First-ever absent row** (the address has no `leases` row yet) is created by an
  **`INSERT INTO leases (address, lease_epoch, owner_instance_id, last_heartbeat) VALUES
  (:addr, 1, :me, :backend_now) ON CONFLICT(address) DO NOTHING`** — the insert both
  **creates the epoch at `1` AND claims ownership** for the inserter (fence and ownership
  columns set together, never a transient ownerless row at epoch 1). If the insert succeeds
  (1 row) the claimant **owns the address at `lease_epoch = 1`** (then `Register` adds
  `occupant` in the same transaction); if it conflicts (0 rows — a row appeared
  concurrently), the claimant falls through to the UPDATE CAS above (which will pin the
  now-observed epoch+owner). A row whose `lease_epoch` column is `NULL` (an **unowned/foreign**
  row) is claimed by a **separate, explicit** path
  (`... WHERE address = :addr AND lease_epoch IS NULL`) that sets `lease_epoch = 1`.
  **`NULL` is never conflated with `0`** in the normal claim predicate.
- The winner's `owner_instance_id` is its stable instance identity for the daemon's life;
  `occupant` is a **human/host label of the attending session** (informational), distinct
  from `owner_instance_id` (the daemon fencing identity) and never overwritten by a claim.

### 11.2 Epoch-guarded heartbeat, non-deleting ReleaseOwnership, and self-demotion (mr2, mr3)

**Heartbeat** is epoch+owner-guarded, returns a rowcount, and updates the **lease-liveness
proof only** (`last_heartbeat`). It is **not** session-presence: membership is in-memory and
explicit-only ([§14.1](#141-identity-and-in-memory-membership)), so the heartbeat says nothing
about whether the attending agent is alive — that is the liveness model's job
([§9](#9-liveness-model)), and it is non-destructive regardless. The heartbeat keeps the
delivery-ownership lease fresh so a stale predecessor can be fenced:

```sql
heartbeat: UPDATE leases SET last_heartbeat = :backend_now
            WHERE address=? AND lease_epoch=? AND owner_instance_id=?
            -- → rows: 0|1
```

**`ReleaseOwnership` does NOT delete the row (R3-2 / spar).**
Deleting would discard the only durable carrier of `lease_epoch` (a later claim would reset
the epoch 7 → 1, breaking monotonicity). `ReleaseOwnership` is the **daemon-stop / crash
handoff** path: it clears the **fencing identity only** (`owner_instance_id`) and **preserves
the epoch high-water**, so the next claimant continues the monotonic sequence
(§16 upgrade continuity):

```sql
ReleaseOwnership: UPDATE leases SET owner_instance_id = NULL
                   WHERE address=? AND lease_epoch=? AND owner_instance_id=?   -- → rows: 0|1
```

This is a **reserved-continuity** state, not removal: the ownerless row is **immediately
re-claimable at the backend** (`owner_instance_id IS NULL`) by whichever daemon next serves the
address. Membership is unaffected — it never lived in the lease row; a session that wants the
address simply (re-)attaches with an explicit `Register`
([§14.1](#141-identity-and-in-memory-membership)).

Delivery ownership is derived from `owner_instance_id IS NOT NULL` (and not stale), **never from
row existence**. **Normative no-delete invariant:** no code path — `ReleaseOwnership`, detach,
cleanup, test helper, or migration — may `DELETE` a lease row whose `lease_epoch` matters; all
of them null the owner and preserve the high-water epoch. **There is no v1 GC of lease rows at
all** (R4-4/R5-3): rows are retained for the store's life (any future reclamation is
out-of-scope issue #24, below). (If true
row reclamation is ever needed, the high-water moves to a separate append-only
`address_epoch(address, epoch)` table; out of scope for v1, where unbounded retired-row
growth is acceptable at single-user scale — GC is issue #24.)

A **0-row heartbeat or `ReleaseOwnership`** means a higher epoch exists. The daemon
**self-demotes** for that address — stop emitting AND stop heartbeating (relinquish the
address), release its waiters, and drop the in-memory station. It must not keep heartbeating
(which would hold the lease fresh and starve a successor). The `heartbeat` backend API
**returns a rowcount** (0 rows means a higher epoch exists, triggering self-demotion); a
`Result<()>` with no rowcount is insufficient for this contract.

### 11.3 Server-side delivery fence (mr1 — at-least-once preserving)

**The fence must preserve the ratified at-least-once contract (ADR 0011) — it must never
introduce message loss.** A message is durably MARKed consumed only **after the agent has
explicitly acked it**, never on a transport event.

**Delivery-selection precondition.** Before the fence runs, an address is **eligible for
delivery only if it is owned and attended** — the daemon is the current delivery owner
(`owner_instance_id = :me` at the current `lease_epoch`) and there is an in-memory membership
record with a blocked waiter ([§5](#5-membership-model-and-record-shapes)). A `Wait` for a
session/address the exchange does not know returns **`NeedsAttach`**
([§6.2](#62-request--response-frames)), not a delivery, so the daemon never EMITs against a
session it has no membership for.

```text
mark_consumed_if_current_owner(recipient, owner_instance_id, lease_epoch, message_id)
    -> Result<DeliveryOutcome>

DeliveryOutcome = Marked | AlreadyConsumed | AckNoOp | NotOwner
```

The new commit boundary is **EMIT → the waiter PRINTS → the AGENT acks → the daemon MARKs**:

1. *(optimization only — not the fence)* if in-memory state already knows it is not the
   current owner, skip and self-demote.
2. **EMIT** `Frame::Message(M, lease_epoch)` to the blocked waiter.
3. **The waiter PRINTS `M` to stdout — transport only.** The one-shot `telex wait` client
   writes/flushes `M` to its stdout and exits. **This stdout flush is NOT the consumed mark**:
   it is pure transport. If the waiter dies, the connection drops, or the print is truncated,
   nothing is MARKed and `M` simply redelivers (at-least-once). No `DeliveryAck`, no per-emit
   nonce, no ACK deadline, no connection-binding correlation — the transport carries no commit
   authority.
4. **The AGENT issues an explicit `Ack{store_key, session_id, address, message_id}`**
   ([§6.2](#62-request--response-frames)), immediately on reading `M`, naming the `address` the
   message was delivered to. The daemon **validates the session attends `address`** and writes
   the **durable, epoch-guarded MARK-consumed** for `(message_id, recipient = address)` via
   `mark_consumed_if_current_owner(...)`. The MARK is **idempotent** on `(message_id,
   recipient)`: a replayed or duplicate ack, an ack arriving after the station went idle, or an
   ack from a second waiter that also printed the same message all converge on the one durable
   consumed row. Because the ack is decoupled from any EMIT-time connection, a late ack is never
   a race — it either records the (still-undelivered) consume or is an idempotent no-op.

   **Ack edge outcomes (frozen, M1 residual).** (a) **PRINT → `Detach` → `Ack`** (the agent
   dropped the station, then acks): membership is gone, so the `Ack` returns **`NeedsAttach`** and
   `M` stays **unconsumed** and **redelivers** — the `Ack` does **not** auto-re-attach or resurrect
   a deliberately detached station (the agent re-attaches explicitly only if it still wants the
   address). (b) **`Ack` for an attended `address` with no `(message_id, recipient)` delivery row**
   (the message was never delivered to that recipient) is a **typed `AckNoOp`** (idempotent no-op):
   the daemon does **not** insert a consumed row, so a later genuine delivery of that
   `(message_id, recipient)` is **not** masked. (`AckNoOp` joins `Marked`/`AlreadyConsumed`/
   `NotOwner` as a mark outcome.)

   **The ownership check and the mark MUST be one atomic step (R3-5).** Under the stated
   Postgres `READ COMMITTED` autocommit model a two-step *read owner → mark* races a
   transfer/reclaim that rotates ownership between the read and the mark (the mark would then
   commit as a non-current owner). The frozen shape locks the lease row first, in **one
   transaction**:
   - **Postgres:** `SELECT owner_instance_id, lease_epoch FROM leases WHERE address=:addr FOR
     UPDATE` (row-lock), compare to the caller's `(owner_instance_id, lease_epoch)`, then write
     the consumed mark, then `COMMIT`.
   - **SQLite:** the same sequence inside a **`BEGIN IMMEDIATE`** transaction. **Framing note
     (R4-S2):** `BEGIN IMMEDIATE` takes a **database-wide** write lock in SQLite, **not** a
     row-level lock — it briefly serializes **all** writers for the short
     lock→compare→mark→commit transaction (correctness is fine: on the single-host path the
     daemon is the lone writer — guaranteed by the OS-singleton + the per-store advisory lock, below — and the tx is short).
     The **per-address critical section** bounds only *in-process* concurrency. (Postgres `FOR
     UPDATE` is genuinely row-level, so unrelated addresses proceed concurrently.)

   The **owner-directed transfer** and the **reclaim CAS** take the **same lease-row lock**, so
   they serialize against the mark — closing the rotate-between-check-and-mark race. The method
   returns one of, with **strict outcome precedence**:
   - **`NotOwner`** (precedence-winning, **fatal**): returned whenever the caller is **not the
     current `(address, owner_instance_id, lease_epoch)`** — **even if the message is already
     consumed**. The daemon **self-demotes immediately**
     ([§11.2](#112-epoch-guarded-heartbeat-non-deleting-releaseownership-and-self-demotion-mr2-mr3))
     and stops draining the rest of the backlog. (Without this precedence, a successor `S` that
     marks first would make a stale predecessor `P` see `AlreadyConsumed` and keep emitting
     stale-epoch frames — the exact race the fence exists to stop.)
   - **`AlreadyConsumed`** → returned **only after** current ownership is confirmed;
     **success** (idempotent), continue draining. *Not* fatal.
   - **`Marked`** → success; continue draining.
   - **`AckNoOp`** → returned (**after** current ownership is confirmed, like `AlreadyConsumed`)
     when there is **no** `(message_id, recipient)` delivery row to mark — i.e. the recipient was
     **never delivered** this message. Consumed rows are **retained** as the durable authority and
     are **not pruned in v1** ([§5.1](#51-durable-lease-row-columns-new)), so a
     delivered-then-consumed `(message_id, recipient)` **always** has a row and resolves to
     `AlreadyConsumed`, **never** `AckNoOp` — the never-delivered meaning is **unambiguous** (no
     compaction race). **Success, idempotent, inserts nothing** — so a never-delivered
     `(message_id, recipient)` cannot be masked by a fabricated consumed row, and a genuine later
     delivery still marks normally. Because `AckNoOp` is gated **behind** the `NotOwner` check, a
     stale/superseded owner acking a never-delivered row still gets `NotOwner` (and self-demotes),
     **never** `AckNoOp`.

**Writer authority — three layers.** **(1) Per config root:** the **OS-singleton** (Unix
flock/fcntl + AF_UNIX bind / Windows named-mutex + named-pipe first-instance,
[§2.2](#22-auto-spawn-connect-or-spawn-and-the-spawn-lock) /
[§7.2](#72-os-level-trust-boundary-mr5)) guarantees exactly one exchange process per
`(user SID, config root, protocol-major)`. **(2) Per physical store — single-host SQLite:**
because the singleton key excludes store identity ([§2.1](#21-singleton-identity)) and one SQLite
file can be reached from **two distinct config roots** (two `--db`/`TELEX_DB` paths resolving to
the same file), the daemon acquires a **canonical-store-scoped advisory lock** when it opens a
SQLite store and **fails closed for that store** (refuses to serve it, surfaced in `Status`) if
another exchange already holds it — so exactly one exchange writes a given SQLite store, even
across config roots. **Lock semantics (frozen):** a **kernel/connection-owned OS advisory lock
keyed _solely_ by the canonical store file identity** (device+inode on Unix /
`GetFileInformationByHandle` file-id on Windows), held in a **per-user, config-root-INVARIANT lock
namespace** so that **every alias of one physical store — across all config roots — resolves to
exactly ONE lock target**. Two properties are required, both frozen. **(a) Separate from the SQLite
db file** — the lock is its own object, never the db file, so it can **never collide with, or be
released by, SQLite's own database-file locking protocol** (`BEGIN IMMEDIATE` etc.; locking the db
file directly would interact with SQLite's own lock bytes / fd-ownership lifetime and could **fail
open**), and it is **not** a path-derived sidecar beside the db (two hardlink paths share one inode
→ one file-id → one lock, closing the hardlink hole). **(b) Config-root-invariant location** — the
lock target is **NOT** placed under `run_dir`: `run_dir` is config-root-dependent (`TELEX_RUN_DIR` /
`--run-dir`,
[§7.4](#74-path-resolution-and-the-portability-of-fail-closed-startup-deferred-to-daemon-core)), so
two config roots that alias the **same physical store** but resolve **different** run_dirs would
otherwise take two different lock files and **both** advisory locks would succeed → **two**
exchanges writing one store (a cross-config-root single-writer hole at the lock-namespace
layer). Instead the namespace root is derived from the **stable per-OS-user identity (uid / SID),
ignoring the telex run-dir override**. **Conformance predicate (frozen):** a mechanism is conforming
only if it yields **exactly one lock target per (physical store file-id, OS user)** that is
invariant across **all config roots, `run_dir` overrides, login/logon sessions, and namespaces**,
and that a **different OS user can neither hold nor squat** (cross-user isolation,
[§7.0](#70-v1-threat-model-normative)). Any mechanism whose OS scope is **narrower** than this
per-user invariant — per-logon-session, per-network-namespace, or a world-bindable/squat-able name —
is **non-conforming** and the daemon **must fail closed** rather than use it. **Conforming mechanism
(both platforms): a filesystem advisory lock on `store-<fileid>.lock` inside a validated,
owner/SID-private, per-user lock directory** — squat-proof because a different OS principal cannot
create the lock file inside a directory it has no access to, and inode/path-scoped (not a session-
or network-namespace object), so it spans login sessions and namespaces. **Unix** — `flock`/`fcntl`
on `store-<fileid>.lock` in a **single, canonical, validated `0700` uid-scoped lock dir**. The dir
must resolve to **one** inode for **all** same-uid contexts — a deterministic per-uid root chosen by
**one** documented rule (e.g. `$XDG_RUNTIME_DIR` *validated* as the real per-user runtime dir, **or**
a fixed uid-derived path such as `/run/user/<uid>/telex/locks/` or `$HOME/.local/state/telex/locks/`),
**not** a per-process best-available fallback chain; if the canonical per-uid root cannot be resolved
and validated, **fail closed** rather than silently use a different inode (a divergent dir would
split one store across two lock targets → two writers). **Windows** — `LockFileEx` on
`store-<fileid>.lock` in a **single, canonical, validated current-SID-only local per-user lock dir**
(e.g. under a local, non-roaming `%LOCALAPPDATA%\telex\locks\` with a current-SID-only DACL),
resolved for **all** same-SID contexts by **one** documented rule (**not** a per-process
env/best-available fallback) and **fail-closed on divergence** — symmetric with the Unix
one-canonical-root rule above. This **mirrors the Unix `0700` approach** and is squat-proof for the
same reason — a different SID cannot create the lock file in a SID-private directory — and spans the
user's logon sessions because it is a filesystem path, **not** a session-named object. Here
**"validated"** is the [§7.4](#74-path-resolution-and-the-portability-of-fail-closed-startup-deferred-to-daemon-core)
**effective-permission postcondition** (owner == running uid / current-SID; effective `0700` /
current-SID-only DACL; a **local, non-roaming, advisory-lock-honoring** filesystem; any
**ambiguous/inconclusive** representation classified **cannot-enforce → fail closed**), applied to
the **canonical-store lock dir** exactly as to `config_root`/`run_dir`. **Known non-conforming
mechanisms (must ALWAYS fail closed):** net-namespace-scoped abstract `AF_UNIX` sockets (and any
world-bindable name); session-scoped `Local\` named objects; **and a bare well-known `Global\` named
object** — the `Global\` namespace is **world-creatable**, so a different OS user can **pre-create**
the predictable `Global\telex-store-<fileid>` first; opening an **already-existing** name returns the
existing object while **ignoring the supplied security descriptor**, so the daemon would either
coordinate single-writer authority through a **foreign-owned object** (squat-to-control) **or** see a
wrong-owner object and **fail closed** (squat-to-**DoS** — the squatter denies the victim its own
store). Because a world-creatable name can **always** be squatted to DoS, it can **never** satisfy
the predicate's *"neither hold nor squat"* and is therefore **never conforming** — there is **no
owner-SID escape hatch**. (An atomic creator/owner-SID check on a foreign pre-existing object remains
a sound **generic fail-closed diagnostic**, but it is **not** a conforming store-lock mechanism; the
SID-private filesystem lock above is the one conforming Windows mechanism and covers Windows fully.) The concrete mechanism is
platform-scoped `daemon-core` latitude **subject to the predicate**; the invariance is frozen. The
per-config-root OS-singleton **spawn** lock ([§7.2](#72-os-level-trust-boundary-mr5)) legitimately
stays in `run_dir`; only this **canonical-store** lock is store-identity-scoped and
config-root-invariant. If
the canonical file identity cannot be computed, **or no config-root-invariant per-user lock target
can be resolved**, **fail closed** (surfaced in `Status`). A **SQLite-concurrent
acceptance probe** holds the store lock, opens the db, runs a `BEGIN IMMEDIATE` write + checkpoint,
and proves a second alias-path daemon **fails closed without deadlock** (tests 2/5). Held for the
**store-serving lifetime**, **auto-released on daemon
crash** (so a dead daemon never wedges the stop/drain/respawn floor), and **released on clean
`Drain`/stop** so a respawn re-acquires it.
**Canonical store identity** = the canonicalized absolute path resolved through symlinks/hardlinks
plus, where available, device+inode / Windows file-id, normalized for case/short-name/UNC. On a
**weak/network filesystem where advisory locking is inconclusive** (NFS/SMB/9p/WSL-DrvFs — the
same family [§7.4](#74-path-resolution-and-the-portability-of-fail-closed-startup-deferred-to-daemon-core)
flags), the daemon **fails closed with an actionable error** (the durable store should sit on a
lock-honoring local FS), never silently runs two writers. **(3) Per store — multi-writer
Postgres:** the **lease-epoch fence** is the arbiter; per-host exchanges legitimately race, higher
epoch wins, the demoted owner stops delivering
([§11.5](#115-postgres-cross-machine-reclaim-in-epochs-not-timing)). Postgres is in v1 scope. The
store-lock is a **SQLite single-host** mechanism; the **live ordered handoff**
([§11.4](#114-ordered-handoff--owner-directed-atomic-transfer-sf3)) — where a successor opens the
backend while the predecessor still serves — is a **multi-writer (Postgres)** mechanism, so the
store-lock and a live two-daemon overlap never apply to the same store.

**Why this is at-least-once with no loss window:** any crash, pipe break, missing ack, or
ownership rotation **after EMIT but before a successful MARK** leaves `M` unconsumed in
`deliveries`, so the current owner redelivers it → a **duplicate**, never a loss. The **only**
thing that prevents a superseded owner from systematically re-delivering is the epoch-guarded
MARK returning `NotOwner` (which wins over `AlreadyConsumed`) and forcing self-demotion (the
in-memory check in step 1 is just an optimization). The at-least-once contract, stated
normatively: **`M` is delivered repeatedly until exactly one current-epoch owner records a
successful agent-acked MARK; consumers dedupe by `message_id`.** The duplicate count is bounded
by the number of failed owners/handoffs, not "exactly one." The `lease_epoch` on the frame is a
**secondary** filter a waiter applies only **after** it has independently learned a newer epoch
(via reconnect/handshake); it is **not** a live defense against a stale daemon — that defense is
the server-side MARK plus self-demotion.

The corresponding [§17](#17-gating-tests--per-backend-conformance-matrix-daemon-core-acceptance)
gating test asserts, across crash-after-PRINT/before-ACK, crash-after-ACK, two-waiters-both-
print-then-one-ack, ack-after-station-idle, ack-replay, and **ownership-rotation-after-EMIT plus
successor-marks-then-predecessor-marks**, that every message reaches a waiter **at least once**
(never zero), that the `(message_id, recipient)` consumed mark is idempotent, and that a
superseded owner stops after one `NotOwner` (even when the message was already consumed by `S`).

**Performance contract (R3-Sf).** The EMIT→PRINT→ACK→MARK path and its lease-row lock sit on the
**per-address hot path**, so the budget is frozen as acceptance, not left implicit: `daemon-core`
freezes a **p95/p99 single-delivery fence latency budget** (local IPC RTT + one lease-row-locked
mark) and a **numeric dedup resource contract** (the per-recipient `message_id` dedup set's
bounded memory/row footprint and its retention window). These are **benchmarked** as part of the
gating matrix; the fence is **not weakened** (e.g. dropping the agent ack or the lock) to meet
them — if a budget cannot be met, it is renegotiated explicitly, the correctness fence stays.

### 11.4 Ordered handoff = owner-directed atomic transfer (sf3)

A graceful handoff (coordinated upgrade/stop where a successor `S` exists) must not lapse
the lease, leave an ownerless window a third daemon could hijack, or double-deliver. The
predecessor `P` transfers ownership **directly to `S` in one guarded statement** — there
is no release-then-claim gap and no generic "claim from a live owner" path (either would
admit a hijack):

```text
prepare  → S is spawned and READY (endpoint bound, backend open, recovery pass done)
quiesce  → P stops accepting new Wait/Register for the address; stops new drains
flush    → P finishes in-flight MARK critical sections; any EMITted-but-unmarked message is left for S to redeliver
transfer → one atomic UPDATE: P@epoch E → S@epoch E+1
```

```sql
UPDATE leases
   SET owner_instance_id = :successor,
       lease_epoch = lease_epoch + 1,
       last_heartbeat = :backend_now
 WHERE address = :addr AND lease_epoch = :E AND owner_instance_id = :predecessor  -- → rows: 0|1
```

The transfer writes only ownership/fence columns (`owner_instance_id`, `lease_epoch`,
`last_heartbeat`) — it does **not** rewrite the lease row's `occupant`/`session_id` (a
daemon-to-daemon transfer carries **delivery ownership**, not a change of which session occupies
the address). **Membership** — the set of addresses a session attends — is in-memory and
explicit-only ([§14](#14-session-identity-and-explicit-membership)), so whether a station is
idle or attended is a function of in-memory waiters in the owning process, not of a durable
column; a transfer cannot make a dead-but-unhooked station look freshly present.

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
queue durably). **Backend scope (M4).** The **live** ordered transfer above — a successor that
has the **backend open** ([§11.4](#114-ordered-handoff--owner-directed-atomic-transfer-sf3)
`prepare`) while the predecessor still serves — applies to the **`shared_multi_writer` (Postgres)**
backend, where two exchanges may legitimately hold the backend open and the epoch arbitrates. On
the **single-host SQLite** path the per-store advisory lock ([§11.3](#113-server-side-delivery-fence-mr1--at-least-once-preserving))
**forbids** a live two-daemon overlap on one store, so a SQLite upgrade/stop uses this
**release + next-call respawn floor** (the outgoing daemon releases the store-lock and
non-deleting-releases ownership; the successor respawns on the next client call and re-acquires
the lock). So the live successor-readiness handoff and the SQLite store-lock never contradict. A **per-step handoff crash matrix** (kill/signal after prepare, quiesce,
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
[§10.1](#101-non-destructive-reaping)) — never
one machine's local `now` compared against another's timestamp. SQLite-local is the
simple single-writer case (commit order == id order); `postgres-parity` proves the
competing-daemon behavior under MVCC. Correctness rests on READ COMMITTED autocommit reads
(the isolation precondition pinned by ADR 0013); the per-backend fault-injection and
isolation matrix is [§17](#17-gating-tests--per-backend-conformance-matrix-daemon-core-acceptance).

## 12. (reserved)

*(Legacy-holder cutover is a one-time migration off the resident-holder model, not part of the
standing design; the mechanism is recorded in [DECISIONS.md](DECISIONS.md) ADR 0024. The only
durable residue is that a `lease_epoch IS NULL` row is treated as **unowned/foreign** and never
conflated with `0` — see [§5.1](#51-durable-lease-row-columns-new) /
[§11.1](#111-epoch-lifecycle-oq1).)*

## 13. Delivery and `seen` dedup

The exchange reuses the **durable per-recipient delivery buffer** of ADRs 0011/0013 (the
`deliveries(message_id, recipient)` table, `UNIQUE(message_id, recipient)`,
`fetch_undelivered`) unchanged as the **cross-epoch / cross-restart dedup authority**.
The live drain remains "deliver the undelivered set, authoritative on delivery state,
never on id ordering" (ADR 0013), now fenced by [§11.3](#113-server-side-delivery-fence-mr1--at-least-once-preserving).

The in-memory `seen` dedup set is **bounded and epoch-scoped** for a long-lived daemon: an
unbounded, never-pruned set would grow without limit and carry stale identity across epochs.
The model is:

- **Durable `deliveries` is the authority** for "has this `(message_id, recipient)` been
  consumed (agent-acked)?" — no behavioral change to 0011/0013 beyond the consumed mark now
  being triggered by the explicit agent `Ack` ([§11.3](#113-server-side-delivery-fence-mr1--at-least-once-preserving)),
  not by a transport event.
- **In-memory dedup is a bounded fast-path** keyed by **`(recipient, message_id,
  lease_epoch)`** (in-flight identity, scoped to the current epoch).
- **Seed** the fast-path from `fetch_undelivered` on claim.
- **Evict** an entry on: a durable mark (`mark_consumed_if_current_owner → Marked`),
  a terminal disposition, or an epoch transition.
- **Reset/drop** the entire fast-path on epoch loss (self-demote, reclaim) — its
  identity is epoch-scoped, so it must not survive a fence.

This keeps dedup bounded and correct without relying on process restart; durable-buffer GC
is out of scope for v1.

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
  than `stale_cutoff` / the lease window.
- **Fence round-trip budget.** The added `mark_consumed_if_current_owner` round-trip
  carries a **p95/p99 latency budget** (benchmarked; the transaction shape is optimized —
  e.g. single round-trip CAS-style upsert) — but the budget is a target to *optimize the
  shape toward*, **never** a license to weaken the EMIT→ACK→MARK ordering for latency.
- These budgets are single-user / pre-beta acceptance limits
  ([§7.0](#70-v1-threat-model-normative)); multi-user / hot-address scaling is revisited at
  beta.

## 14. Session identity and explicit membership

### 14.1 Identity and in-memory membership

The exchange owns an **in-memory** membership map keyed by **`(store_key, session_id)` →
`{addresses}`** — which addresses a session attends. The `store_key` is part of the key
because **one exchange serves multiple stores**: a `session_id` that recurs across stores must
not let one store's hook drop another store's addresses, nor let a `from`-resolution
misattribute a reply. This **reshapes #23 / PR #31**: the Copilot sessionEnd hook plumbing is
reused, but the filesystem `session_registry` (verified on
`feature/copilot-session-end-plugin`: per-session JSON files) is **dropped as the authority**.
The hook becomes a **thin mapper** (`COPILOT_AGENT_SESSION_ID → TELEX_SESSION_ID`), and Copilot
JSON parsing never becomes a core protocol dependency (it lives in the plugin layer).

**Identity = the unique, stable `session_id`.** The session's identity is exactly its ambient
`session_id` (`COPILOT_AGENT_SESSION_ID`, [§4](#4-status-surface-the-frozen-contract-shape)): unique, stable across
dismiss/exit/resume, and **never reused** for a different session. There is **no incarnation
token, no currency table, and no per-session sequence/nonce** — uniqueness of `session_id`
alone is what makes a late or duplicate operation safe, because every durable fact is keyed by
`session_id` (or by `(message_id, recipient)`) and every recovery action is idempotent.
**Provenance + tripwire (S3):** uniqueness/non-reuse is the operator's authoritative statement
about the Copilot CLI harness (same source as the `COPILOT_AGENT_SESSION_ID`/loader probes), **to
be confirmed by the id-scheme + dismiss/resume spike** ([revisit conditions](#design-assumptions-and-revisit-conditions)).
Because it is the **sole** safety fence, `daemon-core` MUST add a **reuse tripwire**: a loud
`Status`/audit warning if a `session_id` is re-presented in a pattern inconsistent with
same-session resume (e.g. after a definite end), so a violated premise is **observable**, not
silent corruption.

**Membership is EXPLICIT-ONLY and IN-MEMORY.** A session attends an address **only** because it
ran a one-off `telex attach` → `Register{store_key, session_id, address}`
([§6.2](#62-request--response-frames)), which is idempotent and establishes the in-memory
`(store_key, session_id) → {addresses}` entry. The exchange **never** rebuilds membership
implicitly from history, the durable buffer, or a hook. Consequently:

- There are **no tombstones** — nothing implicit ever resurrects a removed address, so there is
  nothing to suppress. `Detach{store_key, session_id, address}` simply drops the in-memory
  entry; a removed address stays gone until an explicit `Register` re-adds it.
- For any `Wait`/`Send`/`Reply`/`Ack` naming a `(session_id[, address])` the exchange does not
  currently know, the op returns the typed **`NeedsAttach`** error
  ([§6.2](#62-request--response-frames)) — terminal for that op — and the agent **explicitly
  re-attaches** the addresses it wants. The exchange never guesses.

The durable layer therefore holds **only** lease-ownership (the epoch fence,
[§11](#11-lease-epoch-fence-the-spine)) and the **message/ack buffer**
(`deliveries(message_id, recipient)`, [§5.1](#51-durable-lease-row-columns-new)). Membership
lives in memory; identity lives in the ambient `session_id`.

### 14.2 The sessionEnd hook

The sessionEnd hook is **authoritative but non-destructive** and is specified in
[§9 Liveness model](#9-liveness-model): on receipt it releases the session's blocked waiters and
marks its stations IDLE; it never destroys a station, and a late/spurious hook is harmless
(unique `session_id` + non-destructive + self-healing on the next `Register`). It carries no
incarnation and needs no token-file channel.

### 14.3 Crash recovery and re-attach

When the exchange crashes and respawns it has **no in-memory membership** — the map is empty by
construction (membership is in-memory + explicit-only, never rebuilt from history). Recovery is
fully driven by the agent's next op:

1. The respawned exchange holds **only** the durable lease-ownership rows and the durable
   `deliveries` message/ack buffer.
2. The next client op for a session it does not know returns **`NeedsAttach`**.
3. The agent **explicitly re-attaches** (`Register`) the addresses it wants, re-establishing the
   in-memory membership entry. A removed address is **not** resurrected — only what the agent
   chooses to re-attach comes back.
4. Any in-flight messages were **durably buffered** before the crash and are delivered
   **at-least-once** on the next `attach` + `wait` + `ack`, deduped by `message_id`
   ([§11.3](#113-server-side-delivery-fence-mr1--at-least-once-preserving)) — **no loss, no
   resurrection.**

There is no `suspect`/`verified`/`lapsed` state machine: a session is either known (in the
in-memory map) or not (→ `NeedsAttach`).

### 14.4 `wait` and re-attach on `NeedsAttach`

A blocked `telex wait` is the normal long-lived presence. If the exchange does not know the
waiting session/address — first call, or after a respawn — the `Wait` returns **`NeedsAttach`**;
the `wait` client (or the agent harness) issues a one-off `Register` for its own address and
re-issues `Wait`. Because the client always knows the address it is waiting on (it was given to
it), the re-attach is unambiguous and needs no server-side history. The reconnect/grace behavior
is described in [§3.3](#33-wait-reconnect-on-eof-grace).

### 14.5 Daemon-down and the TTL backstop

If the daemon is down, its leases lapse after the **daemon-down TTL window** and/or are fenced
by the respawned daemon's higher epoch ([§11.1](#111-epoch-lifecycle-oq1)). A session that
**ends while the daemon is down** simply finds its hook no-ops against a down daemon (recorded as
a transient on the harness side, not fatal); on respawn the agent's next op returns
`NeedsAttach` and it re-attaches. Membership is in-memory and was never persisted, so there is
**nothing to resurrect** and **no permanent zombie** — a station is recreated only by an explicit
`Register`.

**Wall-clock dependence and the fail-closed path.** The daemon-down TTL is the one predicate that
inherently depends on real elapsed wall time: the durable high-water clock cannot advance while
the daemon is down, so "has the TTL elapsed?" is only observable if the respawn wall clock has
advanced past the persisted `last_heartbeat + ttl`. If the host **slept** or the wall clock was
**stepped backward**, a respawn may not be able to *prove* the downtime. This is resolved
**fail-closed**: a lease whose TTL cannot be proved elapsed is **left owned** (never auto-lapsed
on an untrustworthy clock), and recovery is routed through the **operator reset**
([§10.2](#102-operator-reset)) — a non-destructive action that releases waiters / marks a station
idle without minting an eviction epoch. (There is no `force`-takeover and no force-nonce rotation:
those existed only to invalidate incarnation tokens, which are gone.)

### 14.6 `from`-resolution and re-attach

`from`-resolution depends on the in-memory membership map, so the design must say what happens
when a `send`/`reply` needs it for a session/address the exchange does not currently know:

- **`from`-resolution** resolves `from` against *that* session's attended addresses for *that*
  store only (never across sessions or stores): exactly one → succeed; multiple → `Ambiguous`;
  none/unknown → the recovery below.
- **send/reply for an unknown session/address** returns **`NeedsAttach`**. The agent
  **re-attaches** (a one-off `Register` for its own address — which it knows, since the agent
  always knows its own attached address), which re-establishes the membership entry, then retries
  the `send`/`reply`. There is **no implicit rebuild** from durable history.
- If resolution still finds nothing after a re-attach, the send **fails actionably**
  (`refused-unrepliable`, as ADR 0010) — **never** a silent `from = None`. This preserves the
  ratified ADR 0010 guarantee (acceptance test: register, no blocked wait, kill+respawn the
  daemon mid-turn, `telex reply` without `--from` → `NeedsAttach` → re-attach → documented
  outcome).
- **One-shot verb env contract.** `TELEX_SESSION_ID` and `store_key` are present in the env of
  every `send`/`reply`/`wait`/`ack` the loader/plugin spawns, so a verb can always name its
  session for re-attach. There is **no `TELEX_SESSION_INCARNATION`, no `(session_seq, nonce)`
  token, and no token-file** — identity is the ambient `session_id` alone, and a one-off verb
  that hits `NeedsAttach` simply re-attaches.
- **Identity-propagation contract (S2 — frozen for every re-attach-capable path).** Re-attach
  works only if every verb that can receive `NeedsAttach` can name its
  `(store_key, session_id, address)`. This is frozen for **all** paths, not just plugin-spawned
  verbs; a path that cannot name its session **fails closed actionably** (never guesses):

  | Path | How identity is carried |
  |---|---|
  | **loader/plugin-spawned** (`send`/`reply`/`wait`/`ack`) | inherited env (`TELEX_SESSION_ID`, `store_key`) + the `--address` it was given |
  | **manual CLI** | explicit `--session` / `--store` / `--address` flags (or the same env vars); absent on a `NeedsAttach` → an **actionable** "name your session to re-attach" error, not a guess |
  | **embeddable SDK (#12)** | the SDK threads the same `(store_key, session_id, address)` it attached with |

  Revisit condition: if a harness cannot propagate identity to a given verb, that verb cannot
  re-attach — a propagation-failure handling path must be added before that verb is supported.

## 15. Verbs, CLI mapping, and the single-source SKILL

### 15.1 Verb mapping

The CLI verbs `attach` / `detach` / `wait` (and `send`/`reply`/etc.) are **one-shot** against
the exchange:

| CLI verb | Behavior (against the exchange) | IPC operation |
|---|---|---|
| `attach` | one-shot: register a station, exit | `Register` |
| `detach` | one-shot: remove the station | `Detach` |
| `wait` | block on the exchange for one delivery, exit | `Wait` |

`Register` / `Detach` / `Ack` are **protocol/IPC operations**, not CLI
renames. The held-stream `SessionConnect` liveness is **not** adopted. The `telex daemon`
entrypoint (and `telex daemon stop --drain`,
`telex daemon status`) is **hidden** from normal user help — the exchange is implicit and
zero-config, like `rust-analyzer`/`gopls`.

### 15.2 Single-source SKILL

One source serves both the CLI command and the plugin skill:

- **Canonical file:** root `SKILL.md` (unchanged; stays at the repo root).
- **CLI consumer:** `telex skill` prints the embedded `SKILL.md`
  (`include_str!` in `src/commands/skill.rs`, unchanged) — add a `--raw` form for
  machine consumption.
- **Plugin-skill consumer:** a plugin manifest pointer if the harness supports pointing
  at a file, otherwise a thin wrapper that `exec`s `telex skill --raw`.
- **Invariant:** **no generated divergent copy** — both consumers resolve to the same
  `SKILL.md`. The `SKILL.md` narrative content is owned by `daemon-core`.

## 16. Minimal upgrade floor

The full seamless-upgrade platform (rollback / gc / UX) is the `seamless-upgrade` node
(last). A **minimal floor** lands in `daemon-core`, because a daemon-aware install can hit
the Windows binary-lock (a running `telex` process locks the binary during swap). **v1
deployment is forward-only** (a newer daemon supersedes an older one; there is no supported
rollback to a pre-epoch binary while `lease_epoch >= 1` rows exist — see the downgrade note
below):

- **Versioned install + launcher shim.** A stable `telex` shim resolves to a versioned
  binary (`telex-<version>`), so an upgrade writes a new versioned binary without
  overwriting the locked one.
- **`telex daemon stop --drain`.** Quiesce + flush in-flight EMIT→ACK→MARK + hand off **by
  backend**: **Postgres** uses the live owner-directed transfer where a successor exists, else
  non-deleting release; **SQLite** always uses **non-deleting release + next-call respawn** (no
  live two-daemon overlap — the store-lock forbids it), in order
  ([§11.4](#114-ordered-handoff--owner-directed-atomic-transfer-sf3)), then exit — freeing
  the binary lock.
- **Next-call respawn.** The next client connect-or-spawn starts the new version (**SQLite**: the
  release + next-call stale-claim floor; **Postgres** may use the live transfer; both reuse
  stale-claim + crash-recovery). Presence across the respawn for a
  mid-task agent is covered by re-attach on `NeedsAttach`
  ([§14.6](#146-from-resolution-and-re-attach)).
- **Downgrade (forward-only v1).** Once rows carry `lease_epoch >= 1`, an **old pre-epoch
  binary must not run** against the store (it would write non-epoch rows and reset the
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
([§11.5](#115-postgres-cross-machine-reclaim-in-epochs-not-timing)). This matrix is the
**standing-design** acceptance; the one-time **legacy-holder cutover** is a migration concern whose
acceptance test is specified separately in [DECISIONS.md](DECISIONS.md) ADR 0024 and is **not**
counted in this matrix.

| # | Test | SQLite | Postgres | Key assertion |
|---|---|---|---|---|
| 1 | **Concurrent first-use** (thundering-herd auto-spawn) | required (multi-process) | required | exactly one daemon bound; losers connect; no duplicate/orphan |
| 2 | **OS-singleton refuses a second instance + single-writer-per-store** (mr5, M4) | required | required | a second exchange process for the same singleton key is refused by the exclusivity primitive (Unix flock/fcntl + AF_UNIX bind / Windows named-mutex + named-pipe first-instance); **and** two **distinct config roots** pointing at the **same physical store** (same physical SQLite file reached via different `--db`/`TELEX_DB` paths **and** resolving **different `run_dir`s** — `TELEX_RUN_DIR`/`--run-dir`) — the second daemon **fails closed for that store** via the canonical-store-scoped advisory lock, whose target is **config-root-invariant** (keyed solely by file-id, **not** under `run_dir`), so exactly one exchange writes a store **even across config roots and run_dirs** (**SQLite**: the canonical-store lock; on **Postgres** per-host exchanges legitimately coexist and the **lease-epoch** arbitrates — no store-lock refusal, see test 6); a **SQLite-concurrent probe** (hold the store lock + open the db + `BEGIN IMMEDIATE` write + checkpoint) proves a second alias-path daemon **fails closed without deadlock** and that the dedicated store lock does **not** collide with SQLite's own file locks; a **per-user-invariance subcase** — same OS user (same SID), two **login sessions** (Windows) and/or two **network namespaces** sharing one store (Linux), distinct config roots/run_dirs, one physical SQLite file — proves **exactly one** daemon serves the store and the others **fail closed**, and that any lock mechanism whose OS namespace is **narrower than the frozen per-user invariant** (per-session, per-net-ns, world-squat-able) is **non-conforming** and must fail closed; a **different-OS-user squat negative** — a *second* OS principal (user B) **pre-creates** the store-lock name/object before user A's daemon starts — proves A's store lock lives in an **owner/SID-private directory B cannot pre-create in** (so B can neither occupy nor deny A's lock target), and that any **world-creatable** namespace (bare `Global\`, abstract `AF_UNIX`) is **rejected as non-conforming** with an actionable error rather than used (this is the subcase that exercises a **different principal**, which the same-SID subcase does not) ([§11.3](#113-server-side-delivery-fence-mr1--at-least-once-preserving)) |
| 3 | **Crash-during-`wait` → `NeedsAttach` → re-attach** | required | required | `wait` against an unknown session/address returns typed **`NeedsAttach`** (no spurious exit 3); after an explicit `Register` + re-`Wait`, the waiter blocks normally; **a previously `Detach`ed address is NOT resurrected** — only addresses the agent explicitly re-attaches come back |
| 4 | **Daemon restart: no loss, no resurrection** | required | required | messages durably buffered before the crash are delivered **at-least-once** on the next `attach` + `wait` + `ack` (no loss); the respawned exchange has **no in-memory membership** and rebuilds nothing from history; a removed address stays gone (no tombstone, no implicit rebuild); a **consumed** `(message_id, recipient)` is **not redelivered** after restart (the **retained** consumed row keeps it out of `fetch_undelivered` — consumed rows are not pruned in v1, so there is no resurrection-by-compaction) |
| 5 | **Explicit-ack at-least-once + idempotent dedup + multi-recipient fan-out** (mr1, M1) — crash-after-PRINT/before-ACK, crash-after-ACK/before-MARK, concurrent-second-waiter rejected (no duplicate handoff before ACK), ack-after-station-idle, ack-replay, **one `message_id` fanned out to >=2 recipient addresses (to/cc/watcher) the session attends** | required | required | every message reaches a waiter **>=1** time (never 0); at most one live waiter is accepted per station so a rapid re-arm before ack cannot create sibling deliveries of the same `(message_id, recipient)`; the stdout flush is **transport only** (never the consumed mark); the durable MARK fires only on the explicit agent `Ack`; the `Ack` carries the **delivered `address`** and marks **only** `(message_id, recipient = address)` — the **same `message_id`** acked for recipient `A` does **not** consume `(message_id, B)`, and an ack naming an address the session does not attend is **rejected**; the `(message_id, recipient)` consumed mark is **idempotent** (duplicate/late/replayed/post-idle acks converge, never double-consume); an `Ack` for an **attended** address with **no delivery row** returns **`AckNoOp`** and inserts no consumed row (the message stays deliverable + markable afterward); consumers dedupe by `message_id` |
| 6 | **Multi-writer Postgres delivery-ownership (epoch)** (mr3) | N/A (single-writer by the OS-singleton **+ the canonical-store lock**, [§11.3](#113-server-side-delivery-fence-mr1--at-least-once-preserving)) — assert single-writer holds | **required** (cross-process/cross-machine, fault injection) | higher `lease_epoch` wins; the demoted owner **stops delivering** on its 0-row heartbeat / `NotOwner` mark; **no double-delivery**; no flip-flop |
| 7 | **Delivery fence ownership-rotation race** (mr1, R3-5) — rotation between the mark's ownership-check and the mark, successor-marks-then-predecessor-marks | required | required | the atomic lease-row-locked mark returns **`NotOwner`** on a between-check-and-mark rotation (precedence over `AlreadyConsumed`, **even when already consumed by the successor**) → the superseded owner self-demotes and stops after one; no systematic stale re-delivery |
| 8 | **Non-destructive reaping: sessionEnd hook** | required | required | an authoritative `sessionEnd` hook `(store_key, session_id, admin_cap)` **releases that session's blocked waiters + marks its stations IDLE**; it **never destroys a station**; a **late/spurious/duplicate hook** (unique `session_id`) costs at most one waiter re-arm, never data loss; the station + durable buffer survive and **wake on the next message** |
| 9 | **Non-destructive reaping: loader-pid death** (mr3) — pid + start-time guard | required | required | death of the watched **loader pid** (negative-only signal) releases that session's waiters + marks idle; **loader-alive is never treated as positive presence**; a **pid-reuse** (same pid, different start-time) does **not** count as the loader still alive; the station is not destroyed |
| 10 | **idle-TTL >= 1 day: non-destructive max-blocked-wait (both observable cases)** (S1) | required | required | a session idling past the TTL with a **surviving loader and no sessionEnd hook** has its blocked waiter released as a **`PresenceEnded` (exit 5)** — and because a **live-idle** waiter and an **unhooked-dead** waiter are **observationally identical**, the test asserts the **non-destructive outcome holds for both**: membership + durable buffer **persist**, the `wait` loop **re-arms** on `PresenceEnded`, a new message **wakes** the station, and **no message is lost**; the TTL **never destroys a station** and is **not** a cap on legitimate idle |
| 11 | **Operator reset** (replaces force-takeover) | required | required | the operator `Reset` action releases a session's waiters / marks a station idle **non-destructively**; it mints **no eviction epoch** and rotates **no force-nonce**; membership/buffer are untouched except the released waiters |
| 12 | **Epoch monotonicity across release/cleanup/re-claim** (mr2) | required | required | after `ReleaseOwnership` at epoch E and a cleanup pass, the next claim is **E+1 (never 1)**; no row deletion of an epoch-bearing address |
| 13 | **Ordered-handoff crash matrix + successor-readiness** (sf3, M4) — kill after prepare / quiesce / flush / transfer, on **both** P and S | **required** (release + next-call respawn floor — no live two-daemon overlap) | **required** (live transfer) | bounded idempotent recovery; no loss; no duplicate beyond at-least-once; no ownerless hijack window; **S-crash-before-transfer aborts the handoff (P keeps ownership), S-crash-after-transfer recovers via stale-claim**; the transfer writes ownership/fence columns only (no session/attendance column exists to refresh). **Backend split:** the **live** ordered transfer (successor opens the backend while predecessor serves) is **Postgres** (multi-writer); on **SQLite** the per-store lock forbids a live two-daemon overlap, so the upgrade path is **release + next-call respawn** ([§11.4](#114-ordered-handoff--owner-directed-atomic-transfer-sf3)) and the matrix covers that floor |
| 14 | **OS trust boundary negatives** (mr5, R3-7) | required (Unix 0700 socket / symlink) | required | a second OS principal cannot `Hello`/`Register`/`Wait`; symlinked cap/lock rejected; **a pre-bound hostile server is rejected client-side BEFORE any metadata disclosure** (before `Hello`/`store_key`, via `GetNamedPipeServerProcessId`/connected-`SO_PEERCRED`); a **PID-reuse race** does not authenticate the wrong process; **`admin_cap` never appears** in `Status`/`Error`/logs/traces; non-owner-private `run_dir`/**canonical-store lock dir** rejected at startup (`config_root` is identity-only and not a secret-bearing failpoint unless that role changes); a `run_dir` **or canonical-store lock dir** whose **effective** owner-only permission cannot be represented/verified (ACL/DACL inconclusive, or a non-local/roaming/non-advisory-lock-honoring FS) **fails closed** with an **actionable** error that **names the configured override** (e.g. `TELEX_RUN_DIR`), not an opaque failure ([§7.4](#74-path-resolution-and-the-portability-of-fail-closed-startup-deferred-to-daemon-core)) |
| 15 | **IPC version/capability compatibility** (sf2) — N/N-1 and N+1/N | required | required | security-sensitive `required_capabilities` mismatch fails closed (`Incompatible`/`Unauthorized`); attach/wait-reconnect/Drain/Detach/Ack/Status behave per the **`daemon-core`-owned IPC compatibility table** ([§6.1](#61-version-handshake--capability-negotiation-hello--helloack-sf2)); the **`NeedsAttach` error frame, the `Ack{store_key, session_id, address, message_id}` frame, and the typed `Ack` `DeliveryOutcome` result (`Marked`/`AlreadyConsumed`/`AckNoOp`/`NotOwner`)** are part of that versioned surface — N/N-1 cases assert an N-1 client/daemon either negotiates each or **fails closed**, never silently degrades |
| 16 | **N / N+1 protocol-major parallel** (mr8) | required | required | two protocol-major-parallel daemons under one config root each authenticate against their own `daemon-<H>.cap`; neither clobbers the other |
| 17 | **Durable BackendClock + daemon-down TTL fail-closed** (R4-6, R5-Sb) | required (SQLite high-water) | required (PG server clock) | persisted `last_heartbeat` stamped before a restart compares correctly against the respawned daemon's clock; the SQLite high-water never moves backward across restart/suspend/skew; a **slept / backward-wall-clock restart whose real downtime exceeds the TTL** does **not** fail open (no auto-lapse of a live address on an untrustworthy clock); recovery of a wedged lease is via the non-destructive **operator reset** ([§10.2](#102-operator-reset)) — no eviction epoch, no permanent zombie |
| 18 | **Cross-store isolation + from-Ambiguity** (M6) | required | required | the **same `session_id` registered in store A and store B**: a `SessionEndHook` / reap for store A releases **only A's** waiters and leaves B's station intact (the `(store_key, session_id)` keying, [§14.1](#141-identity-and-in-memory-membership)); a `from`-resolution that finds **multiple** attended addresses returns **`Ambiguous`** (never an arbitrary pick), and one that finds **none/unknown** returns **`NeedsAttach`** (never a silent `from = None`, [§14.6](#146-from-resolution-and-re-attach)) — fixtures MUST exercise multi-store and multi-address, not single-store/single-address; and a `session_id` re-presented inconsistently with same-session resume raises the **reuse tripwire** `Status`/audit warning (S3, [§14.1](#141-identity-and-in-memory-membership)), never silently proceeding |
| 19 | **Delivery-fence latency + dedup-retention budget** (sf6, R3-Sf, S5) | required (benchmark) | required (benchmark) | **falsifiable shape**: workload = N concurrent single-deliveries; metric = per-delivery `EMIT→PRINT→ACK→MARK` fence latency measured at the agent-ack boundary (p95/p99) + the durable `deliveries(message_id, recipient)` dedup-buffer size; the gate **fails** if the measured p95/p99 exceeds, or the buffer/`max_in_flight_entries` exceed, the values in the `daemon-core` **delivery-budget spec** (a committed artifact / tracked `daemon-core` issue the gate runner reads; numeric thresholds are `daemon-core`-owned, the artifact + this gate are not). Unacked retained until ack/terminal; **acked rows retained as the durable consumed authority (NOT pruned in v1; bulk GC is the deferred #24)**; the fence is **never weakened** (dropping the agent ack or the lock) to meet a budget |

Tests 1–7 cover the delivery/identity core; 8–11 the non-destructive reaping + idle-TTL +
operator-reset model; 12–17 the epoch lifecycle, handoff, OS-trust, IPC-version, protocol-major,
and clock axes; 18–19 cross-store isolation / from-Ambiguity and the delivery latency +
dedup-retention budget. `fencing-proof` owns 5/6/7/12/13 on Postgres; `postgres-parity` owns the
cross-machine axis of 6. Test 5 additionally asserts the **mark/transfer lease-row-lock
serialization is deadlock-free on both backends** (the SQLite db-wide `BEGIN IMMEDIATE` tx is
short and never nested under the per-address section).


## Out of scope for v1 (deferred)

The following are explicitly deferred so they are not silently dropped:

- **Full non-binary station status policy** (attended/idle/free) — the minimal idle/attended
  signal (release-waiters + mark-idle on a definite reaping signal) is in scope; the full state
  machine and any richer idle policy are deferred and **never drive teardown**.
- **fd-over-IPC pid-reuse-immune backstop** — awkward with a singleton daemon; the lighter
  pid+start-time guard is in scope; the fd path is deferred.
- **Daemon subsuming directory/occupancy reads** (`address list`) — v1 reads the backend lease
  table; the daemon does not own directory reads yet.
- **`per_session_cap` / multi-tier capability** — fields reserved now; tiers deferred (same-trust
  user-private threat model in v1).
- **Durable-buffer GC** (`mark_delivered` cap, registry GC) — the durable `deliveries` buffer is
  retained, not pruned, in v1; bulk GC is deferred.
- **Embeddable SDK client** — a separate solve that reuses this stabilized Layer-1 IPC.
- **Startup path-resolution + portability policy** (ADR 0022 / ADR 0025,
  [§7.4](#74-path-resolution-and-the-portability-of-fail-closed-startup-deferred-to-daemon-core)) —
  the `run_dir` resolution algorithm, config-root role taxonomy, the cannot-enforce-owner-only
  handling, and the single-tenant opt-out are deferred to `daemon-core`; the runtime owner-only
  **requirement** + **fail-closed** behavior are frozen.
- **Status rendering / format / verbosity** ([§4](#4-status-surface-the-frozen-contract-shape)) —
  the field **set + meaning** + the gating tests' observable assertions are frozen; how `Status`
  is *rendered* (format, verbosity, human vs JSON) is `daemon-core`'s.
- **Dismiss/resume + id-scheme spike (binding pre-implementation action).** Owner: `daemon-core`.
  Trigger: **before implementing `sessionEnd` dismiss behavior**. Action: run the dismiss/resume +
  `session_id` id-scheme spike; if dismiss does **not** fire `sessionEnd`, set the **idle-TTL as
  the primary dismiss bound** and tune its window ([§9](#9-liveness-model) caveat / revisit
  condition). Also confirms the `session_id`-uniqueness assumption
  ([§14.1](#141-identity-and-in-memory-membership)).

## Design assumptions and revisit conditions

The standing design rests on the assumptions below; each names the condition under which the
corresponding decision should be revisited.

- **`session_id` is unique and stable per session.** If `session_id` is ever reused across two
  distinct sessions (the ambient identity stops being unique/stable), an incarnation/currency
  fence returns, because uniqueness of `session_id` is the sole thing that makes late/duplicate
  ops and non-destructive recovery safe.
- **The single-host backend is self-serializing under one OS-singleton.** If a multi-writer or
  otherwise non-self-serializing single-host backend appears (something the OS-singleton cannot
  make a single writer), **or a zero-downtime hot daemon handoff is introduced** (a brief
  intentional two-daemon overlap), revisit the **single-host writer stack (OS-singleton +
  canonical-store lock)** for the single-host path (the lease-epoch fence already covers
  multi-writer Postgres, and would extend to the hot-handoff overlap).
- **SQLite store-scoped locking and a reliable canonical store identity are available.** If the
  SQLite single-host backend cannot enforce store-scoped locking or compute a reliable canonical
  store identity (a weak/network FS where advisory locks are inconclusive —
  [§7.4](#74-path-resolution-and-the-portability-of-fail-closed-startup-deferred-to-daemon-core)
  flags the family — or unresolvable path aliasing across symlink/case/short-name/UNC), the
  per-store single-writer guarantee
  ([§11.3](#113-server-side-delivery-fence-mr1--at-least-once-preserving)) cannot hold; the daemon
  **fails closed for that store**, and a portable store-identity/locking primitive must be found
  before that deployment is supported.
- **The harness can propagate identity to re-attach-capable verbs.** If a harness cannot propagate
  identity (`store_key`/`session_id`/`address`) to a re-attach-capable verb
  ([§14.6](#146-from-resolution-and-re-attach)), that verb cannot re-attach on `NeedsAttach`; a
  propagation-failure handling path must be added before that verb is supported.
- **Same-session membership mutations arrive in receipt order.** On evidence that same-session
  membership mutations can reorder over IPC (a later `Register`/`Detach` landing before an earlier
  one), add a **server-side, never-client-threaded monotonic membership op-seq** to order them; v1
  relies on the exchange applying membership ops in receipt order.
- **The `sessionEnd` hook fires on dismiss.** If the hook turns out NOT to fire on dismiss (to be
  spiked), the **idle-TTL becomes the primary dismiss bound** rather than the backstop, and its
  window is retuned accordingly.
- **The `sessionEnd` hook cannot be given a per-life token.** If a Copilot plugin API appears that
  lets the plugin pre-populate the hook's env from a value captured at `attach`, a **per-session
  cap** becomes the v1 path and
  [§7.1](#71-scoped-capability-model-v1-one-instance-admin-token) should re-tighten (not loosen).
- **`wait` can observe socket-EOF for reconnect.** If the `wait` re-attach-on-`NeedsAttach` path
  ([§14.4](#144-wait-and-re-attach-on-needsattach)) cannot be implemented because the chosen IPC
  transport masks socket-EOF, a positive-presence heartbeat from `wait` is forced.
- **The single-source SKILL mechanism is expressible without code changes.** If it
  ([§15.2](#152-single-source-skill)) hits a
  harness constraint (manifest cannot point outside the plugin dir **and** `exec` is rejected), a
  code-touching deviation is forced.
