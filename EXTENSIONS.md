# Telex — Extensions & Capability Cards

> How addresses advertise what they can do, and how messages carry typed,
> namespaced payloads — without Telex core ever learning what any of it *means*.

## Status

Proposal / exploratory design capture. A **forward-looking layer** rather than a
V0 requirement, in the same spirit as [DISPATCH.md](DISPATCH.md). Telex is fully
useful today with free-form `kind`, opaque `metadata`, and self-registered
directory descriptions (see [DESIGN.md](DESIGN.md)). This document describes the
small, deliberate conventions that turn those existing fields into a real
extension mechanism — and the boundary that keeps Telex a switchboard rather than
a framework.

The motivating use case: a session running as a **service to a small list of
developers**, performing a whitelisted set of tasks on a machine, that can tell a
caller "here is what I can do and how to ask." Historically this is a **login
banner / MOTD** (Unix `/etc/motd`, BBS welcome screens, IRC MOTD) fused with
**protocol capability advertisement** (SMTP `EHLO`, IMAP `CAPABILITY`, IRCv3
`CAP`). This proposal generalizes that into an extension model.

## The core claim: the mechanism already exists

Extensions need almost no new core. Three fields already carry the whole idea:

| Extension need | Existing Telex field | Change required |
|---|---|---|
| Typed message *kinds* | `MessageRow.kind: String` (free-form) | none — namespace it |
| Typed *payload* fields | `MessageRow.metadata: Option<String>` (JSON) | none — convention only |
| Address self-description | `AddressRow`/`LeaseRow.description`, `scope`, `tags` | small — advertise `supports` |
| "What can this do?" doc | `SKILL.md` + `telex skill --address` | generalize to per-address |
| Discovery ladder | rungs 1–3 in [DISPATCH.md](DISPATCH.md) | extends cleanly |

`kind` + `metadata` already behaves like a **type-URL plus opaque payload**
(gRPC/protobuf `Any`, or a CloudEvents `type` + `data`): the core moves the
envelope, the consumer interprets the body. The backend `Backend` trait never has
to know an extension exists — it stores `kind`, `metadata`, and `tags` and nothing
about their meaning. That boundary is already drawn correctly and must stay drawn.

The `telex skill` command is the **capability-card precedent**: it already serves
an embedded, agent-readable, self-describing document, and already special-cases
`--address` to print an assignment. A per-address capability card is `telex skill`
aimed at a *responsibility* instead of the binary.

## Naming (and a collision to avoid)

The codebase already uses **"profile"** for *backend configuration*
(`profiles.rs`, `BackendProfile` in `~/.telex/config.toml`). The protocol-layer
concept therefore must not be called "profile." This document fixes the
vocabulary:

| Term | Meaning |
|---|---|
| **Extension** | A hosted bundle that adds message kinds and/or `metadata` fields, with a JSON Schema for machines and an `AGENT.md` for the agent reading it. STAC-style. |
| **Extension ID** | The canonical identity of an extension. Prefer a URI, usually versioned. |
| **Shortname** | The compact alias used in `kind` prefixes and `metadata.ext` keys. It is bound by the descriptor to one Extension ID; it is not authoritative by itself. |
| **Capability card** | The rendered, agent-readable "what this address can do and how to ask" document for a specific address. The MOTD/service-banner. |
| **Supports** | The list of extension IDs an address/lease advertises. |
| **Profile** | Reserved for backend config. **Not** reused at the protocol layer. |

(If a higher-level *bundle* of extensions ever needs a name — e.g. a Streamliner
work-geometry pack — call it a **suite** or **pack**, never a "profile.")

Shortnames should be lower-kebab-case (`service-card`, `rob-machine`) and treated
as a readability device. A message kind like `service-card.request` is only fully
identified once the caller also knows that `service-card` means
`https://telex.dev/ext/service-card/v1` for this address/message. That binding
comes from the advertised `supports`, the message `metadata.extensions` map, or
the fetched descriptor. Reserve bare, un-prefixed kinds (`note`, future core
kinds) for Telex core and legacy usage.

## Prior art worth stealing from

Beyond STAC extensions and A2A agent cards, several systems map unusually well:

| System | The idea to steal | Why it fits |
|---|---|---|
| **XMPP service discovery (XEP-0030/0115)** | An entity advertises **feature namespaces** (URIs); peers query `disco#info`; a **caps hash** avoids re-fetching | Closest analogue to "an address advertises capabilities" |
| **CloudEvents** | Envelope with `type`, `source`, and a **`dataschema` URI** + flat namespaced extension attributes | Closest analogue to the message envelope / `metadata` |
| **IRCv3 `CAP` negotiation** | Both sides **name** capabilities and use the intersection | Apt given the IRC-MOTD origin of the idea |
| **STAC extensions** | Hosted JSON Schema + shortname + field prefix + (we add) a human/agent doc | The packaging model |
| **MCP `initialize` capabilities** | Declare supported features at **handshake** | Maps onto answerback |
| **RFC 6906 `profile` link relation** | Apply a profile to a resource **without changing its media type** | The web's literal "profile" concept |
| **gRPC `Any` + type URLs** | `type_url` + opaque bytes; reader resolves the type | Already how `kind` + `metadata` behave |
| **`/.well-known/` (RFC 8615)** | A conventional discovery path | A well-known card kind/address |

The two to lean on hardest:

- **XMPP disco** is almost exactly this problem. Its **caps-hash trick** — advertise
  a short hash of your capability set, and let a peer fetch the full descriptor only
  when it sees a hash it hasn't cached — is the clean answer to "don't make
  answerback fat, and don't re-fetch the card every message."
- **CloudEvents** gives the envelope discipline: a `dataschema` pointer that says
  "here is the schema for this payload," and extension attributes that are flat,
  namespaced, and optional. That is the right rule for what may ride in `metadata`.

## Proposed shape

The smallest design that works on today's code.

### 1. Namespaced message kinds

`kind` becomes `<extension-shortname>.<type>`:

```text
service-card.request
service-card.response
streamliner.split-request
rob-machine.run-whitelisted-task
```

No schema change; `kind` is already a free-form `String`.

Decision: keep `kind` human-readable and cheap to route, but make the URI the
authority. Extension descriptors **must** list their message kinds and shortname,
and senders should include the shortname-to-URI binding in `metadata.extensions`
when ambiguity matters. Telex core does not reject unknown or ambiguous kinds; an
extension-aware recipient may reject them as unsupported.

### 2. `metadata` carries extension blocks + a schema pointer

A light convention inside the existing `metadata` JSON:

```json
{
  "extensions": {
    "service-card": "https://telex.dev/ext/service-card/v1"
  },
  "dataschema": "https://telex.dev/ext/service-card/v1/message.schema.json",
  "ext": {
    "service-card": { "requestType": "describe" }
  }
}
```

`extensions` maps shortnames used in this message to canonical extension IDs;
`ext` carries payload blocks keyed by the same shortnames. Core stores this blob
verbatim and interprets none of it.

Decision: `metadata` remains an opaque string to Telex core. The convention above
is the canonical envelope for extension-aware tools, not a send-time requirement.
Extension-aware commands such as `telex ext validate` may require valid JSON and
schema conformance; `telex send`, delivery, and disposition must not. Use `body`
for human/agent-readable prose and large natural-language payloads; use
`metadata.ext` for typed fields that a tool or waiter needs to inspect.

### 3. An address advertises what it supports

The directory already carries `description`, `scope`, and `tags`
([DESIGN.md](DESIGN.md) "Address directory and registration"). In V0 an address
can advertise extensions by **piggybacking on `tags`**:

```text
tags = "ext:service-card.v1, ext:rob-machine.v1, repo:telex"
```

Later, promote this to a first-class `supports` field (and optionally a `caps`
hash) on `AddressRow`/`LeaseRow` once the convention proves out. Both the
durable **address-declared** and ephemeral **occupant-declared** layers already
exist; `supports` slots into whichever the deployment uses.

Decision: tags are a bootstrap mechanism only. Anything that treats advertised
extensions as protocol input should split comma-separated tags and match exact,
normalized tags; substring matching is directory convenience, not capability
negotiation. The target shape is `supports: [extension-id, ...]` plus a `caps`
hash, present on the durable address, the live lease, or both.

### 4. Discovery: extend `address show`, optionally alias `describe`

A read-only address detail surface — the per-address analogue of
`telex skill --address` — should return the description, occupancy/liveness,
advertised extension IDs, and (if present) a capability-card pointer or caps hash:

```text
$ telex --address role:rob-machine/service address show
address:     role:rob-machine/service
occupancy:   occupied (line open)
description: whitelisted maintenance tasks for Rob's machine
supports:
  - https://telex.dev/ext/service-card/v1
  - https://example.com/telex/ext/rob-machine/v1
caps:        sha256:1f3a…   # fetch the card only if this hash is new
```

Decision: avoid inventing a parallel discovery model. First extend
`telex address show` because that command already owns address detail, lease, and
occupancy. A friendly top-level `telex describe <address>` can exist later as a
thin alias for agents and humans who think in "describe this service" terms.

### 5. The capability card can ride the fabric itself

A caller does not need a new transport to *get* the card. It sends a well-known
message:

```text
telex send --to role:rob-machine/service --kind service-card.request
  → reply  --kind service-card.response   (card in body/metadata)
```

This reuses store-and-forward, threading, disposition, and the audit trail for
free, and composes with answerback: **answerback (terse) says which extensions
exist; the describe/request exchange (richer) retrieves their content.** For a
static service, the card can equally be served straight from `telex describe`
without a round trip — the request/response form is for dynamic or
occupant-specific cards.

Decision: make `service-card` the bootstrap standard extension, not just a random
third-party convention. `address show`/`describe` must expose enough minimal
information to discover whether the service-card extension is supported; the
message exchange retrieves the richer card only when needed. If an address does
not support `service-card`, the caller still has the static directory description,
tags/scope, and liveness.

### 6. Resolving an extension descriptor

Extension IDs are URIs. A resolver verb fetches and caches the bundle:

```text
telex ext fetch https://example.com/telex/ext/rob-machine/v1
  → message.schema.json     # machine: validate / generate payloads
  → AGENT.md                # agent: what it means, how to ask, safety rules, examples
  → README.md               # human: overview
```

The **`AGENT.md` is the part STAC lacks and agents need most**: not just field
shapes, but operating guidance — what each message kind means, how to form a
request, required safety constraints, failure/disposition behavior, and worked
examples. An extension is "schema for machines, `AGENT.md` for the agent."

### Extension descriptor sketch

```yaml
id: https://example.com/telex/ext/rob-machine/v1
shortname: rob-machine
name: Rob Machine Service
version: 1.0.0
appliesTo: [message, address, answerback]
messageKinds:
  - rob-machine.run-whitelisted-task
  - rob-machine.task-result
schemas:
  message: schema/message.schema.json
docs:
  agentGuide: AGENT.md
  humanGuide: README.md
semantics:
  requiredToProcess: true   # a recipient that doesn't understand this must not pretend it did
  safeToIgnore: false
```

`requiredToProcess` / `safeToIgnore` borrow the spirit of MIME `Content-Disposition`
and protocol "mandatory-to-understand" flags: an extension can declare whether a
recipient lacking it may proceed or must decline (`rejected`, with a reason).

Decision: descriptor resolution is useful but security-sensitive. Prefer the
resolution order embedded -> local cache -> HTTPS fetch. Cache descriptors with
their source URI, retrieval time, and digest; show provenance when rendering
`AGENT.md`. Treat fetched `AGENT.md` as untrusted service-supplied instruction
until allowed by local policy, a trusted host list, or eventually a signature.

## Decisions needed to close the design gaps

These are the concrete choices that turn the proposal from "extension-shaped"
into a stable convention without making Telex core interpret extension meaning.

| Gap | Decision to make | Current lean |
|---|---|---|
| Shortname collision | Is the compact `kind` prefix globally meaningful, or only an alias? | Alias only. The URI Extension ID is canonical; the descriptor and `metadata.extensions` bind shortname to URI. |
| Metadata envelope | Is `metadata` arbitrary JSON, or a canonical extension envelope? | Core accepts any string. Extension-aware tools use the canonical `{ extensions, dataschema, ext }` envelope. |
| `supports` maturity | Can tags be depended on for capability negotiation? | No. Tags are a prototype affordance; first-class `supports` + `caps` is the stable target. |
| Discovery command | Should `describe` be a new concept or existing address detail? | Extend `address show`; make `describe` a thin alias if the UX warrants it. |
| Capability-card bootstrap | How does a caller discover the mechanism for discovering cards? | Standardize `service-card` as the first Telex extension and expose its support in address detail. |
| Descriptor trust | May fetched extension docs become agent instructions automatically? | No. Cache and render them with provenance; trust is an explicit local policy decision. |
| Mandatory-to-understand | Who rejects a message when a required extension is unsupported? | The extension-aware recipient/waiter, not Telex core. Core only carries the facts and the disposition record. |
| Version compatibility | How does a caller pick a compatible extension version? | Start exact: versioned URI IDs, advertise every supported version, and use intersection. Add semver/range syntax only once needed. |

## The guardrail: declare, advertise, discover, carry — never interpret

Consistent with [DISPATCH.md](DISPATCH.md)'s "discovery, not orchestration" rule:

> **Telex standardizes how extensions are declared, advertised, discovered, and
> carried. It does not standardize what any extension means.**

Concretely, Telex core:

- **does** namespace `kind`, store `metadata` verbatim, advertise `supports`,
  expose `describe`, and resolve/cache descriptors by URI;
- **does not** parse `ext` payloads, enforce extension schemas to move a message,
  decide mandatory-to-understand outcomes, run capability/auth logic, or execute
  any extension's tasks.

Optional convenience validation (`telex ext validate <message>`) may exist, but
**must never be required to send, deliver, or disposition a message.** The moment
moving a message depends on understanding `serviceCard`, Telex has become the
framework it is explicitly not. Authorization for a service agent's whitelisted
tasks lives in the *agent*, never in Telex. If a recipient lacks a required
extension, the recipient or its waiter records a normal `rejected` disposition
with a machine-readable reason; core does not infer that outcome.

## Why this stays faithful to the metaphor

Telex exchanges had **directory books**, a **directory-enquiry** service, and
**answerback** to confirm the far party before transmitting. This proposal is the
same discipline: a stable address advertises who it is and what it handles, a
caller looks it up and reads the card, and only then commits a directed request —
with a printed record at both ends. Capability advertisement is just answerback
grown a vocabulary: not only *"who are you?"* but *"and what are you prepared to
do?"* — still answered by the line, not by interrupting the working agent.

## Open questions

- **Resolvable URL vs opaque namespace.** Should an extension ID be a fetchable URL
  (STAC/CloudEvents — self-service onboarding: fetch the card, read `AGENT.md`,
  start using it) or an opaque namespace whose meaning is out-of-band (XMPP)?
  Current lean: **resolvable**, with the XMPP caps-hash trick to keep it cheap.
- **Where extension descriptors are hosted and cached** — arbitrary HTTPS, a
  convention dir, or embedded/bundled like `SKILL.md`? Current lean:
  embedded -> cached -> fetched, with provenance and local trust policy.
- **When `supports` graduates** from a `tags` convention to a first-class
  `supports`/`caps` column on `AddressRow`/`LeaseRow`.
- **Static vs dynamic cards** — served directly from `telex describe` vs requested
  via `service-card.request`/`response` for occupant- or caller-specific cards.
- **Mandatory-to-understand reason shape** — exact machine-readable rejection
  reason when a recipient/waiter lacks a `requiredToProcess` extension.
- **Versioning beyond exact URI intersection** — whether Telex ever needs
  IRCv3/MCP-style range negotiation, or whether advertised versioned URIs are
  enough.
- **Relationship to dispatch** — an `enquiry` ([DISPATCH.md](DISPATCH.md)) could
  filter on advertised `supports`, making extensions the vocabulary that bidding
  reasons over.
- **A2A / MCP bridges** — whether a Telex extension can re-export an address as an
  A2A agent card or an MCP tool surface, so Telex interoperates rather than competes.
