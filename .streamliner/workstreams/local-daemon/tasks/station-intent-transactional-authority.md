# Transactional station-intent authority and fair maintenance

- **Workstream:** `local-daemon`
- **Node:** `station-intent-transactional-authority`
- **Type:** implementation
- **Status:** blocked (2026-10-08) on Streamliner preparation long-context
  propagation; the worker exists but is not write-authorized
- **Attention:** focus
- **Depends on:** completed `station-intent-reconciliation` (PR #138 merged as
  `6ab6143a`)
- **Owner:** Local Daemon workstream orchestrator until an authorized implementer is assigned
- **Tracker:** [lossyrob/telex#153](https://github.com/lossyrob/telex/issues/153)
- **Parent workstream:** [lossyrob/telex#32](https://github.com/lossyrob/telex/issues/32)
- **Campaign:** [Addressable Attention #102](https://github.com/lossyrob/telex/issues/102)

## Readiness (2026-10-08)

The prerequisite is met. PR #138 merged as
`6ab6143ad0d0e8b9832df41e4e180c4acd1204c4` (tree
`a9bd5f0dc4a8867b3dc04b7358d0d2fdd3eef902`) under campaign merge authority,
and issue #106 was closed as completed at 2026-10-08T21:50:45Z by an explicit
provider action (event 32828313724). The campaign's 2026-10-08 post-release
direction promotes this node after that merge from the landed authority with a
complete task specification. Design steward `af271672` found this
specification ready for one supported preparation, with no missing outcome,
acceptance criterion, or operator choice. Preparation starts from source
`6ab6143a` and the canonical task revision that records this section.

The worker chooses the exact store, layout, and cutover mechanism as node
research; that choice is not an upfront operator vote. Accepted contracts
cannot be weakened: no split writers, no discarding newer data, no manual
reattachment in place of fair or exact maintenance, no weaker fencing, no
longer response deadline, and no promise of transparent downgrade to
unsupported versions. Any such departure is a material decision for the
operator; ordinary implementation choices are not. Preparation comes before
any writer launch, and new sessions explicitly request `gpt-6-astra`, high
reasoning effort, and long context. Builder hardening and closure gates stay
separate decisions.

### Current blocker (2026-10-08)

Supported preparation `beb72642` completed normally from clean source
`43d5e43f` and created one isolated worktree on local branch
`feature/station-intent-transactional-authority`. Worker session `62bbe494`
was created at 2026-10-08T22:13:52Z and correctly recorded the requested
`gpt-6-astra`, high reasoning, `long_context` profile. The preparation
initializer did not meet that profile: it ran with high reasoning but no
context tier and a 272,000-token prompt limit across 22 calls. The cause is in
Streamliner preparation, not in this task. Its SDK 0.3.0 session
configuration and create/resume calls have no context-tier field, and the
adapter passes long context only as a startup CLI argument. The
1,050,000-token catalog figure is model capability, not the effective
setting. The worker has never been write-authorized; its worktree is clean at
`43d5e43f`, and it is held before any product write.

The graph records this as external condition
`streamliner-preparation-long-context-propagation`, owned by Streamliner
service `79bfffa9`, with campaign authority over any repair or restart. It
clears after a reviewed and merged preparation-only adapter repair that sets
the context tier explicitly on session create and resume, a controlled
restart of the owned API, and one new initializer run on the same checkout
that proves long context on the wire, at start, and in usage, with normal
context loading and PAW initialization once. The earlier run stays frozen as
evidence, and no silent downgrade is accepted. The campaign has not yet
authorized the repair. A global SDK migration, a duplicate server, or a
`node_modules` patch is not part of the fix. The task outcome, scope, success
criteria, single worker, and prepared branch are unchanged.

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

- The merged `station-intent-reconciliation` outcome at `6ab6143a`:
  desired-state semantics,
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

- The worker owns routine research, planning, implementation and validation
  within the accepted task. Consult the design steward on persistent layout,
  migration/cutover, rollback and old-writer-refusal coherence; no per-plan or
  per-test acknowledgement gate applies.
- Route any new material contract, trust/support, compatibility or
  accepted-gap decision through Local to the operator before encoding it.
  Preserve the existing exact-head review, CI, design-inspection and
  merge-authority requirements.
- Require an end-to-end migration, restart, fairness, and over-cap recovery
  demonstration before merge readiness.
