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

Promotion boundaries below distinguish accepted intended changes from current
repository behavior and non-authoritative proposals.

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

### Accepted intended change: Application Client daemon bootstrap

**Promotion boundary:** this is accepted workstream direction for Application
Client issue #152. The current implementation still derives daemon peer and
spawn identity from the calling executable. The contract below must be promoted
through the normative daemon and Application Client design, implementation,
review, and conformance evidence before external consumers may rely on it.

- The production Application Client selects `InstalledCurrent` from an explicit
  trusted Telex install root. Configuration rejects a relative root or captures
  its canonical absolute identity when the configuration is created; reconnect
  never reinterprets the root against a later process working directory. A
  caller may instead select one exact, explicitly version-pinned executable for
  development and tests. Neither mode searches `PATH`, embeds daemon service
  behavior in the consumer, opens raw daemon IPC as a public seam, talks
  directly to a backend, or falls back to a foreign executable.
- `InstalledCurrent` resolves the root's `current` selector, selected version
  manifest, and versioned executable as one fail-closed snapshot. The root,
  selector-coordination lock, selector, manifest, version directory, and
  executable must resolve without symlink or reparse-point escape; be owned by
  the current OS user; and deny write, delete, or ownership control to other
  principals. Unsupported or inconclusive ownership, permissions, containment,
  file type, filesystem identity, or lock semantics is an actionable failure.
- The manifest must bind the selected tag and executable and supply validated
  build identity, package version, protocol major/minor, schema range, and
  required daemon capabilities. These fields are compatibility and selection
  metadata. They do not provide an executable-content digest, signature,
  publisher or package provenance, protection from malicious same-user
  administration, or intra-user isolation.
- The selected tag, manifest identity and load-bearing fields, canonical
  executable path, and platform file identity form an internal selection token.
  The token witnesses one resolution attempt and is not a bearer secret.
  Selector admission and platform file identity close selector and
  process-image races. The connected `Hello` handshake remains the final
  auth-policy, capability, and protocol check.
- One immutable resolved selection is used for both pre-`Hello` peer
  authentication and any spawn attempt. An existing daemon is accepted only
  when its reuse-safe PID/start-time, UID or SID, canonical process-image path,
  and platform file identity match that selection. Canonical path alone is not
  enough to prove that a prestarted process is the selected image.
- Selector movement is serialized by one persistent OS-backed coordination lock
  under the trusted install root. `InstalledCurrent` connect-or-spawn acquires a
  shared/read admission lock before reading the selector and holds it through
  manifest and executable resolution, prestarted or spawned peer
  authentication, and `HelloAck` readiness. Upgrade and rollback acquire the
  exclusive/write lock before resolving the old selection and hold it across
  authenticated drain, validated atomic selector switch, and publication.
  Their drain path operates inside that exclusive lock context and must not
  recursively acquire a shared lock. Lock acquisition order is selector lock
  before daemon singleton or spawn admission; the daemon does not acquire the
  selector lock while serving a drain request.
- Unix uses a local-filesystem advisory lock with process-crash release; Windows
  uses the equivalent owner-restricted shared/exclusive `LockFileEx` range lock.
  The persistent lock file is never replaced or deleted as part of selection.
  Filesystems that cannot prove the required ownership and shared/exclusive lock
  semantics are unsupported and fail closed. This is a coordination primitive,
  not a shell sidecar, service, or second runtime actor.
- While holding the shared lock, the client opens and identifies the selector,
  manifest, and executable, then re-reads and compares the complete selection
  immediately before spawning. Any unexpected selector, manifest, path, or file
  identity movement abandons the attempt and starts a fresh bounded resolution;
  exhaustion fails closed without using the stale or previous version.
- The child receives the non-secret selection token and, before publishing its
  capability, binding the serving endpoint, or reporting readiness,
  independently acquires a shared selector-admission lock and verifies that its
  own process image and a fresh `InstalledCurrent` resolution match the token.
  The child holds that lock through endpoint, capability, and readiness
  publication; the parent retains its own shared lock through authenticated
  `HelloAck`. If either process dies, the remaining or next admission still
  prevents stale-child publication. A mismatch or lock failure exits without
  serving. On Linux, open file descriptors plus canonical path and device/inode
  identity provide the race witness. On Windows, where process creation remains
  path-based, canonical final paths and volume/file identity are captured from
  handles, the executable handle is held with platform-compatible sharing
  through process creation, and the child performs the same final admission
  check before readiness.
- Upgrade installs and validates a candidate before drain, drains the selected
  daemon under the exclusive selector lock, atomically moves `current`,
  publishes the new selection, and lets the next connection resolve and spawn
  it. Rollback follows the same rule. `previous` is never independently
  acceptable; it becomes acceptable only after validated rollback makes it
  `current`. A skipped drain or selector mutation outside the coordinated
  writer path may leave an old process occupying the singleton, but new clients
  reject it and fail actionably rather than weakening peer identity or spawning
  around it. The root Telex CLI may use the same installed-current resolver;
  exact-target development mode remains pinned, applies the same canonical
  process-image and untrusted-writability checks, and does not follow upgrade.
- The same mechanism must have Windows and Linux process tests covering an
  already-running matching daemon, current-version spawn, upgrade, rollback,
  movement at each resolution/spawn admission boundary, stale prestarted
  images, selector-client death during spawn, PID reuse, symlink/reparse escape,
  untrusted writability, incompatible manifest or build metadata, platform file
  identity mismatch, and readiness refusal. SQLite and Postgres use the same
  daemon selection and peer-authentication contract.
- Normative promotion for issue #152 reserves ADR 0053 for this decision,
  preserving ADR 0052 for the independent station-intent work in PR #138.

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

## Accepted intended change: daemon-replacement station intent

**Promotion boundary:** the operator accepted this workstream direction for
issue #106 and PR #138 on 2026-09-02. The current implementation does not
provide station intent. PR #138 must promote the matching normative design,
implementation, review, and conformance evidence before clients may rely on
this behavior.

The accepted design intentionally clears in-memory membership, including its
`on_deliver` handler, when the daemon is replaced. Durable messages survive,
but a still-live push producer cannot by itself prove that the replacement
daemon should restore that handler. A host-local, owner-private, versioned
**desired station intent** records the requested push registration for one
`(store_key, session_id, address)` binding. The intent is never membership,
attendance, a lease claim, positive liveness, or permission to deliver.
Restoration remains subordinate to:

- verified live producer identity and an authenticated, bounded probe;
- matching host, boot, protocol, handler, store, and owner-private credential
  scope;
- explicit detach tombstones and operator reset;
- the existing epoch claim and delivery fence;
- bounded reconciliation, backoff, quarantine, garbage collection, drain, and
  successor-handoff behavior; and
- the merged Copilot App bridge-host lifecycle semantics above.

Each canonical intent has one persistent, owner-private OS advisory lock file,
and every pathname mutation for that intent holds its lock. Unix uses `flock`;
Windows uses the equivalent `LockFileEx` lock. The lock file is never removed
or replaced, and ownership is never taken over because of age. Acquisition is
bounded, process death releases the OS lock, and an alive but hung owner causes
a bounded operation failure instead of concurrent mutation. Unsupported or
inconclusive filesystem, ownership, or lock semantics fail closed. In
particular, station-intent authority does not assume safe lock behavior on NFS,
SMB, 9p, or another remapped filesystem without platform evidence.

The four-second daemon response bounds how long the caller waits; it cannot
cancel a synchronous filesystem operation already entered. One admitted
single-file mutation may therefore complete after the response. The
non-stealable OS lock prevents a newer writer from publishing while that
operation remains alive, and the generation check still prevents stale work
from committing after a newer generation has legitimately acquired admission.
A stale generation must never delete or replace newer station-intent state.

Directory discovery and garbage collection retain a bounded response contract,
not unconditional completion under a persistently slow or blocked directory
enumeration. Truncation is observable as degraded discovery.
`observed_count` is a lower bound after a partial scan. A partial scan cannot
claim complete garbage collection, exact over-cap recovery, or automatic
restoration of stable tail entries. Eventual coverage is conditional on each
required enumeration and read operation eventually completing during a
maintenance opportunity. Recovery from persistent truncation is relocation to
supported local storage or an offline complete scan.

This degraded contract is an accepted temporary gap. The mandatory downstream
`station-intent-transactional-authority` node
([issue #153](https://github.com/lossyrob/telex/issues/153)) must replace flat-file generation
and root enumeration with transactional generation authority, seekable fair
discovery and garbage collection, exact counts, and exact over-cap recovery.
That node follows PR #138 and does not block PR #138, but it blocks Local
Daemon closure so the accepted gap cannot disappear from required work.

PR #138 must promote ADR 0052 and the normative station-intent section of
`docs/design/daemon.md`. The promotion must define the lock support floor,
late-operation boundary, degraded status and lower-bound counts, conditional
liveness, recovery path, and transactional follow-up. It must preserve
`MemberRecord` registration, owner-private runtime storage, Copilot producer
credentials and protocol, detach/reset precedence, status projection,
reconciliation scheduling, upgrade/rollback drain handoff, and SQLite and
Postgres recovery behavior.

## Remaining questions and confidence

- **High confidence:** the accepted current design outside the named promotion
  boundaries is directly represented by merged normative documents and
  accepted ADRs.
- **High confidence:** the accepted installed-current direction closes the
  external-consumer bootstrap gap without weakening pre-`Hello` peer
  authentication or making the consumer a daemon host.
- **Implementation detail to prove:** the exact cross-platform file-identity,
  handle-sharing, selector-token transport, child admission, and bounded retry
  APIs remain for issue #152 to implement and test without claiming
  content-integrity, signature, or package provenance.
- **High confidence:** issue #106 exposes a gap between durable messages and
  volatile desired push registration without weakening the explicit-membership
  rule.
- **High confidence:** persistent OS locking closes the demonstrated
  stale-generation pathname race on supported local filesystems without
  claiming cancellation of an already-entered filesystem operation.
- **High confidence:** bounded restart-at-head directory enumeration cannot
  guarantee stable-tail coverage unless the required enumeration and read
  operations eventually complete.
- **Implementation detail to prove:** PR #138 must prove equivalent Unix and
  Windows lock behavior, fail-closed support detection, degraded status, and
  offline recovery without weakening the four-second response contract.
- **Downstream design detail:** the transactional node owns the exact local
  storage, migration, cutover, rollback, corruption, and old-writer refusal
  design needed to restore unconditional authority.
