# Operator Station: Direct Human-Attended Telex Endpoint

## Status and authority

This document is the normative Operator Station product contract for
[issue #134](https://github.com/lossyrob/telex/issues/134).

The load-bearing product boundary is
[ADR 0051](DECISIONS.md#0051--operator-station-ships-a-direct-human-attended-endpoint-mediation-remains-external).
ADR 0051 narrows the applicable parts of
[ADR 0047](DECISIONS.md#0047--operator-station-mediation-remains-application-logic-outside-telex-core)
and supersedes the Operator Station topology in
[ADR 0048](DECISIONS.md#0048--direct-and-assisted-routing-use-exclusive-ingress-attendance).

The shared
[Application Client contract](application-client.md) is the supported semantic
boundary between Operator Station and Telex. This document defines product
behavior and the Station-visible use of that contract. It does not define a
package, language binding, public socket protocol, private IPC surface, or
desktop implementation.

The terms **MUST**, **MUST NOT**, **SHOULD**, **SHOULD NOT**, and **MAY** are
normative.

## Purpose and product boundary

Operator Station is an optional, separately installable, human-facing
application that directly attends one or more explicitly configured Telex
addresses. Agents and other Telex senders address ordinary messages directly
to those addresses. The Station presents an actionable feed and threads,
publishes local notifications, sends ordinary Telex messages and replies, and
records exact-recipient dispositions.

```text
agent or application
        |
        | ordinary Telex message
        v
configured Station address  <- directly attended by Operator Station
        |
        v
human feed, notification, reply, and disposition
```

Operator Station is a control surface over Telex. It is not a workflow control
plane, semantic router, operator-agent host, command executor, or replacement
for authoritative project systems.

### Responsibilities

Telex core:

- stores and transports opaque messages;
- provides address registration, exclusive attendance, durable queueing,
  delivery identity, acknowledgment, threading, disposition, and liveness;
- preserves recipient-specific state and backend semantics;
- does not decide what deserves human attention.

The Application Client:

- provides the supported lifecycle, messaging, receipt, receive,
  acknowledgment, history, recovery, source, health, backend, and cleanup
  semantics in [application-client.md](application-client.md);
- exposes only capabilities that the Station actually holds;
- does not own Station notification policy, human UI, or product vocabulary.

Operator Station:

- owns configured-address attendance and the human feed, thread, notification,
  reply, disposition, and local-read experience;
- durably ingests each primary delivery before acknowledging it;
- preserves exact recipient identity and source provenance;
- presents health and principal evidence without overstating what is known;
- renders all message-derived content inertly;
- never interprets opaque metadata as authority to override core fields or
  Station behavior.

Deployment configuration:

- selects the logical Telex store and one or more attended addresses;
- supplies local notification, mute, history, retention, and safe-link policy;
- owns explicit address-set changes without ambiguous or competing ownership.

### Non-goals

This contract does not define or require:

- a shipped operator-agent skill, broker, policy package, or intermediary;
- a required ingress/human address pair;
- direct/assisted/quiet routing modes or topology transitions;
- semantic filtering, aggregation, recommendation, escalation, digest, or
  route-back lifecycle;
- an Operator-specific extension, message kind, metadata schema, or Telex core
  behavior;
- general chat, contacts, rooms, reactions, typing indicators, or social
  presence;
- session/process launching, stopping, supervision, workflow mutation, or
  arbitrary command execution;
- multi-device fan-out, cross-platform UI, packaging, signing, auto-start, or
  final operational-hardening evidence.

## Terms

**Configured address**
: A durable Telex responsibility that this Station is configured to attend
  bidirectionally.

**Runtime identity**
: The fresh, never-reused identity for one Station process incarnation.

**Exact delivery identity**
: The logical store, message ID, recipient address, and delivery-row identity
  that identify one recipient delivery.

**Primary delivery**
: A delivery for which the configured Station address is a primary recipient.
  It may carry a workflow disposition obligation.

**CC delivery**
: A visibility-only recipient delivery governed by core Telex CC semantics. It
  is not a workflow obligation for the CC recipient.

**Human obligation**
: A primary delivery whose disposition requirement is set for that exact
  configured recipient delivery.

**Local projection**
: Restart-safe Station-owned state used to render and recover messages,
  recipient state, local read state, notification evidence, and operation
  progress. It is not the Telex record of authority.

## Core invariants

1. Operator Station directly attends configured addresses. No intermediary is
   required for send, receive, reply, or disposition.
2. One address has at most one attending owner under the
   [lease-collision contract](DESIGN.md#lease-collision-and-takeover).
   Operator Station consumes the client's typed collision result and adds no
   shared-attendance exception.
3. Message acceptance, target occupancy, push, delivery, acknowledgment, local
   read state, notification submission, and workflow disposition remain
   separate facts.
4. A primary delivery is acknowledged only after restart-safe local ingest.
5. Reply is an ordinary Telex reply in the original source thread.
6. Reply never implicitly dispositions an obligation. Compound actions preserve
   durable ordering and visible partial state.
7. Every operation names an exact sender or recipient identity. Ambiguity fails
   closed.
8. At-least-once delivery is expected. Duplicate presentation, notification,
   reply, and disposition are prevented by stable identities.
9. Source address, authenticated principal evidence, and message content are
   distinct. The UI never presents one actor as another.
10. Message bodies, subjects, kinds, metadata, and links are untrusted input and
    cannot cause implicit execution.
11. Notification policy is local application behavior. It does not change
    transport priority, delivery, or disposition semantics.
12. Unknown metadata remains opaque and inert. It cannot override `from`, `to`,
    parent/thread identity, attention, recipient role, disposition requirement,
    or Station policy.

## Configured addresses and lifecycle

### Explicit configuration

The Station MUST be configured with:

- a backend/profile or logical store selection;
- one or more exact addresses to attend;
- a stable application responsibility and a fresh runtime identity;
- local notification, bounded history/retention, and cleanup policy.

Station semantics remain identical across supported SQLite and credentialed
Postgres backends. Backend availability and principal-provenance differences
remain visible evidence; they do not change message, acknowledgment,
disposition, reply, or recovery meaning.

Station identity, collision handling, and backend access inherit the daemon's
same-user, shared-store trust model. They do not create a stronger
cross-principal authorization boundary. Stronger principal isolation remains
outside this contract and belongs to operational hardening.

The Station MUST NOT derive a second address, operator address, or intermediary
topology from a configured address. Address names are deployment choices, not
Operator Station protocol vocabulary.

### Multi-address readiness

Attach, recovery, and detach over multiple addresses MUST follow
[AC-C03](application-client.md#ac-c03-multi-address-lifecycle-is-atomic-or-compensable).
The Station MUST either establish the requested address set atomically or show
explicit per-address results and compensation state. It MUST NOT report the
application as fully ready while only part of the configured set is attached.

Readiness, backlog, membership loss, collision, and latest error evidence MUST
remain visible per address. A summary state MAY exist, but it MUST identify
which addresses prevent full readiness.

Every feed row MUST retain the exact configured recipient address and delivery
role. A reply to a received primary message defaults to that exact attended
address as sender. New compose and any explicit sender override MUST select an
unambiguous attached sender; omission with multiple possible senders is an
error.

### Collision and reclaim

Collision and membership loss follow
[AC-C05](application-client.md#ac-c05-membership-loss-and-collision-are-typed)
and the ownership rules in [daemon.md](daemon.md).

The Station MUST:

- fail closed when another live owner or an unprovable predecessor holds an
  address;
- retain owner, lease-epoch, runtime, and liveness evidence made available by
  the client;
- wait a bounded liveness grace before an authorized explicit detach or stop of
  the exact predecessor;
- never silently force takeover;
- treat daemon `Reset` as diagnostic recovery, not membership removal;
- never present two live Station instances on one address as a valid state.

Messages queue durably while a configured address is unoccupied. An honest
unoccupied interval is preferable to competing owners.

### Address-set reconfiguration

Changing the configured address set is an explicit application operation.
Retained addresses keep their local projections and recipient state. Removed or
replaced address projections remain available until the Station performs an
explicit evidence-preserving cleanup under unambiguous application ownership.

Detaching an address either removes it from the configured set or records an
explicit durable detached state. The Station MUST surface unresolved primary
obligations and in-flight operations before either transition. It MUST NOT
detach or remove the address until each item:

- becomes terminal;
- is explicitly reassigned to another application responsibility that can act
  under the original configured address, with durable evidence; or
- is deliberately abandoned with a recorded reason and final local state.

Reassignment transfers Station-local recovery responsibility only. It preserves
the original exact-recipient, sender, and operation identities and does not move
an in-flight operation to another sender address. The receiving application
must be able to act under the original address through ordinary Application
Client ownership and collision rules. Otherwise the Station records an explicit
final local abandonment state without claiming a Telex-terminal outcome.

Retained evidence keeps the original address attribution. Retention policy MUST
NOT prune unresolved obligations or in-flight operations. Restart preserves the
durable configured/removed/detached state and the recorded outcome for every
item. Automatic recovery MUST NOT resurrect a deliberately removed or detached
address.

## Receive, durable ingest, and acknowledgment

Operator Station uses bidirectional Application Client capability. Each receive
result MUST include the complete message and opaque metadata, logical store,
message ID, exact recipient and delivery-row identity, delivery role, an
acknowledgment capability bound to that delivery, and ordering evidence needed
for restart-safe resynchronization.

For a primary delivery, the Station MUST durably store enough local projection
state to resume without losing the human obligation before acknowledging:

- logical store and message identity;
- exact recipient and delivery-row identity;
- delivery role;
- message envelope, subject, body, kind, attention, and opaque metadata;
- disposition requirement and observed recipient disposition state;
- thread/parent identity;
- receive cursor, snapshot fence, or monotonic ordering evidence.

In-memory insertion, rendering, local read state, or toast submission is not
durable ingest. Acknowledging one recipient MUST NOT consume another
recipient's delivery.

CC deliveries remain visibility-only under core Telex semantics. Current
Station support is bounded history/backfill visibility through generic history
queries; it is not a guarantee of complete live CC acquisition. The Station
MUST show the configured history bound and whether live CC observation is
supported, disabled, or incomplete for the selected client/backend.

The Station MUST NOT offer a workflow disposition for its CC recipient, present
it as a primary obligation, or infer authority from the primary recipient's
disposition requirement.

Any non-primary role, including `watcher` or an unknown future role, MUST remain
visibly classified and MUST NOT be inferred to be a human obligation or offered
a workflow disposition. The Station durably ingests it before using any
available acknowledgment capability, or fails visibly when the role is
unsupported.

At-least-once redelivery is normal. The Station dedupes by logical store and
exact recipient delivery identity. Redelivery MUST NOT create a second feed
row, toast, reply operation, or disposition operation.

## Feed, history, threads, and local read state

The feed is the authoritative human surface. Notifications are supplemental.

On startup and recovery, the Station requests:

1. every unresolved primary obligation for its configured addresses;
2. bounded recent message and delivery history;
3. bounded thread history on demand;
4. bounded CC history when supported, with explicit completeness evidence;
5. a snapshot fence or monotonic per-axis versions for subsequent deltas.

Recovery MUST NOT require full-store materialization. Snapshot, backfill, and
delta application MUST NOT regress newer message, delivery, acknowledgment,
disposition, health, or recovery state.

Each feed row shows at least:

- logical store, message, and exact delivery identity in a copyable diagnostic
  surface;
- recipient address and delivery role;
- source address;
- subject, kind, attention, and sent time;
- disposition requirement and current exact-recipient state;
- delivery/acknowledgment evidence relevant to the selected recipient;
- principal provenance when available;
- local read/unread state.

Local read/unread is a Station preference. It MUST NOT acknowledge delivery,
record a Telex disposition, or imply human approval.

The thread view MUST use ordinary Telex parent/thread relationships and show
the complete bounded source thread. It MUST expose disposition history without
merging another recipient's state. There is no separate mediated thread or
route-back thread in the product contract.

## Compose, reply, and disposition

### Direct compose

The Station MAY compose a new ordinary Telex message from any unambiguous
configured sender address. The user selects the target, subject/body, attention,
disposition requirement, and any opaque metadata supported by the general
client surface.

The Station MUST NOT generate Operator-specific kinds or metadata. It MUST NOT
claim that a send was received, read, or handled merely because durable
acceptance or target occupancy was observed.

### Ordinary reply

A reply is an ordinary Telex reply in the selected message's original source
thread. It is authored from the selected delivery's exact configured recipient
address and targets the ordinary reply recipient determined by Telex reply
semantics.

Reply attention defaults to `next-checkpoint`. The human MAY explicitly select
`interrupt` for a genuinely urgent response. A reply expected to unblock an
agent MUST require disposition from its target recipient; an informational
reply MAY deliberately omit that obligation, but the UI MUST make the choice
explicit rather than silently defaulting it away.

Reply uses the Application Client's metadata-bearing reply operation so opaque
metadata can be preserved or supplied generically. Operator Station defines no
reply schema and does not depend on implementation from closed PR #130.

The Station verifies that the durable reply receipt identifies the expected
logical store, parent/thread, sender, and recipient. A mismatch or indeterminate
result remains visible and is reconciled before retry.

The authoring state and reply result model MUST distinguish:

- accepted while the target is unoccupied and durably queued;
- rejection before acceptance, plus retryability or permanent-unresolvability
  only when named typed Application Client evidence supplies that subtype;
- pre-send non-authoritative source state;
- post-send receipt identity mismatch;
- already-terminal source obligation;
- indeterminate acceptance.

The supported Application Client currently guarantees rejection before
acceptance, not retryable/permanent rejection subtyping. An unclassified
rejection fails closed: preserve the operation identity and obligation, do not
retry automatically, and surface the missing shared semantic tracked by
[Issue #12](https://github.com/lossyrob/telex/issues/12).

The UI never presents durable acceptance or queueing as human or agent
consumption. If reply delivery is rejected or indeterminate, the selected
human obligation remains explicitly open unless the human separately chooses a
disposition.

### Exact-recipient disposition

Disposition applies only to the selected primary recipient delivery. Available
actions are:

- **Handle**;
- **Defer**;
- **Reject**;
- **Close**, when the conversation is explicitly complete.

The Station MAY offer **Reply** and **Reply & Handle** as convenience actions.
Plain Reply leaves the selected obligation unchanged.

The direct Station intentionally does not expose the core `escalate`
disposition. A future direct-mode meaning requires a separate product decision;
the retired mediation lifecycle is not restored implicitly.

### Reply & Handle ordering

`Reply & Handle` is a compound application operation:

1. mint or reuse a retry-stable operation identity;
2. persist it in restart-safe local state;
3. require authoritative source resolution and verify that the selected
   obligation is not terminal;
4. send the ordinary reply from the exact attended recipient address;
5. verify durable acceptance and expected receipt identity;
6. only then record `handled` for the selected recipient delivery.

Failure remains explicit:

| Failure | Required state |
|---|---|
| Unclassified rejection before acceptance | Obligation remains open; preserve the operation identity, do not retry automatically or record `handled`, and expose the missing typed classification |
| Typed retryable rejection before acceptance | Obligation remains open; preserve the operation identity and retry only from AC-C14 reconciliation evidence after the rejection condition changes |
| Typed permanently unresolvable target | Obligation remains open; do not retry automatically or record `handled`; require target repair, an explicit new directed message, or a separate human disposition |
| Pre-send source is `captured-only`, `mismatch`, or `unavailable` | Do not send or disposition; preserve the operation and obligation in reconciliation-pending state until authoritative resolution is restored |
| Post-send receipt identity mismatch | Do not record `handled`; show expected and actual receipt identities, preserve the obligation, and reconcile the AC-C14 operation evidence before retry |
| Source already terminal before send | Do not run `Reply & Handle`; offer an explicitly confirmed ordinary follow-up reply without changing the existing terminal state |
| Source becomes or is discovered terminal after reply acceptance | Show `reply sent / source already terminal`; preserve the existing terminal evidence and do not overwrite it with `handled` |
| Reply accepted, disposition fails | Show `reply sent / handle pending`; retry only the disposition |
| Indeterminate reply | Show partial/unknown; reconcile operation and receipt before replacement |
| Restart after reply acceptance | Recover operation state and complete pending disposition without resending |
| Disposition attempted before durable reply | Fail closed |

Disposition-only actions require no synthetic message or route-back
notification. They record the selected exact-recipient disposition directly.

## Notification policy

Notification behavior is local Station policy over ordinary message facts. It
MUST NOT reinterpret transport semantics or let metadata create privileged
behavior.

### Default matrix

| Delivery | Attention / disposition | Default human behavior |
|---|---|---|
| Primary | `interrupt`, disposition required | Toast eligible; prominent actionable feed |
| Primary | `next-checkpoint`, disposition required | Actionable feed; toast configurable |
| Primary | `background` or `fyi`, disposition required | Actionable feed and badge; no toast |
| Primary | no disposition required | Feed/history; toast configurable only by explicit local policy |
| CC | any | Feed/history visibility; no toast and no obligation by default |
| Other non-primary role | any | Feed/history visibility; no workflow obligation; no toast by default unless explicit local policy enables it |

Local policy resolves collisions in this order:

1. application or OS notifications disabled;
2. explicit address/source/thread mute;
3. user quiet schedule or observable OS focus posture;
4. the default matrix and any explicit local preference.

`interrupt` does not bypass explicit user or OS suppression. The message
remains prominent in the feed.

The Station MAY aggregate notification presentation, but MUST NOT aggregate,
merge, or implicitly disposition the underlying messages. Each message retains
its own feed identity, thread, recipient state, and notification evidence.

For each toast-eligible delivery, the Station records the resolved decision,
policy reason, submission attempt and time, observable OS result, and aggregate
identity when used. Toast submission is not proof of human delivery, reading,
or approval. Coalesced and suppressed notification counts remain observable so
later usability and pressure validation can measure the direct topology.

## Provenance and non-impersonation

The source message is the ordinary Telex record identified by logical store and
message ID. The Station presents separately:

- source address;
- configured recipient address and delivery role;
- message content and opaque metadata;
- authenticated principal and provenance when supplied by the backend/client.

An address is routing identity. A principal is separate evidence. A principal
is labeled verified only when the Application Client supplies authenticated
evidence; otherwise it is unverified or unavailable.

Source resolution remains Station-visible:

| State | Meaning | Presentation |
|---|---|---|
| `authoritative` | The selected logical store resolves the message and identity fields agree | Show the current Telex record; authoring may proceed only after the terminal-state check |
| `captured-only` | The selected store cannot reproduce the source, but a sufficient durable local projection remains | Show the projection as non-authoritative evidence; reply/disposition authoring is reconciliation-pending and MUST NOT proceed |
| `mismatch` | A same-number record resolves but store, sender, recipient, or thread identity differs | Show both identities with a warning; reply/disposition authoring is refused pending explicit source repair |
| `unavailable` | Neither an authoritative record nor a sufficient captured projection exists | Show unavailable, do not guess or substitute a source, and refuse reply/disposition authoring |

Every reply or disposition rechecks source resolution and selected-obligation
terminal state immediately before authoring. A later durable receipt that does
not match the expected store, parent/thread, sender, or recipient is a distinct
post-send receipt mismatch and enters AC-C14 reconciliation; it is not treated
as pre-send source authorization.

The Station MUST NOT:

- style a message as authored by a different address or principal;
- treat prose, metadata, display names, or link labels as authenticated
  identity;
- claim human presence, reading, or approval from Station attendance,
  notification submission, or local read state;
- open a same-number message from another logical store as the source.

## Health and observability

The UI MAY summarize health, but MUST retain evidence and MUST NOT collapse
membership, receive readiness, backlog, notification posture, or human
availability into one unqualified `online` state.

| Axis | Required states or evidence |
|---|---|
| Application lifecycle | configured, attaching, ready, partially ready, recovering, deliberately detached, stopped |
| Per-address membership | owner/runtime, lease epoch, capability, collision or loss reason, latest error |
| Receive path | healthy, recovering, degraded, attended but deaf, stopped, unknown; client/daemon evidence and latest transition |
| Delivery/ack | pending unconsumed count, ack-pending count, oldest age, stalled evidence |
| Actionable backlog | unresolved primary count and oldest age per address |
| Resynchronization | current fence/version, gap or mismatch, resync progress and result |
| Notification posture | enabled, locally suppressed, OS-suppressed when observable, unknown, failed |
| Principal provenance | verified, unverified, unavailable, with evidence source |

Occupancy alone is never healthy receive status. Human availability is unknown
unless a separate explicit local signal exists.

Receive-state precedence is:

1. `stopped` when membership is deliberately detached or receive capability is
   absent;
2. `attended but deaf` when membership remains but no healthy receive path is
   armed past the configured deaf threshold;
3. `recovering` while typed reconnect, reattach, or resynchronization work is in
   progress;
4. `degraded` for recent receive failures, gaps, or stalled actionable backlog
   before the deaf threshold;
5. `healthy` only when membership and receive evidence are current and no
   stalled actionable backlog exists;
6. `unknown` when required evidence is unavailable.

The Application Client/daemon supplies membership, push/wait, backlog, gap, and
latest-error evidence. Station configuration owns threshold values; operational
hardening validates and tunes them.

`stalled actionable backlog` is a configured predicate over unresolved primary
count and oldest actionable age. The health surface MUST report the predicate's
threshold and current evidence. Threshold selection and validation remain
downstream, but the resulting state is computable and testable.

## Restart, recovery, and operation reconciliation

On restart, the Station:

1. restores its durable local projection and in-flight operation identities;
2. explicitly reattaches configured addresses that are not in a durable
   detached state;
3. reconciles partial membership and collision state;
4. requests unresolved primary obligations and bounded recent history;
5. resumes from a durable cursor/fence or performs explicit resynchronization;
6. dedupes redelivery by exact recipient identity;
7. reconciles accepted, duplicate, rejected, partial, or indeterminate authored
   operations before retry;
8. suppresses duplicate startup notifications.

Recovery MUST NOT depend on a prior process transcript, CLI output parsing, raw
daemon IPC, spike helpers, or a product-private client.

## Inert rendering, links, and safe actions

Subjects, bodies, kinds, metadata, addresses, principal labels, and
notification text are untrusted.

Every human-visible surface MUST:

- render message-derived text inertly;
- escape markup for the active renderer;
- remove or visibly encode terminal/control sequences and unsafe bidirectional
  controls;
- avoid inserting untrusted content into an executable HTML/markup path;
- preserve raw bytes only in a separately labeled inert inspection view.

Telex message and thread actions are internal Station navigation. `https` links
may open only after explicit user action and MUST display the destination.
`http`, `file`, custom schemes, and local process actions are disabled by
default. A local allowlist MAY enable a bounded action only after explicit
per-invocation confirmation that displays the fully resolved target. The
allowlist MUST identify the scheme/action and target constraint. Message-derived
values MAY fill only an explicitly constrained parameter and MUST NOT select a
new action or be treated as executable instructions.
Link labels MUST NOT hide a different destination.

Messages and metadata are never executed as commands or agent instructions.
Operator Station may send Telex messages and dispositions; it does not directly
merge a PR, stop a session, mutate a workflow, or run a source-provided command.

## Local projection discovery and cleanup

The Station provides bounded discovery of its local projections by logical
store, configured address, and application responsibility. It identifies
projections whose store or address configuration was removed or replaced.

Cleanup MUST be explicit, scoped, and evidence-preserving. It MUST NOT delete:

- Telex messages, deliveries, acknowledgments, or dispositions;
- another application's membership or local projection;
- a projection whose ownership is ambiguous.

Cleanup failure is visible and retryable. Removing a local projection does not
change the Telex record.

## Legacy Operator requirement disposition

The `AC-01` through `AC-15` identifiers originated in the issue #114 Operator
contract and remain useful traceability aliases. The normative shared client
semantics are now `AC-C01` through `AC-C20` in
[application-client.md](application-client.md). The historical
[requirements crosswalk](../notes/application-client/requirements-crosswalk.md)
remains `application-client-ready` provenance and is not the current Station
product contract.

| Legacy ID | Direct Station disposition | Current producer / consumer |
|---|---|---|
| AC-01 | Keep: stable responsibility, fresh runtime, explicit attach/detach/recovery | Application Client lifecycle / Station configured-address lifecycle |
| AC-02 | Keep: opaque logical store identity | Application Client identity / Station feed, recovery, and provenance |
| AC-03 | Keep: atomic or compensable multi-address lifecycle | Application Client / Station readiness and reconfiguration |
| AC-04 | Keep: exact-delivery receive with opaque metadata and bound acknowledgment | Application Client receive / Station durable ingest |
| AC-05 | Keep: acknowledge only after durable ingest and expose backlog/deaf state | Station ingest / Application Client acknowledgment and health |
| AC-06 | Keep: per-recipient dedupe and no-regression resynchronization | Application Client ordering / Station recovery |
| AC-07 | Keep: unresolved obligations plus bounded recent/thread history | Application Client query / Station startup and thread view |
| AC-08 | Keep: typed send, metadata-bearing reply, thread read, exact-recipient disposition | Application Client operations / Station compose, reply, and disposition |
| AC-09 | Keep: retry-stable operation identity and reconciliation | Application Client operation results / Station compound-action recovery |
| AC-10 | Narrow: keep generic ordered compound operations; retire Station mediation notification and route-back requirements | Application Client generic AC-C20 / Station `Reply & Handle`; no current Station route-back consumer |
| AC-11 | Keep: store-scoped source identity and explicit resolution state | Application Client source identity / Station provenance |
| AC-12 | Keep: evidence-bearing lifecycle and health projection | Application Client health / Station observability |
| AC-13 | Keep: backend-neutral semantics and authenticated-principal provenance | Application Client backend selection / Station configuration and identity display |
| AC-14 | Keep: ordered delta, gap, and resync behavior | Application Client deltas / Station feed and recovery |
| AC-15 | Keep: receipt cross-checks, bounded retry, local discovery, and cleanup | Application Client / Station operation safety and local projection maintenance |

Mediation-only rationales, kinds, source cards, human-response obligations,
route-back outcomes, and operator replacement behavior from the earlier
contract are historical and have no current Station producer or consumer.
Generic Application Client compound primitives remain available to other
applications without becoming Operator Station requirements.

The negative assisted/quiet/mediation vocabulary in the Application Client
product-boundary list is historical provenance. Its downstream Operator
integration bullet that names route-back ordering must be re-baselined by the
shared client owner; ADR 0051 and this document govern current Station
requirements. This node does not edit or weaken generic AC-C20 semantics.

If implementation discovers that the accepted Application Client contract
cannot express a required direct Station semantic, Station implementation is
blocked on that shared owner. A private client, CLI parsing, raw IPC, spike
helper, or closed PR #130 implementation is not a supported fallback.

## External mediation

Users MAY build external mediation applications with ordinary Telex addresses,
messages, replies, dispositions, and opaque metadata. Such applications are
independent Telex participants, not part of Operator Station.

External mediation:

- is optional and user-developed;
- chooses its own addresses, namespace, policy, and lifecycle;
- cannot override core message fields or Station behavior;
- cannot require Station to interpret its metadata or route outcomes;
- must preserve source provenance and avoid impersonation if it republishes or
  summarizes messages.

The historical `urn:telex:operator-station:v1` extension and
`operator-station.*` kinds are retired and reserved. They remain historical
identifiers and are not available for reuse by Station, Telex, or an external
mediation convention.

## Downstream obligations and deferrals

### `direct-station-direction-gate`

The builder evaluates whether this direct product boundary is accepted. This
document supplies the contract; it does not pass or simulate the gate.

### Application Client `client-conformance`

Before production integration, the supported client must conform the lifecycle,
exact-delivery acknowledgment, history, reply, disposition, retry, source,
health, backend, resync, compound-operation, and cleanup semantics consumed
here. Operator Station MUST NOT introduce a private production integration
while conformance is incomplete.

### `station-app`

`station-app` implements:

- configured multi-address attendance and per-address readiness;
- actionable feed, bounded history, thread reading, and local read state;
- direct compose, ordinary reply, exact-recipient disposition, and ordered
  `Reply & Handle`;
- local notification decisions and evidence;
- source/principal presentation and non-impersonation;
- receive, backlog, collision, restart, and resynchronization health;
- inert rendering, safe links, and explicit safe actions;
- evidence-preserving local projection discovery and cleanup.

It does not implement an operator agent, mediation schema, route-back lifecycle,
or Operator-specific Telex core behavior.

### Usability validation

The builder validates direct agent-to-Station send, notification usefulness,
thread/reply/disposition clarity, exact source identity, health legibility,
restart continuity, and product optionality.

### Operational hardening

Later work validates credentialed Postgres, remote principals, duplicate and
delayed delivery, offline/unoccupied periods, collision and recovery,
notification pressure and OS suppression, security, packaging, install/upgrade,
auto-start, signing, diagnostics, and cleanup.

### Direct-topology carry-forward

| Item | Owner |
|---|---|
| Complete live CC acquisition, if required beyond bounded history | Application Client design/conformance decision |
| Re-baseline stale Station product-boundary, readiness, integration, and route-back references in `application-client.md` | [Issue #12](https://github.com/lossyrob/telex/issues/12) / Application Client workstream |
| Define named typed evidence for retryable versus permanently unresolvable rejection if Station needs that distinction | [Issue #12](https://github.com/lossyrob/telex/issues/12) / Application Client workstream |
| Receive-health threshold values and latency tuning | `station-app` and operational hardening |
| Optimistic display of an accepted reply | `station-app`; must preserve durable receipt and partial state |
| Notification pressure limits and coalescing policy | Usability validation and operational hardening |
| Restart artifact and in-flight operation evidence | `station-app` validation |

## Revisit conditions

Revisit this contract if:

- Telex accepts non-exclusive or multi-device address attendance;
- Application Client conformance cannot satisfy a required direct Station
  semantic;
- production evidence shows exact-recipient reply/disposition cannot remain
  clear across multiple attended addresses;
- backend principal evidence cannot be presented without overstating trust;
- the daemon's same-user/shared-store trust model is insufficient for required
  cross-principal isolation;
- notification pressure requires a different default matrix;
- safe-link or rendering requirements need a shared security contract;
- an external mediation convention demonstrates a broadly useful capability,
  in which case it still requires a separate product and design decision rather
  than silent import into Station or core.
