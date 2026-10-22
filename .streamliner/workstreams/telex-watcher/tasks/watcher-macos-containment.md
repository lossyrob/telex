# macOS Watcher process containment

- **Workstream:** `telex-watcher`
- **Node:** `watcher-macos-containment`
- **Type:** investigation and implementation
- **Status:** planned; investigation may start after tracker/task
  reconciliation, implementation lands after #162 merges
- **Attention:** focus
- **Depends on:** completed `dumb-watcher-contract-gate`
- **Implementation sequencing:** issue #162 must merge before implementation
  lands
- **Tracker:** [lossyrob/telex#165](https://github.com/lossyrob/telex/issues/165)
- **Parent workstream:** [lossyrob/telex#100](https://github.com/lossyrob/telex/issues/100)
- **Campaign:** [Addressable Attention #102](https://github.com/lossyrob/telex/issues/102)

## Outcome

Establish production macOS process-tree containment for Telex Watcher so abrupt
runtime exit cannot leave a detector or detached descendant running. macOS
remains a required Watcher product platform. The fail-closed behavior required
from issue #162 is a safe interim boundary, not completion of this outcome.

This is one phased node. Investigation and the operator decision packet may
begin after this tracker and its canonical task are reconciled. Implementation
lands only after issue #162 merges and, when an option requires reserved
authority, after the operator selects that architecture.

This node does not block the Windows-path five-minute custom-watch gate. It does
block product-level `usable-watcher` completion and final Watcher closure.

This node does not block the Windows-path `five-minute-custom-watch-gate`. It
does block the product-level `usable-watcher` checkpoint and `closure-gate`,
because macOS remains a required Watcher product platform and fail-closed
behavior is not the completed product outcome.

## Authority

- Parent workstream: [#100](https://github.com/lossyrob/telex/issues/100)
- Campaign: [#102](https://github.com/lossyrob/telex/issues/102)
- Runtime-core predecessor: [#162](https://github.com/lossyrob/telex/issues/162)
- Canonical task:
  `.streamliner/workstreams/telex-watcher/tasks/watcher-macos-containment.md`
- Canonical workstream design:
  `.streamliner/workstreams/telex-watcher/design/current-design.md`
- Accepted dependency ordering:
  `.streamliner/workstreams/telex-watcher/graph.json`
- Normative contract: `docs/design/watcher.md`
- Current implementation evidence:
  [PR #164](https://github.com/lossyrob/telex/pull/164)
- Confirmed blocker review: [review
  5620319072](https://github.com/lossyrob/telex/pull/164#pullrequestreview-5620319072)

The operator decision dated 2026-10-22 scopes #162 containment proof to Windows
and Linux, requires #162 to fail closed on macOS, preserves macOS as a required
product platform, permits this investigation now, and sequences implementation
after #162 merges. It grants no privileged, entitlement-dependent,
helper-service, broker, VM, release, tag, publish, fleet, I161-1, CLI,
packaging, or Operator Station authority.

## Phase 1: investigation and decision packet

- Reproduce and preserve the current XNU evidence, including rejection of
  descendant `NOTE_TRACK`, with minimal owned-process probes.
- Identify candidate unprivileged mechanisms that can contain a detector and all
  detached descendants across abrupt Watcher runtime exit.
- Prove or disprove each serious candidate against daemonization,
  session/process-group escape, ignored signals, parent exit races, PID reuse,
  startup recovery, and supported Darwin architectures.
- Separate mechanisms available to an ordinary per-user process from mechanisms
  that require privilege, entitlements, a helper service, a broker,
  virtualization, or a packaging/deployment change.
- Produce an operator decision packet that states the recommended architecture,
  alternatives, security boundary, lifecycle, failure behavior, rollback path,
  packaging and operational implications, validation plan, and exact new
  authority each option requires.
- If no conforming unprivileged mechanism exists, report `decision-needed`. Do
  not silently narrow macOS support or implement a reserved architecture.

## Phase 2: implementation sequencing

- Do not land implementation before issue #162 merges.
- Rebase the selected design on merged runtime-core authority rather than the
  draft implementation shape.
- If the selected design requires privilege, entitlements, a helper service, a
  broker, a VM, or broader packaging/deployment authority, wait for the
  operator's explicit architecture decision before implementation.
- Preserve issue #162's public Application Client boundary, exact-operation
  receipt recovery, fixed routing, runtime-owned identity, pending-operation
  safety, store-binding lifecycle/recovery repair, and readiness-membership
  repair.
- Keep macOS containment changes bounded to this node. Do not absorb CLI,
  examples, release, fleet, Station, or general service-management work.

## Required safety boundaries

- A detector must not start until the runtime has established a durable
  containment identity and confirmed that the selected mechanism can terminate
  the detector and descendants if the runtime exits.
- Unsupported, unavailable, denied, or indeterminate containment fails closed
  before detector launch with typed machine-readable evidence.
- Process groups, sessions, parent-death signals, or periodic process-table
  polling are insufficient unless destructive evidence proves detached
  descendants cannot escape and PID reuse cannot target unrelated processes.
- Startup recovery must distinguish proven cleanup from orphan-containment
  uncertainty. Unproven cleanup blocks the affected watch and never starts a
  replacement execution.
- Tests may terminate only processes created inside the isolated test plane. No
  design may scan for or kill unrelated same-user processes.
- No containment result may authorize message resend, local state commit,
  automatic resume, or weakening of receipt and store-recovery safety.

## Out of scope and non-authority

This tracker does not authorize or deliver:

- implementation landing before issue #162 merges;
- a privileged, entitlement-dependent, helper-service, broker, or VM
  architecture before the operator selects it;
- release publication, tags, fleet rollout, deployment, or production-readiness
  claims;
- the `watcher-cli-ux` management experience or the Windows-path five-minute
  gate;
- issue #144, detector examples, provider actions, or workflow automation;
- broad packaging, installer, updater, service management, or upgrade UX beyond
  an explicitly selected containment architecture;
- Operator Station product or UI work;
- changes to public Application Client, registry, receipt, routing, sender,
  target, or detector-result semantics except the minimum containment
  integration approved here; or
- Streamliner state, workflow records, approval records, or orchestration
  artifacts in the product diff.

## Success criteria

- The decision packet gives the operator a reproducible evidence chain and a
  bounded choice among viable architectures, or a precise `decision-needed`
  result when no current option satisfies the contract.
- On each supported macOS architecture, destructive native evidence proves
  abrupt runtime exit terminates the detector and detached descendants without
  killing unrelated processes.
- Detector launch fails closed with typed evidence whenever containment setup is
  unavailable, denied, incomplete, or indeterminate.
- Startup and crash recovery retain containment identity and distinguish proven
  cleanup from blocked orphan uncertainty.
- The implementation preserves public-client-only integration, receipt-gated
  state commit, exact-operation recovery, fixed routing, runtime-owned identity,
  store-binding lifecycle/recovery, and readiness reconciliation.
- The Windows and Linux containment behavior from issue #162 remains green.
- Canonical design, graph, task, product documentation, and operational evidence
  describe the selected macOS guarantee without overstating unsupported
  mechanisms.
- Every material investigation result has a durable disposition, and no accepted
  node-owned work remains deferred at merge readiness.

## Validation and review

- Use owned disposable processes and isolated absolute `TELEX_HOME`, `TELEX_DB`,
  and `TELEX_INSTALL_ROOT` values. Do not use installed, user, or campaign
  coordination state.
- Run destructive native macOS tests for ordinary children and descendants that
  fork, create new sessions or process groups, ignore signals, outlive their
  immediate parent, and race runtime death during launch and containment
  registration.
- Exercise containment unavailable, permission denied, partial setup, runtime
  crash, restart, stale identity, PID reuse, and cleanup failure paths.
- Compile and test every supported Darwin target and prove the runtime fails
  closed where the selected mechanism is unavailable.
- Run formatting, warnings-denied Clippy, affected Watcher unit and integration
  tests, SQLite and credentialed-Postgres matrices, and the repository feature
  matrix.
- Establish a complete independent implementation-review baseline, run required
  CI, and obtain fresh exact-head design inspection before merge readiness.
- Final review, required CI, native destructive evidence, and design inspection
  must cover the same final head. Head movement invalidates affected evidence
  and requires the corresponding delta or full rerun.

## Invalidation conditions

Stop and report `decision-needed` if investigation or implementation requires:

- privilege, entitlements, a helper service, a broker, a VM, or broader
  packaging/deployment authority not explicitly selected by the operator;
- narrowing or dropping macOS as a required Watcher product platform;
- claiming containment from process groups, parent tracking, polling, or
  best-effort cleanup without destructive detached-descendant proof;
- starting a detector before containment is established or continuing after
  containment becomes indeterminate;
- targeting processes without PID/start-time-safe owned identity;
- weakening public Application Client, exact-operation receipt, fixed-route,
  runtime-owned identity, pending-operation, store-binding recovery, or
  readiness safety;
- implementation landing before issue #162 merges;
- release, tag, publish, fleet, I161-1, CLI, examples, general
  service-management, or Operator Station authority;
- destructive testing against shared user or campaign state; or
- splitting the node without an independently useful boundary and explicit join
  conditions.
