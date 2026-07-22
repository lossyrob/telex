## Operator Station production contract: Application Client requirements

Source: [Operator Station contract #114](https://github.com/lossyrob/telex/issues/114)
and [`docs/design/operator-station.md`](https://github.com/lossyrob/telex/blob/2d99e552292a4401d3403540b6d2eaa90272282d/docs/design/operator-station.md)
at `2d99e552292a4401d3403540b6d2eaa90272282d`.
The workstream-approved domain bundle is
`6e0188139ed6dae5ef19289cec02aac8a4058a540dd7ccb4a8d9421482253125`.

This comment exports Operator Station requirements to the campaign-owned
Application Client seam. It does not select an API, package, language binding,
daemon wire format, or implementation. It does not accept the eventual shared
contract; campaign convergence through this issue does that after reconciling
Operator Station and Watcher requirements.

### Material delta from the prior Operator Station export

Paired review of PR #116 strengthened six shared semantics:

- AC-04 and AC-06 now preserve per-recipient/delivery-row identity and bind
  acknowledgment to the exact recipient delivery.
- AC-08 and AC-09 now require metadata-bearing replies plus restart-safe
  operation-result/receipt reconciliation.
- AC-10 now requires a machine-readable raw-thread outcome before every
  non-stale terminal assisted closure.
- AC-14 now requires a snapshot fence or monotonic per-axis ordering so resync
  cannot regress workflow state.

The contract also strengthens assisted-to-direct handoff, inert rendering,
unknown OS-focus fallback, and minimum escalation provenance. Those remain
Station/operator behavior and do not add shared-client requirements.

### Shared semantic requirements from Operator Station

1. **Stable application station identity and lifecycle.** A long-lived
   application responsibility has explicit attach, detach, reattach/recovery,
   and typed membership-loss outcomes. Stable responsibility survives process
   replacement; runtime/session identity remains ephemeral.
2. **Opaque logical-store identity.** Status, receipts, receive records, and
   source references carry a stable equality-comparable logical-store identity
   without exposing paths, credentials, connection strings, or tokens.
3. **Multi-address lifecycle.** Attach/reconcile/detach across configured
   addresses is atomic or returns explicit partial results, compensation, and
   collision evidence.
4. **Application receive.** A streaming, callback, or async receive semantic
   yields the complete message, recipient/delivery-row identity, delivery-role
   context, opaque metadata, and an acknowledgment capability bound to that
   exact recipient delivery without a subprocess courier or required follow-up
   read.
5. **Ack after durable ingest.** A receiver can acknowledge only after writing
   restart-replayable application state. Ack-pending, pending-unconsumed,
   inbound-actionable, deaf/backlog, and failure state are observable.
6. **At-least-once recovery.** Duplicate/redelivery identity is defined per
   recipient, with restart-safe snapshot-fence or monotonic per-axis
   cursor/resync semantics.
7. **Unresolved and bounded history queries.** Applications can query every
   unresolved primary obligation plus bounded recent and thread history without
   full-store materialization or pre-filter limit ambiguity.
8. **Typed message operations.** Explicit-sender send, metadata-bearing reply,
   read-thread, and per-recipient disposition operations return additive,
   identity-checkable results.
9. **Retry-safe application operations.** Application-authored escalation,
   reply, disposition, operator notification, and route-back have stable
   operation identity/idempotency plus an explicit accepted-send/local-commit
   duplicate window and post-restart operation-result/receipt reconciliation.
10. **Compound human-response semantics.** Reply & Handle, assisted
    disposition-only operator notification, and route-back preserve durable
    ordering, expose partial/indeterminate outcomes, and provide recovery
    handles. The selected human obligation is never terminally dispositioned
    before any required reply or assisted-mode operator notification is durably
    accepted, and every non-stale terminal assisted outcome has a
    machine-readable raw-thread route-back record.
11. **Source-reference resolution.** `(logical store identity, message ID)`
    resolves to authoritative, captured-only, mismatch, or unavailable source
    state without opening a same-number message from another store.
12. **Lifecycle and health projection.** Applications can inspect registration,
    runtime/session identity, lease epoch/owner, receive readiness, membership
    loss, pending unconsumed, inbound actionable, ack pending, reconciliation,
    and detach/recovery outcomes.
13. **Backend-profile selection and principal provenance.** Applications
    explicitly select a configured backend/profile with the same semantic
    contract for current SQLite and credentialed Postgres. Authenticated
    principal evidence includes its provenance when available.
14. **Delta-oriented events and resync.** Application events describe message,
    delivery, disposition, health, and recovery deltas without serializing the
    complete feed on every mutation. A snapshot fence or monotonic per-axis
    ordering plus explicit resync/backfill prevents workflow-state regression.
15. **Bounded operational recovery.** Receipt identity cross-checks, bounded
    retry/throttling, and local scope discovery/cleanup are supported without
    parsing CLI stderr or exposing raw daemon IPC.

### Alignment with Watcher #110

The Operator Station requirements agree with Watcher on stable responsibility
versus ephemeral runtime, typed membership loss and recovery, multi-address
lifecycle, explicit sender selection, receipt-state separation, lifecycle
health, deduplication, backend selection, and opaque logical-store identity.

Operator Station adds receiver- and human-loop pressure on the shared seam:
ack-after-ingest, unresolved-obligation/history queries, reply and
per-recipient disposition, application operation identity, compound
reply/disposition and route-back recovery, source resolution, and delta events.
Watcher adds send-only station capability, process-bound liveness predicates,
caller-selected strict-versus-bounded-automatic repair, and receipt-gated
reconcile-and-send requirements. The shared contract should support both
without a desktop-specific or Watcher-specific client fork.

### Station-specific behavior not assigned to the shared client

The shared client does not own direct/assisted/quiet policy, operator-agent
judgment, the `urn:telex:operator-station:v1` extension, notification defaults,
feed layout, source-card presentation, local read state, or safe-link UX.
Those remain Operator Station/operator application behavior.

### Spike mechanisms explicitly not promoted

- CLI subprocesses for application operations;
- repeated one-shot waiter supervision;
- full-history export for unresolved recovery;
- store-path fingerprints;
- local app-data UUID/high-water files as shared identity/cursor facilities;
- the `operator-station-spike.*` namespace;
- campaign-local `attention.*` kinds and `campaignAttention` metadata;
- development Tauri launch, current UI layout, and HKCU AUMID behavior;
- SQLite-only or Windows-only shared-client semantics.

Production `station-app` and `operator-broker` remain blocked on the accepted
Operator Station contract and the campaign's `application-client-ready`
checkpoint. This requirements comment is one domain input; campaign/#12
convergence accepts the eventual shared Application Client contract.
