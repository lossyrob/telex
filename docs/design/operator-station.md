# Operator Station and Mediated Attention

## Status

Accepted Operator Station domain contract for
[issue #114](https://github.com/lossyrob/telex/issues/114).

This document is normative for the Operator Station product, the reusable
operator-agent role, and their use of existing Telex semantics. The shared
Application Client needed to implement this contract remains owned by
[issue #12](https://github.com/lossyrob/telex/issues/12). The client requirements
in this document are semantic requirements, not an API, package, binding, wire
format, or implementation choice.

The extension envelope defined here uses Telex's existing opaque `kind` and
`metadata` fields. General extension advertisement, descriptor discovery, and
packaging remain forward-looking work in
[proposals/EXTENSIONS.md](proposals/EXTENSIONS.md).

## Purpose and boundary

Operator Station is an optional, human-facing Telex application. It attends one
or more durable addresses, presents actionable messages and threads, publishes
local notifications, sends replies, and records dispositions. An operator agent
may attend a worker-facing ingress address, resolve routine matters, and
escalate selected obligations to the Station.

The arrangement preserves the core Telex model:

```text
durable address + exclusive station registration + message + disposition
```

Operator Station does not add a human transport primitive. The desktop
application is a Telex station, and the operator agent is application logic.
The existing contracts in [DESIGN.md](DESIGN.md) and
[daemon.md](daemon.md) continue to govern addresses, membership, leases,
delivery, acknowledgment, threading, dispositions, liveness, and backend
behavior.

### Responsibilities

Telex core:

- stores and routes opaque messages;
- provides exclusive address attendance, durable queueing, delivery context,
  explicit acknowledgment, and per-recipient disposition;
- exposes station, delivery, backlog, and liveness status;
- does not decide what deserves human attention.

Operator Station:

- owns the human feed, thread, notification, reply, and disposition experience;
- durably ingests a delivery before acknowledging it;
- presents transport identity, principal evidence, source provenance, and
  health without overstating what is known;
- never interprets unsupported extensions as trusted actions.

Operator agent:

- resolves, clarifies, aggregates, recommends, escalates, routes back, and
  dispositions within its assignment;
- preserves raw source provenance and never impersonates a source;
- does not become Telex core, a general router, or a workflow engine.

Deployment configuration:

- supplies the ingress and human-facing addresses;
- selects direct or assisted routing and, for assisted routing, normal or quiet
  policy;
- owns an ordered transition between occupants without dual ownership.

Application Client:

- supplies the supported long-lived application semantics listed in
  [Shared Application Client requirements](#shared-application-client-requirements);
- does not own Station UX, operator judgment, or notification policy.

### Non-goals

This contract does not define:

- the production desktop implementation or reusable operator-agent package;
- a public Application Client API;
- general chat, contacts, rooms, reactions, or typing indicators;
- session/process launching, stopping, supervision, or workflow execution;
- a generic router, alias engine, or semantic filter in Telex core;
- arbitrary command execution from message content or metadata;
- packaging, signing, auto-start, multi-device fan-out, or cross-platform UI;
- final Postgres, security, notification-pressure, or operational-hardening
  evidence.

## Terms

**Ingress address**
: The durable worker-facing responsibility address. A Station attends it in
  direct mode; an operator agent attends it in assisted mode.

**Human address**
: A distinct durable address attended by Operator Station in assisted mode.

**Raw thread**
: The source conversation between a worker or other sender and the ingress
  address.

**Mediated thread**
: The separate conversation between the operator agent and the human address.

**Human obligation**
: A message for which the Station's current address is the primary recipient
  and `requiresDisposition` is true.

**Logical store identity**
: An opaque, stable, equality-comparable identity for the selected Telex store.
  It persists across application and daemon restart and contains no raw path,
  credential, or connection string.

**Mediation ID**
: An application-generated, retry-stable identifier connecting one mediation
  episode across escalation, human reply, and route-back attempts.

**Approved source anchor**
: The workstream-approved tuple of Station-contract review message ID, source
  head, and Operator Station domain-bundle digest used to derive the issue #12
  requirements export.

## Core invariants

1. One address has at most one attending owner. Shared visibility uses distinct
   addresses or Telex recipient roles, not competing station registrations.
2. Application attendance, operator-agent attendance, notification submission,
   and human availability are separate facts.
3. Delivery, acknowledgment, local read state, and workflow disposition are
   separate state axes.
4. The operator agent authors mediated messages from its own address and
   preserves source references. It never sends as the worker.
5. Raw and mediated threads remain distinct. The human reply stays in the
   mediated thread; the routed outcome stays in the raw thread.
6. A delivery is acknowledged only after restart-safe application ingest.
7. At-least-once delivery is expected. Every receive and authoring path dedupes
   or retries by stable identity.
8. Replying does not silently leave the selected human obligation unresolved.
9. Telex core carries the Operator Station extension but does not interpret,
   validate, route, or execute it.
10. Messages and metadata are untrusted input. No message can cause arbitrary
    command execution.

## Address topology and routing policy

Addresses are explicit deployment configuration. This contract does not
standardize a global naming scheme or derive a human address from an ingress
address. `attention:rob` and `operator:rob` are campaign examples, not
production defaults.

In assisted mode, ingress and human addresses must be distinct.

### Direct topology

```text
worker/source
    |
    v
ingress address  <- attended exclusively by Operator Station
    |
    v
human
```

The Station presents raw messages and replies directly in the raw thread. No
operator agent mediates the path.

### Assisted topology

```text
worker/source
    |
    v
ingress address  <- attended exclusively by operator agent
    |
    | operator-station.escalation
    v
human address    <- attended exclusively by Operator Station
    |
    | operator-station.human-reply
    v
operator agent
    |
    | reply in raw thread
    v
worker/source
```

The operator agent may handle or clarify raw messages without involving the
human. Human escalation creates a new mediated thread.

### Quiet posture

The issue and workstream use "direct, assisted, and quiet modes" as an umbrella
description. Normatively:

- direct and assisted are routing topologies;
- quiet is an assisted-mode operator and notification policy;
- direct plus quiet is not a production mode.

In quiet posture the operator agent handles routine traffic, aggregates
compatible informational traffic, and sends digests. It still sends an
individual escalation for an interrupt-grade or explicit human obligation.
Quiet posture does not change address occupancy.

### Allowed configurations

| Routing | Policy | Valid | Ingress occupant | Human-address occupant |
|---|---|---:|---|---|
| direct | normal | yes | Station | not required |
| assisted | normal | yes | operator agent | Station |
| assisted | quiet | yes | operator agent | Station |
| direct | quiet | no | - | - |

Local OS quiet hours and user notification suppression are available in every
valid configuration. They are not the assisted quiet posture.

### Direct/assisted transition

Changing topology is an application-owned sequence over existing daemon
membership and lease semantics. It is not daemon upgrade handoff
([daemon.md section 11.4](daemon.md#114-ordered-handoff--owner-directed-atomic-transfer-sf3))
and does not add a router.

1. Persist the desired configuration as `transitioning`; do not report the new
   mode as active yet.
2. The old occupant stops receive activity and explicitly detaches or performs
   `station stop`.
3. Verify through station status that the old session no longer owns the
   ingress registration. Do not use operator reset as a routine transition.
4. Attach the new occupant. An ownership collision fails closed.
5. Verify receive health and drain the durable backlog.
6. Mark the new configuration active.

Messages may queue while the ingress address is unoccupied. That gap is honest
and preferable to competing owners. If the new attach fails, the deployment
either reattaches the old occupant or remains visibly transitioning with a
durable backlog. It never runs both occupants.

Assisted normal/quiet transitions update policy without changing occupancy.

## Attendance and human-visible health

An occupied address proves that a station registration exists. It does not
prove:

- that the application receive path is healthy;
- that a delivered message was durably ingested;
- that a notification was presented;
- that a human is present or has read the message.

Operator Station presents health as separate axes:

| Axis | Required states/evidence |
|---|---|
| Station receive | healthy, recovering, degraded, stopped, unknown; registration and latest receive error |
| Delivery/ack | pending count, oldest pending age, ack-pending state, stalled/deaf warning |
| Operator ingress | attended, unattended, unknown; address and latest status evidence |
| Source resolution | authoritative, captured-only, unavailable, mismatch |
| Notification posture | enabled, locally suppressed, OS-suppressed when observable, unknown, failed |

The UI may summarize these axes, but it must retain the evidence and must not
collapse them into an unqualified "online" state.

Human availability is `unknown` unless a separate explicit local signal exists.
It is never inferred from Station occupancy or notification submission.

## Operator-agent authority and raw-message lifecycle

The operator assignment defines the scope in which the agent may act. Within
that scope it may:

- resolve a routine matter from available evidence;
- ask a precise clarification in the raw thread;
- aggregate related informational messages;
- form a recommendation;
- escalate a human obligation;
- route a human outcome to the source;
- disposition raw and mediated obligations according to what happened.

It may not:

- impersonate the source;
- hide or rewrite the message of record;
- invent source availability or principal assurance;
- execute source-provided commands;
- mutate GitHub, Streamliner, or another authoritative system merely because a
  message requests it;
- claim human approval from delivery, queueing, or notification evidence.

The raw obligation lifecycle is:

| Situation | Operator action | Raw disposition |
|---|---|---|
| Routine and within authority | Resolve, reply when useful | `handled` |
| Evidence missing | Ask in raw thread | `deferred` |
| Human judgment required | Send escalation successfully | `escalated` |
| Human outcome routed durably | Reply in raw thread | `closed` |

`escalated` is not terminal. The raw obligation closes only after route-back is
durably accepted or an explicit stale-origin resolution is recorded.

## Production Operator Station extension

### Identity and compatibility

- Extension ID: `urn:telex:operator-station:v1`
- Shortname: `operator-station`
- Authoritative descriptor: this document until general extension packaging is
  accepted

The shortname is an alias. The extension ID in `metadata.extensions` is the
authority.

Supported v1 messages may add fields. Recipients ignore and preserve unknown
fields inside a recognized v1 envelope. A different extension ID or major
version is unsupported.

An unsupported message is shown as a feed-only raw diagnostic. If it is a human
obligation, the obligation remains visible and may be explicitly rejected as
unsupported; the Station does not auto-handle or auto-reject it.

### Kind inventory

| Kind | Direction | Purpose | Required to process | Safe to ignore |
|---|---|---|---:|---:|
| `operator-station.escalation` | operator agent -> Station | New human obligation with recommendation and sources | yes | no |
| `operator-station.human-reply` | Station -> operator agent | Human text or disposition outcome requiring raw-lifecycle update or stale-origin resolution | yes | no |
| `operator-station.digest` | operator agent -> Station | Aggregated informational summary | no | yes |

`operator-station.escalation` and `operator-station.human-reply` normally set
`requiresDisposition: true`. `operator-station.digest` normally does not.

Clarifications and routed outcomes remain ordinary replies in the raw thread or
operator-role conventions. The Station does not need to interpret distinct
production kinds for them.

### Escalation envelope

An escalation uses:

```json
{
  "extensions": {
    "operator-station": "urn:telex:operator-station:v1"
  },
  "dataschema": "urn:telex:operator-station:v1#escalation",
  "ext": {
    "operator-station": {
      "mediationId": "opaque-retry-stable-id",
      "ingressAddress": "configured-ingress-address",
      "humanAddress": "configured-human-address",
      "requestedOutcome": "One concrete question or requested decision",
      "recommendation": "Optional operator-authored recommendation",
      "sourceMessages": [
        {
          "storeId": "opaque-logical-store-id",
          "messageId": 123,
          "threadId": 120,
          "from": "worker-address",
          "to": "ingress-address",
          "kind": "decision-request",
          "attention": "next-checkpoint",
          "requiresDisposition": true,
          "subject": "Captured subject",
          "sentAtMs": 1780000000000
        }
      ]
    }
  }
}
```

The body remains understandable without metadata. It states:

- what happened;
- why human judgment is needed;
- the relevant evidence;
- the operator recommendation, if any;
- one concrete requested outcome.

`recommendation` is operator-authored and must be presented as such. It is not
source text and is not human approval.

### Human-reply envelope

A Station-authored reply stays in the mediated thread and uses:

```json
{
  "extensions": {
    "operator-station": "urn:telex:operator-station:v1"
  },
  "dataschema": "urn:telex:operator-station:v1#human-reply",
  "ext": {
    "operator-station": {
      "mediationId": "same-mediation-id",
      "operationId": "retry-stable-reply-operation-id",
      "responseType": "text-reply",
      "rootEscalation": {
        "storeId": "opaque-logical-store-id",
        "messageId": 456,
        "threadId": 456
      },
      "humanDispositionIntent": "handled",
      "humanNote": "Optional disposition note"
    }
  }
}
```

The reply is sent from the human address to the operator-agent address with
`requiresDisposition: true`. `next-checkpoint` is the default attention;
the human may select `interrupt` only for a genuinely urgent outcome.

`responseType` is:

- `text-reply` when the human supplied reply body text;
- `disposition-only` when assisted-mode Handle, Defer, Reject, or Close has no
  human reply text.

For `disposition-only`, the Station supplies a concise generated body such as
"Human deferred this escalation without a textual reply." The message is a
durable operator notification, not invented decision content. The
`humanDispositionIntent` is one of `handled`, `deferred`, `rejected`, or
`closed`; `humanNote` carries an optional human-authored reason.

### Digest envelope

A digest uses dataschema `urn:telex:operator-station:v1#digest` and carries a
retry-stable digest ID, a bounded summary period, and source references for each
included item. A digest never replaces the underlying messages or their
individual obligations.

### Experimental and campaign convention disposition

| Experimental convention | Production disposition |
|---|---|
| `operator-station-spike.escalation` | renamed to `operator-station.escalation` |
| `operator-station-spike.human-reply` | renamed to `operator-station.human-reply` |
| `operator-station-spike.clarification` | operator-role/raw-thread convention; not Station-interpreted |
| `operator-station-spike.routed-outcome` | ordinary raw-thread reply; not Station-interpreted |
| `operator-station-spike.stress-fyi` | retired harness-only kind |
| experimental v1 URN and schema | replaced by `urn:telex:operator-station:v1` |
| evidence-file schema strings | remain evidence-only |
| campaign `attention.*` kinds | remain campaign-local source kinds |
| `campaignAttention` metadata | remains opaque campaign evidence |

The Station may display campaign-local kinds and metadata as raw source
evidence. It does not grant them production extension semantics or a
notification override.

## Source provenance and trust

### Source references

A source reference is the tuple `(logical store identity, message ID)` plus a
captured display snapshot. Numeric message IDs are store-local and are never
opened against a different logical store merely because the number matches.

The logical store identity must:

- remain stable across application and daemon restart;
- distinguish different SQLite stores and Postgres deployments;
- be equality-comparable without revealing a path, credential, connection
  string, or token;
- come from the shared Application Client contract.

The spike's path fingerprint is not the production identity.

### Presentation states

| State | Meaning | Presentation |
|---|---|---|
| authoritative | Active store identity matches; current record resolves and captured identity fields agree | Show current source record and captured summary |
| mismatch | Record resolves but captured sender/thread/subject identity does not agree | Show both with a warning; do not route automatically |
| captured-only | Current app cannot access the referenced store, but a safe snapshot exists | Show snapshot as captured evidence, not verified current state |
| unavailable | Envelope invalid, store identity mismatched without a usable snapshot, or source missing | Show unavailable; never guess a source |

Source snapshots are evidence supplied by the operator agent. They are not
cryptographic proof. Unknown metadata remains inspectable but untrusted.

### Non-impersonation

The mediated message `from` is the operator-agent address. The UI presents:

- operator-agent sender;
- recommendation as operator-authored;
- each source address and reference separately;
- any authenticated principal evidence separately from the address.

The UI never styles the operator escalation as if the worker sent it directly.

## Feed, threads, local read state, and history

The feed is the authoritative human surface. Notification delivery is
supplemental.

On startup and recovery, the Station requests:

1. every unresolved primary obligation for its configured addresses;
2. bounded recent history;
3. thread expansion on demand;
4. subsequent delta events from a restart-safe cursor.

The Station does not require full-store materialization.

Live and backfill results merge by
`(logical store identity, message ID, delivery role)`. Duplicate delivery does
not create a duplicate row, toast, reply, or disposition operation.

The Station acknowledges a primary delivery only after it has durably written
the envelope, delivery role, metadata, disposition requirement, and receive
cursor to restart-replayable application state. In-memory insertion, rendering,
or toast submission is not durable ingest.

Local read/unread state is a UI preference. It does not acknowledge delivery or
disposition an obligation.

The thread view:

- displays the complete mediated thread;
- exposes disposition history;
- keeps raw and mediated thread IDs distinct;
- shows source cards with the trust states above;
- loads raw source threads only through an explicit source action;
- never silently merges raw and mediated conversations.

## Notification policy

### Default decision matrix

| Message | Attention / disposition | Default outcome |
|---|---|---|
| supported escalation | `interrupt`, disposition required | toast eligible; prominent actionable feed |
| supported escalation | `next-checkpoint`, disposition required | toast eligible; actionable feed |
| supported escalation | `background`, disposition required | actionable feed and badge; no toast |
| other supported/direct message | `interrupt`, disposition required | toast eligible; prominent actionable feed |
| other supported/direct message | `next-checkpoint`, disposition required | actionable feed; no toast by default |
| any message | `background`, disposition required | actionable feed and badge; no toast |
| any message | `fyi`, disposition required | actionable feed and badge; no toast |
| any message | no disposition required | feed/history only |
| digest | any normal digest attention | feed only |
| unsupported extension/kind | any | feed-only raw diagnostic |

### Precedence

The Station resolves collisions in this order:

1. OS/application notifications disabled;
2. explicit user source/address mute;
3. user quiet schedule or OS focus posture;
4. supported-kind override;
5. attention and disposition defaults.

`interrupt` does not bypass explicit user or OS suppression. It remains
prominent in the feed.

### Quiet posture and aggregation

The operator agent performs semantic aggregation. The Station may aggregate
notification presentation, but not obligations.

An aggregate notification has:

- a configured bounded window;
- an aggregation key such as source, thread, and kind;
- the count and newest subject;
- links to every underlying feed row.

Every message retains its own feed identity, thread, disposition state, and
source references. Unrelated human obligations are not collapsed into one
action.

### Notification evidence

For each toast-eligible delivery the Station records:

- resolved decision (`toast`, `feed-only`, `suppressed`, `aggregated`);
- policy reason;
- submission attempt and timestamp;
- OS result when observable;
- aggregate identity when used.

Toast submission is an attempt, not delivery to or consumption by a human.
When Windows Focus Assist or another OS posture is not observable, the state is
`unknown`. The spike did not validate Focus Assist perception; production
validation remains downstream work.

## Human reply and disposition

### Available actions

For a human obligation the Station offers:

- **Reply & Handle** - default response flow;
- **Reply** - continue the conversation without completing the selected
  obligation;
- **Handle** - complete without a reply;
- **Defer**;
- **Reject**;
- **Close** when the conversation is explicitly complete.

The selected recipient's disposition is always explicit. Reply does not
implicitly mutate another recipient's disposition.

In direct mode, disposition-only actions apply to the raw obligation locally
because the Station attends ingress. In assisted mode, every disposition-only
action first sends a durable `operator-station.human-reply` with
`responseType: disposition-only`. The Station changes the mediated root only
after that operator notification is durably accepted.

### Reply & Handle ordering

`Reply & Handle` is a higher-level application operation:

1. Mint or reuse a retry-stable operation ID.
2. Send `operator-station.human-reply`.
3. Verify the receipt identifies the expected parent, mediated thread, sender,
   and operator-agent recipient.
4. Only after durable reply acceptance, record `handled` for the selected human
   obligation.

The Station command handler enforces this order even when the Application
Client exposes a compound convenience operation.

Failure states are explicit:

| Failure | Required state/recovery |
|---|---|
| reply fails | root remains open; retry same operation ID |
| reply succeeds, Handle fails | show `reply sent / handle pending`; retry Handle only |
| Handle before durable reply | forbidden; fail closed |
| restart after reply receipt | recover operation and complete pending Handle without resending |
| indeterminate result | show partial/unknown; reconcile by operation ID and receipt before retry |

### Assisted disposition-only ordering

Handle, Defer, Reject, and Close without human text use the same retry and
ordering discipline:

1. Mint or reuse a retry-stable operation ID.
2. Send a disposition-only `operator-station.human-reply` identifying the
   intended root disposition and optional human note.
3. Verify the receipt identifies the expected mediated parent/thread, sender,
   and operator-agent recipient.
4. Only after durable acceptance, apply the intended disposition to the
   mediated root.

If notification send fails, the root remains unchanged. If notification
succeeds but root disposition fails, the Station shows
`operator notified / disposition pending` and retries only the root
disposition. Restart recovery reconciles by operation ID and never resends a
confirmed operator notification.

The operator-visible message remains a disposition-required obligation until
the raw lifecycle transition below succeeds. This prevents a terminal mediated
root from stranding an `escalated` raw source.

### Operator handling of the human response

Every assisted-mode human response, including disposition-only outcomes, is a
separate operator obligation. The operator agent:

1. reads the mediated root and validates its v1 envelope;
2. resolves the original source;
3. applies the response according to `responseType` and
   `humanDispositionIntent`;
4. verifies the raw-thread reply or disposition transition;
5. acknowledges and terminally handles the human-response message.

For `text-reply`, the operator routes the human text in the raw thread using the
same mediation ID and a retry-stable route operation ID, then closes the raw
obligation when appropriate.

For `disposition-only`:

| Human intent | Required raw outcome |
|---|---|
| `handled` | send a generic completion only when the source needs notice, then `closed` |
| `deferred` | record `deferred`; keep the raw obligation open for later human input |
| `rejected` | notify the source when safely addressable, then `rejected` with the human note |
| `closed` | notify the source when policy requires it, then `closed` |

The operator does not terminally handle the human-response message until the
required raw reply/disposition succeeds. Operator replacement recovers the
unresolved human-response message and repeats reconciliation by operation ID.

Durable queueing to an active but unoccupied source address counts as accepted
route-back. Human consumption by the source is not required before the
operator's response obligation becomes terminal.

### Stale-origin outcomes

An origin is stale when:

- its logical store or message cannot be resolved;
- the source identity mismatches;
- the target address is retired or rejects delivery;
- the source obligation was superseded;
- the source is already terminal and no route-back is needed.

An unoccupied active address is not stale.

The operator never guesses a replacement source. It records one of:

- `deferred` while asking the human or deployment owner for repair;
- `handled` with a human-visible note when the source is already terminal or no
  route-back is required;
- `rejected` or `closed` with a human-visible stale-origin reason when policy
  determines no safe route exists;
- a new directed message only after explicit policy or human confirmation.

The mediated thread shows the outcome. A late response to a closed escalation
is a new operator obligation and follows the same validation.

## Restart, replacement, duplicates, and recovery

### Station restart

The Station has stable application identity and explicit attach/detach/recovery
semantics supplied by the Application Client. On restart it:

1. restores its durable ingest projection;
2. reattaches configured addresses explicitly;
3. requests unresolved obligations and bounded recent history;
4. resumes from a durable cursor or performs an explicit resync;
5. dedupes redelivery by store and message identity;
6. suppresses duplicate startup toasts.

Application-local state may implement the projection, but the supported client
contract owns the identity, receive, ack, and recovery semantics. A local path,
session UUID, or high-water file is not the shared production contract.

### Operator-agent replacement

The durable ingress address survives the agent session. A replacement operator:

1. explicitly attaches the ingress address;
2. loads unresolved raw obligations and unresolved human responses, including
   disposition-only outcomes;
3. reads bounded mediated/raw thread context;
4. reconstructs episodes from mediation IDs and source references;
5. reconciles any prior operation ID before authoring a duplicate;
6. resumes deferred, escalated, or route-back work.

Recovery does not depend on the previous model transcript or local-only memory.

### Duplicate and partial authoring

Escalation, human response, route-back, and compound disposition operations use
retry-stable application operation IDs. The shared client must expose whether
an operation was accepted, duplicated, rejected, or indeterminate.

The operator does not create a second escalation merely because a delivery was
redelivered. The Station does not send a second reply merely because Handle
failed.

## Identity, principals, links, and safe actions

### Address and principal presentation

An address is the Telex routing identity. A principal is separate evidence.
The Station shows both when available.

A principal is labeled verified only when the Application Client supplies an
authenticated principal plus provenance from the selected backend. Otherwise
the UI says `unverified` or `unavailable`. Backend access alone is not
cryptographic proof that message prose is trustworthy.

The current daemon v1 same-user trust boundary in
[daemon.md section 7](daemon.md#7-authorization-and-the-trust-boundary) remains
visible. Strong cross-principal verification is deferred to operational
hardening.

### Safe links

- Telex message/thread/source actions are internal Station navigation.
- `https` links may open only after an explicit user action and must display the
  destination.
- `http`, `file`, custom schemes, and local process actions are disabled by
  default. A local allowlist may enable a bounded action with confirmation.
- Link labels never hide a different destination.
- Message bodies, metadata, fetched extension documentation, and recommendations
  are never executed as commands or agent instructions automatically.

The Station is a control surface: it sends Telex messages and dispositions. It
does not directly stop a session, merge a PR, mutate a workflow, or run a
source-provided command.

## Shared Application Client requirements

The following requirements are exported to issue #12. They are shared
semantics, not Station API design.

| ID | Shared requirement |
|---|---|
| AC-01 | Stable application station identity with explicit attach, detach, reattach/recovery, and typed membership-loss outcomes |
| AC-02 | Opaque stable logical-store identity with no path, credential, or connection-string exposure |
| AC-03 | Multi-address lifecycle with explicit partial results and compensation |
| AC-04 | Streaming/callback/async receive yielding message, delivery-role context, metadata, and ack capability |
| AC-05 | Ack-after-durable-ingest and observable ack-pending, deaf, and backlog state |
| AC-06 | At-least-once duplicate/redelivery identity and restart-safe cursor/resync semantics |
| AC-07 | Unresolved-obligation query plus bounded recent/thread history without full-store materialization |
| AC-08 | Typed send, reply, read-thread, and per-recipient disposition operations with explicit sender selection |
| AC-09 | Retry-safe application operation identity/idempotency with an explicit accepted-send duplicate window |
| AC-10 | Reply/disposition, disposition-only operator notification, and route-back compound semantics with durable ordering, partial outcomes, and recovery handles |
| AC-11 | Source resolution using logical-store identity plus message ID, with authoritative/captured/unavailable states |
| AC-12 | Lifecycle/health projection covering registration, epoch/owner, receive health, pending unconsumed, inbound actionable, ack pending, and detach/recovery outcomes |
| AC-13 | Backend-profile selection without backend-specific message semantics, covering current SQLite and credentialed Postgres, with authenticated principal provenance when available |
| AC-14 | Delta-oriented application events and explicit resync/backfill behavior |
| AC-15 | Receipt identity cross-checks, bounded retry/throttling, and local scope discovery/cleanup |

### Domain-to-client traceability

| Domain section | Shared requirements |
|---|---|
| Address topology and transitions | AC-01, AC-03, AC-12 |
| Attendance and health | AC-04, AC-05, AC-12, AC-14 |
| Production extension and provenance | AC-02, AC-04, AC-08, AC-11 |
| Feed/history | AC-04, AC-05, AC-06, AC-07, AC-14 |
| Human reply/disposition and route-back | AC-08, AC-09, AC-10, AC-15 |
| Restart and replacement | AC-01, AC-05, AC-06, AC-07, AC-09, AC-12, AC-14 |
| Identity and principals | AC-02, AC-11, AC-13 |

Station-specific behavior that is not exported to #12 includes the desktop
layout, notification matrix, source-card presentation, routing policy,
operator-agent judgment, extension vocabulary, and local safe-link policy.

Campaign convergence and issue #12 accept the eventual shared Application
Client contract. This domain document and its issue comment are requirements
inputs, not acceptance of that shared contract.

## Downstream obligations

### `station-app`

- implement configured direct and assisted attendance;
- implement the feed, thread, source, health, notification, and safe-link
  behavior in this document;
- implement Reply & Handle with fail-closed ordering and visible partial state;
- implement assisted disposition-only operator notification before root
  disposition;
- preserve local read state as separate from ack/disposition;
- validate Windows notification behavior and restart continuity.

### `operator-broker`

- package the operator-agent authority and lifecycle in this document;
- implement retry-stable mediation, clarification, escalation, aggregation, and
  route-back;
- apply disposition-only human outcomes to the raw lifecycle;
- rehydrate unresolved raw and human-response obligations after replacement;
- preserve source trust and non-impersonation;
- apply stale-origin outcomes explicitly.

### Issue #12 / Application Client

- accept, revise, or reject AC-01 through AC-15 as one shared contract with
  Watcher requirements;
- avoid desktop-specific or Watcher-specific API forks;
- export the `application-client-ready` checkpoint before production
  application nodes freeze integration.

### Operational hardening

- validate credentialed Postgres, remote principals, and source trust;
- validate duplicate, delayed, stale, and replacement cases under failure;
- validate Focus Assist, quiet hours, user-disabled notifications, noisy load,
  and aggregation;
- validate packaging, install/upgrade, auto-start, signing, and cleanup.

## Open-question and carry-forward disposition

| Input question or carry-forward | Disposition | Owner / tracker | Rationale and downstream impact |
|---|---|---|---|
| Production client surface | deferred to shared contract | issue #12 | This node defines semantics, not API shape |
| Operator role launch and recovery | contract accepted; implementation deferred | `operator-broker` | Stable address, explicit attach, unresolved rehydration, and operation identity are fixed here |
| Production kinds and metadata | accepted with replacement | this document / `operator-broker` / `station-app` | Experimental namespace is retired; v1 Station convention is fixed |
| Reply plus disposition | accepted | `station-app`, `operator-broker`, issue #12 | Reply & Handle and assisted disposition-only notification order are normative |
| Direct/assisted/quiet transitions | accepted with vocabulary refinement | `station-app`, deployment docs | Quiet is assisted policy; direct/assisted are exclusive topologies |
| Notification defaults | accepted; experiential validation deferred | `station-app`, operational hardening | Deterministic defaults are fixed; Focus Assist/noisy-load evidence remains |
| Principal assurance | presentation accepted; cryptographic assurance deferred | issue #12, operational hardening | Address and principal are separate; missing evidence is explicit |
| Focus Assist and quiet-hours validation | deferred | operational hardening | Spike did not verify perception/suppression behavior |
| Paged unresolved/history and delta events | promoted | issue #12 AC-07, AC-14 | Full export is rejected |
| Shutdown, duplicate ingest, restart cursor, receipt mismatch fault injection | deferred | operational hardening | Contract fixes expected outcomes; production evidence remains |
| Optimistic display of just-sent reply | optional, deferred | `station-app` | Must not hide durable receipt/partial state |
| Reply attention selection | accepted | `station-app`, issue #12 AC-08 | Human reply defaults to next-checkpoint with explicit urgent override |
| Retry throttling and richer notes | promoted | issue #12 AC-15, `station-app` | Needed for visible and safe recovery |
| Persisted-scope restart artifact capture | deferred | `station-app` validation | Contract requires restart-safe projection; evidence remains downstream |
| CLI subprocess courier | rejected as production contract | issue #12 | Shared client must replace it |
| Repeated one-shot waiter supervision | rejected as production contract | issue #12 | Application receive is supported client behavior |
| Full-history export recovery | rejected as production contract | issue #12 AC-07 | Use unresolved query plus bounded history |
| Store path fingerprint | rejected as production identity | issue #12 AC-02 | Use opaque logical-store identity |
| SQLite-only behavior | rejected as production semantic boundary | issue #12 AC-13, hardening | Same semantics cover SQLite and Postgres |
| Windows-first desktop | accepted for first app; cross-platform UI deferred | `station-app`, later work | Client semantics remain backend/platform independent |
| Development Tauri launch and HKCU AUMID registration | rejected as contract | packaging/hardening | Production install owns registration and cleanup |
| Local app-data session UUID/high-water | rejected as shared facility | issue #12 AC-01, AC-06 | Supported identity/cursor semantics are required |
| Current spike UI layout | rejected as normative | `station-app` | Product behavior is fixed; layout remains implementation work |
| Campaign `attention.*` / `campaignAttention` | retained only as campaign evidence | campaign orchestration | Station production schema is independent |
| Multi-device fan-out | deferred | future design | Exclusive occupancy remains the safe default |
| Arbitrary structured actions/command execution | rejected | future bounded extensions only | Message content cannot become implicit execution |

## Revisit conditions

Revisit this contract if:

- Telex accepts non-exclusive or multi-device address attendance;
- issue #12 cannot satisfy the required ingest, identity, unresolved-query, or
  retry-safe operation semantics;
- production dogfood shows the direct/assisted topology cannot transition
  safely with a durable unoccupied gap;
- the v1 extension cannot evolve additively;
- authenticated principal evidence cannot be presented without overstating
  trust;
- notification pressure requires a different default matrix;
- route-back cannot be recovered without adding a durable application-level
  correlation primitive.
