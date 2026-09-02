# Transactional station-intent authority and fair maintenance

- **Workstream:** `local-daemon`
- **Node:** `station-intent-transactional-authority`
- **Type:** implementation
- **Status:** planned; launch requires a GitHub tracker and separate authorization
- **Attention:** focus
- **Depends on:** completed `station-intent-reconciliation`
- **Owner:** Local Daemon workstream orchestrator until an authorized implementer is assigned
- **Tracker:** unassigned; create a Local Daemon GitHub issue before launch
- **Parent workstream:** [lossyrob/telex#32](https://github.com/lossyrob/telex/issues/32)
- **Campaign:** [Addressable Attention #102](https://github.com/lossyrob/telex/issues/102)

## Outcome

Replace flat-file station-intent mutation and restart-at-head enumeration with
one versioned transactional authority per daemon singleton scope. Generation
changes are atomic, ordered continuation provides fair eventual discovery and
garbage collection, counts are exact, and over-cap scopes recover without
depending on repeated directory-prefix scans. Migration, rollback, corruption,
and old-writer behavior preserve one authority through every transition.

The daemon still returns within the accepted four-second bound. A late
transaction may complete only under atomic generation authority and cannot
delete or replace a newer generation. Station intent remains desired push
registration only; it never establishes membership, attendance, lease
ownership, positive liveness, or permission to deliver.

## Design references

- [`../design/current-design.md`](../design/current-design.md) - accepted
  transactional outcome, preserved station-intent boundary, and promotion
  boundary.
- [`../discovered-work.json`](../discovered-work.json) - durable disposition of
  `local-daemon-pr138-m5-enumeration-liveness`.
- [`../graph.json`](../graph.json) - downstream ordering and closure dependency.
- [`../../../../docs/design/daemon.md`](../../../../docs/design/daemon.md) -
  normative daemon, reconciliation, response-bound, and filesystem support
  contracts.
- [`../../../../docs/design/DECISIONS.md`](../../../../docs/design/DECISIONS.md)
  - ADR 0052 station-intent authority after PR #138 promotion.

## Inputs

- The merged `station-intent-reconciliation` outcome: desired-state semantics,
  producer proof, detach/reset precedence, four-second response, persistent OS
  lock containment, observable partial-scan degradation, and both-backend
  behavior.

## Exports

- One versioned transactional station-intent authority with atomic generation
  mutation, seekable ordered continuation, fair discovery and garbage
  collection, exact counts, and exact over-cap recovery.
- A crash-safe migration, cutover, rollback, corruption-recovery, and
  old-writer-refusal contract that keeps one authority at every point.
- Normative design and operator guidance that remove the temporary degraded
  flat-directory contract only after the replacement authority is proven.

## Boundaries

### In scope

- Transactional authority for host-local station intent.
- Migration from the PR #138 flat-file layout without split authority.
- Bounded caller response with generation-safe late completion.
- Seekable fair reconciliation and garbage collection.
- Exact scope counts and over-cap recovery.
- Windows and Unix behavior for SQLite-backed and Postgres-backed stations.

### Out of scope

- Changing station intent into attendance, membership, liveness, lease, or
  delivery authority.
- Application Client installed-current bootstrap from issue #152.
- Product-specific Watcher or Operator Station policy.
- Weakening daemon peer authentication, epoch fencing, detach tombstones, or
  Copilot producer proof.
- Treating a sidecar index, random directory rotation, retained directory
  iterator, or bounded sharding assumption as unconditional authority.

## Inherited decisions

- **Transactional convergence is required.** The operator accepted PR #138's
  bounded degraded-enumeration contract only as a temporary gap. This node
  restores unconditional generation and maintenance authority before Local
  Daemon closure.
- **PR #138 remains independently complete.** This node follows
  `station-intent-reconciliation`; it does not block that PR or its hardening
  gate. The split is justified by the persistent-layout migration boundary.
- **One authority through cutover.** Old and new writers may not mutate separate
  representations concurrently. Versioning, migration, rollback, and refusal
  behavior must make the authoritative representation unambiguous after a
  crash or downgrade attempt.
- **The four-second bound remains.** The caller may stop waiting for blocking
  work, but atomic generation authority must make any admitted late completion
  safe. Response latency is not proof that underlying work was cancelled.

## Design-impact expectation

Expect material updates to `docs/design/daemon.md` and a decision record for the
transactional layout, migration, cutover, rollback, and compatibility contract.
Update ADR 0052 only through an explicit superseding or follow-up decision
consistent with the append-only decision log.

## Success criteria

- A reviewer can prove that generation mutation, withdrawal, finalization, and
  garbage collection cannot delete or replace a newer generation, including
  after caller timeout, process death, restart, migration, and rollback.
- Ordered continuation eventually reaches every stable intent and eligible
  garbage-collection row whenever the transactional authority continues to
  admit maintenance work; repeated early entries cannot starve a stable tail.
- Scope counts and over-cap state are exact, and removing eligible records
  restores admission without an offline directory scan.
- Migration and rollback preserve exactly one authority, refuse incompatible
  old writers, preserve unsupported newer data, and recover actionably from an
  interrupted or corrupt transition.
- The same station-intent contract works for SQLite-backed and Postgres-backed
  stations on supported Windows and Unix hosts.
- The final exact head has complete implementation review, required CI, and a
  fresh design inspection, and its promoted design removes the accepted
  degraded gap from current authority.

## Engagement

- Review the worker plan before repository mutation, with specific attention to
  authority selection, migration, rollback, and old-writer refusal.
- Review the proposed persistent-layout and product-design change before
  implementation is treated as stable.
- Require an end-to-end migration, restart, fairness, and over-cap recovery
  demonstration before merge readiness.
