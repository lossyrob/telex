# Telex — The Name, The History, The Metaphor

The problems telex solved in the twentieth century are the problems 
agent coordination faces in the early AI era.

This document keeps the name, history, and metaphor. For the build-facing product
thesis and design capture, see [PRODUCT-THESIS.md](PRODUCT-THESIS.md) and
[DESIGN.md](DESIGN.md).

## The name

**Telex** was the world's first widely adopted machine-to-machine text network.
Not the telephone, which carried voices and left no record. Not the telegram,
which still passed through a human office. Telex let one machine address another
machine directly, send typed text across the world, and produce a printed record
at both ends.

We took the name because that is exactly the gap in agent work today. Agents can
reason, write, and act. What they lack is a disciplined way to **address each
other, deliver a message reliably, confirm who received it, and leave a record of
what was said.** Telex did that for businesses in 1935. We want it for agents now.

## The history

Telex emerged from German research in the 1920s and became an operational
teleprinter service in 1933. After the Second World War it spread across Europe
and then the world, becoming the backbone of international business communication
for decades. By the late 1970s, a single country like West Germany carried well
over a hundred thousand telex connections, and even remote regions maintained at
least a few shortwave telex links.

It worked unlike the telephone. The telephone network carried analog voice. Telex
carried discrete binary signals — marks and spaces — over its own exchanges, with
its own numbering plan, its own signaling standards, and its own operating
discipline. A telex subscriber did not have a phone number; they had a **telex
number**, an address on a separate, text-only network built for records rather
than conversation.

Four properties made telex more than "a slow text message," and each one is a
design lesson for agent messaging. The first three are the bedrock — addressing,
delivery, and the record. The fourth, answerback, is the refinement that makes
addressing trustworthy.

### 1. The telex number: address a responsibility, not a wire

A telex subscriber was reachable by a **telex number** — a stable address on the
network, independent of which physical machine or operator happened to be sitting
at the other end. You addressed the *subscriber*, and the network's job was to
connect you to whoever was currently serving that number.

This is the foundation everything else rests on. In agent work the occupant is the
most ephemeral thing in the system: sessions start cold, crash, get interrupted,
and are replaced. The work — the role, the node, the workstream — endures. Telex's
lesson is to **address the durable responsibility, not the fragile process that
happens to be holding it right now.**

### 2. Store and forward: delivery that survives an absent recipient

Telex grew up inside the store-and-forward tradition. Early relay centers were
literally "torn-tape" rooms: a message arrived as punched paper tape, an operator
read its address, and if the outbound line was busy, the tape went onto a hook in
a physical queue until the line was free. Automatic systems soon replaced the
operators, but the principle held — a message could be **accepted, held, and
delivered later**, so the sender and recipient never had to be present at the
same instant.

This is the same lineage that later produced UUCP, FidoNet, SMTP mail relays, and
the SMS service center that holds your text until your phone reconnects. For
agents it is essential: the worker you need to reach may not have started yet, may
be mid-task, or may be on another machine. The message should wait for it.

### 3. The record: communication you could audit

Telex left paper at both ends. A telex was a **document**, not an utterance. It
could be filed, referenced, and trusted as a record of what was agreed,
instructed, or reported. Businesses ran on telex precisely because it combined
speed with accountability — the message and its provenance were durable.

For agents this is what makes behavior explainable. When you later ask a session
*why did you do this?*, it can point to the message of record: "the orchestrator
for the adjacent workstream asked me to split this node," with the message itself
as proof — not a half-remembered fragment of a context window.

### 4. Answerback: automatic identity, provided by the line — not the operator

Telex's most quietly brilliant feature was **answerback**. Before sending, the
operator could transmit a WRU signal — *Who are you?* — and the receiving machine
would automatically reply with its own encoded identity, set physically on a
rotating drum of pegs like a music box. The same exchange could run again at the
end of the message to confirm the line had stayed connected throughout.

The crucial detail is *who* answered. **The receiving operator was never
interrupted.** The machine responded on its own, from its pegs, while the typist
kept working. Answerback was identity confirmation supplied by the infrastructure,
not a question the recipient had to stop and answer.

That distinction is exactly what makes answerback fit agent sessions rather than
fight them. A working agent should not be interrupted by a "hey, is this you?"
ping. In the way these sessions run today, the **background waiter loop is the
answerback drum**: it holds the address's lease and a live connection, and it
confirms identity and liveness automatically while the foreground agent keeps
reasoning. The sender gets a machine-verified answer — *yes, this address is
served, and alive* — without the working agent ever being disturbed.

Telex even ran answerback at two moments — at the start of a message and again at
the end — and that maps onto two useful grades of liveness:

- **The line is open.** A session is attached and its loop is alive, so the
  address has an occupant. This is the start-of-message WRU.
- **The message was received and dispositioned.** Not just delivered, but
  acknowledged and handled, closed, or deferred by the occupant. This is the
  end-of-message confirmation that the exchange completed intact.

How faithfully Telex can answer depends on the backend, and we are honest about
that: on a local SQLite store the loop's heartbeat gives "last seen within its
lease window"; on Postgres a connection-bound lease can release the instant a
session dies, giving true real-time liveness. The mechanism that holds the lease
may also change as agent platforms evolve — today it is a background loop — but the
concept is durable: **answerback is identity supplied by the line, decoupled from
the agent's attention.**

Telex was also a transitional technology, and that is part of why it resonates. It
sat at the moment when communication became machine-addressable but still demanded
human-readable records and operator discipline. It automated away the skilled
Morse operator, replacing interpretive human signaling with typewriters, standard
codes, and automatic exchanges — without yet becoming the frictionless, recordless
chat that came later.

## Why this is the right metaphor for the early AI era

We are in our own transitional moment. Coordination between AI agents today looks
a lot like the world *before* telex: messages are carried by hand.

In practice that means a developer asks one session to write a file, commits it,
pushes it, pulls it on another machine, and asks a second session to read it.
Context is shuttled manually between ephemeral workers. Git history and scratch
notes become an improvised runtime coordination layer they were never designed to
be. It works, barely, and only because a human is standing in the relay center
tearing tape by hand.

Agents are capable but **ephemeral, context-limited, and isolated**. The session
that planned the work is not the session that implements it. A worker sees only
its own context window. The orchestrator cannot tell whether the node it is
counting on is still alive. None of these are intelligence problems. They are
**coordination-infrastructure problems** — the exact class telex solved.

So the early AI era needs what telex provided:

- a way to **address a responsibility, not a process**, because the occupant
  changes but the work endures;
- **store-and-forward** delivery so an offline or not-yet-started worker still
  gets its message;
- a **durable record** so any agent can later explain *why* it acted — "the
  orchestrator instructed this," with the message to prove it;
- **answerback** — automatic, infrastructure-supplied confirmation that an address
  is served and alive, without interrupting the working agent;
- **operator discipline**, now expressed as a message protocol agents follow:
  acknowledge, handle, defer, close, escalate.

Telex is deliberately *not* a general chat tool, a heavyweight control plane, or a
replacement for durable project artifacts. Those artifacts — design docs,
workstream graphs, source repositories, trackers — remain the source of truth.
Telex is the operational text network that carries the coordination *between* the
agents working on them: questions, handoffs, field reports, blockers, evidence,
map corrections, and decisions that require disposition.

## The metaphor, in one table

| Historical telex | Telex for agents | The problem it solves |
|---|---|---|
| Telex number | Durable address, e.g. `workstream:foo/role:orchestrator` | Reach a responsibility, not a fragile process id |
| Answerback / WRU | Lease + waiter loop answering automatically: *this address is served and alive* | Confirm a live recipient without interrupting the working agent |
| Teleprinter | A CLI session endpoint that sends, receives, waits, reports | Give each agent a real terminal on the network |
| Telex number ≠ phone number | A separate coordination plane, distinct from the work itself | Keep messaging out of the artifacts of record |
| Switched text network | Pluggable backend: SQLite locally, Postgres across machines | Same protocol from one box to many |
| Paper tape prepared offline | Structured message composed before sending | Send concise, typed payloads, not transcripts |
| Store-and-forward relay | Queued delivery when a recipient is offline or unoccupied | Sender and recipient need not be present at once |
| Printed record at both ends | Auditable history and explainable provenance | Any agent can later show *why* it acted |
| Standard signaling (ITA2) | Thin semantic core: kinds, attention, disposition, receipts | Interoperable meaning across every backend |
| Operator discipline | Agent protocol: acknowledge, handle, defer, close, escalate | Messages get dispositioned, not just read |
| Expensive line time | Context-budget pressure | Read the new message first; expand history only on demand |

## The thesis

**Telex makes agent sessions addressable without making them permanent.**

A session attaches to a role, node, or workstream for the duration of its mission
and holds that address by a lease. Other sessions message the *address*, never
needing to know which terminal, machine, or process currently occupies it. If the
occupant is live, the message can wake an actionable waiter. If none is live, the
message queues, warns, or bounces according to the address's policy. If the
address has been retired, the sender learns so immediately rather than shouting
into a closed line.

That is the whole idea, and it is an old one: a durable, typed, machine-addressable
text network with answerback identity, store-and-forward delivery, and a record you
can trust.

In short — **Telex is a modern answerback network for AI agents.**
