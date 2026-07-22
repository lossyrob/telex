## Telex Watcher production Application Client requirements

This comment is the Telex Watcher domain requirement export from
[issue #110](https://github.com/lossyrob/telex/issues/110).

Source contract:

- branch: `feature/watcher-contract`
- reviewed source head: `9df7d25c41b2eca827361db11a7a01c416721d36`
- design: `docs/design/watcher.md`
- immutable design permalink:
  https://github.com/lossyrob/telex/blob/9df7d25c41b2eca827361db11a7a01c416721d36/docs/design/watcher.md
- decision: ADR 0046
- canonical schemas:
  - `docs/design/schemas/watcher-detector-request-v1.schema.json`
  - `docs/design/schemas/watcher-detector-result-v1.schema.json`
  - `docs/design/schemas/watcher-event-metadata-v1.schema.json`
  - `docs/design/schemas/watcher-health-v1.schema.json`

This export states shared semantic requirements. It does not select an API
shape, package, language binding, daemon wire format, or implementation.
Campaign convergence through issue #12 remains the sole owner of the accepted
Application Client contract and the `application-client-ready` checkpoint.

Please record an accepted, deferred, or rejected disposition for each numbered
requirement during issue #12 convergence. Any requirement that is not accepted
keeps the affected Watcher runtime or template promotion blocked.

### Shared semantic requirements

1. **Stable responsibility and ephemeral runtime identity**

   A long-lived application must attach stable sender responsibilities/addresses
   using a fresh, never-reused process/session identity for each runtime
   incarnation. Stable sender addresses survive process replacement; runtime
   identity does not.

2. **Process-bound liveness**

   Application membership must support typed PID plus process-start-time
   predicates so abrupt process death becomes observable without PID-reuse false
   positives.

3. **Atomic or compensable multi-address lifecycle**

   Attach, reconcile, and detach across multiple addresses must either complete
   atomically or return explicit per-address partial results and compensation
   handles. A partial attach must not be reported as application-ready.

4. **Caller-selected membership recovery**

   The caller must be able to choose strict typed `NeedsAttach` failure or a
   bounded automatic repair policy. Repair must preserve the original liveness
   predicates and must not silently convert strict application membership into
   generic auto-registration.

5. **Typed membership-loss outcomes**

   Daemon restart, predicate death, collision, deliberate detach, unknown
   membership, and owner demotion must remain distinguishable. Applications need
   stable typed reasons for status, recovery, and audit.

6. **Bounded reconcile-and-send**

   One semantic operation must be able to reconcile typed membership loss and
   retry under a caller-selected bounded retry budget without parsing CLI stderr
   or exposing raw daemon IPC. Watcher selects one retry in its v1 policy.

7. **Explicit sender selection**

   Multi-address applications must select the sender explicitly. They cannot
   depend on ambiguous default-from inference.

8. **Observable collision and takeover state**

   Address collision must expose the current owner/epoch and bounded
   retry/reset guidance. The client must not hide force takeover or silently
   replace another live application.

9. **Typed receipt separation**

   The client must distinguish:

   - durable message acceptance;
   - target occupancy at acceptance time;
   - push attempt/acceptance;
   - recipient transport consumption/acknowledgment; and
   - workflow disposition.

   Watcher commits detector event state on durable acceptance only. Occupancy,
   push, consumption, and disposition are separate later facts.

10. **Explicit station capability**

    Send-only and bidirectional application membership must be distinct.
    Send-only membership must not advertise inbound application attendance. A
    send addressed to a send-only application responsibility must receive the
    address policy's unoccupied or rejected result, not a false
    application-delivered result.

11. **Inbound application semantics**

    A bidirectional receiver needs restart-safe cursor/receive, acknowledgment,
    disposition, and reply semantics. Ack must be a deliberate application
    action rather than an inference from occupancy or transport.

12. **Lifecycle and health projection**

    Applications need machine-readable status for runtime identity, PID
    predicates, lease epoch/owner, per-address readiness, typed membership loss,
    reconciliation, partial compensation, detach outcomes, pending unconsumed
    work, and inbound actionable backlog.

13. **Daemon restart recovery**

    Applications need ergonomic explicit reattachment or a typed restart-loss
    signal without a resident per-application waiter. Deliberately detached
    membership must not be resurrected.

14. **Deduplication guidance**

    The client contract must document message/event identity and the
    accepted-send/local-commit duplicate window. At-least-once duplicates are
    preferable to consume-before-send loss.

15. **Backend selection and opaque logical-store provenance**

    Applications must explicitly select a configured backend/profile and receive
    a stable, equality-comparable opaque logical-store identity on status,
    receipt, and receive records. The identity must not expose raw paths,
    credentials, or connection strings.

### Watcher-specific semantics that remain outside the shared client

The following are Watcher domain behavior, not Application Client requirements:

- trusted local detector execution;
- detector request/result schema and outcome meanings;
- Watcher registration, scheduling, timeout, concurrency, jitter, and backoff;
- script pin/follow policy and digest validation;
- detector state, event-ID collision, and receipt-gated local transaction;
- allowed event-kind policy;
- detector environment allowlisting and diagnostic redaction;
- process-tree containment and registry ownership;
- Watcher health schema and retention policy; and
- GitHub, Azure DevOps, HTTP, file, or other provider templates.

### Spike-private mechanisms that are not accepted client contracts

Do not promote:

- CLI subprocess parsing for attach/status/detach/send;
- `TELEX_WATCHER_INTERNAL_SEND_ONCE_V1`;
- direct use of raw daemon IPC or current internal Rust library seams;
- sender occupancy as proof of application consumption; or
- provider-specific behavior in the shared client.

Production Watcher runtime work remains blocked until issue #12/campaign
convergence accepts the required shared semantics and publishes the
`application-client-ready` checkpoint.
