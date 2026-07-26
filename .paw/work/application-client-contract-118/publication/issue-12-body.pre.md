## Status

Design capture / forward-looking. **Not a V0 requirement.** This issue records the
design thinking for an embeddable Telex client so it can be picked up at
implementation time. In the same spirit as `DISPATCH.md` and `EXTENSIONS.md`:
a deliberate layer described before it is built.

## Motivation

A Copilot **SDK** session (e.g. `@github/copilot-sdk`, as used by Streamliner) and a
Copilot **CLI** session are both *just sessions*: each can occupy a durable Telex
address and exchange messages with other sessions. But the **mechanics** of how each
runtime holds its lease and ingests deliveries differ, and Telex's current
holder/waiter two-process pattern is shaped specifically by the **CLI** runtime.

This issue works out how an SDK session should integrate, and proposes an
**embeddable Telex client** that serves programmable hosts natively while keeping the
CLI path and the wire protocol unchanged.

**Key claim: the session-to-session fabric is runtime-agnostic; only the local
integration differs.** A CLI session and an SDK session can already talk to each
other transparently because they share the backend and the lease/message model. The
work here is purely about the *local embedding*, not the protocol.

## Background: two planes

From `DISPATCH.md`:

- **Plane A — intra-session control.** A session's runtime and how it steers itself
  (foreground turn, sub-agents, tools). Local, ephemeral, runtime-specific.
- **Plane B — inter-session fabric.** Session to backend to other sessions. Durable,
  networked, audited. **This is Telex.**

Telex standardizes Plane B. **The holder/waiter split is a Plane A integration detail
for one runtime (the Copilot CLI), not part of the Telex protocol.** Keeping that
boundary clean is what lets a new runtime (the SDK) integrate differently without
touching the fabric.

## Why the holder/waiter split exists (the CLI case)

The Copilot CLI is an **opaque** agent loop steered from outside via prompts, tools,
and hooks; its only ingestion mechanism is "run a command that exits with output."
`DESIGN.md` states the forcing constraint:

> an agent can only reason about a message once the call delivering it **returns** ...
> a single call cannot both block indefinitely and invoke agent turns mid-wait.

So Telex splits delivery into two processes:

- **holder** (`telex attach`) — a separate, long-lived process that holds the backend
  connection + TTL heartbeat (the answerback drum), polls for actionable messages,
  and buffers them. It never takes an agent turn, so it can stay up for the mission.
- **waiter** (`telex wait`) — an ephemeral subprocess the agent runs each turn; it
  blocks on the holder over local IPC and **exits** to hand exactly one message to
  the agent (exit codes: 0 delivered, 2 idle-timeout, 3 holder-gone, 4 holder-hung).

The two-process IPC dance exists purely to reconcile "the lease must stay alive
across turns" with "the agent only receives by exiting a subprocess." Those are the
**CLI's** constraints.

## Why an SDK session does not need two processes

In an SDK session you **own the loop.** "Taking a turn" is a function call
(`session.sendAndWait(...)`) you await. So a single process can concurrently:

- hold the lease + heartbeat as an async task, **and**
- await a model turn,

…on the same event loop. The contradiction that forced the split disappears, because
the heartbeat keeps firing while the model "thinks" (a server-side network call). The
**roles** survive but the **two-process realization collapses**:

| Role | CLI realization | SDK realization |
|---|---|---|
| Hold lease + heartbeat (answerback) | separate `telex attach` process | a concurrent task in the host process |
| Buffer messages arriving mid-turn | holder's in-memory queue | host's own queue |
| Deliver one unit of work to the model | `telex wait` subprocess exit | host calls `session.send(...)` with the buffered message |
| Decide *when* to deliver (attention level) | foreground reads between `wait` calls | host code at its own checkpoints |

For the SDK there is **no `telex wait`, no local IPC, no second process.** "Delivery"
stops being a subprocess exit and becomes an in-process callback/queue that the host
drains into the next turn.

Because **Plane B is identical**, CLI↔SDK, SDK↔SDK, and CLI↔CLI all interoperate by
construction. A session sending to an address cannot tell — and does not care —
whether the occupant is a CLI session or an SDK host. It sees: line open
(answerback), message landed, occupant dispositioned. The occupant's runtime type is
invisible on the wire.

## Differences that actually matter (not cosmetic)

- **Interrupt fidelity.** The CLI can never truly preempt a running turn — its
  "interrupt" is really "deliver at the next subprocess boundary." An SDK host that
  can cancel an in-flight stream may honor `interrupt` more faithfully
  (cancel → inject → restart). Usually undesirable, but the capability is real.
- **Multiplexed holder.** The CLI is essentially one-holder-per-agent-process. An SDK
  host (Streamliner is the obvious one) can hold leases and route inbound/outbound for
  **N sessions in one process** — a fleet-wide presence manager. The holder role
  centralizes instead of fanning out to one sidecar per session.
- **Waiter-as-reasoning-receptionist (dispatch/enquiry) is cleaner.** In the CLI, the
  "waiter reasons about an enquiry without interrupting the foreground" trick depends
  on the runtime's sub-agent steering over Plane A. An SDK host just makes a cheap
  side model call to triage. No dependence on a particular runtime's mechanism.
- **Work-scope brief sync is trivial.** The SDK host *is* the thing that knows the
  work scope (it is orchestrating the session), so it updates the lease description /
  triage logic directly rather than pushing a brief across Plane A.

## Hazard to design against

The CLI's separate holder process is *insulated* — the agent's heavy thinking happens
in a different process, so it cannot starve the heartbeat. In the SDK, if the host
blocks its event loop (a synchronous model call, heavy local compute), the heartbeat
task can lapse and answerback **falsely degrades to "line dead"** while the session is
actually fine. The hard rule for the SDK integration: **the heartbeat must run on a
task/thread that cannot be starved by turn execution** (see the napi-rs note below,
which solves this structurally).

## Proposed: an embeddable Telex client

Expose the holder core once; let each host choose its delivery shape.

### Layering (the multi-language strategy)

Telex core is Rust, so multi-language support is a layering problem, not a rewrite:

```
Layer 0  telex-core (Rust)        lease, heartbeat, cursor, buffer, send, disposition, directory
                                  - already exists; the CLI is one consumer of it
Layer 1  stable embedding ABI     (a) C ABI   (b) framed JSON-lines socket protocol
                                  (the existing IPC, stabilized as a documented contract)
Layer 2  idiomatic wrappers       TS (napi-rs)  |  Python (PyO3)  |  Go/C#/Java (C ABI/cgo/P-Invoke/JNI)
                                  |  anything (socket)
```

Two transports, one semantic API:

- **In-process binding** (napi-rs for TS/Node **first**) — telex-core runs *inside* the
  host process. Best DX and lifecycle coupling. Primary target since
  `@github/copilot-sdk` is TypeScript.
- **Sidecar daemon + thin client** — the holder runs as a local process; the host
  speaks the documented socket protocol. This is just *generalizing what `telex
  attach` already does*, and is the lowest-common-denominator for any language that
  can open a Unix socket / named pipe.

Crucially, the existing CLI becomes a **consumer of the same core**:
`telex attach` = "run Layer 0 as a process and serve Layer 1's socket";
`telex wait` = "a thin Layer-2 client that drains one delivery and exits." One holder
implementation, exposed three ways.

### Delivery shapes (one core, multiple drivers)

The lease/heartbeat/cursor/buffer logic is identical regardless of how a host wants to
receive. The API exposes:

- **Push** — a self-driven async iterable (`inbox()`); the heartbeat runs concurrently
  while you await the next item. The natural shape for SDK hosts.
- **Pull** — `poll()` returns everything new since a cursor; the natural shape for
  interval-driven, serverless, or cron-style hosts.
- **Callback** — `onMessage()` registration for hosts that prefer event-style code.

`telex wait` is simply the CLI's blocking projection over the same core.

### TypeScript API sketch

```ts
import { connect, Attention, Delivery } from "@telex/sdk";

// 1. Connect to a backend (reads ~/.telex/config.toml, or explicit).
const telex = await connect({ backend: "prod" });   // or { sqlite: "~/.telex/telex.db" }

// 2. Attach = claim the lease + start the heartbeat. The returned handle IS the holder,
//    collapsed in-process. `await using` ties the lease to this scope (TS 5.2 disposal).
await using station = await telex.attach({
  address: "workstream:proj/node:issue-215",
  description: "SDK worker on issue 215",
  occupant: session.id,          // the Copilot SDK sessionId becomes the lease occupant
  scope: "project:proj",
  tags: ["repo:telex", "ext:streamliner.v1"],
});
// heartbeat now runs on a background thread - no user turn required.
```

The `Station` (a held address) is the whole surface:

```ts
interface Station extends AsyncDisposable {
  readonly address: string;

  // ---- SEND (from this address) ----
  send(to: string, msg: OutgoingMessage): Promise<Receipt>;
  reply(toMessage: number, msg: OutgoingMessage): Promise<Receipt>;

  // ---- RECEIVE: three shapes over one core ----

  // PUSH - self-driven loop for SDK hosts. The heartbeat runs concurrently while you await.
  inbox(opts?: {
    signal?: AbortSignal;
    attention?: Attention[];     // optional filter, e.g. only ["interrupt","next-checkpoint"]
    since?: number;              // resume from a cursor after a crash
  }): AsyncIterable<Delivery>;

  // PULL - for interval/serverless/cron hosts. Drain everything new since cursor.
  poll(opts?: { max?: number; since?: number }): Promise<Delivery[]>;

  // CALLBACK - event-style registration.
  onMessage(handler: (d: Delivery) => void | Promise<void>): Unsubscribe;

  // ---- PRESENCE / answerback ----
  updateDescription(text: string): Promise<void>;   // publish work-scope brief to the lease (Plane B)
  readonly cursor: number;                           // last delivered id, for durable resume
  detach(): Promise<void>;                           // also runs on Symbol.asyncDispose
}

interface Delivery {
  readonly message: Message;     // id, threadId, parentId, from, to, kind, attention, subject, body, metadata
  // disposition is ergonomic and closes the requires_disposition loop:
  handled(note?: string): Promise<void>;
  rejected(reason: string): Promise<void>;
  closed(note?: string): Promise<void>;
  escalated(note?: string): Promise<void>;
}
```

Directory/discovery lives on the client (no lease needed):

```ts
await telex.resolve({ match: "issue 215" });          // -> addresses + descriptions
await telex.addressShow("workstream:proj/node:215");  // description, occupancy, supports, caps
await telex.list({ scope: "project:proj" });
```

### Wiring it into a Copilot SDK session

This is the mechanics of session-to-session comms. The host owns the loop, so push
delivery and the model loop are just two awaits on one event loop:

```ts
await using station = await telex.attach({ address, occupant: session.id, description });

// (a) Give the MODEL a way to send - expose a tool that calls station.send().
session.registerTool("telex_send", async ({ to, body, kind, attention }) => {
  const receipt = await station.send(to, { body, kind, attention });
  return { messageId: receipt.id, status: receipt.status };
});

// (b) Drive RECEIVE from the host loop, honoring attention as the *checkpoint* discipline.
const pending: Delivery[] = [];
(async () => {
  for await (const d of station.inbox({ signal })) {
    if (d.message.attention === "interrupt") {
      // SDK can honor interrupt faithfully: cancel/redirect the in-flight stream if you choose.
      await deliverNow(d);
    } else {
      pending.push(d);            // buffer for the next safe boundary (next-checkpoint / background)
    }
  }
})();

async function deliverNow(d: Delivery) {
  const turn = await session.sendAndWait(formatForModel(d.message));  // "delivery" = a function call
  await d.handled(summarize(turn));                                   // disposition after the model acts
}

// At each checkpoint the host defines, flush buffered work:
async function atCheckpoint() {
  while (pending.length) await deliverNow(pending.shift()!);
  await station.updateDescription(currentWorkScopeBrief());          // refresh answerback brief
}
```

Two SDK sessions talking is symmetric: each host attaches its address,
`station.send(otherAddress, …)` lands a durable threaded message, the peer's `inbox()`
yields it, the peer dispositions it, and the sender sees line-open + dispositioned
answerback by watching the thread. **A CLI session on the other end is
indistinguishable** — it consumes the same address via `telex wait` / `telex inbox`.

The same `Station` backs the pull regime; a host that prefers intervals just calls
`poll()` instead of running the `inbox()` loop:

```ts
for (const d of await station.poll({ max: 15 })) {
  await handle(d);   // act + d.handled()/d.rejected()
}
```

### Implementation win: napi-rs heartbeat insulation

Earlier hazard: a blocked host event loop could starve the heartbeat and falsely
degrade answerback. **napi-rs fixes this structurally** — the heartbeat runs on a
Rust-side Tokio task on its own thread, *not* the Node event loop. So even if a JS
turn blocks, the lease stays alive. This restores the process-isolation insulation the
CLI got from a separate holder process, **without** a separate process. A pure-JS
reimplementation would lose this — a strong reason to bind the Rust core rather than
reimplement the holder in each language.

### Multi-language mapping

| Language | Transport | Notes |
|---|---|---|
| **TypeScript/Node** (primary) | napi-rs in-process | Best DX; `await using`, async iterables, background-thread heartbeat |
| **Python** | PyO3 in-process | `async for` over `inbox()`; if/when Copilot SDK Python |
| **Go / C# / Java** | C ABI (cgo / P-Invoke / JNI) or sidecar | C ABI for tight coupling; sidecar if FFI is undesirable |
| **Anything else** | sidecar socket protocol | Generalized `telex attach` IPC; only needs sockets + JSON |

## Recommended phasing

1. **napi-rs TypeScript binding first** — covers Streamliner and primary usage.
2. **Stabilize the Layer-1 socket protocol** — gives every other language a path for
   free and is mostly already built (the current `wait` IPC frames).
3. **C ABI bindings on demand.**

Design the **semantic `Station` API once** so all bindings are thin and identical in
shape.

## Open questions / next steps

- **Define the Layer-1 framed socket protocol precisely** — the stabilized version of
  the current `wait` IPC frames, as the language-agnostic contract every Layer-2
  wrapper targets.
- **Map `Station` onto Streamliner's `launchClaim` / `runtime.lifecycleState`** so a
  Copilot SDK session and its Telex lease share exactly one lifecycle (attach on
  session start, heartbeat while alive, detach on session end).
- **Backend selection from the binding** — read `~/.telex/config.toml` vs explicit
  config, and which backends compile into the addon (matching `telex backend kinds`).
- **Cursor persistence / crash-resume** semantics for both push (`inbox`) and pull
  (`poll`).
- **Model-initiated vs host-initiated sends** — how a `telex_send` tool surface and
  direct host-code sends are both expressed through the same `station.send()`.
- **Interrupt semantics** — whether/when an SDK host should cancel an in-flight stream
  for `interrupt`-attention messages, vs always deferring to a checkpoint.
