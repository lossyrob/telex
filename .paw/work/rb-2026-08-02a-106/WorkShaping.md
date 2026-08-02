# Work Shaping: Station Intent Reconciliation

## Problem Statement

Live push stations lose daemon membership and their generic `on_deliver` handler when the shared daemon drains, crashes, restarts, or is replaced during upgrade. The Copilot bridge can remain healthy, but durable messages stop arriving as turns because the replacement daemon has no registration intent to reconstruct. Generic command recovery can make the state worse by recreating the station without push, silently downgrading it to an unattended pull station.

The work must make same-major daemon replacement self-healing for live local push producers without treating durable state as proof of attendance, resurrecting detached stations, weakening epoch fencing, or coupling the daemon to Copilot-specific files.

## Core Functionality

- Replace the address-only Copilot binding list with an owner-private, atomic, versioned local station-intent manifest.
- Represent each exact binding by store identity, session, address, delivery mode, CC settings, metadata, recovery generation, producer PID/start time, typed handler descriptor, endpoint challenge data, and compatibility bounds.
- Keep membership in daemon memory. Intent is desired local registration state, not attendance.
- Reconcile compatible live intents when a same-major successor daemon starts.
- Validate producer PID/start time and complete a secret-bound endpoint challenge before claiming membership.
- Honor explicit revocation and durable detach tombstones. Only explicit attach/resume may clear a tombstone and create new intent.
- Claim a new lease epoch through the existing backend CAS and fence the previous owner before enabling push.
- Rebuild the typed handler through the stable launcher, preserve exact wake-on-CC behavior, and sweep durable undelivered messages.
- Prevent generic `NeedsAttach` recovery from silently registering pull when a known live push intent exists.
- Surface intent/member/producer incompatibility and recovery state in diagnostics.
- Report recovery exposure during graceful drain and upgrades.

## Supporting Work

- Migrate legacy `.bindings.json` conservatively. Legacy address-only data is a cleanup/ref-count hint, not sufficient recovery authority.
- Add bridge protocol support for authenticated liveness challenges and daemon-instance observation.
- Add a bounded reconciliation monitor so idle bridges recover after a successor appears or after a hard crash followed by later respawn.
- Update daemon, architecture, Copilot bridge, operating, troubleshooting, and destructive-testing documentation.
- Extend SQLite, Postgres, conformance, process, Copilot plugin, and Node bridge tests.

## Edge Cases and Expected Handling

| Case | Expected handling |
|---|---|
| Explicit Copilot detach | Revoke/remove intent and retain the durable tombstone; never auto-restore. |
| Explicit resume after detach | Clear the tombstone, write a new intent generation, challenge the live bridge, register, and sweep backlog. |
| Dead bridge or stale registry | Reject reconciliation; leave backlog durable; report missing live producer. |
| Fresh file but failed endpoint challenge | Reject as stale/broken; do not claim the station. |
| PID reused with a different start time | Reject the intent. |
| Protocol/schema mismatch | Fail closed with actionable compatibility diagnostics; never downgrade to pull. |
| Same session with multiple addresses/stores | Restore every compatible binding independently with exact store/mode/CC settings. |
| Competing live sessions for one address | Use normal lease epoch CAS; do not force-steal a fresh owner; report the loser. |
| Message committed during restart gap | Deliver after successful reconciliation from the durable backlog. |
| Push accepted before drain but unacked | The same message ID may redeliver at least once; never mark it consumed during recovery. |
| Consumed or terminally dispositioned message | Never redeliver. |
| In-flight old handler during fencing | Old owner cannot mark consumption after epoch advancement. |
| Repeated reconciliation | Idempotent member refresh with no duplicate concurrent push ownership. |
| Pull waiter | Preserve the existing reconnect and `NeedsAttach` behavior when no push intent applies. |
| No successor daemon | Keep the bridge/intents alive and surface degraded state; recovery begins only after a successor exists. |

## Rough Architecture

1. Copilot attach/resume provisions the bridge and atomically writes `StationIntentV1` entries under the existing owner-private bridge root.
2. The bridge registry carries producer PID/start time, protocol support, endpoint identity, secret material, heartbeat, and the last observed daemon transport instance.
3. A successor daemon scans local manifests during startup and on a bounded recovery cadence.
4. For each candidate, the daemon validates schema and compatibility, checks revocation/tombstones, validates PID/start time, and performs a secret-bound challenge against the live endpoint.
5. The daemon claims the backend epoch lease through normal CAS, creates the in-memory `MemberRecord` from a validated typed handler descriptor, and runs the existing durable backlog sweep.
6. The bridge observes daemon capability/instance changes and can issue a bounded idempotent wake signal if startup discovery alone cannot satisfy hard-crash recovery timing. The daemon remains the sole state mutation and retry owner.
7. Generic send/reply/ack/wait recovery consults intent state before creating a fresh non-push member. A live compatible push intent reconciles; incompatible or unverifiable intent fails visibly.
8. Status projects intents and members together so missing-member, missing-producer, incompatible-protocol, pending-reconciliation, and failed-reconciliation states are visible even when no `MemberRecord` exists.

## Critical Analysis

### Chosen direction

Use host-local manifests as the authoritative intent surface and keep backend rows as the attendance/ownership fence. This makes the cross-host Postgres prohibition structural, avoids treating shared backend data as local-process proof, and limits backend schema changes. The manifest must be generic and typed so the daemon remains harness-neutral.

### Rejected primary directions

- Backend-primary intent requires a second local proof layer anyway and adds cross-host, migration, and SQLite/Postgres parity risk.
- Graceful state transfer cannot solve hard crashes and conflicts with SQLite's single-holder lifecycle.
- A stable supervisor is substantially larger than the required recovery mechanism.
- Copilot-specific repair preserves the current harness coupling problem and does not safely fix generic recovery.

### Council record

The bounded architecture council recommended local `StationIntentV1` manifests with daemon-owned reconciliation, typed stable-launcher handlers, authenticated endpoint challenge, and fail-closed generic recovery. Confidence was medium because challenge framing and startup readiness need implementation-level validation. Preserved dissent favored a one-way bridge wake signal and, secondarily, backend-resident intent if future cross-host operator discovery becomes a requirement.

Council artifacts are stored outside the repository at:

- `C:\Users\robemanuele\.copilot\session-state\bb45616b-fea7-4c68-8862-cd54c002f89e\files\council-issue-106-architecture\brief.md`
- `C:\Users\robemanuele\.copilot\session-state\bb45616b-fea7-4c68-8862-cd54c002f89e\files\council-issue-106-architecture\synthesis.md`

## Codebase Fit

- `MemberRecord` and push-attempt state remain in `src/daemon.rs`.
- Existing `Register { recovery, on_deliver, replace_on_deliver, on_deliver_wake_on_cc }` semantics are extended rather than replaced.
- Existing bridge root, registry, bindings, provisioning, attach/resume/detach, and GC code in `src/commands/copilot.rs` provide the local persistence and lifecycle boundary.
- Existing PID/start-time helpers in `src/session_watch.rs` supply process-reuse protection.
- Existing detach tombstones, epoch leases, ownership-fenced consume, and durable backlog APIs remain authoritative backend protections.
- Existing `PushDeliveryHealth`, `MemberStatus`, station status, turn guard, and drain reporting are extended to project intent-only and incompatibility states.
- Existing process and conformance harnesses cover restart, crash, SQLite, Postgres, and at-least-once semantics.

## Risk Assessment

- Persisted handler intent can become an arbitrary execution surface unless descriptors are closed, typed, versioned, and resolved through a trusted stable launcher.
- Startup discovery can race ordinary client recovery unless reconciliation readiness gates pull-only registration or generic recovery performs the same guarded reconciliation.
- A bridge-side active recovery loop can race daemon startup recovery; bridge behavior must be limited to observation/challenge and, only if necessary, an idempotent wake signal.
- File permissions and atomic replacement are security boundaries. Insecure or malformed manifests must be ignored with diagnostics.
- Large or corrupt manifest sets must not indefinitely block daemon startup; scanning needs bounded work and observable per-intent failures.
- Upgrade skew must never silently restore a reduced mode. Compatibility mismatch is a visible failure.
- Legacy address-only bindings cannot preserve exact store/mode/CC semantics and therefore cannot be trusted for automatic restoration.

## Open Questions

- Finalize the challenge request/response framing, timeout, secret binding, and protocol negotiation.
- Determine whether synchronous startup scanning meets the documented recovery bound or needs a readiness-gated asynchronous reconciler.
- Prototype whether bridge observation requires an idempotent wake signal for hard-crash recovery after a successor is spawned by an unrelated command.
