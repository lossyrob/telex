# Telex Local Exchange -- Architecture Overview (visual)

> **Non-normative explanatory diagrams.** These teach the local-exchange architecture; the
> governing specification is [`daemon.md`](daemon.md). **If any diagram conflicts with `daemon.md`,
> `daemon.md` wins.** Each diagram below names the `daemon.md` section(s) it is drawn from.

This is the **visual on-ramp** to the daemon design: read it before the normative contract to build
a mental model, then drop into [`daemon.md`](daemon.md) for the precise rules. Five diagrams, in
learning order:

1. **Component map** -- what the pieces are and how the daemon comes to exist.
2. **Message delivery** -- how a message reaches a recipient at-least-once, and when it is "consumed".
3. **Restart & re-attach** -- what a daemon restart loses vs. retains, and how a station recovers.
4. **Station liveness** -- how a station is deemed idle, and why that is never destructive.
5. **Single-writer correctness** -- how exactly one writer per store is guaranteed across restart,
   upgrade, and multi-host.

The word **epoch** appears by name in diagrams 2-4 and is *defined* in diagram 5.

---

## 1. Local exchange component map

**Answers:** What are the pieces, where is the per-user singleton boundary, and how does the daemon
come to exist with no manual start command?

```mermaid
flowchart TD
    A["Session A<br/>attach / wait / ack / detach"] -->|local IPC| EP
    B["Session B<br/>send / reply / status"] -->|local IPC| EP

    subgraph EXCH["Local exchange -- ONE auto-spawned daemon per (user SID, config root, protocol-major)"]
        direction TB
        EP["IPC endpoint<br/>(owner-only socket / named pipe)"]
        REG["Attendance registry<br/>(in-memory, explicit-only membership)"]
        BUF["Durable delivery buffer<br/>(at-least-once + message_id dedup)"]
        POLL["Poll / LISTEN-NOTIFY loop"]
        HB["Lease heartbeat<br/>(single writer of liveness)"]
        PID["pid-watch<br/>(loader-pid liveness)"]
        EP --- REG
        EP --- BUF
        POLL --- BUF
        HB --- REG
        PID --- REG
    end

    EXCH --> DRV["Backend driver<br/>(single writer)"]
    DRV --> STORE[("SQLite / Postgres<br/>durable leases + deliveries")]

    SPAWN["Auto-spawn: there is no 'telexd start'.<br/>The first client does connect-or-spawn under a spawn-lock;<br/>exactly one winner binds the singleton, the rest connect."]
    SPAWN -.->|bootstraps| EXCH
```

Sessions run **one-shot verbs** (no resident process). A **station** is a registration in the
exchange: a durable lease row plus the in-memory attendance record. The exchange is the only writer
of liveness/delivery state for its store.

*Governing spec:* [daemon.md sec.1](daemon.md#1-the-local-exchange) ,
[sec.2.2 auto-spawn](daemon.md#22-auto-spawn-connect-or-spawn-and-the-spawn-lock) | Last reviewed: 2026-06-24

---

## 2. Message delivery & the at-least-once fence

**Answers:** How does a message reach a recipient at-least-once, and when is it "consumed"?

```mermaid
sequenceDiagram
    actor S as Sender session
    participant X as Local exchange
    participant DB as deliveries(message_id, recipient)
    participant W as Recipient wait (blocking)
    actor R as Recipient agent

    Note over W: a long-running 'wait' is already blocked on the address (not a push/WebSocket)
    S->>X: send / reply (message)
    X->>DB: INSERT (message_id, recipient) -- durable
    X-->>W: EMIT (one delivery)
    W-->>R: PRINT to stdout (TRANSPORT ONLY, not the consumed mark)
    R->>X: ack (message_id, address)
    X->>DB: epoch-guarded MARK consumed
    Note over DB: the consumed row is RETAINED, so a consumed message is never resurrected, and recipients dedup by message_id
    alt crash before the ack
        Note over X,R: nothing was marked, so the message is redelivered (at-least-once)
    end
```

The durable `deliveries` row is the dedup source of truth. The **stdout PRINT is transport only**;
the **agent's explicit `ack` is the durable fence** (a single ack -- there is no separate waiter
`DeliveryAck`). "Consumed" is decided by the agent, epoch-guarded so a superseded daemon cannot mark.

*Governing spec:* [daemon.md sec.11.3 delivery fence](daemon.md#113-server-side-delivery-fence-mr1--at-least-once-preserving) ,
[sec.13 dedup](daemon.md#13-delivery-and-seen-dedup) | Last reviewed: 2026-06-24

---

## 3. Restart & re-attach recovery

**Answers:** After a daemon restart, what is lost vs. retained, and how does a station regain
membership?

```mermaid
sequenceDiagram
    actor R as Agent
    participant X as Local exchange
    participant DB as Durable store (lease rows + deliveries)

    Note over R,X: agent is attached, membership is IN-MEMORY, and messages are buffered durably in the store
    Note over X: crash / restart
    Note over X,DB: restart loses ONLY the in-memory membership. Lease rows and the durable buffer PERSIST, and nothing is rebuilt from history.
    R->>X: wait (or any op)
    X-->>R: NeedsAttach  (unknown session)
    R->>X: attach / Register  (explicit re-attach)
    R->>X: wait
    X-->>R: delivers the durably-buffered messages (at-least-once, no loss)
    Note over R,X: a previously Detached address is NOT resurrected. Only the addresses the agent explicitly re-attaches come back
```

Recovery is an **ordered handshake**, not an automatic rebuild: the exchange never reverse-indexes
durable rows into membership. The agent is told (`NeedsAttach`) and re-establishes exactly the
addresses it wants.

*Governing spec:* [daemon.md sec.14.3 crash recovery](daemon.md#143-crash-recovery-and-re-attach) ,
[sec.14.4 NeedsAttach](daemon.md#144-wait-and-re-attach-on-needsattach) | Last reviewed: 2026-06-24

---

## 4. Station liveness states (non-destructive reaping)

**Answers:** How is a station deemed idle, and why is that never destructive?

```mermaid
stateDiagram-v2
    [*] --> Unregistered
    Unregistered --> Attended: attach (Register)
    Attended --> Idle: reap signal (sessionEnd hook / loader-pid death / idle-TTL at least 1 day)
    Idle --> Attended: a new message wakes it, or the agent re-arms wait
    Attended --> Unregistered: explicit Detach
    Idle --> Unregistered: explicit Detach
    note right of Idle
        Reaping = release blocked waiters (PresenceEnded, exit 5) + mark Idle.
        Membership + the durable buffer PERSIST across Idle.
        A station is NEVER destroyed by liveness; a wrong call costs
        at most one waiter re-arm, never data loss.
        Operator Reset = the same non-destructive path. There are no tombstones.
    end note
```

Liveness is a **non-destructive UX dial, not a correctness gate**. There is deliberately **no
Destroyed state**: a station can idle for days and wake on the next message.

*Governing spec:* [daemon.md sec.9 liveness](daemon.md#9-liveness-model) ,
[sec.10 reaping + idle-TTL](daemon.md#10-reaping-and-the-idle-ttl-backstop) | Last reviewed: 2026-06-24

---

## 5. Single-writer correctness: the epoch fence + ownership handoff

**Answers:** How does telex guarantee exactly one writer per store across restart, upgrade, and
multi-host?

An **epoch** is the single-writer fence: a monotonic `lease_epoch` plus the owning daemon's
`owner_instance_id`. A successor wins by atomically incrementing the epoch; the old owner discovers
it has been superseded on its next write and steps down.

```mermaid
sequenceDiagram
    participant A as Daemon A (owner @ epoch E)
    participant DB as Lease row (lease_epoch, owner_instance_id)
    participant B as Daemon B (successor)

    A->>DB: heartbeat @ epoch E (rowcount = 1, still owner)
    Note over A,B: a handoff / upgrade / reclaim brings B
    B->>DB: claim (CAS lease_epoch E to E+1, owner = B)
    A->>DB: next heartbeat / mark @ epoch E
    DB-->>A: 0 rows, NotOwner
    Note over A: A self-demotes, stopping emitting AND heartbeating
    Note over A,B: exactly one writer, so no double-delivery and no ownership flip-flop
    alt SQLite (single host)
        Note over A,DB: OS-singleton + canonical-store lock allow only one daemon at a time. An upgrade is release + next-call respawn (no live two-daemon overlap)
    else Postgres (multi-host)
        Note over A,B: a live ordered handoff (owner-directed transfer). Cross-host reclaim is arbitrated in epochs, not timing
    end
```

Three layers enforce single-writer: the **OS-singleton** (per config root), a **canonical-store
lock** (per SQLite store), and the **lease-epoch fence** (the multi-writer Postgres authority).

*Governing spec:* [daemon.md sec.11 lease-epoch fence](daemon.md#11-lease-epoch-fence-the-spine) ,
[sec.11.4 ordered handoff](daemon.md#114-ordered-handoff--owner-directed-atomic-transfer-sf3) | Last reviewed: 2026-06-24

---

## Keeping these honest (drift policy)

These diagrams are explanatory companions, capped at **5** to stay maintainable:

- **One question per diagram**; a diagram may not introduce states or terms not justified by its
  `Governing spec` footer anchors.
- **Update trigger.** A PR that changes daemon semantics, the referenced `daemon.md` anchors, or the
  code implementing delivery, attach/re-attach, liveness/reaping, or lease/single-writer behavior
  must update this file (or state why no diagram changed).
- **Restamp.** Refresh each `Last reviewed` date when a diagram is re-verified against its anchors;
  a sweep restamps or removes stale diagrams.
- `daemon.md` remains the single source of truth; these never encode a contract that is not already
  in it.
