# Telex Watcher Current Design

This document is the canonical integrated design for the Telex Watcher
workstream. Project-level documents under `docs/design/` remain authoritative
for their contracts; this document records how those contracts, accepted
decisions, workstream boundaries, and downstream gates fit together.

## Authority

The current design is grounded in:

- the merged [Watcher production contract](../../../../docs/design/watcher.md);
- [ADR 0046](../../../../docs/design/DECISIONS.md#0046--watcher-runs-provider-neutral-trusted-local-detectors-with-receipt-gated-state),
  retaining the provider-neutral, trusted-local, fixed-route, receipt-gated
  architecture;
- [ADR 0050](../../../../docs/design/DECISIONS.md#0050--watcher-v2-uses-minimal-command-registration-and-runtime-owned-event-identity),
  superseding ADR 0046's mandatory authoring ceremony and detector-authored
  identity;
- the [Application Client contract](../../../../docs/design/application-client.md),
  which owns the supported Telex integration seam;
- the [campaign roadmap](../../../shaping/roadmap.md), [workstream brief](../brief.md),
  [graph](../graph.json), and [reconciliation note](../reconciliation-note.md).

The minimal v2 contract was promoted through
[issue #133](https://github.com/lossyrob/telex/issues/133) and
[PR #135](https://github.com/lossyrob/telex/pull/135), merged as
`b91e8301899351c0411d6e2e9ac5290af8a3cb4c`.

If this summary conflicts with a normative project design document, the project
design document wins and this file must be reconciled.

## Intended outcome

Telex Watcher is a separately supervised, per-user, headless application that
runs trusted local observational commands outside agent sessions and emits
durable Telex messages:

```text
external condition
      -> trusted agent-authored detector
      -> Telex Watcher
      -> fixed-route durable Telex send
      -> responsible agent or human-attended station
```

An agent can write or copy a small detector, optionally exercise it, register
its command plus generic execution and routing policy, and leave it running
without retaining a session-owned waiter or polling task.

## Responsibility boundary

Watcher owns:

- persistent local registration, lifecycle, opaque state, attempts, pending
  operations, recurrence evidence, diagnostics, and health;
- generic cadence, timeout, concurrency, retry, backoff, process containment,
  output, and retention bounds;
- immutable backend profile, sender, and target for one permanent watch ID;
- stable runtime identity and send-only sender membership;
- runtime allocation of event sequence, event ID, and exact send operation ID;
- durable staging, receipt reconciliation, and atomic receipt-gated state
  commit; and
- one reaction only: sending a Telex message.

The detector owns provider and repository semantics, event kind and content,
cursor and replay behavior, recurrence intent, and provider-specific gap or
downtime policy. Telex owns durable transport, receipt axes, address semantics,
and disposition. The recipient owns all consequential action after wakeup.

Watcher does not become a workflow engine, mutate providers, launch agents,
host human UX, accept remote executable registration, claim to sandbox
same-user code, provide hosted webhook ingestion, or define a private Telex
client API.

## Minimal v2 authoring contract

Ordinary registration requires only:

- a permanent watch ID;
- an argv command;
- cadence and timeout;
- an explicit backend profile;
- a stable sender; and
- a fixed target.

Attention, disposition, working directory, inherited environment names,
parameters, and initial opaque state have defaults or optional generic
semantics. Watcher executes argv directly without implied shell interpolation.
Registration and executable mutation remain local-only.

Manifests, script digests or pinning, event-kind allowlists, provider preflight,
cursor or downtime declarations, fixtures, conformance suites, provider
budgets, and template frameworks are optional project choices. They may live in
detector code, wrappers, examples, tests, or separately versioned tooling, but
they are not fields or prerequisites of the v2 runtime contract.

## Identity and receipt transaction

A watch ID is a permanent registry namespace. Removed IDs are tombstoned and
never reused. Within that namespace, Watcher allocates a strictly increasing,
never-reused event sequence, a runtime-generated event ID bound one-to-one to
that sequence, and a restart-stable Application Client operation ID for the
exact send.

Before sending, Watcher durably stages the complete operation: identities,
registration revision, logical-store binding, fixed route and policy, prior and
proposed state, normalized payload, canonical hashes, and bounded recovery
budgets. Retry never changes the staged identity, route, policy, payload, or
state transition.

Event-producing state advances only after typed durable Telex acceptance. It
does not advance from occupancy, push, transport consumption, workflow
disposition, timeout, or an assumed outcome. An indeterminate send blocks new
detector execution while Watcher reconciles the exact staged operation. Watcher
never blindly resends, forces a local commit, or discards unresolved evidence
while keeping the watch active.

After a successful commit, a later detector event is a new recurrence with a
new sequence and event ID even when its content is identical. Watcher exposes
recurrence and duplicate evidence but does not infer provider intent or suppress
events by content.

## Version and migration boundary

The canonical v2 machine contracts are:

- [registration v2](../../../../docs/design/schemas/watcher-registration-v2.schema.json);
- [detector request v2](../../../../docs/design/schemas/watcher-detector-request-v2.schema.json);
- [detector result v2](../../../../docs/design/schemas/watcher-detector-result-v2.schema.json).

The v1 schemas and spike implementation remain frozen historical evidence. A v2
runtime does not silently execute a v1 registration or accept a v1 result.
Migration requires a new v2 watch ID, a detector update or wrapper, an explicit
opaque-state decision, and preservation of the old tombstone. No event-ID,
cursor, or script-provenance translation is implied.

## Application Client seam

Production Watcher consumes the shared Application Client and has no private
fallback. The client contract currently does not state the authoritative
exact-store, exact-operation `not-recorded` result required to prove that a
staged operation was not durably accepted. Watcher's contract also requires
identity-checkable retry of the exact same operation after authoritative
non-acceptance.

This is a controlled cross-workstream design gap. The Application Client owner
must promote the missing semantic into its normative contract and prove it
through `client-conformance`. Until then, Watcher recovery is query-only when
acceptance is uncertain, and production runtime work remains blocked rather
than parsing CLI output, using raw daemon IPC, or inventing a Watcher-private
client.

## Acceptance and dependency boundaries

Merging the v2 contract does not accept `dumb-watcher-contract-gate` or the
`minimal-contract-accepted` checkpoint. The builder must still establish that
the shortest authoring path is useful and free of mandatory framework ceremony.

The minimal example pack remains blocked on that gate. Runtime work remains
blocked on both that gate and the Application Client `client-conformance`
export. No downstream node may infer launch readiness from the design promotion
alone.

## Superseded assumptions

ADR 0050 supersedes ADR 0046 only for mandatory script provenance and pinning,
allowed-kind policy, registration-owned downtime declarations, required
template framework or conformance, and detector-authored event identity. ADR
0046 remains binding for provider-neutral trusted-local execution, local-only
registration, fixed routing, structured results, generic bounds and
diagnostics, no workflow actions, durable acceptance before state commit,
visible at-least-once duplicates, and the shared Application Client boundary.

Issue #127 and unmerged PR #131 remain optional-example and implementation
learning, not current product authority.

## Unresolved design questions

- Whether an explicit v1 compatibility adapter is worth maintaining instead of
  documented re-registration.
- Which bounded examples or hardening techniques from PR #131 should be
  extracted after the minimal-authoring gate passes.
- The exact Application Client promotion that adds authoritative
  `not-recorded` and corresponding conformance evidence without weakening
  AC-C14 or regenerating the historical convergence bundle.
