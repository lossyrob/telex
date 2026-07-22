## Operator Station domain contract merged

The production Operator Station domain contract from
[#114](https://github.com/lossyrob/telex/issues/114) merged in
[PR #116](https://github.com/lossyrob/telex/pull/116).

Authoritative merged source:

- merge commit:
  `0722051760bab569d3f947fd7b29f2dabe13ef77`
- reviewed head:
  `2d99e552292a4401d3403540b6d2eaa90272282d`
- normative design:
  https://github.com/lossyrob/telex/blob/0722051760bab569d3f947fd7b29f2dabe13ef77/docs/design/operator-station.md
- decisions: ADR 0047 and ADR 0048
- corrected requirements export:
  https://github.com/lossyrob/telex/issues/12#issuecomment-5042612298
- corrected export SHA-256:
  `adf2f8e439e5c224059ca51142701f604a82203c39c9829d1323a88c58889f7e`

This addendum supersedes the branch-head provenance in the earlier Operator
Station export. It does not change the accepted Operator requirements.
Campaign convergence through issue #12 remains the sole authority for the
shared Application Client contract and `application-client-ready`.

### Material delta incorporated before merge

Paired review strengthened the final contract and export:

- per-recipient/delivery-row identity and exact-recipient acknowledgement;
- metadata-bearing replies and post-restart operation/receipt reconciliation;
- machine-readable raw-thread route-back before every non-stale terminal
  assisted outcome;
- snapshot-fence or monotonic per-axis ordering so resync cannot regress state;
- assisted-to-direct unresolved-work handoff;
- inert rendering of untrusted human-visible fields;
- deterministic unknown OS-focus fallback; and
- mandatory source provenance for raw-derived escalations.

### Final Operator Station Application Client requirements

| ID | Shared semantic requirement |
|---|---|
| AC-01 | Stable application station identity with explicit attach, detach, reattach/recovery, and typed membership-loss outcomes |
| AC-02 | Opaque stable logical-store identity with no path, credential, or connection-string exposure |
| AC-03 | Multi-address lifecycle with explicit partial results and compensation |
| AC-04 | Streaming/callback/async receive yielding message, recipient/delivery-row identity, delivery-role context, opaque metadata, and an ack capability bound to that exact recipient delivery |
| AC-05 | Ack-after-durable-ingest and observable ack-pending, deaf, and backlog state |
| AC-06 | At-least-once duplicate/redelivery identity per recipient plus restart-safe snapshot-fence or monotonic per-axis cursor/resync semantics |
| AC-07 | Unresolved-obligation query plus bounded recent/thread history without full-store materialization |
| AC-08 | Typed send, metadata-bearing reply, read-thread, and per-recipient disposition operations with explicit sender selection and identity-checkable results |
| AC-09 | Retry-safe application operation identity/idempotency with an explicit accepted-send duplicate window and post-restart operation-result/receipt reconciliation |
| AC-10 | Reply/disposition, disposition-only operator notification, and route-back compound semantics with durable ordering, partial outcomes, recovery handles, and a machine-readable raw-thread outcome before every non-stale terminal closure |
| AC-11 | Source resolution using logical-store identity plus message ID, with authoritative/captured/unavailable states |
| AC-12 | Lifecycle/health projection covering registration, epoch/owner, receive health, pending unconsumed, inbound actionable, ack pending, and detach/recovery outcomes |
| AC-13 | Backend-profile selection without backend-specific message semantics, covering current SQLite and credentialed Postgres, with authenticated principal provenance when available |
| AC-14 | Delta-oriented application events with a snapshot fence or monotonic per-axis ordering plus explicit resync/backfill behavior that cannot regress workflow state |
| AC-15 | Receipt identity cross-checks, bounded retry/throttling, and local scope discovery/cleanup |

Production `station-app` and `operator-broker` remain blocked on issue #12
convergence and the campaign's `application-client-ready` checkpoint. No
spike-private CLI, waiter, export, path-fingerprint, campaign metadata, or UI
seam is an allowed production fallback.
