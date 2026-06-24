# Plan

## Approach Summary

Outcome anchor: implement `docs/design/daemon.md` for the SQLite axis so the daemon core and one-shot verbs satisfy the section 17 gating tests. This is a core replacement, not an extension: the resident holder (`attach.rs` as a long-lived process), address-keyed IPC, and old backend conformance tests are superseded.

The implementation will proceed as one large PR with phased internal validation. Each phase should land executable tests for its own slice before the next phase depends on it. The final PR may use `Closes #38` only if the full SQLite section 17 axis is green; otherwise the session keeps working in this PR and the handoff uses `Refs #38`.

## Work Items

- [x] P1: Implement the epoch-aware SQLite storage spine.
  - Add schema-version tracking, epoch lease columns, retained consumed delivery state, durable SQLite `BackendClock` high-water, and the store-level hard-fail barrier for incompatible non-epoch writers.
  - The schema hard-fail must be a store-level rename/constrain barrier (CHECK/NOT NULL or equivalent that old non-epoch write paths violate), not a shim-only or additive-only policy.
  - Replace destructive `release_lease` behavior with non-deleting `ReleaseOwnership` semantics.
  - Add the canonical-store SQLite advisory lock with a config-root-invariant, owner-private per-OS-user lock directory. The lock target must be keyed by physical store file identity, invariant across config roots and `TELEX_RUN_DIR` overrides, and fail closed for narrower or world-squattable namespaces.
  - Validate epoch monotonicity, non-deleting release, consumed-row retention/no-prune, durable clock monotonicity, alias-path lock behavior, owner-private lock-dir checks, and store-lock single-writer behavior.
- [x] P2: Implement the daemon singleton, auto-spawn, spawn-lock, daemon-scoped IPC, handshake, and trust boundary.
  - Replace address-keyed holder IPC with a daemon-scoped endpoint keyed by user SID, config root, and protocol major.
  - Implement connect-or-spawn, readiness via `HelloAck`, bounded backoff/crashloop guard, protocol/capability negotiation, admin capability file, privileged proof handling, and same-user endpoint checks.
  - Client-side server authentication must happen before sending `Hello` (before disclosing `store_key`): verify server PID + start time + owner SID/uid + canonical executable path/hash. Server-side peer checks are necessary but not a substitute.
  - Add the hidden `telex daemon` lifecycle entrypoint posture; normal user help should keep daemon management implicit.
  - Validate concurrent first-use, second instance refusal, protocol-major parallel cap isolation, hostile/pre-bound endpoint rejection before metadata disclosure where the platform supports it, and required-capability fail-closed behavior.
- [x] P3: Implement explicit-only in-memory membership and one-shot verb rewrites.
  - `attach` becomes one-shot `Register`; `detach` becomes one-shot `Detach`; `wait` becomes a daemon client issuing `Wait`; add explicit `Ack`; route `send`/`reply` through daemon `Send`/`Reply`.
  - Pin the `Ack { store_key, session_id, address, message_id }` frame shape. The daemon must validate that the session attends the acked address before marking only `(message_id, recipient = address)`.
  - Implement `NeedsAttach` for unknown session/address and re-attach behavior where identity is available. `wait` must include reconnect-on-EOF grace and re-register on `NeedsAttach`; deliberate `Detach` remains terminal and must not auto-resurrect.
  - Freeze one-shot non-zero exit codes, including idle timeout, daemon gone, daemon hung, and `PresenceEnded`.
  - Validate no implicit membership rebuild, no resurrection after `Detach`, multi-store `(store_key, session_id)` keying, from ambiguity, manual identity fail-closed paths, and wait reconnect/reattach after daemon restart.
- [x] P4: Implement the lease-epoch delivery fence and handoff floor.
  - Implement claim CAS, epoch-guarded heartbeat with rowcount, self-demotion on 0-row heartbeat or `NotOwner`, `mark_consumed_if_current_owner`, per-recipient fan-out delivery rows, and SQLite release + next-call respawn handoff floor.
  - `mark_consumed_if_current_owner` must lock/compare/mark atomically in `BEGIN IMMEDIATE`; `NotOwner` has precedence over `AlreadyConsumed` and `AckNoOp`.
  - Delivery rows for acked messages are retained in v1 as the durable consumed authority; no pruning path may re-admit consumed messages.
  - Validate at-least-once delivery, stdout-flush-is-transport-only, explicit agent ack, idempotent consumed outcomes (`Marked`, `AlreadyConsumed`, `AckNoOp`, `NotOwner`), multi-recipient fan-out without cross-consume, ownership-rotation race precedence, no stale owner drain, and no consumed-message resurrection across restart.
- [x] P5: Implement liveness, non-destructive reaping, idle TTL, operator reset, and Status.
  - Before implementing dismiss-sensitive `sessionEnd` behavior, run the dismiss/resume + `session_id` id-scheme spike and record whether idle-TTL remains a backstop or becomes the primary dismiss bound.
  - Add typed `--watch-pid` with loader-anchor v1 floor and start-time reuse guard, session-end hook handling, non-destructive waiter reaping, idle-TTL `PresenceEnded`, operator reset, and the frozen Status fields.
  - Status must include the idle-station counter/warn threshold, retention counters, audit/recent-error ring (`NeedsAttach`, `NotOwner`, idle-TTL reaps, backend disconnects, reset with prior occupant), members, epochs, stores, backoff, and diagnostic redaction.
  - Validate sessionEnd, loader death, pid reuse, idle TTL, reset audit, status inspectability, reuse tripwire, idle-station synthetic stress, retention warnings, and diagnostic redaction.
- [x] P6: Implement the minimal upgrade floor and legacy cutover.
  - Add hidden daemon lifecycle verbs including `daemon stop --drain`, release + next-call respawn for SQLite, versioned shim support where applicable, and deterministic legacy-holder / non-epoch-lease cutover behavior.
  - Validate drain, respawn, no epoch reset, old-holder/non-epoch row handling, blocked wait reconnect behavior across daemon restart, and the SQLite release + next-call floor for the ordered-handoff crash matrix.
- [x] P7: Replace the old conformance suite with the SQLite section 17 gating matrix and final budgets.
  - Replace the old backend-trait conformance tests with executable SQLite daemon-core tests mapped to section 17.
  - Keep CI useful during the large refactor by replacing or disabling superseded resident-holder/old-trait tests as soon as the corresponding surface is replaced, then adding phase-specific daemon tests before moving on.
  - Add a committed delivery-budget spec artifact with numeric p95/p99 fence-latency and dedup/retention thresholds so test 19 is falsifiable.
  - Add crash/negative/benchmark coverage that did not fit earlier phases, including hostile/pre-bound endpoint cases to the extent supported locally, crash matrices, protocol-major compatibility, and delivery-fence latency/dedup budget tests.
  - Validate `cargo test` and the full §17 SQLite matrix.
- [x] Documentation and operator surfaces.
  - Update `SKILL.md`, README/help text, design-linked operator docs/runbook, and any command documentation to match one-shot daemon verbs, `telex skill --raw`, the single-source SKILL invariant, reset/drain usage, and ack behavior.
- [x] PR lifecycle and field report.
  - PR/lifecycle TODOs are seeded for Review Response, PR Sentry, merge observation, and post-merge field report. The final PR stage proceeds with those operational lifecycle items tracked in session TODOs.

## Key Decisions

- Follow the orchestrator decision: do not split issue #38; this PAW session produces one large PR anchored to SQLite section 17 green.
- Use phased internal validation (P1-P7) to de-risk rather than big-bang implementation.
- Treat the old resident-holder model and old conformance trait tests as superseded, not compatibility surfaces.
- Keep Postgres parity out of scope except where shared trait or compile compatibility requires non-behavioral stubs.
- Preserve the node outcome anchor throughout planning review and implementation; prerequisite work is not sufficient unless it culminates in the SQLite gating suite.
- For non-SQLite/shared surfaces touched by the refactor, prefer typed unsupported errors or minimal compile-compatible stubs over `unimplemented!()` so default-feature builds remain stable while SQLite behavior is completed.

## Phase Ordering Rationale

- P1 establishes the durable invariants that the daemon must not violate: epoch high-water, no-delete release, retained consumed rows, durable clock, and single-writer store authority.
- P2 establishes the single process that owns presence and delivery, plus the endpoint and trust boundary that all verbs depend on.
- P3 moves user-facing verbs onto daemon IPC and makes explicit-only membership observable before the delivery fence is tightened.
- P4 closes the at-least-once delivery and stale-owner correctness loop once membership and daemon ownership exist.
- P5 layers liveness and Status on a functioning transport so reaping remains non-destructive and inspectable.
- P6 adds the minimum upgrade/cutover floor after stop/drain and reattach behavior exist.
- P7 completes the crash, negative, compatibility, and benchmark matrix after every primitive has an implementation surface.

## SQLite Section 17 Mapping

| §17 Test | Primary phase(s) | Validation hook |
|---|---|---|
| 1 Concurrent first-use | P2 | Multi-process connect-or-spawn test: exactly one daemon, losers connect |
| 2 Singleton + store lock | P1, P2, P7 | Endpoint exclusivity + canonical-store lock alias/config-root/run-dir cases |
| 3 Crash during wait -> NeedsAttach | P3, P6 | Restart daemon while wait is blocked, re-register, no spurious exit 3 |
| 4 Restart no loss/no resurrection | P1, P3, P4 | Durable buffer retained; consumed rows retained; no membership rebuild |
| 5 Explicit ack/fan-out/dedup | P3, P4 | Ack frame shape, per-recipient marking, `AckNoOp`, replay/late ack cases |
| 6 Multi-writer Postgres epoch | P4 | SQLite asserts single-writer holds; Postgres axis remains downstream |
| 7 Ownership-rotation race | P4, P7 | Atomic mark transaction, `NotOwner` precedence over consumed/no-op outcomes |
| 8 SessionEnd reaping | P5 | Hook releases waiters and marks idle only |
| 9 Loader-pid death | P5 | PID + start-time guard; loader-alive is not positive presence |
| 10 Idle-TTL | P5 | `PresenceEnded` exit 5; station/buffer persist; new message wakes |
| 11 Operator reset | P5 | Reset audit + non-destructive waiter release |
| 12 Epoch monotonicity | P1, P4 | Release/cleanup/reclaim yields E+1; no epoch-bearing lease delete |
| 13 Ordered handoff crash matrix | P2, P3, P4, P6, P7 | SQLite release + next-call respawn floor; wait reconnect grace |
| 14 OS trust negatives | P2, P7 | owner-only endpoint/cap/lock, client-auth before Hello, redaction |
| 15 IPC compatibility | P2, P3, P7 | required capabilities, unknown op/field fail-closed, Ack/NeedsAttach surface |
| 16 Protocol-major parallel | P2, P7 | separate singleton endpoint/cap paths per protocol major |
| 17 Durable BackendClock | P1, P5 | high-water clock across restart/skew; TTL fail-closed path |
| 18 Cross-store/from ambiguity | P3, P5 | `(store_key, session_id)` keying; ambiguous/unknown from resolution |
| 19 Latency + retention budget | P4, P7 | committed budget spec + benchmark and retained-row assertions |

## Field Report Sections To Capture

- Outcome and whether the PR used `Closes #38`.
- Dismiss/resume spike result and resulting idle-TTL semantics.
- Path-resolution policy and owner-only effective-permission behavior.
- Canonical-store lock-dir resolution and any platform caveats.
- Postgres compile-compatibility strategy and downstream parity implications.
- Delivery-budget thresholds and observed benchmark results.
- Deferred or out-of-node work (Postgres parity, plugin mapping, full seamless-upgrade, validation harness hardening).
- Scope-drift attestation against daemon.md and the workstream boundary.

## Open Questions

None blocking planning. Implementation checkpoints to resolve and record in code/docs/field report: dismiss/resume spike result, owner-only path-resolution policy, explicit single-tenant opt-out posture if any, budget thresholds, and platform coverage caveats for OS-trust negative tests.
