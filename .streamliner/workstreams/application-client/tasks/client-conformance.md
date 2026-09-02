# Application Client conformance and consumer consumability

- **Workstream:** `application-client`
- **Node:** `client-conformance`
- **Type:** implementation
- **Status:** ready, unlaunched
- **Attention:** focus
- **Depends on:** completed `client-core`, completed `first-binding`
- **Tracker:** [lossyrob/telex#152](https://github.com/lossyrob/telex/issues/152)
- **Parent workstream:** [lossyrob/telex#117](https://github.com/lossyrob/telex/issues/117)
- **Campaign:** [Addressable Attention #102](https://github.com/lossyrob/telex/issues/102)

## Outcome

Deliver one executable conformance bundle for the supported Rust Application
Client. The same public `telex::application_client` cases must pass against
isolated SQLite and credentialed Postgres. Public-only Watcher-shaped send-only
and Operator Station-shaped bidirectional fixtures must prove that both
consumers can use the supported seam without CLI parsing, raw private daemon
IPC, spike helpers, consumer DTO promotion, or a product-private client.

This is one node, one tracker, and one delivery PR. It completes
`client-conformance` and supplies evidence for the later
`consumer-integration-gate`. It does not implement either product, pass that
gate by itself, publish a release, or establish production readiness.

## Authority

- [Issue #152](https://github.com/lossyrob/telex/issues/152) - tracker and
  complete node outcome; verified ASCII body SHA-256
  `D37C020B141801A106B7089EC61748C9810AEC6049EFE58C80227C576FA66AD0`.
- [Issue #12](https://github.com/lossyrob/telex/issues/12) - sole semantic owner,
  publication revision 4.
- [`../design/current-design.md`](../design/current-design.md) - canonical
  integrated design and conformance boundary.
- [`../../../../docs/design/application-client.md`](../../../../docs/design/application-client.md)
  - normative API-neutral contract, AC-C01 through AC-C20.
- [`../../../../docs/application-client-core.md`](../../../../docs/application-client-core.md)
  - supported Rust binding and the ten required conformance families.
- [`../../../../docs/notes/application-client/requirements-crosswalk.md`](../../../../docs/notes/application-client/requirements-crosswalk.md)
  - non-normative Watcher W-01 through W-15 and Operator AC-01 through AC-15
  traceability.
- [`../graph.json`](../graph.json) and
  [`../../../shaping/roadmap.md`](../../../shaping/roadmap.md) - accepted node,
  gate, and campaign ordering.

## Bundle rule

Keep every node-owned proof in issue #152 and one PR. Do not split by backend,
test family, fixture, migration, cleanup, or reviewability. A split requires an
independently useful confidence, release, provider, or cross-repository boundary
with explicit join conditions and campaign approval.

If a real prerequisite appears, preserve the same branch and PR, model the
owning `dependsOn` or `externalDependsOn` entry with the exact resume condition,
and continue after it closes. Node-owned unfinished work is in progress, not
blocked. If no modeled dependency owns a real blocker, report `blocked` with
`graphGap: true`; do not downscope the result or present an incomplete PR as
complete.

## Required conformance matrix

Run the same public Rust semantic cases against isolated SQLite and credentialed
Postgres:

1. Fresh runtime identity with stable application responsibility and logical
   store identity across reconnect and path or profile presentation changes.
2. Strict and bounded recovery; restart membership loss; deliberate detach;
   predicate death; owner demotion; collision evidence; and raw preservation of
   unknown future loss reasons.
3. Atomic-or-compensable multi-address attach, reconcile, and detach, including
   new membership -> `Detach`, changed existing membership ->
   `Reattach(previous_spec)`, idempotent refresh -> no destructive compensation,
   cancellation evidence, and crash continuation.
4. Send-only false-attendance prevention and bidirectional receive with exact
   delivery-row identity and bound acknowledgement.
5. Independent acceptance, occupancy, push, recipient-consumption, and
   workflow-disposition evidence, including acknowledgement-after-durable-ingest
   restart recovery.
6. Retry-stable operation replay; fingerprint, payload, store, and duplicate
   evidence; authoritative exact-tuple `NotRecorded`; retention-boundary
   invalidation; accepted-send indeterminate windows; prepared recovery handles;
   and post-restart reconciliation.
7. Unresolved, recent, and thread filtering before bounds, plus store-scoped
   source resolution and fail-closed source ambiguity.
8. Monotonic delta ordering, gap detection, resync, and no-regression backfill.
9. Compound prerequisite ordering, partial and indeterminate outcomes, recovery
   handles, terminal-step fencing, and crash continuation.
10. Schema v2-to-v3 migration, newer-schema refusal, bounded cleanup, retention
    generations, principal provenance, and exclusion of raw paths, credentials,
    backend rows, daemon frames, and private storage details from public
    evidence.

Reuse established backend, schema, Postgres-service, and test-isolation helpers.
Prefer plain repository tests and fixtures over a new conformance framework or
evidence schema unless a concrete missing failure requires one.

## Consumer-shaped evidence

The PR must include thin public-only fixtures or probes for both capability
families:

- **Watcher send-only:** defaults-disabled backend selection, stable
  responsibility/runtime/store identity, durable acceptance separated from
  occupancy/push/consumption/disposition, exact-operation recovery,
  retention-boundary behavior, SQLite/Postgres parity, and no false inbound
  attendance.
- **Operator Station bidirectional:** multi-address lifecycle and compensation,
  receive/acknowledgement after durable ingest, unresolved/history/thread
  recovery, metadata-bearing ordinary reply, exact-recipient disposition, source
  resolution, compound `Reply & Handle` ordering, health, delta/gap/resync,
  deliberate detach, cleanup, and provenance.

These fixtures use only the shared public client. They do not promote consumer
DTOs or implement detectors, scheduling, presentation, notifications,
mediation, installation, usability, or product workflow policy.

Any missing implementation of an already-accepted shared semantic required by
either merged consumer contract is repaired in this PR. A newly required public
semantic triggers `decision-needed`. Product-only behavior is routed to its
owning downstream node.

## Consumer attestations and gate evidence

After implementation review and required CI succeed on one exact head, but
before merge readiness:

- Watcher authority independently checks the exact bundle against its merged
  requirement set and confirms that no CLI, raw IPC, or Watcher-private client
  is needed.
- Operator Station authority independently checks the same exact bundle against
  its direct-attendance contract and confirms that no CLI, raw IPC, spike
  helper, product DTO promotion, or Station-private client is needed.

Both attestations name the same reviewed and green head. Head movement
invalidates them. After conformance merges, the attestations become evidence
for `consumer-integration-gate`; they do not pass the gate or authorize
consumer launch from this PR. Product runtime, UI, usability, packaging, and
operational evidence remain downstream.

## Work conservation and discovered work

Every material discovery from shaping, implementation, review, CI, runtime
proof, consumer attestation, design inspection, or the field report receives one
durable disposition in
[`../discovered-work.json`](../discovered-work.json).

- Required current-PR work is absorbed into this PR.
- A real prerequisite is modeled, and the same PR resumes after it closes.
- Product-only work is routed to the owning existing node or external tracker.
- Rejected work records why its cost exceeds concrete incremental value.

No material item may remain `untriaged` at merge readiness. Accepted same-PR
items must be delivered; downstream items need an owner and connected terminal
path.

## Out of scope

- Watcher detector/runtime/CLI implementation or Operator Station UI,
  notification, mediation, installation, and usability behavior.
- Passing `consumer-integration-gate`, launching either consumer, or completing
  `supported-client` from this PR.
- TypeScript/napi-rs, a separate client crate, C ABI, public socket or sidecar
  protocol, or consumer-specific DTO contract.
- Release publication, version-based consumption, installer/updater work,
  signing, broad packaging, upgrade readiness, and operational hardening.
- CLI parsing, raw private daemon IPC, subprocess courier behavior, or any
  product-private fallback.
- Streamliner mutation, gate operation, or issue #12 publication updates from
  the implementation branch.

## Success criteria

- One backend-neutral public conformance suite executes every required family
  against isolated SQLite and credentialed Postgres with equivalent typed
  outcomes.
- Credentialed Postgres is mandatory in authoritative CI; a missing required
  environment fails instead of becoming success-shaped skip evidence.
- Public-only send-only and bidirectional consumer fixtures compile under
  documented `default-features = false` profiles and exercise representative
  lifecycle, messaging, recovery, history, and resync paths.
- The suite proves exact identities, typed failures,
  cancellation/indeterminate evidence, compensation, retry/reconciliation,
  migration, cleanup, and provenance without making backend-private assertions
  part of the public contract.
- No product-private seam, hidden runtime, sidecar, new ABI/process boundary,
  consumer DTO authority, or unrelated framework is introduced.
- Watcher and Operator Station provide exact-head consumer-authority
  attestations after review and CI.
- Every modeled blocker is closed and every material discovery is durably
  dispositioned.
- The final exact head has contiguous implementation review coverage, no
  unresolved actionable thread, successful required CI, both consumer
  attestations, and a passing fresh design inspection.
- The PR closes issue #152, contains only product deliverables, and contains no
  `.paw/**`, `.streamliner/**`, workflow transcripts, or scratch artifacts.

## Validation, review, and reporting

- Run formatting, warnings-denied Clippy, the complete Application Client
  SQLite suite, the same credentialed-Postgres conformance battery, schema
  migration/newer-schema tests, external public-consumer fixture profiles, and
  the repository feature matrix.
- Keep destructive or persistent tests isolated with unique temporary SQLite
  paths and per-test Postgres schemas. Do not operate against installed or user
  coordination state.
- Start review and CI concurrently only when the complete node bundle is
  present. Establish one full PAW baseline for the first stable,
  feature-complete head. Later clean deltas produce internal approval without
  another GitHub comment.
- After implementation review and required CI pass on one exact head, obtain the
  two consumer attestations and a fresh read-only design inspection. Use a new
  Claude Opus 4.8, high-reasoning, long-context inspector child. Any head movement
  invalidates same-head review, CI, attestation, and inspection evidence as
  applicable.
- Use the campaign wait coordinator only for immutable external preparation,
  CI, mergeability, or provider-run conditions. Child review, inspection, and
  design results use direct App messages.
- Before completion, post an issue #152 field report covering outcome, exact PR
  head, decisions, validation, review and CI coverage, consumer attestations,
  design inspection, discovery dispositions, deferred work, downstream impact,
  and clean-worktree state.

## Dependency and promotion

`client-core` and `first-binding` are complete. Issue #152 alone does not make
this node ready: this task, the discovery ledger, and cross-workstream
dependency reconciliation must land on `main` first.

After that authority lands, the Application Client orchestrator may prepare and
launch one implementation session under the App launch protocol. Consumer work
remains blocked until conformance merges, both exact-head attestations are
accepted, and `consumer-integration-gate` is separately passed and reconciled.

## Invalidation conditions

Stop and report `decision-needed` if conformance requires a new public semantic,
weakened accepted distinction, new language/ABI/process boundary,
consumer-specific shared policy, private fallback, hidden runtime or sidecar,
unmodeled dependency, material validation gap, or split without a real
independent boundary.
