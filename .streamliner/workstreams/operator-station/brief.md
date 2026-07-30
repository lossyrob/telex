# Operator Station (direct human-attended Telex endpoint)

## Purpose

Create an optional Windows-first desktop Station that attends one or more
durable Telex addresses so agents can send meaningful operational messages
directly to a human. The Station provides an actionable inbox, threads,
notifications, ordinary replies, per-recipient dispositions, and trustworthy
health/recovery without turning Telex into chat, workflow automation, or a
semantic router.

## Approach

The workstream preserves the original spike and mediated-loop implementation as
evidence that a human-attended Telex surface is valuable. It no longer treats
the operator-agent intermediary as product architecture.

The next confidence transition is a design-only reset:
`direct-station-contract-reset` rewrites the intended Station contract around
direct address attendance and records ADR 0051. A builder direction gate then
accepts the product boundary before implementation.

After the direction gate, `station-app` implements the desktop endpoint using
the supported/conformant Application Client. A direct usability gate validates
the human experience, followed by operational hardening for Postgres,
restart/offline behavior, notification pressure, provenance, security,
packaging, and cleanup.

Mediation remains possible as an external user-developed convention over
ordinary Telex messages. It is not shipped, interpreted, or required by Telex
or Operator Station.

## Design References

- `telex:docs/design/index.md` - Telex intended-system design entry point.
- `telex:PRODUCT-THESIS.md` - durable responsibilities, store-and-forward,
  auditability, and the non-chat boundary.
- `telex:docs/design/daemon.md` - address attendance, delivery, acknowledgment,
  disposition, liveness, and backend semantics.
- `telex:docs/design/application-client.md` - supported long-lived application
  lifecycle, receive/reply/disposition, identity, health, and recovery.
- `telex:docs/design/operator-station.md` - current contract, to be reset by
  issue #134.
- `telex:docs/design/DECISIONS.md` - ADRs 0047/0048 and allocated ADR 0051.
- `telex:docs/design/proposals/EXTENSIONS.md` - opaque extension boundary;
  message metadata does not become Telex semantic routing.
- `telex:telex-console/README.md` - existing feed, address, thread, delivery,
  and disposition presentation concepts.

## Boundaries

- **In scope:** separately installable Windows desktop Station; explicitly
  configured attended addresses; durable ingest-before-ack; actionable feed and
  bounded history; thread reading; ordinary reply; exact-recipient disposition;
  attention-aware local notifications; backend selection; source/principal
  presentation; health and backlog; restart/resync; inert rendering; safe links;
  local read state; evidence-preserving local cleanup; SQLite and credentialed
  Postgres.
- **Out of scope:** shipped operator-agent skill or policy; semantic filtering,
  recommendation, aggregation, digest, route-back, or intermediary topology;
  general chat, contacts, rooms, reactions, typing indicators, or social
  presence; process/session supervision; arbitrary command execution; a generic
  router/alias engine; replacing `telex-console`; making Station mandatory.
- **Deferred:** multi-device fan-out; macOS/Linux/mobile clients; structured
  decision widgets; cryptographic cross-principal identity beyond backend
  provenance; rich session-opening or terminal-control integration.
- **External convention:** users may build mediation agents with ordinary Telex
  messages and opaque metadata, but Station and Telex do not ship or interpret
  that convention.

## Current State

The workstream belongs to the
[Addressable Attention campaign #102](https://github.com/lossyrob/telex/issues/102)
and parent [#92](https://github.com/lossyrob/telex/issues/92).

Historical confidence:

- [#93](https://github.com/lossyrob/telex/issues/93) /
  [PR #104](https://github.com/lossyrob/telex/pull/104) proved a Windows
  human-attended feed, notification, durable reply path, provenance, restart,
  and delivery/ack health. Its mediated topology is evidence, not current
  product authority.
- The builder passed the viability gate: a human-facing Telex inbox materially
  reduced terminal inspection and preserved useful reply context.
- [#114](https://github.com/lossyrob/telex/issues/114) /
  [PR #116](https://github.com/lossyrob/telex/pull/116) landed the first
  production contract and ADRs 0047/0048. Direct Station findings remain input;
  mediation-specific product requirements are being narrowed by ADR 0051.
- Application Client contract convergence is complete through
  [#118](https://github.com/lossyrob/telex/issues/118) /
  [PR #126](https://github.com/lossyrob/telex/pull/126). The
  `application-client-ready` gate is complete, client-core
  [#129](https://github.com/lossyrob/telex/issues/129) is active, and
  `station-app` waits on Application Client `client-conformance`.

Superseded scope:

- [#128](https://github.com/lossyrob/telex/issues/128) is closed
  `NOT_PLANNED`.
- [PR #130](https://github.com/lossyrob/telex/pull/130) is closed without
  merge at `961e51e5a7d8da4a4867b2ae01efe75af47476b3`.
- The implementation reached green CI and reviewer +1; closure is a product
  direction decision, not an implementation-defect finding.
- Branch, review history, and worktrees remain preserved pending cleanup
  authorization.
- Metadata-bearing reply is not extracted from #130. Its generic requirement is
  handed to Application Client client-core/conformance.

Active transition:

- Campaign authorization message `1292` approves the direct-Station reshape.
- ADR allocation message `1304` reserves ADR 0051,
  `direct-station-product-boundary`.
- [#134](https://github.com/lossyrob/telex/issues/134)
  `direct-station-contract-reset` is in progress.
- Initial preparation run `cda96dff-f2b8-49cc-b258-350b901e29ff` caught stale
  model resolution before terminal launch. Streamliner repaired paw-init to
  resolve the live model catalog and reject mismatched WorkflowContext fields.
- Proven preparation run `c7d3f616-466d-4bb2-a3b4-16f1d975f025` validated
  current base `53afb88`, `never-commit`, local/untracked `.paw`, no workflow
  commit, `["--yolo"]`, `gpt-5.6-sol`, and
  `general-reviewer:claude-opus-5`.
- Paired launch run `419ba98f-6c1e-4829-8255-7e4de25a2a70` started:
  - implementer `9b7f8a6c-79f1-4967-afad-37b8dfbad5df` /
    `telex://lossyrob/telex/T-A:operator-impl-134`;
  - reviewer `e32f377b-49f6-4d23-a09c-96b107f32075` /
    `telex://lossyrob/telex/T-A:operator-review-134`.
- Both stations are `attended_push`; both sessions are live/working and
  graph-bound. Scope/ADR control messages are durably queued for their current
  turns.
- The implementer opened
  [PR #136](https://github.com/lossyrob/telex/pull/136) at
  `59043ce5bf6b3fdd2bb7084777d15141929fbf41`.
- The diff contains only `docs/design/DECISIONS.md`,
  `docs/design/index.md`, and `docs/design/operator-station.md`. All six CI
  checks pass and GitHub reports the PR mergeable.
- Paired review is active. The builder-owned direction gate remains separate
  and cannot be passed by the worker or reviewer.
- [Review 4824128333](https://github.com/lossyrob/telex/pull/136#pullrequestreview-4824128333)
  posted three blocking contract gaps and five additional clarifications. The
  blockers require:
  1. a closed state machine for detach/address removal with unresolved work;
  2. exhaustive post-conditions for every declared Reply & Handle result; and
  3. a default notification policy for all non-primary delivery roles.
- These are node-scoped design-completeness repairs. They do not reopen the
  direct product boundary, ADR 0051 allocation, or shared Application Client
  ownership.
- `station-app` remains planned and cannot launch until:
  1. `direct-station-direction-gate` passes; and
  2. Application Client `client-conformance` completes.

## Decisions

- **Direct Station is the product:** agents send ordinary Telex messages directly
  to configured human-attended Station addresses.
- **Telex remains semantically dumb:** core transports opaque messages and
  lifecycle facts; it does not decide what deserves human attention.
- **No first-party mediation product:** Telex and Station ship no operator-agent
  skill, semantic filter, required intermediary addresses, or route-back
  lifecycle.
- **External mediation remains possible:** users may create optional conventions
  outside the product; unknown metadata stays opaque and cannot override core
  fields or Station behavior.
- **The desktop is a Station, not a protocol actor:** it consumes existing Telex
  and Application Client semantics.
- **Reply is ordinary Telex reply:** direct human responses stay in the original
  thread; no product route-back intermediary is required.
- **Local notification policy is application behavior:** Station maps attention,
  disposition requirement, configured address, and local OS posture without
  changing transport semantics.
- **PR #130 remains closed evidence:** no extraction or merge occurs now.
- **Generic reply metadata belongs to Application Client:** client-core decides
  and conforms any supported implementation.
- **Decision history is append-only:** ADR 0051 narrows/supersedes applicable
  portions of ADRs 0047/0048 without rewriting them.
- **Design gate before implementation:** #134 must land and the builder must pass
  `direct-station-direction-gate` before `station-app`.
- **Station uses the conformant client:** `station-app` depends on Application
  Client `client-conformance`, not only semantic acceptance or first binding.
- **Node launches use Streamliner:** launch-preparations API, configured v2
  defaults, fetched CLI args including `--yolo`, `never-commit`, dynamic
  latest-family model resolution, and preparation validation.

## Open Questions

- None for workstream execution. The builder-owned direction gate remains the
  authority for accepting or rejecting the #134 design output.

## Imports and Exports

### Imports

- Telex local-exchange delivery, acknowledgment, disposition, liveness, and
  backend contracts.
- Application Client `client-conformance` before desktop implementation.
- Historical spike, dogfood, and contract evidence without mediated topology
  authority.
- Streamliner Desktop and `telex-console` as UI/reference material only.

### Exports

- An accepted direct Station product contract and ADR 0051.
- A separately installable human-attended desktop endpoint.
- Direct agent-to-human durable messaging and reply UX.
- Station-specific notification, provenance, health, recovery, and safety
  evidence.
- Non-normative lessons for users who independently build mediation conventions.
- Operational evidence for future portfolio-level attention surfaces.

## Closeout Observations

Parking lot for bounded desktop polish, notification tuning, message rendering,
and safe-link improvements discovered during direct Station dogfood. Anything
that changes Telex semantics, Application Client contracts, identity guarantees,
or routing architecture must be promoted into its owning workstream or a new
design decision.

- Reply/disposition clarity remains a `station-app` UX concern, but direct
  operation no longer requires operator notification or route-back sequencing.
- Reviewer/implementer feedback from #128 is captured in
  `reconciliation-note.md`: settle product-boundary questions before launching
  policy/topology implementation nodes.
