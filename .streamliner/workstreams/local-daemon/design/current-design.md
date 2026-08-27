# Local Daemon Current Design

## Status and authority

This file is the canonical integrated design for the `local-daemon` workstream.
It summarizes accepted repository authority; it does not replace the normative
documents.

Authority is ordered as follows:

1. [`docs/design/daemon.md`](../../../../docs/design/daemon.md) defines the
   mechanism-level daemon, IPC, membership, liveness, fencing, delivery, and
   lifecycle contracts.
2. [`docs/design/DESIGN.md`](../../../../docs/design/DESIGN.md) defines the
   architecture and product framing.
3. Accepted entries in
   [`docs/design/DECISIONS.md`](../../../../docs/design/DECISIONS.md) preserve
   the decision trail. ADR 0023 governs wherever earlier local-daemon ADRs
   describe superseded incarnation, stale-attendance, takeover, or
   waiter-acknowledgement machinery.
4. [`docs/design/copilot-bridge-push.md`](../../../../docs/design/copilot-bridge-push.md)
   defines the accepted Copilot push integration without changing the
   harness-neutral daemon boundary.

The pending-promotion section below is intentionally non-authoritative.

## Accepted current design

### Local exchange responsibility

- Telex supplies one auto-spawned local exchange per
  `(user identity, canonical config root, protocol major)`. One exchange serves
  multiple stores; every request names its store explicitly.
- Sessions use one-shot `attach`, `wait`, `send`, `reply`, disposition, and
  `detach` clients. There is no resident per-session lease holder.
- `attach` is the deliberate spawning and membership-establishing verb. Other
  operations fail actionably rather than creating an exchange or station from
  guesses.
- SQLite and Postgres implement the same station and delivery semantics.
  SQLite relies on the owner-private local singleton; Postgres additionally
  relies on the durable lease-epoch fence for competing writers.

### Trust, startup, and IPC

- The runtime directory and daemon capability are owner-private and fail
  closed. Clients authenticate the local server process before sending
  metadata, then perform the version/capability handshake.
- The v1 trust boundary is same-user administration, not intra-user isolation.
  Harness identities such as a Copilot session id identify a station but are
  not security principals.
- Startup uses the singleton lock, readiness acknowledgement, bounded
  connect-or-spawn behavior, and protocol-major isolation. A blocked `wait`
  reconnects across an ordered daemon restart within its grace window.

### Station identity, membership, and liveness

- A station combines a durable, epoch-fenced lease with an in-memory
  `MemberRecord` for `(store_key, session_id, address)`. The member record is
  the attendance authority while the daemon is running.
- `session_id` is unique and stable. Membership is explicit-only: `Register`
  creates or refreshes it, and no durable history, lease row, bridge file, or
  liveness observation implicitly recreates it.
- After a daemon replacement, membership starts empty. An unknown operation
  returns `NeedsAttach`; the agent explicitly re-attaches only the addresses it
  still wants. Durable messages and lease high-water state survive, but removed
  membership is never resurrected.
- Liveness is non-destructive. The authoritative `sessionEnd` hook,
  negative-only PID death, the at-least-one-day idle backstop, and operator
  reset may release blocked waiters and mark a member idle; they do not delete
  membership or buffered messages.
- Pull/non-bridge Copilot sessions use the loader PID plus process start time as
  a negative-only anchor. A live PID is not positive proof of attendance.

### Fencing and durable delivery

- Lease ownership is fenced by monotonically increasing epochs. Heartbeat,
  release, consumption marking, and delivery ownership are conditional on the
  current epoch; a loser self-demotes. Ordered handoff quiesces the old owner
  before the new owner claims a higher epoch.
- The backend delivery row is authoritative. Delivery follows
  `EMIT -> agent Ack -> MARK consumed`; printing, IPC handoff, or accepting a
  pushed turn is never consumption.
- Delivery is at-least-once. Duplicate presentation is the safe failure
  direction and is handled by `message_id` plus a bounded, epoch-scoped
  in-memory fast path. Unacknowledged work remains durably recoverable.
- Explicit detach removes the live member and atomically records the durable
  detach tombstone used by push self-stop. A later explicit attach clears it.
  The tombstone is a stop fence, not a source for rebuilding attendance.

### Pull and push delivery

- `Wait` is the harness-neutral pull path. It blocks on daemon IPC and returns
  one eligible durable message; the agent then acknowledges and dispositions
  that exact recipient delivery.
- Push is an opt-in, harness-neutral `on_deliver` argv on the member record.
  The daemon executes it after durable commit, outside the acknowledgement
  critical path, under timeout, concurrency, retry, suppression, and status
  bounds. Daemon core does not contain Copilot or SDK concepts.
- The Copilot bridge and `telex copilot push` form the harness adapter. A push
  accepts or defers a turn but never acknowledges the Telex message.
  Non-interrupt work is deferred while the root agent is busy and revalidated at
  turn-stop; `interrupt` uses immediate delivery.
- Push health is derived from daemon-observed attempt outcomes. Accepted and
  busy-deferred outcomes are healthy push coverage; probing, stale accepted,
  failing, suppressed, and dead-letter states remain distinguishable. The
  daemon never treats the Copilot registry itself as attendance authority.

### Copilot App lifecycle

- Bridge attach begins without inheriting ambient loader/session PID anchors.
- In Copilot App, `sessionEnd(reason=complete)` can mean turn-idle rather than
  durable session termination. When the bridge registry is fresh, that event
  preserves membership and turn-guard state and explicitly replaces the
  member's watched predicates with the bridge host PID.
- The bridge host PID is stable across extension reloads. Its death is the
  negative teardown signal for a true App or one-shot CLI exit. Terminal
  lifecycle reasons, or a missing/stale bridge, continue through the normal
  non-destructive reap path.
- A bridge busy deferral remains `station_health: attended_push` with
  `push_delivery: deferred`; it is not reported as an unreachable station.

### Boundaries and exports

- The daemon owns local presence, epoch-fenced transport, buffering, and the
  harness-neutral push hook. Harness lifecycle interpretation and bridge
  credentials stay in the Copilot adapter.
- Application-level workflows, routing policy, operator mediation, and the
  shared Application Client API remain outside daemon core.
- The daemon exports its versioned Layer-1 IPC and station semantics for the
  Application Client workstream. It does not require a hosted Telex control
  plane.

## Superseded assumptions

- A resident per-session holder and repeatedly re-armed waiter are not the
  presence architecture.
- Session incarnation tokens, durable session-currency rows, implicit
  re-registration from history, destructive stale-attendance teardown,
  force-takeover nonces, and their per-address tombstones are superseded by
  stable `session_id`, explicit membership, and non-destructive liveness.
- Waiter receipt or push acceptance is not the durable consume fence; explicit
  agent acknowledgement is.
- Daemon core does not learn Copilot bridge, port, pipe, registry, or SDK
  semantics.

## Pending promotion: daemon-replacement station intent

**Non-authoritative provenance:** issue #106 and PR #138.

The accepted design intentionally clears in-memory membership, including its
`on_deliver` handler, when the daemon is replaced. Durable messages survive,
but a still-live push producer cannot by itself prove that the replacement
daemon should restore that handler. Generic explicit recovery can therefore
re-attach a station without restoring push intent. This is a real design
pressure, not accepted station-intent authority.

The proposed direction is a host-local, owner-private, versioned **desired
station intent** for one `(store_key, session_id, address)` push binding. Such
an intent would describe desired registration only. It must never become
membership, attendance, a lease claim, positive liveness, or permission to
deliver. Any future restoration must remain subordinate to:

- verified live producer identity and an authenticated, bounded probe;
- matching host, boot, protocol, handler, store, and owner-private credential
  scope;
- explicit detach tombstones and operator reset;
- the existing epoch claim and delivery fence;
- bounded reconciliation, backoff, quarantine, garbage collection, drain, and
  successor-handoff behavior; and
- the merged Copilot App bridge-host lifecycle semantics above.

No station-intent manifest, reconciler, proposed ADR 0050 behavior, or
successor restoration rule is current authority. Promotion requires an
integrated implementation and design change to pass review against the
then-current daemon, IPC, upgrade, and Copilot bridge contracts and merge into the
repository authority. Until then, daemon replacement continues to require
explicit re-attachment and may require explicit push re-provisioning.

If promoted, the affected contracts are `MemberRecord` registration,
owner-private runtime storage, Copilot producer credentials and protocol,
detach/reset precedence, status projection, reconciliation scheduling,
upgrade/rollback drain handoff, and both SQLite and Postgres recovery tests.
The workstream design must then replace this section with the accepted
contract and link the new ADR and normative daemon section.

## Remaining questions and confidence

- **High confidence:** the accepted design above is directly represented by
  merged normative documents and accepted ADRs.
- **High confidence:** issue #106 exposes a gap between durable messages and
  volatile desired push registration without weakening the explicit-membership
  rule.
- **Not yet accepted:** the exact station-intent schema, proof protocol,
  reconciliation state machine, retry/GC policy, anti-downgrade behavior, and
  successor command contract remain behind the promotion boundary.
