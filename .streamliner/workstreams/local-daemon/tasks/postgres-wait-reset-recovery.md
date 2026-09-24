# PostgreSQL wait-reset recovery

- **Workstream:** `local-daemon`
- **Node:** `postgres-wait-reset-recovery`
- **Type:** implementation
- **Status:** planned; launch requires Tier B landing and exact verification
- **Attention:** focus
- **Depends on:** none
- **Blocks:** `schema3-release-preparation`
- **Owner:** Local Daemon workstream orchestrator until one authorized implementer is verified online
- **Tracker:** [lossyrob/telex#155](https://github.com/lossyrob/telex/issues/155)
- **Adopted PR:** [lossyrob/telex#156](https://github.com/lossyrob/telex/pull/156)
- **Published branch/head:** `copilot/fix-postgres-connection-reset` at `5c302dacb1c3e659e7f89c3c9670e3cf5cbc5105`
- **Parent workstream:** [lossyrob/telex#32](https://github.com/lossyrob/telex/issues/32)
- **Campaign:** [Addressable Attention #102](https://github.com/lossyrob/telex/issues/102)

## Outcome

Complete issue #155 by adopting open draft PR #156 in place. Preserve its code,
branch, commits, and review history. Do not replace the branch or recreate the
repair.

Recover active waiters from transient PostgreSQL query and `LISTEN` connection
resets while pull and push stations share one daemon. Preserve finite wait
deadlines during recovery, bound retry, and return an actionable terminal outcome
when recovery is exhausted. The operator selected a repair, not an accepted
limitation.

## Inputs

- Issue #155 incident sequence and expected recovery behavior.
- Draft PR #156 at published head
  `5c302dacb1c3e659e7f89c3c9670e3cf5cbc5105`, four commits ahead of exact main
  `ed417c6b938f92fe3bfb84f3b0cc0bea719fbbd0`.
- Existing daemon reconnect, watchdog, finite-deadline, status, and exit-code
  contracts.

## Boundaries

### In scope

- Structural classification of transient query and `LISTEN` failures.
- Bounded reconnect/retry that keeps an eligible waiter armed.
- Preservation of the original finite wait deadline across recovery.
- Actionable exhausted-recovery status and documented exit behavior.
- Fault-injection coverage with pull and push stations on one daemon.
- Compatibility and design-impact inspection, exact-head review, and required CI.

### Out of scope

- Replacing, rebasing, force-pushing, or rewriting PR #156 or its branch.
- Treating the failure as an accepted limitation.
- Relaxing PostgreSQL identity, attendance, delivery, deadline, or schema guards.
- PR #138, issues #152/#153, Watcher/Station runtimes, and unrelated features.

## Success criteria

- Transient query and `LISTEN` resets recover for representative pull and push
  delivery without dropping station registration.
- Recovery never extends a finite wait beyond its original deadline.
- Retry is bounded and exhausted recovery produces an actionable documented
  outcome instead of generic exit 1.
- Fault-injection tests exercise query reset and `LISTEN` reset while pull and push
  stations share one daemon.
- The exact head passes required CI, implementation review, and compatibility and
  design inspection before campaign-authorized merge.

## Engagement

- Product launch is conditional on campaign-approved Tier B landing and exact
  verification. Before resuming PR #156, establish one writer and verify that no
  cloud or legacy writer overlaps.
- The Local Daemon orchestrator prepares the external repair session through
  Streamliner, registers the exact prepared checkout path in branch mode, verifies
  `session-online`, grants standalone write authority, and obtains acknowledgement
  before product writes.
- This is one of exactly three new delivery sessions in the packet: two repair
  sessions and one later release worker. Do not create dormant placeholders.
- Every new session or delegated agent must explicitly set
  `model=gpt-6-astra`, `reasoning_effort=high`, and
  `context_tier=long_context`; silent downgrade is not authorized. This artifact
  role creates or delegates none.
- Review checkouts must be physically distinct and read-only. Never use
  `open_pr_session` for a reviewer.
- Register external waits only through WATCHER
  `2a4bc4c8-1211-49d4-ba68-9d05d5d7530d`.
- Merge only the exact reviewed, green, inspected PR #156 head after campaign
  merge authorization.
