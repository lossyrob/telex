# Work Shaping: Station Intent Reconciliation

Revised after the society-of-thought planning review and again after the cycle-2 verification review
(MF2-1, MF2-2, SF2-1..SF2-6, CO2-1..CO2-4). Contradictory and superseded text has been removed;
every previously open question is now decided. `Plan.md` holds the authoritative decision record and
constants; this document holds the shaping context those decisions rest on.

## Problem Statement

Live push stations lose daemon membership and their generic `on_deliver` handler when the shared
daemon drains, crashes, restarts, or is replaced during upgrade. The Copilot bridge can remain
healthy, but durable messages stop arriving as turns because the replacement daemon has no
registration intent to reconstruct. Generic command recovery can make the state worse by recreating
the station without push, silently downgrading it to an unattended pull station.

The work must make same-major daemon replacement self-healing for live local push producers without
treating durable state as proof of attendance, resurrecting detached stations, weakening epoch
fencing, or coupling the daemon to Copilot-specific files.

## Core Functionality

- Replace the address-only Copilot binding list with an owner-private, atomic, versioned local
  station-intent manifest stored under the daemon runtime directory, namespaced by the daemon
  singleton hash (`<run_dir>/intents/<singleton_hash>/`). That root is the one the codebase already
  hardens fail-closed on both Windows and Unix, and the namespace preserves config-root isolation
  and protocol-major separation.
- Represent each exact binding by store identity, session, address, delivery mode, CC settings and
  CC watermark, metadata, recovery generation, a typed handler descriptor, and a typed producer
  descriptor carrying transport, endpoint, process identity (pid, start time, executable, host id,
  boot id), protocol range, and a *pointer* to the producer-managed credential. The pointer is
  constrained rather than free-form: it names a **registered producer root**, must canonicalize
  strictly under that root with no symlink or reparse point on the chain, must pass an owner-private
  **per-file** security check on both platforms (Unix uid + mode; a new Windows owner-SID/DACL/
  inherited-ACE validator, since containment in the intent scope cannot apply to a file that lives
  outside it by construction), and carries a bounded `max_age_ms` whose expiry makes the intent
  unverifiable with no secret read and no connection attempt.
- Keep membership in daemon memory. Intent is desired local registration state, not attendance.
- Keep the daemon harness-neutral: it resolves generic descriptor kinds and registered producer
  roots, never Copilot files, filenames, or symbols. The producer credential is read from the
  descriptor's pointer at reconcile time, so a per-process rotating secret is always current and is
  never persisted in the intent.
- Reconcile compatible live intents when a same-major successor daemon starts and on the existing
  heartbeat tick, in passes that are bounded in wall clock by a pass deadline shorter than the tick
  interval, so scheduled work can never overrun its own cadence.
- Verify the endpoint peer before presenting any credential (same-user, executable, pid and start
  time), then complete a nonce challenge; both checks are one identity proof, not two independent
  ones.
- Honor explicit revocation and durable detach tombstones, checked before the epoch claim and again
  after it, unconditionally on the reconcile path. Only explicit attach/resume may clear a
  tombstone and publish a new generation; the reconcile path cannot reach the clearing branch at
  all.
- Claim a new lease epoch through the existing backend CAS and fence the previous owner before
  enabling push. Treat "the previous owner's lease is simply not stale yet" as a *deferred* outcome
  retried at a fixed short cadence, not as a failure that backs off exponentially - that
  distinction is what makes the published crash-recovery bound derivable.
- Rebuild the handler through a registered handler kind whose argv comes from one pure shared
  builder - the same one the attach path uses - parameterized by the currently running installed
  versioned binary, the daemon's resolved store selector, the validated session id, and the daemon
  instance id used as the crash-window fence. Preserve the exact wake-on-CC watermark and sweep
  durable undelivered messages.
- Prevent generic recovery from silently registering pull when a known live push intent exists, with
  the guard inside the daemon so old clients are covered too, and with the inline reconcile entered
  through a guard-free inner API so the register path's outermost admission guard is never
  re-acquired.
- Surface intent, member, and producer state - including intent-only rows and per-attempt evidence -
  in diagnostics, and record an append-only reconcile event trail.
- Report recovery exposure during graceful drain, upgrade, and rollback from a cached in-memory
  intent index, with no directory scan, probe, or network I/O, so it cannot exceed the drain
  timeout.

## Supporting Work

- Promote the owner-private filesystem primitives, a new per-file Windows owner/DACL/reparse
  validator, and the process-identity primitives (`process_exe_path`, `host_id`, `boot_id`) out of
  the daemon module so writer and reader share one cross-platform, fail-closed implementation and no
  parallel implementation appears in the Copilot command layer.
- Split the existing client-only argv constructor into a pure shared builder plus a thin
  client-side wrapper, and add one named, fallible `store_key -> selector` mapping for the daemon
  side.
- Add a `probe` verb and a pure, importable protocol module to the bridge, and make CI run the JS
  tests by glob instead of a single hardcoded file.
- Add a fake producer endpoint harness, a startup-path test daemon, credential-file security
  fixtures, and an explicit reconcile trigger plus per-pass report subscription as the test seam -
  no cadence environment override, and no change to the heartbeat interval's tested invariant.
- Make upgrade and rollback start the successor they installed and wait, bounded, for the reconcile
  report on that same trigger/report seam.
- Update daemon, architecture, Copilot bridge, operating, troubleshooting, release-runbook, and
  destructive-testing documentation, plus one new ADR.
- Extend SQLite, Postgres, conformance, process, Copilot plugin, and Node bridge tests, including
  negative controls that fail if reconciliation never runs.

## Edge Cases and Expected Handling

| Case | Expected handling |
|---|---|
| Explicit Copilot detach | Durable tombstone first, then exact per-binding intent revocation; never auto-restore. |
| Crash between tombstone and revocation | Tombstone wins; the reconciler refuses the intent and GC removes it. |
| Explicit resume after detach | Clear the tombstone, write a new generation, probe the live producer, register, sweep backlog. |
| Crash during first attach | Intent stays `pending`, is never reconciled, and is GC'd after the pending TTL. |
| Dead producer or stale registry | Reject reconciliation; leave backlog durable; report missing live producer. |
| Predecessor's lease not yet stale after a crash | Deferred, not failed: retried at a fixed short cadence bounded by the reconcile interval, with no exponential backoff and no quarantine counting, so recovery still lands inside the published crash bound. |
| Credential file older than its declared `max_age_ms` | Intent becomes unverifiable: no secret is read, no connection is attempted, the state is reported, and the intent is GC-eligible after the unverifiable TTL. A fresh attach/resume is the way out. |
| Credential path outside its registered producer root, or reached through a symlink/reparse point | Refused before any read; the intent is unverifiable and no secret leaves disk. |
| Credential file with relaxed permissions (including a Windows DACL granting another principal) | Refused by the per-file owner/DACL validator; the intent is insecure, reported, and GC-eligible. |
| Scope holding more intents than the write-time cap | Nothing is deleted for being over cap; writes are refused with a typed error, the condition is reported, and the round-robin cursor still gives every entry eventual coverage. |
| Scope larger than one pass budget | Not covered by the published recovery bounds; covered instead by a deterministic maximum queue delay derived from guaranteed per-pass progress and the tick interval. |
| Fresh file but failed peer verification or challenge | Reject as stale/broken; do not claim the station. |
| PID reused with a different start time, or uncapturable start time | Reject the intent; the check fails closed, never open. |
| Same pid/start time after reboot, or a synced/network home | Host id and boot id mismatch make the intent stale. |
| Producer that predates the probe verb | State `legacy_producer`: no auto-recovery, no wedge, documented manual `copilot resume` still works. |
| Advertised but unverifiable or incompatible protocol | Fail closed with actionable diagnostics; never downgrade to pull; never block agent turns. |
| Insecure or malformed manifest | State `insecure`; ignored, reported, GC-eligible. |
| Same session with multiple addresses/stores | Restore every compatible binding independently with exact store/mode/CC settings. |
| Competing live sessions for one address | Normal lease epoch CAS; deterministic scan order; never force-steal any incumbent, reconciled or fresh; report the loser. |
| Message committed during restart gap | Delivered after successful reconciliation, including CC messages, because the CC watermark is preserved rather than re-stamped. |
| Push accepted before drain but unacked | The same message ID may redeliver at least once; never marked consumed during recovery. |
| Consumed or terminally dispositioned message | Never redelivered. |
| In-flight old handler during fencing | Graceful drain waits, bounded, for in-flight handlers; on the crash path the helper aborts when the cap file instance id has changed; the old owner can never mark consumption after epoch advancement. |
| Previously suppressed push | Restarts with clean attempt state and may be attempted once more; bounded by the existing backoff/hard-cap policy and documented as at-least-once. |
| Repeated reconciliation | Idempotent refresh with no retry-state reset, no duplicate sweep, and no duplicate concurrent push ownership. |
| Pull waiter armed on the same address | Existing precedence stands: the waiter wins, the intent is deferred with backoff rather than permanently failed, and the conflict is reported. |
| Mass restart or host resume | Bounded pass budget, per-pass wall-clock deadline, concurrency cap, and jittered failure backoff prevent a reconciliation herd; passes never overrun the tick that started them. |
| Orphaned or accumulated intents | Count and size caps, TTL, and GC bound the set; the turn guard warns and allows, so an orphan cannot wedge a session. |
| No successor daemon | Keep intents and producers alive, surface degraded state; recovery begins when a successor exists. Upgrade and rollback start their own successor. |
| Old daemon, new client | Client-side capability gate refuses to write or finalize intent and refuses pull auto-registration over a live push intent. |
| Protocol-major change | Separate singleton hash means a separate intent scope; no cross-major discovery. |

## Rough Architecture

1. Copilot attach/resume writes a `pending` `StationIntentV1` under
   `<run_dir>/intents/<singleton_hash>/`, resolved through `DaemonPaths::current()` by both writer
   and daemon, using the shared owner-private atomic write primitive.
2. The bridge registry gains protocol version, bridge generation, and process start time; the bridge
   gains a `probe` verb implemented in a pure, importable, unit-tested protocol module.
3. After the daemon confirms push is armed, the Copilot CLI performs the probe itself, captures
   producer identity and the CC watermark, and atomically finalizes the intent to `live`. Failure
   rolls back the bridge and removes the intent.
4. A successor daemon scans the intent scope asynchronously at startup and on the existing heartbeat
   tick - one loop, pulsed through an explicit trigger that startup, upgrade/rollback, the admin
   request, and tests all share - under single-flight, drain suppression, a pass budget, a per-pass
   wall-clock deadline shorter than the tick, a concurrency cap, per-intent timeouts, and failure
   backoff. Each completed pass publishes a report, which is how upgrade/rollback and tests observe
   progress without polling.
5. For each candidate the daemon validates schema and compatibility, per-file security of both the
   manifest and the pointed-to credential (including containment under a registered producer root),
   revocation and durable tombstone, producer identity and host/boot binding, then verifies the
   endpoint peer before presenting the credential resolved from the descriptor pointer, then
   completes the nonce challenge.
6. The daemon claims the backend epoch lease through normal CAS - with an unconditional tombstone
   check before the claim and a re-check after it, and with the tombstone-clearing branch
   structurally unreachable from this path - creates the `MemberRecord` from the registered handler
   kind via the shared argv builder, restores the preserved CC watermark, and runs the existing
   durable backlog sweep. An incumbent lease that is not yet stale defers at a fixed cadence rather
   than failing. Refresh passes preserve push retry state and perform no backend write.
7. Generic register paths inside the daemon consult intent state before creating a new member, so a
   live compatible push intent reconciles or fails visibly and can never be downgraded to pull; the
   inline reconcile uses the guard-free inner entry point because the register path already holds
   the outermost per-station admission guard.
8. Status projects intents and members together from a cached in-memory intent index, with typed
   states, precedence, and per-attempt evidence, plus a rotating NDJSON reconcile event log. Drain,
   upgrade, and rollback report recovery exposure from that index alone, with an explicit
   as-of timestamp; upgrade and rollback additionally start the successor and wait, bounded, for its
   report.

## Critical Analysis

### Chosen direction

Use host-local manifests as the authoritative intent surface and keep backend rows as the
attendance/ownership fence. This makes the cross-host Postgres prohibition structural, avoids
treating shared backend data as local-process proof, and limits backend schema changes to none. The
manifest is generic and typed - producer and handler are descriptor kinds - so the daemon remains
harness-neutral and ADR 0039's boundary holds without an exception.

### Rejected primary directions

- Backend-primary intent requires a second local proof layer anyway and adds cross-host, migration,
  and SQLite/Postgres parity risk.
- Graceful state transfer cannot solve hard crashes and conflicts with SQLite's single-holder
  lifecycle.
- A stable supervisor is substantially larger than the required recovery mechanism.
- Copilot-specific repair preserves the current harness coupling problem and does not safely fix
  generic recovery.
- Storing intents under the Copilot bridge root was rejected: it is not authority-bearing, its
  permission helpers are Unix-only, and one shared set across all config roots would break the
  isolated destructive-testing guidance this work must add. (This work does now harden that root
  with the shared owner-private primitive so the credential file it holds is checkable on Windows -
  but it remains a *producer* root that is read, never the intent home.)
- Persisting the bridge secret in the intent was rejected: it rotates per bridge process, so it
  would convert the headline recovery scenario into permanent fail-closed non-recovery and would put
  a long-lived capability at rest.

### Council record and preserved dissent

The bounded architecture council recommended local `StationIntentV1` manifests with daemon-owned
reconciliation, typed stable-launcher handlers, authenticated endpoint challenge, and fail-closed
generic recovery. Its preserved dissent favored a one-way bridge wake signal and, secondarily,
backend-resident intent if cross-host operator discovery ever becomes a requirement.

Both dissents are now adjudicated rather than left open. The wake signal is declined: the bridge
stays observation-only, and the triggers that replace it are the daemon startup scan, the heartbeat
tick, the upgrade/rollback successor spawn, and an admin-proofed explicit reconcile request issued
by the existing `agentStop` hook when a daemon is already reachable. That covers the planned-upgrade
case the dissent was most concerned about without adding a bridge-side loop that races daemon
startup or a new process-spawn surface in the harness. Backend-resident intent remains rejected for
this issue and is recorded in the ADR as the reopen condition if cross-host discovery is ever
required.

Council artifacts are stored outside the repository at:

- `C:\Users\robemanuele\.copilot\session-state\bb45616b-fea7-4c68-8862-cd54c002f89e\files\council-issue-106-architecture\brief.md`
- `C:\Users\robemanuele\.copilot\session-state\bb45616b-fea7-4c68-8862-cd54c002f89e\files\council-issue-106-architecture\synthesis.md`

## Codebase Fit

- `MemberRecord` and push-attempt state remain in `src/daemon.rs`; reconciliation gets its own module
  (`src/daemon_reconcile.rs`) and the model gets `src/station_intent.rs`, so neither lands in the
  largest file in the repo.
- The reconcile tick runs inside the existing `heartbeat_loop` (`src/daemon.rs:3253`,
  `HEARTBEAT_INTERVAL` = 5 s at `:49`), reusing its drain gating and task lifecycle instead of
  adding a second loop with a duplicated cadence. `HEARTBEAT_INTERVAL` stays a `const` - it carries
  a test-enforced invariant against `ON_DELIVER_DEFERRED_BACKSTOP` (`:2370-2375`, asserted at
  `:7039`) - so the test seam is an explicit trigger plus a per-pass report subscription, not an
  environment override that could only ever lengthen the interval.
- Existing `Register { recovery, on_deliver, replace_on_deliver, on_deliver_wake_on_cc }` semantics
  are extended, and a separate reconcile path owns its own ordering rather than inheriting the
  `recovery` flag. Note precisely what the existing path does: `register_member` already checks the
  detach tombstone before `claim_epoch_lease` (`src/daemon.rs:3962` before `:3994`) and re-checks
  after (`:4022`, releasing at `:4026`), but both checks are gated on `recovery`, and the
  `recovery = false` branch *clears* the tombstone at `:4046`. That clearing branch - not a missing
  pre-claim check - is the hazard a reconciler must be structurally unable to reach.
- `platform::ensure_owner_private_dir` / `write_owner_only_file` (`src/daemon.rs:10528`, `:10569`,
  `:10896`, `:10914`) are the real cross-platform fail-closed primitives; they move to a shared
  module. The existing Windows validators are directory-scoped only
  (`validate_owner_private_dir_shape` `:11088` with the reparse rejection at `:11112`, and
  `validate_owner_private_dir_security` `:11121`), so a new per-file sibling is required for the
  credential file, which lives outside the intent scope by construction. The Unix-only
  `ensure_private_dir` / `write_private_file` in `src/commands/copilot.rs` are not used for intents
  or for the credential read.
- `platform::verify_server_peer` (`src/daemon.rs:10597`, `:10956`) already verifies same-user,
  executable, pid, and start time on a client connection - exactly the missing endpoint-authenticity
  and responder-binding check.
- `capture_process_start_time` (`src/session_watch.rs:98`) is used for capture, but
  `process_alive_with_start_time` (`:107`) is not used for reconciliation because it fails open when
  a start time is unknown. The other identity inputs have no shared primitive today: exe-from-pid
  resolution exists only inside `daemon::platform` (`proc_pidpath` `:10703`,
  `QueryFullProcessImageNameW` `:11413`, and the Unix `server_executable` at `:10614`), and there is
  no host-id or boot-id helper at all - so all three are promoted alongside the filesystem
  primitives rather than reimplemented in the Copilot command layer.
- Existing bridge root, registry, provisioning, attach/resume/detach, and GC code in
  `src/commands/copilot.rs` remain the lifecycle boundary. `bridge_handler_argv`
  (`src/commands/copilot.rs:288-306`) cannot serve the daemon as written - it takes a client `Ctx`,
  takes no executable parameter, and derives `--backend`/`--db` from `ctx.cfg` (`:293-302`) - so it
  is split into a pure shared builder plus a thin client wrapper, and the daemon gets one named
  `store_key -> selector` mapping built on `resolve_postgres_profile_for_store_key`
  (`src/daemon.rs:2184-2196`). `canonical_current_exe()` (`:10340`) is `current_exe()` +
  `canonicalize`, i.e. the currently running installed versioned binary under `versions/<tag>/`
  (`src/install.rs:1`, `:189-240`), not a stable launcher shim; argv is re-derived by whichever
  daemon is running rather than persisted, which is what keeps it correct across upgrade and
  rollback.
- `CapFile.instance_id` (`src/daemon.rs:286`, read via `read_cap_file` `:1731`, already read by the
  client connect path at `:1815`, rewritten by `new_state` at `:2072-2090`) is the fence value both
  the attach path and the daemon-side builder put into helper argv through the shared builder.
- Existing detach tombstones, epoch leases, ownership-fenced consume, and durable backlog APIs remain
  the authoritative backend protections; no backend schema change is required.
- Existing `PushDeliveryHealth`, `MemberStatus`, station status, turn guard, and drain reporting are
  extended to project intent-only and incompatibility states, with the existing `*_since_ms` /
  `*_count` evidence idiom and the existing rotating-NDJSON log idiom
  (`src/commands/copilot.rs:29`, `:2748-2765`).
- Existing process and conformance harnesses cover restart, crash, SQLite, Postgres, and
  at-least-once semantics, but lack a producer endpoint double, a startup-path test daemon, a
  deterministic reconcile trigger/report seam, credential-file security fixtures, and JS test
  discovery by glob - all of which this work adds.

## Risk Assessment

Each risk below now names the control that bounds it.

- Persisted handler intent as an execution surface - controlled by a registered descriptor kind with
  no persisted executable path, no persisted backend/db parameters, a validated session id, and argv
  built by one pure shared builder from the currently running installed binary plus the daemon's own
  resolved store selector.
- Startup discovery racing ordinary client recovery - controlled by putting the anti-downgrade guard
  inside `register_member`, so correctness does not depend on scan timing, and by the shared
  per-`MemberKey` admission guard.
- Reconciliation racing an in-flight attach - controlled by the pending/finalize split and the same
  admission guard.
- The inline reconcile deadlocking the register path - controlled by splitting the reconcile API
  into an acquiring outer entry point and a guard-free inner one, with the register path permitted
  to call only the latter, plus a regression test that exercises the inline call under a held guard.
- Bridge-side active recovery racing daemon startup - eliminated by declining the wake signal; the
  bridge remains observation-only.
- File permissions and atomic replacement as a security boundary - controlled by shared fail-closed
  create and read primitives on both platforms. Windows credential files may carry the normal safe
  current-user/SYSTEM/Administrators DACL produced by the process token, but broad or unrelated
  principals and reparse points are rejected; `insecure` is an explicit, reported state.
- Reading a producer-managed secret from a path outside the hardened intent scope - controlled by
  requiring the path to resolve under a registered producer root that telex itself creates with
  owner-only permissions, by canonical containment with no symlink or reparse point on the chain, by
  a per-file owner/DACL check that works on Windows as well as Unix, and by a bounded credential age
  after which the intent is unverifiable and no secret is read at all.
- Unbounded or corrupt manifest sets blocking startup - controlled by an asynchronous non-blocking
  startup scan, pass budget, per-pass wall-clock deadline, concurrency cap, per-intent timeout,
  failure backoff with quarantine, count and size caps, defined over-cap behavior, TTL, and GC.
- Publishing a recovery bound the implementation cannot honour - controlled by deriving both bounds
  from named constants, by classifying a not-yet-stale incumbent lease as a fixed-cadence deferral
  rather than an exponential backoff, by qualifying the bounds to scopes within one pass budget and
  to intents not backed off or quarantined, and by publishing a separate queue-delay formula for
  larger scopes.
- Upgrade skew silently restoring a reduced mode - controlled by the client capability gate, the
  daemon-side guard, and the legacy-versus-incompatible distinction, with rollback warning on live
  intents.
- Fail-closed becoming denial of service for a victim session - controlled by warn-and-allow in the
  turn guard, by treating pre-feature producers as legacy rather than failed, and by TTL/GC.
- Legacy address-only bindings cannot preserve exact store/mode/CC semantics - controlled by never
  synthesizing intent from them; they remain a ref-count-only artifact with a scheduled removal.
- Duplicate turn injection across a crash boundary - controlled by the cap-file instance check in the
  push helper and by epoch-fenced consumption; the residual is documented at-least-once behavior.

## Resolved Questions

- **Challenge framing, timeout, secret binding, negotiation.** Decided: peer verification first via
  `verify_server_peer`, then a nonce challenge whose credential is resolved at reconcile time from a
  generic descriptor pointer; bounded by a probe timeout constant; negotiated by a bridge protocol
  minimum that distinguishes legacy from incompatible.
- **Synchronous startup scan versus a readiness-gated asynchronous reconciler.** Decided:
  asynchronous, non-blocking, budgeted. Readiness is not gated on it, because the anti-downgrade
  invariant is enforced in the daemon register path rather than by scan ordering.
- **Whether a bridge wake signal is required.** Decided: no. The successor and wake triggers are the
  startup scan, the heartbeat tick, the upgrade/rollback successor spawn, and the admin-proofed
  explicit reconcile request from the existing hook. The idle hard-crash case with no successor is
  out of scope by the issue's own non-goal and is surfaced as degraded state.
