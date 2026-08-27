# Operator Station Current Design

## Status and authority

This file is the canonical integrated design for the `operator-station`
workstream. It summarizes accepted repository authority; it does not replace the
normative documents.

Authority is ordered as follows:

1. [`docs/design/operator-station.md`](../../../../docs/design/operator-station.md)
   is the normative product contract.
2. [ADR 0051](../../../../docs/design/DECISIONS.md#0051--operator-station-ships-a-direct-human-attended-endpoint-mediation-remains-external)
   defines the direct Station boundary, narrows ADR 0047, and supersedes ADR
   0048's Operator Station topology.
3. [`docs/design/application-client.md`](../../../../docs/design/application-client.md)
   is the shared, API-neutral client contract that Station consumes.
4. [`docs/design/daemon.md`](../../../../docs/design/daemon.md) governs durable
   transport, attendance, delivery, acknowledgement, disposition, and backend
   behavior beneath the client.
5. The [workstream brief](../brief.md) and
   [campaign roadmap](../../../shaping/roadmap.md) define workstream scope,
   dependencies, and gates without overriding product design.

PR [#136](https://github.com/lossyrob/telex/pull/136) promoted the direct
contract from exact reviewed head
[`f77be107f602cfd06495e591857b20ce199c8a4d`](https://github.com/lossyrob/telex/commit/f77be107f602cfd06495e591857b20ce199c8a4d).
It merged to `main` as
[`e071e3170c19ab1b8a753b502c67be2ee80688ec`](https://github.com/lossyrob/telex/commit/e071e3170c19ab1b8a753b502c67be2ee80688ec).

If this summary conflicts with a normative project design document, the project
design document wins and this file must be reconciled.

## Intended outcome

Operator Station is an optional, separately installable, Windows-first desktop
application that directly attends one or more explicitly configured Telex
addresses. Agents and applications send ordinary Telex messages to those
addresses. The Station gives a human an actionable feed, threads, local
notifications, ordinary compose and reply, and exact-recipient disposition.

The Station is a control surface over existing Telex and Application Client
semantics. It does not make Telex a chat system, workflow control plane,
semantic router, command executor, or required human-attention product.

## Responsibility boundary

Telex core owns durable opaque transport, exclusive attendance, delivery rows,
acknowledgement, threading, disposition, liveness, and backend authority.

The Application Client owns generic application lifecycle, capability,
multi-address attach and recovery, exact-delivery receive and acknowledgement,
history, source resolution, retry-stable operations, compound ordering, health,
resynchronization, cleanup, and backend-neutral behavior.

Operator Station owns presentation and local human workflow policy:

- configured multi-address attendance and per-address readiness;
- actionable feed, bounded history, threads, and local read state;
- ordinary compose and reply from an exact attended address;
- exact-recipient disposition and ordered `Reply & Handle`;
- local notification decisions and retained notification evidence;
- source and principal presentation without impersonation;
- health, backlog, collision, restart, and resynchronization presentation;
- inert rendering, safe links, and explicit safe actions; and
- evidence-preserving discovery and cleanup of Station-owned local projections.

Station must use the supported Application Client. It does not parse CLI output,
use raw private daemon IPC, revive spike helpers, or create a Station-private
client when a shared semantic is missing.

## Direct attendance and human workflow

Station attends only explicitly configured addresses. It derives no operator
address, intermediary address, or routing topology from them. Multi-address
attach, recovery, detach, and compensation preserve per-address outcomes and
must not report full readiness while any required address is not ready.

Every received item retains its logical store, message, exact recipient,
delivery row, and recipient role. A primary delivery is acknowledged only after
restart-safe local ingest. Acknowledgement, local read state, notification,
human handling, and workflow disposition remain separate evidence.

Reply is an ordinary Telex reply in the source thread. New compose and reply use
an unambiguous attended sender. Disposition changes only the selected primary
recipient delivery. `Reply & Handle` durably orders accepted reply before
`handled`; partial, rejected, mismatched, and indeterminate results remain
visible and reconcile without duplicate authoring.

The feed is the authoritative human surface. Notifications supplement it under
configurable local and operating-system policy. By default, primary interrupt
obligations are toast-eligible, lower-attention obligations remain visible
without forcing a toast, and CC or other non-primary roles create no human
workflow obligation. Suppression, aggregation, submission, and observable OS
outcomes remain evidence rather than proof of human receipt or approval.

## Provenance, safety, health, and recovery

Address, principal, message content, and metadata are distinct. Station labels
principal evidence as verified only when the shared client supplies
authenticated provenance. Unknown metadata remains opaque and cannot override
core fields, identity, recipient state, or Station policy.

All message-derived content is untrusted and renders inertly. External links
require explicit user action and show their resolved destination. Message
content and metadata never select or execute commands, workflow actions, or
privileged local behavior.

Health keeps application lifecycle, per-address membership, receive readiness,
delivery and acknowledgement backlog, unresolved human obligations,
resynchronization, notification posture, and principal provenance separate.
Occupancy alone is not healthy receive evidence, and Station attendance does
not prove human availability.

Restart restores the durable local projection and in-flight operation
identities, reattaches only configured addresses that are not deliberately
detached, recovers unresolved obligations plus bounded history, reconciles
ordered deltas and authored operations, deduplicates exact deliveries, and
suppresses duplicate startup notifications.

## Mediation boundary

Telex and Operator Station ship no operator-agent skill, semantic filter or
router, required intermediary address pair, direct/assisted/quiet topology,
routed-outcome lifecycle, or Operator-specific extension and client semantic.
The retired `urn:telex:operator-station:v1` namespace remains reserved
historical evidence.

Users may build mediation as an external convention over ordinary Telex
messages, addresses, replies, dispositions, and opaque metadata. Such software
is an independent participant. It cannot override Station behavior, impersonate
a source, or become an undeclared Station dependency.

## Dependency and gate boundary

Merged product design does not pass the builder-owned
`direct-station-direction-gate`. The gate must accept the direct product
boundary, ADR 0051 supersession, external-only mediation, shared-client
dependency, and downstream work geometry before implementation starts.

`station-app` is unlaunched. It waits for both the direction gate and Application
Client `client-conformance`. The conformance dependency must prove the shared
lifecycle, exact-delivery acknowledgement, history, reply, disposition,
retryability, source, health, backend, resynchronization, compound-operation,
and cleanup semantics against the supported client. A merged contract or first
binding alone does not satisfy that dependency.

The later direct usability gate remains separate from the direction gate. It
must validate direct send, notification usefulness, thread/reply/disposition
clarity, source identity, health legibility, restart continuity, and product
optionality before operational hardening proceeds.

## Historical evidence and non-promoted mechanisms

The issue #93 spike proved that a Windows human-attended feed, notification,
durable reply path, provenance, restart continuity, and delivery/acknowledgement
health are worth productionizing. Its mediated topology remains historical
evidence rather than current architecture.

The
[Postgres dogfood report](../docs/postgres-dogfood-evidence.md) preserves
code-backed UI and operational lessons from PR #143. Its useful criteria include
separate local read state, bounded responsive thread navigation,
receipt-gated optimistic presentation, configurable evidence-bearing
notifications, live receive before expensive backfill, and honest health.
Those observations lack the runtime proof required for production claims.

Historical evidence does not restore:

- mediated operator-agent topology or experimental message kinds;
- CLI process invocation, stdout parsing, stderr substring matching, or a
  child-process waiter;
- backend-profile hashing as logical-store identity;
- synthetic occupancy or health; or
- a fixed recent-inbox bound as complete unresolved recovery.

## Downstream choices and confidence

`station-app` and operational hardening still own Windows production packaging,
installation, update behavior, signing, auto-start, diagnostics, and
evidence-preserving cleanup. They also own implementation UX such as concrete
feed layout, thread navigation, notification controls, optimistic action
presentation, accessibility, and health detail.

Those implementation choices must remain within accepted product authority.
They do not select a new public language binding, API, process boundary,
identity or trust model, routing topology, notification semantic, or workflow
authority. Any such expansion requires promotion through its owning design and
decision process.

- **High confidence:** direct human-attended multi-address Station, ordinary
  Telex messaging and reply, exact-delivery acknowledgement after durable
  ingest, exact-recipient disposition, local notification policy, provenance,
  health, recovery, safety, and external-only mediation are merged authority.
- **Blocked before implementation:** the builder direction gate and Application
  Client `client-conformance`.
- **Not yet proven:** production Station integration, direct usability,
  credentialed Postgres operation, notification pressure, restart/offline
  behavior, packaging, update, signing, auto-start, and cleanup.
