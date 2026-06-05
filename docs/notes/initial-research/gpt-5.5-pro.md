I’m using your brief’s intended shape: CLI-first, datastore-backed message fabric; durable role addresses; ephemeral leases; answerback; typed messages; dispositions; SQLite/Postgres reference backends; no bespoke server; not chat, not orchestration, not a tracker. 

## Verdict

Build Telex only if it owns a very specific missing primitive: **responsibility-addressed coordination between ephemeral agent sessions**.

Do **not** build “agent email.” That already exists, close enough to be dangerous. Do **not** build an orchestration framework. That room is full of forklifts. Do **not** build a broker. Postgres, Redis, NATS, Temporal, and friends are already sharpening knives in that alley.

The defensible Telex shape is smaller and stranger: **a CLI-native, local-first, backend-as-broker paper trail where agents can address work roles, prove liveness, exchange typed obligations, and close the loop with auditable disposition.**

That is not a huge product. That is the point.

## Prior art that most threatens the need to build

The most direct prior art is **MCP Agent Mail**. It is explicitly a mail-like coordination layer for coding agents, exposed as an HTTP-only FastMCP server. It has agent identities, inbox/outbox, searchable message history, Git-backed Markdown records, SQLite FTS, acknowledgements, unread inbox fetches, file reservations or leases, pre-commit guards, and hooks that remind agents about unread mail. That is not adjacent. That is standing in the driveway with your house key. ([GitHub][1])

That means Telex should not compete by being “cleaner agent mail.” It needs a sharper wedge:

Telex should differ by having **no server in V0**, using SQLite/Postgres directly as the broker; by addressing **durable responsibilities** rather than merely named agents; by offering real lease-backed answerback; by separating delivery, attention, liveness, acknowledgement, and handled state; and by making disposition a first-class protocol object instead of a courtesy flag.

**Interagent** is another close signal. It frames the same basic pain: parallel Claude Code agents cannot coordinate cleanly, so humans copy Markdown around. It offers an MCP server with agent registry, Markdown messaging, signal files, inbox polling, shared specs, status boards, and escalation. That validates the pain, but it also warns that the obvious solution degenerates into “shared Markdown plus polling.” ([GitHub][2])

**Claude Code Agent Teams** partially solves the same problem inside one vendor ecosystem: multiple Claude Code instances, one lead coordinating teammates, independent context windows, and direct communication. But it is experimental, disabled by default, and the docs themselves call out limitations around resumption, task coordination, and shutdown. That is precisely the crack Telex can crawl through: vendor-neutral coordination where session death and handoff are normal, not exceptional. ([Claude Code][3])

**OpenAI Codex cloud and the Agents SDK** reduce the need for Telex when the work fits inside OpenAI-owned orchestration. Codex can run coding tasks in cloud environments, work in parallel, connect to GitHub, and create PRs; the Agents SDK supports specialist agents, handoffs, state, approvals, custom storage, and tracing. If your actual problem is “run parallel coding agents and collect PRs,” these tools may already solve enough. ([OpenAI Developers][4])

**LangGraph, AutoGen, and Google ADK** solve orchestration, routing, state, human-in-the-loop, and multi-agent workflow inside an application runtime. LangGraph is explicitly for long-running stateful agents with durable execution, persistence, streaming, debugging, and deployment. AutoGen has routed agents, messages as serializable data, pub/sub topics, and subscriptions. Google ADK supports structured workflow composition, graph workflows, coordinator/sub-agent patterns, and deployment. If Telex grows upward into planning, graph execution, routing policy, or agent lifecycle management, these systems already own that hill. ([GitHub][5])

**A2A, MCP, and ACP** are protocol terrain Telex must respect, not replace. A2A is an open standard for agent interoperability across frameworks and vendors. MCP is an open protocol for connecting LLM applications to external tools, data, and workflows. Agent Client Protocol standardizes editor/IDE-to-agent communication, roughly the LSP-shaped path for coding agents. Also beware the acronym swamp: IBM/BeeAI’s Agent Communication Protocol has been folded toward A2A, while Zed/JetBrains ACP is editor-agent protocol work. Telex should interoperate with these where useful, but its core should stay lower and dumber: durable mailbox, liveness, receipts, audit. ([A2A Protocol][6])

**GitHub Issues, Linear, Slack, and Mattermost** already provide durable human-visible coordination, comments, APIs, webhooks, and message history. They are good enough when humans are the primary operators and agents merely leave breadcrumbs. They are bad Telex substitutes when agents need quiet polling, role-level liveness, machine-readable dispositions, context-budget discipline, and non-human attention semantics. ([GitHub Docs][7])

## The gap Telex can actually own

The unsolved part is not “agents need to talk.” That has been solved many times, usually with more ceremony than taste.

The gap is this:

A session is not the same thing as a responsibility.

Today, agent sessions are ephemeral, cold-started, interrupted, restarted, and replaced. But the work role persists: “orchestrator for stream X,” “node Y implementer,” “review blocker owner,” “CI triage responder.” Telex’s best idea is making the address belong to the **work geometry**, not the process currently wearing the hat.

That gives you a real primitive: “send this decision request to whoever currently owns `workstream:foo/role:orchestrator`; queue it if unoccupied; prove whether a live loop is attached; later show who saw it, who acknowledged it, who handled it, and why.”

That is different from chat. Different from an issue comment. Different from an agent framework callback. Smaller, meaner, more useful.

## Known problems Telex could solve well

**Liveness lies.** Most systems blur “message accepted,” “message delivered,” “recipient process alive,” “agent saw it,” and “work was handled.” Telex should make those separate states. Historical telex had the right instinct: answerback confirmed the destination machine identity, not the operator’s judgment. Telex should preserve that distinction: line open, message delivered, seen, acknowledged, handled, closed.

**Handoff sludge.** The current workaround is Git files, scratch notes, copied prompts, and “read this thing I wrote earlier.” That is not coordination. That is archaeology with a keyboard. Telex can make handoff a typed message with recipients, thread ID, required disposition, subject, metadata, and exportable audit.

**Context spam.** Agents should not reload the whole novel every time they check mail. Telex’s “newest/unseen by default, expandable thread on demand” is exactly right. The context window is a tiny apartment; do not furnish it with every chair ever built.

**Wrong-recipient interruption.** Separating `to`, `cc`, `watchers`, and `attention` is important. “For your awareness” and “stop what you are doing” are not the same packet.

**Stale workers and zombie addresses.** Address lifecycle matters: active, dormant, retired. Sends to retired addresses should bounce. Sends to dormant addresses should queue honestly. Sends to active addresses should have liveness semantics.

**Audit without pretending Git is the runtime.** Git is great as durable artifact history. It is a clumsy message bus. Telex can export Markdown for Git, but runtime coordination should live in the message store.

## Ideas worth stealing without remorse

Use **Postgres advisory locks** for serious liveness. Postgres session-level advisory locks are application-defined locks that are released when the session ends, which is exactly the flavor you want for “this ephemeral loop currently holds the lease.” Pair that with `LISTEN/NOTIFY`, which provides async notifications to sessions listening on a channel. This makes the proposed Postgres backend unusually strong for V0: no custom daemon, crash-sensitive lease semantics, networked coordination, and honest enough push. ([PostgreSQL][8])

Use **Redis Streams** and **NATS JetStream/KV** as later backend references, not as the first product shape. Redis Streams already gives append-only event logs, consumer groups, pending tracking, acknowledgement, and claiming stuck work. NATS JetStream/KV gives persistence, watch, history, TTL, and CAS-style updates. These are excellent transport patterns, but they do not give you Telex semantics by themselves. ([Redis][9])

Borrow from **Temporal** for message-handler discipline. Temporal’s Signals, Updates, and Queries model is useful because it treats external messages as state-machine inputs and forces you to think about ordering, reentrancy, handler lifecycle, and whether a message merely changed state or produced a validated result. Telex does not need Temporal, but it should inherit the paranoia. ([Temporal Docs][10])

Borrow from **actor systems**, but do not become one. Actors teach durable identity, mailboxes, supervision, and “send don’t call.” But Telex is not trying to execute remote actor methods. Keep it to coordination records and receipts.

Borrow lightly from **KQML/FIPA** agent communication history. The useful part is not their full ontology machinery. The useful part is speech-act shape: ask, tell, reply, propose, accept, reject, inform, subscribe. Telex message kinds should be pragmatic performatives, not an academic crystal cathedral. KQML was explicitly about runtime knowledge sharing among agents, while FIPA standardized agent lifecycle, message transport, message structure, interaction protocols, ontologies, and security. Good ancestors. Also, a warning label. ([AAAI][11])

Borrow from **Linda/tuple spaces and blackboard systems**: decoupling in time and space. Processes write facts into a shared associative space; others read or take them later. Telex’s dormant address queue and durable message store rhyme with this model. The trick is adding modern session leases, disposition, and context-aware reads. ([Inference Systems][12])

Borrow observability ideas from **OpenAI Agents SDK tracing**: traces, spans, handoffs, tool calls, and custom events. Telex’s audit log should be boringly queryable: thread, recipient, lease holder, delivery event, acknowledgment, handled event, export. Future you debugging a bizarre agent decision will need breadcrumbs, not vibes. ([OpenAI GitHub][13])

## Reasons to build Telex

Build it if the target user has multiple autonomous or semi-autonomous agent sessions running across terminals, machines, repos, or time, and the pain is coordination rather than reasoning.

Build it if Streamliner’s “work geometry” is real. Durable role addresses map beautifully onto workstreams, nodes, gates, checkpoints, orchestrators, and workers. That is Telex’s strongest product gravity.

Build it if the core demo is this: two independent coding agents on two machines, one Postgres backend, role leases, answerback, a queued dormant-address message, an interrupt-level blocker, an acknowledged handoff, a handled decision request, and a Markdown audit export that explains why a PR changed. That would be a missing primitive, not a toy.

Build it if “no bespoke server” is a hard constraint. A CLI plus SQLite/Postgres backend is deployable in places where a hosted coordination service is dead on arrival.

Build it if you want agent coordination that is **vendor-neutral** and **session-native**, not tied to Claude, OpenAI, a specific IDE, a specific orchestrator, or a repo-native team abstraction.

## Reasons not to build Telex

Do not build it if MCP Agent Mail already covers the workflows you care about. Its feature overlap is substantial enough that the default posture should be “reuse, fork, or interoperate,” not “pretend we are first to the tavern.” ([GitHub][1])

Do not build it if the real goal is multi-agent planning, routing, execution graphs, approvals, deployment, and memory. Use LangGraph, AutoGen, Google ADK, OpenAI Agents SDK, or similar. Telex should be infrastructure between sessions, not the cathedral where all agents kneel. ([GitHub][5])

Do not build it if GitHub Issues, Linear, or repo files are good enough. Many teams do not need a message fabric. They need discipline.

Do not build it if the product cannot enforce usage. A coordination layer nobody checks is just a haunted inbox.

Do not build it if security and identity are fuzzy. A shared Postgres database with every agent able to address every other agent is powerful, and also a lovely footgun wearing a bowtie. You need principal identity, project scoping, address ownership, permission checks, and audit from the start.

Do not build it if “answerback” becomes fake. The whole point is truthful liveness. If the implementation says “probably alive” when it means “last heartbeat row looked recent,” the abstraction starts smelling like warm cheese.

## Recommended shape

V0 should be brutally narrow:

`telex attach` to bind a session to an address.

`telex who` or `telex answerback` to show lease holder and liveness grade.

`telex send` with `kind`, `attention`, `requiresResponse`, `requiresDisposition`, `to`, `cc`, `watchers`, subject/body/metadata.

`telex wait` for loop-friendly blocking or polling.

`telex read --unseen` with context-budget defaults.

`telex ack`, `telex defer`, `telex reject`, `telex handled`, `telex close`.

`telex retire-address`.

`telex export` to Markdown or JSONL.

Schema should center on `addresses`, `leases`, `messages`, `message_recipients`, `threads`, and append-only `events`. Keep semantic logic in the client. Make backends advertise capabilities. Start with SQLite and Postgres exactly as the brief suggests.

Do not build a web UI, hosted relay, vector memory, ontology engine, agent planner, chat client, or custom broker in V0. Those are glitter traps.

Compatibility move: study MCP Agent Mail closely. Import/export its Markdown-ish records if practical. Add an MCP adapter later if agents need to call Telex as a tool. Add A2A bridge later only if there is a real cross-agent-service use case. Core Telex should remain the quiet datastore-backed switchboard.

## Historical namesake and analogy

The historical telex network is a better analogy than it first appears. Telex was an international network of teleprinters and switched exchanges that allowed subscribers to directly exchange text and data; destination machines could respond with an identity code, messages could be printed immediately or stored for later, and secure lines plus verification made messages legally meaningful business records. It emerged in the early 1930s and declined as fax and later systems took over. ([Encyclopedia Britannica][14])

The key point: **telex was not chat.** It was accountable machine-addressed text.

The ITU store-and-forward recommendations are especially relevant. They describe telex store-and-forward units that accept messages, assign unique IDs, support single or multi-address delivery, report delivery or non-delivery per address, preserve traceability after failures, and use answerback comparison for secure delivery procedures. That is almost a product requirements document for Telex if you squint with intent. ([ITU][15])

The analogy maps cleanly:

The **telex number/address** becomes a durable work address.

The **operator** becomes the foreground agent.

The **machine** becomes the background waiter loop.

**Answerback** becomes lease-backed liveness and identity.

The **printed paper trail** becomes an append-only audit log.

**Store-and-forward** becomes queued delivery to dormant responsibilities.

**Delivery reports** become per-recipient disposition state.

The caution is equally useful. In historical telex, answerback proved the machine, not the human mind. Telex should not confuse “line open” with “agent understood and handled.” That separation may be the whole project in miniature.

Final read: Telex is worth a prototype, not a crusade. Build the Postgres-backed answerback/disposition slice first. If that feels materially better than MCP Agent Mail plus discipline, continue. If it feels like agent email with a sterner haircut, stop there.

[1]: https://github.com/Dicklesworthstone/mcp_agent_mail "GitHub - Dicklesworthstone/mcp_agent_mail: Asynchronous coordination layer for AI coding agents: identities, inboxes, searchable threads, and advisory file leases over FastMCP + Git + SQLite · GitHub"
[2]: https://github.com/signalclaude/interagent "GitHub - signalclaude/interagent: MCP server for inter-agent communication between Claude Code instances. Send bug reports, feature requests, API specs, and status updates between agents. · GitHub"
[3]: https://code.claude.com/docs/en/agent-teams "Orchestrate teams of Claude Code sessions - Claude Code Docs"
[4]: https://developers.openai.com/codex/cloud "Web – Codex | OpenAI Developers"
[5]: https://github.com/langchain-ai/langgraph "GitHub - langchain-ai/langgraph: Build resilient agents. · GitHub"
[6]: https://a2a-protocol.org/latest/ "A2A Protocol"
[7]: https://docs.github.com/en/rest/issues?utm_source=chatgpt.com "REST API endpoints for issues - GitHub Docs"
[8]: https://www.postgresql.org/docs/current/explicit-locking.html "PostgreSQL: Documentation: 18: 13.3. Explicit Locking"
[9]: https://redis.io/docs/latest/develop/data-types/streams/ "Redis Streams | Docs"
[10]: https://docs.temporal.io/handling-messages "Handling Signals, Queries, & Updates | Temporal Platform Documentation"
[11]: https://cdn.aaai.org/Workshops/1994/WS-94-02/WS94-02-007.pdf?utm_source=chatgpt.com "KQML--A Language and Protocol for Knowledge and Information Exchange"
[12]: https://inferensys.com/glossary/agentic-memory-and-context-management/agentic-memory-architectures/tuple-spaces?utm_source=chatgpt.com "Tuple Spaces: Coordination Model for AI Agents"
[13]: https://openai.github.io/openai-agents-python/tracing/ "Tracing - OpenAI Agents SDK"
[14]: https://www.britannica.com/technology/telex "Telex | History, Advantages & Disadvantages | Britannica"
[15]: https://www.itu.int/rec/dologin_pub.asp?id=T-REC-F.72-198811-S%21%21PDF-E&lang=f&type=items "CCITT Rec. F.72 (11/1988) - INTERNATIONAL TELEX STORE AND FORWARD - GENERAL PRINCIPLES AND OPERATIONAL ASPECTS"
