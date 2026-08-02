# Station Intent Reconciliation Plan

## Approach Summary

Add a host-local, owner-private, versioned station-intent layer under the daemon config root. Intents describe exact desired push registration but never count as attendance. Copilot attach/resume writes `StationIntentV1`; detach revokes it. The daemon validates compatible intents with PID/start-time and authenticated endpoint challenge, claims the normal backend epoch, reconstructs the in-memory push member through a typed stable-launcher handler, and re-sweeps durable backlog.

Reconciliation runs at daemon startup and on a five-second bounded maintenance cadence, yielding a documented recovery bound of ten seconds after both successor daemon and live producer exist. Generic `NeedsAttach` recovery consults intent state and cannot silently create pull membership over a live or incompatible push intent. Status and drain reporting project intent/member/producer state even when no member exists.

The manifest is local rather than backend-resident. SQLite and Postgres continue to own leases, tombstones, and durable messages; therefore cross-host Postgres cannot restore another host's bridge by construction.

## Work Items

- [ ] **Define station-intent model and secure local persistence** (`intent-model`)
  - Add a generic `station_intent` module with `StationIntentV1`, versioned typed handler and producer descriptors, recovery state, validation, atomic owner-private writes, enumeration, revocation, legacy binding migration helpers, and deterministic per-binding identity.
  - Store manifests beneath the singleton config root so all same-major daemon instances discover the same local intent set.
  - Treat legacy address-only `.bindings.json` as cleanup metadata only; never auto-recover from incomplete legacy data.

- [ ] **Upgrade Copilot bridge protocol and lifecycle integration** (`copilot-intent`)
  - Extend the bridge registry with protocol version, PID start time, instance generation, and observed daemon identity diagnostics.
  - Add a secret-authenticated `probe` request that echoes a caller nonce and reports the live session, bridge generation, PID, and protocol.
  - Make Copilot attach/resume write exact per-store intent before registration and finalize it only after daemon push verification.
  - Make detach, failed provisioning rollback, fallback downgrade, GC, and last-binding teardown revoke/remove the exact intent without disturbing other stores or addresses.
  - Preserve the compatibility bindings file for extension ref-count only.

- [ ] **Implement daemon reconciliation and typed handler restoration** (`daemon-reconcile`)
  - Add startup plus five-second maintenance reconciliation owned by the daemon.
  - Validate schema/protocol range, local file security, producer PID/start time, endpoint nonce challenge, detach tombstone, and exact store/address/session metadata.
  - Resolve only trusted typed handlers through the current stable Telex launcher; never execute persisted arbitrary argv.
  - Claim a new epoch through existing backend CAS, register exact push/CC metadata with recovery semantics, and run the existing backlog sweep.
  - Make repeated reconciliation idempotent, preserve existing push retry state on no-op refreshes, and reject competing fresh owners without force takeover.

- [ ] **Prevent generic push-to-pull downgrade** (`generic-recovery`)
  - Extend register/retry behavior so send/reply/ack/wait recovery checks station intent before creating a missing member.
  - Reconcile a live compatible push intent or fail with a typed actionable incompatibility/stale-producer error.
  - Preserve current pull reconnect behavior when no push intent exists and continue honoring deliberate-detach tombstones.

- [ ] **Add diagnostics and drain/upgrade signaling** (`diagnostics`)
  - Bump the protocol minor and add typed intent status projections for `live_intent_missing_member`, `member_missing_live_producer`, `intent_protocol_incompatible`, pending, restored, revoked, challenge-failed, tombstoned, and ownership-conflict states.
  - Include intent-only rows in daemon/station status and teach Copilot turn guard to block or warn on a live binding without push membership rather than returning `no_attended_stations`.
  - Return graceful drain counts for recoverable, degraded, and incompatible intents and surface them through `daemon stop --drain` and upgrade/rollback output.
  - Keep diagnostics free of secrets and raw executable arguments.

- [ ] **Cover lifecycle, storage, bridge, and delivery semantics with tests** (`tests`)
  - Add station-intent unit tests for versioning, atomic writes, file permissions, malformed data, legacy migration, revocation, and PID reuse.
  - Add bridge Node tests for authenticated probe, nonce echo, wrong secret, generation, and protocol response.
  - Extend daemon core SQLite/Postgres tests for exact multi-address/multi-store restoration, tombstones, ownership conflicts, idempotence, protocol incompatibility, and at-least-once backlog behavior.
  - Extend real process SQLite tests for graceful drain, hard crash, idle live bridge, restart-gap messages, unacked redelivery, consumed-message suppression, bridge death, challenge failure, and bounded recovery.
  - Preserve existing pull-wait reconnect tests and add generic downgrade regression coverage.

- [ ] **Document the revised lifecycle and operator contract** (`documentation`)
  - Create `.paw/work/rb-2026-08-02a-106/Docs.md` as the as-built technical reference.
  - Add a design decision that layers durable intent beside explicit in-memory membership and records compatibility/security invariants.
  - Update daemon/architecture/Copilot bridge design docs plus operating and troubleshooting guides.
  - Document the recovery bound, fail-closed states, legacy behavior, and isolated config-root guidance for destructive daemon testing.

- [ ] **Run full verification and prepare reviewable commits** (`verification`)
  - Run formatting, targeted Rust and Node tests during implementation, then workspace tests, clippy, feature builds, and available Postgres conformance.
  - Confirm the exact issue acceptance matrix, artifact consistency, and selective staging before resolving implementation.

## Key Decisions

1. **Intent location:** owner-private local files under the daemon singleton config root, not shared backend rows.
2. **Authority split:** intent is desired state; in-memory membership plus backend epoch lease remains attendance and delivery authority.
3. **Recovery owner:** the daemon exclusively mutates membership and owns retry/reconciliation. The bridge supplies liveness challenge and daemon-identity observation.
4. **Recovery bound:** startup reconciliation plus a five-second maintenance cadence; recovery is documented as completing within ten seconds after a compatible successor and live producer coexist, excluding backend outage or a competing fresh owner.
5. **Handler safety:** persist a closed typed `telex_copilot_push_v1` descriptor and resolve it through the current stable launcher. Do not persist arbitrary argv as recovery authority.
6. **Liveness proof:** require both PID/start-time match and a secret-authenticated nonce challenge; heartbeat freshness alone is insufficient.
7. **Tombstones:** explicit attach/resume is the only operation allowed to clear a detach tombstone and publish a new intent generation.
8. **Generic recovery:** known push intent can only reconcile or fail visibly; it cannot degrade to pull.
9. **Legacy data:** address-only bindings remain ref-count/cleanup compatibility data and are not auto-restored.
10. **Protocol compatibility:** unsupported manifest or bridge protocol fails closed and appears in status; no reduced-mode fallback.
11. **Cross-host Postgres:** local intents are never discovered remotely; the backend continues to arbitrate epochs and consumption only.
12. **Graceful handoff:** no daemon-to-daemon state transfer in this issue. Durable intent handles both graceful and crash recovery; pre-drain reporting is the coordination signal.

## Open Questions

None.
