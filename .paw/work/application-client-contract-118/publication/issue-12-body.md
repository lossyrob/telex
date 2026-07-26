## Status

`application-client-ready` is published as a **design-only semantic checkpoint**.
The shared Application Client contract is accepted for downstream implementation
planning. This checkpoint does not claim that a client core, language binding,
conformance suite, Watcher integration, Operator Station integration, or production
deployment exists.

## Normative contract

- [Application Client contract](https://github.com/lossyrob/telex/blob/8854e5e36ac5c18320b208d1a90bffaffbff33a5/docs/design/application-client.md)
- [Watcher and Operator requirement crosswalk](https://github.com/lossyrob/telex/blob/8854e5e36ac5c18320b208d1a90bffaffbff33a5/docs/design/application-client-crosswalk.md)
- [Canonical bundle manifest](https://github.com/lossyrob/telex/blob/8854e5e36ac5c18320b208d1a90bffaffbff33a5/docs/design/application-client.bundle.json)
- [ADR 0049](https://github.com/lossyrob/telex/blob/8854e5e36ac5c18320b208d1a90bffaffbff33a5/docs/design/DECISIONS.md#0049--one-api-neutral-application-client-contract-governs-explicit-station-capabilities-and-forbids-private-fallbacks)
- [Exact pre-convergence issue body](https://github.com/lossyrob/telex/blob/8854e5e36ac5c18320b208d1a90bffaffbff33a5/docs/design/history/application-client-issue-12-original.md)

Publication revision: `1`.

- bundle source head:
  `8854e5e36ac5c18320b208d1a90bffaffbff33a5`;
- manifest Git blob:
  `04c294f0790a5ba764354def07d1108575918d2e`;
- manifest SHA-256:
  `abe8abb8f5be5544651d18e1c535b16a94ff99690b835b7c08bb3541292b0e87`.

The manifest is the machine-readable source of file membership and byte identities.

## Merged-source provenance

### Telex Watcher

- requirement export comment
  [`5042702401`](https://github.com/lossyrob/telex/issues/12#issuecomment-5042702401),
  SHA-256
  `9a037f94af84516592a56dc9c0c701ce0277e305c83dad368227fc25a5b18d9a`;
- merged-source addendum comment
  [`5043498697`](https://github.com/lossyrob/telex/issues/12#issuecomment-5043498697),
  SHA-256
  `fa02b844c62eef17f4c08b9bc1d7d94539e525034e3f0d474b2bfe2d45caed94`;
- canonical final source head
  `e007a8067b3b91b5c57a2a756ce878e310595a05`;
- merge commit `09aa6f45f213b45207adc4cf80676dcce91250da`.

### Operator Station

- requirement export comment
  [`5042612298`](https://github.com/lossyrob/telex/issues/12#issuecomment-5042612298),
  SHA-256
  `adf2f8e439e5c224059ca51142701f604a82203c39c9829d1323a88c58889f7e`;
- merged-source addendum comment
  [`5044388908`](https://github.com/lossyrob/telex/issues/12#issuecomment-5044388908),
  SHA-256
  `702ebdb1ea81329294c35a452670b2313625142bc0281c9100d8ae892890c9ea`;
- canonical final source head
  `2d99e552292a4401d3403540b6d2eaa90272282d`;
- merge commit `0722051760bab569d3f947fd7b29f2dabe13ef77`.

## Accepted shared semantics

The API-neutral contract defines:

1. stable application responsibility distinct from fresh, never-reused runtime
   identity;
2. typed process liveness using process ID plus process start time;
3. atomic-or-compensable multi-address attach, reconcile, and detach;
4. caller-selected strict membership loss or bounded automatic repair;
5. typed membership-loss and collision evidence, with no silent takeover;
6. explicit sender selection for every application-authored operation;
7. explicit send-only and bidirectional capabilities;
8. separate durable-acceptance, occupancy, push, recipient-consumption, and
   workflow-disposition axes;
9. bidirectional receive with exact recipient/delivery-row identity and bound
   acknowledgment;
10. acknowledgment only after durable application ingest, with backlog and deafness
    observable;
11. per-recipient at-least-once identity and no-regression fenced resynchronization;
12. unresolved-obligation and bounded recent/thread-history queries;
13. typed send, metadata reply, read-thread, acknowledgment, disposition, and source
    operations;
14. retry-stable operation identity and post-restart result/receipt reconciliation;
15. source identity as `(opaque logical-store identity, message ID)`;
16. one semantic contract across SQLite and credentialed Postgres, with principal
    provenance when available;
17. evidence-bearing lifecycle, readiness, recovery, and backlog health projection;
18. bounded discovery, retry, receipt cross-checking, and scoped cleanup without CLI
    parsing or raw IPC;
19. ordered delta events with explicit resync/backfill;
20. ordered compound-operation primitives with partial/indeterminate outcomes,
    recovery handles, and machine-readable terminal-closure evidence.

## Requirement disposition

| Source set | Accepted | Deferred | Rejected | Total |
|---|---:|---:|---:|---:|
| Watcher W-01 through W-15 | 15 | 0 | 0 | 15 |
| Operator AC-01 through AC-15 | 15 | 0 | 0 | 15 |
| **Combined** | **30** | **0** | **0** | **30** |

Every accepted row still requires supported implementation and conformance. The
complete rationale, remaining owner, downstream blocking effect, source digest, and
strength-preservation assertion for each row are in the crosswalk.

## Historical proposal disposition

The previous issue body is preserved byte-for-byte as a historical artifact. Its
runtime-agnostic integration goal and semantic operation categories are accepted.
The holder/waiter generalization, client-owned heartbeat/cursor assumptions, and
generic automatic registration are replaced by current daemon membership,
`NeedsAttach`, exact-recipient acknowledgment, caller-selected recovery, and fenced
at-least-once resynchronization.

Package names, TypeScript/napi-rs priority, C ABI or FFI shape, public socket
protocol, delivery ergonomics, language sequence, and interrupt policy are deferred
to implementation nodes. The historical TypeScript interface is rejected as
contract authority.

## Product boundary

Watcher continues to own detector execution, scheduling, script policy, state
transactions, event policy, provider templates, and runtime health. Operator Station
continues to own direct/assisted/quiet routing, mediation judgment, notification
policy, human UI, and route policy.

The shared client must not be replaced by:

- Telex CLI stdout/stderr parsing;
- raw private daemon IPC or internal Rust seams;
- subprocess courier or spike helper behavior;
- sender occupancy or push attempt as proof of recipient consumption;
- store-path fingerprints, private local UUID files, or spike namespaces;
- a Watcher-private or Operator-private production client.

If an implementation cannot satisfy an accepted semantic, the affected consumer
remains blocked. Deferral never authorizes an undocumented fallback seam.

## What `application-client-ready` unlocks

The checkpoint unlocks detailed work for:

1. the supported Application Client core;
2. the first public language binding;
3. semantic and backend-parity conformance;
4. Watcher integration;
5. Operator Station integration;
6. operational hardening and packaging.

Those nodes remain responsible for implementation, testing, integration, and
production readiness. This issue remains the sole shared semantic contract owner.
