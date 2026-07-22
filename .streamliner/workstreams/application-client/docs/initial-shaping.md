# Application Client workstream formation context

## Why this became a workstream

The Addressable Attention campaign deliberately let Operator Station and Telex
Watcher prove viability and define their production domains before freezing a
shared client. Both contracts are now merged. Their exports show that the shared
surface is much larger than an embeddable send helper: it spans application
identity, process liveness, station capability, multi-address lifecycle,
receipt-state separation, exact delivery acknowledgment, retry-safe compound
operations, source resolution, backend provenance, health, and ordered resync.

That is workstream-sized. Issue #12 remains the contract authority, while the
new workstream provides execution geometry, review, implementation, and
conformance around it.

## Requirement families to converge

1. Stable responsibility versus never-reused runtime/session identity.
2. Process-bound liveness and typed membership loss.
3. Send-only versus bidirectional capability.
4. Multi-address attach/reconcile/detach, collision, and compensation.
5. Caller-selected strict versus bounded automatic recovery.
6. Explicit sender selection and retry-bounded reconcile-and-send.
7. Durable acceptance, occupancy, push, transport acknowledgment, and workflow
   disposition as separate facts.
8. Receive records keyed by logical store, message, and recipient/delivery row.
9. Ack-after-durable-ingest, at-least-once recovery, unresolved queries, bounded
   history, and ordered resync.
10. Metadata-bearing reply, per-recipient disposition, retry-safe operation
    identity, and post-restart reconciliation.
11. Compound human-response and route-back recovery without making
    Station-specific policy part of the client.
12. Source-reference resolution and authenticated principal provenance.
13. Lifecycle/health projection, backlog visibility, and delta events.
14. SQLite/Postgres backend/profile parity with opaque logical-store identity.
15. Receipt cross-checks, throttling, local scope cleanup, and explicit
    diagnostics without parsing CLI stderr or exposing raw IPC.

## Boundary tensions the first node must resolve

- Watcher requires send-only membership that cannot advertise inbound
  attendance; Operator requires a full bidirectional receive surface.
- Watcher requires PID/start-time liveness and caller-selected repair; desktop
  Station also needs stable responsibility across process replacement.
- Operator requires exact recipient delivery identity and compound operations;
  Watcher requires receipt-gated send/state evidence and explicit sender
  capability.
- The old issue #12 body contains a useful TypeScript/napi-rs API sketch, but it
  predates daemon and domain-contract reality and is not accepted authority.
- `application-client-ready` must be useful without implying implementation is
  complete: it accepts semantics and unblocks coordinated promotion, while later
  implementation/conformance checkpoints govern actual integration.

## Formation decisions

- Use issue #12 as the durable semantic owner.
- Form parent workstream #117 and first contract node #118.
- Require final contract review from both consumer workstream orchestrators,
  plus Application Client workstream and campaign approval.
- Make the first node design/tracker-only and API-neutral.
- Sketch later core, binding, conformance, integration, and hardening nodes; do
  not pre-plan their code structure.
- Preserve the campaign's isolated-plane rule for destructive daemon and
  branch-binary validation.
