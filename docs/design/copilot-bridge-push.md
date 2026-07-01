# Copilot bridge push: converged path forward (issue #53)

> Status: design of record for Copilot push delivery (issue #53). It supersedes the
> `--ui-server`-only framing in the original #53 body and folds in the in-session bridge
> proposal with a small set of deliberate changes; the "why" behind each change is spelled
> out so the reasoning stays visible to future maintainers.
>
> **Normative contract:** the daemon primitive this doc introduces is specified in
> [daemon.md sec.13.2](daemon.md#132-on-deliver-push-opt-in-harness-neutral) and
> [DECISIONS.md ADR 0039](DECISIONS.md#0039--push-delivery-via-a-generic-on-deliver-exec--copilot-session-bridge);
> [ARCHITECTURE.md sec.9](ARCHITECTURE.md#9-push-delivery-on-deliver-exec--in-session-bridge)
> carries the sequence diagram. This doc is the design narrative behind those contracts.

## TL;DR

A telex-aware Copilot session receives pushed messages as real turns without
running, re-arming, or babysitting a `telex wait` waiter. The transport is an
**in-session extension bridge** (Namra's mechanism), but **loaded on bind**
rather than auto-loaded everywhere, **provisioned and addressed by telex**
rather than by a separate install, and fed by a **generic, Copilot-agnostic
on-deliver exec primitive** in the daemon. The daemon never learns what Copilot,
a session, or a port is; all Copilot specifics stay in the existing `copilot.rs`
harness-boundary module plus a small embedded bridge script.

The agent's entire setup is two one-time calls at bind:

```
telex --address <addr> copilot attach --copilot-bridge   # telex writes the bridge + registers the handler
extensions_reload                                 # agent tool: loads the bridge live, same turn
```

After that, delivery is push: daemon -> exec `telex copilot push` -> bridge ->
`session.send` -> the message lands as a turn the agent sees and dispositions.
No loop. No re-arm. That removes the #45 "deaf station" and per-turn coordination
tax at its root, because the fragile thing (the agent-managed waiter) is gone.

## How it works (data flow)

1. **Bind (once).** `telex --address <addr> copilot attach --copilot-bridge` does two things:
   (a) materializes the bridge `extension.mjs` into this session's extension
   discovery dir, and (b) registers an on-deliver handler command with the
   daemon for this address: `telex copilot push --session <id>`. Neither step
   needs the bridge to be running yet.
2. **Load (once).** The agent calls `extensions_reload` (a first-party Copilot
   CLI tool available to every agent). Copilot forks the bridge as a child
   process; it calls `joinSession()` to attach to the live foreground session,
   opens a private local endpoint, and writes a registry entry keyed by the
   Copilot session id.
3. **Deliver (per message).** When a durable message is committed for the
   address, the daemon execs the registered handler argv. `telex copilot push`
   derives the bridge endpoint from the session id (checking the registry only for
   liveness / session ownership, not trusting its path), hands the message body to
   the bridge over the local endpoint, and the bridge calls `session.send(...)`,
   which injects the message as a queued user turn.
4. **See and disposition.** The agent processes the injected turn like any other
   message and records a normal telex disposition (`ack` / `rejected` / ...).
   Only then is the message marked consumed.

The two registrations are independent: the bridge endpoint (Copilot side, keyed
by session id) and the address -> handler mapping (daemon side, keyed by telex
address). `telex copilot push` is the join point between them.

## What changes vs Namra's proposal, and why

Namra's published bridge (`namra98/copilot-session-bridge`) is the right
transport and we are adopting it. The changes below are about *when* it loads,
*who* owns it, and *how* the daemon stays agnostic and durable. Each one is a
response to a concrete constraint, not a stylistic preference.

### 1. Load on bind (lazy self-load), not blanket auto-load

- **Namra:** install the extension at user scope so it loads in every Copilot
  session automatically.
- **Change:** telex writes the bridge into the *session* discovery dir at bind
  time, and the agent loads it with `extensions_reload`.
- **Why:** memory pressure on sessions that never use telex. An extension is a
  forked Node child process. Measured here (Node v24): a bridge-shaped extension
  is roughly 50-65 MB RSS / ~25 MB private per session. Auto-loading it at user
  scope makes *every* Copilot session -- including the large majority that never
  touch telex -- pay that cost for nothing. Loading on bind means non-telex
  sessions pay zero and there is no always-on resident process for them.
- **Proven:** a session can load an extension into itself at runtime. Verified
  empirically: scaffolding `extension.mjs` into
  `session-state/<id>/extensions/<name>/` (a dir absent at session start) and
  calling `extensions_reload` forks it live, same turn, and its registered tool
  is immediately callable. Removing the dir + reload unloads it. The newer
  build's `agent-author.md` documents exactly this loop: write the file ->
  `extensions_reload({})` -> "New tools are available immediately in the same
  turn (mid-turn refresh)."

### 2. telex owns and writes the bridge; no MCP server

- **Considered:** ship an MCP server in the plugin that runs the bind and copies
  in the extension and reloads.
- **Change:** rejected the MCP server. telex materializes the bridge bytes
  (embedded in the binary via `include_str!`) and the agent does the reload.
- **Why, decisively:** extensions have exactly two reload triggers -- `/clear`
  (or foreground-session replacement) and the `extensions_reload` agent tool.
  There is no filesystem-watch and no external/programmatic reload trigger. An
  MCP server runs in a subprocess and **cannot** call `extensions_reload`; that
  tool is exposed only to the agent. So an MCP server could copy the file but
  could not perform the one step we would want it for -- the agent must reload
  regardless. On top of that, an MCP server is itself an always-on per-session
  process, which reintroduces the exact blanket-memory cost change #1 removes,
  for every plugin-enabled session. Net: an MCP server adds cost and removes no
  agent step. A skill that documents the two-call sequence, plus telex owning
  the bridge bytes, is strictly better: single source of truth, versioned with
  the daemon protocol, ~100 lines, no new heavy dependencies.

### 3. Generic daemon on-deliver exec, not a Copilot-coupled daemon

- **Change:** the daemon gains a generic primitive: an address may register a
  delivery-handler command; on commit the daemon execs that argv, same-user,
  timeout- and concurrency-capped, off the durable critical path, retried from
  unacked store state. The daemon never parses Copilot payloads, never knows
  about sessions, ports, or the SDK.
- **Why:** hard architectural constraint -- telex core/daemon must hold zero
  Copilot/SDK coupling. This is already how the codebase is organized:
  `src/commands/copilot.rs` is explicitly "the harness boundary ... Core daemon
  protocol and identity helpers intentionally remain unaware of Copilot-specific
  names." The on-deliver exec keeps that boundary: Copilot-specifics live in
  `telex copilot push` (the registered handler) and the embedded bridge; the
  daemon only execs an opaque, operator-registered command. This also makes the
  same primitive reusable by any future harness (Claude Code, a plain webhook,
  etc.) with a different handler.
- **Note on the existing `--push` flag:** the current `AttachArgs.push` is a
  deprecated no-op (`cli.rs`: "Deprecated compatibility flag; daemon delivery
  owns push/poll behavior"; `attach.rs` warns it is ignored). It is unrelated
  to this on-deliver exec, which is new daemon work.

### 4. Keep agent disposition; do not ack on push

- **Change:** the bridge is a dumb transport. It does not ack telex. The agent
  dispositions after seeing the message in a turn.
- **Why:** the durability invariant is sacred -- a message must not be marked
  consumed until its content provably reached a turn. Acking at push time (when
  the daemon hands off, or when the bridge POSTs) would mark consume-before-see,
  which is silent loss on a crash between push and turn. Deferring to agent
  disposition keeps at-least-once (a duplicate redelivery) as the only failure
  direction, which is the safe one. The operator's #45 pain was the waiter's
  *fragility*, never the disposition step; we keep disposition and delete the
  waiter. An inject-receipt-gated handler ack is a possible future optimization,
  not a v1 requirement.

### 5. Lazy endpoint resolution (order-independent, self-healing)

- **Change:** `telex copilot push` resolves the bridge endpoint at delivery time,
  not at bind time. As shipped it **derives** the endpoint from the session id and
  checks the registry only for liveness / session ownership, so a tampered registry
  path cannot redirect a push.
- **Why:** it removes a bind-vs-load ordering dependency and a chicken-and-egg.
  Bind can register the handler before the bridge is loaded; if a message
  arrives before the bridge is up, the handler simply fails to find a live
  endpoint and the daemon's retry redelivers later. The same path absorbs the
  bridge's endpoint changing across a `/clear` reload. This is the #46
  deaf-evidence + retry half doing its job.

### 6. Named pipe, not loopback TCP + bearer token

- **Namra:** loopback HTTP on `127.0.0.1:<random>` with a random bearer token
  written into a plaintext registry file.
- **Change (recommended):** bind a per-session OS named pipe
  (`\\.\pipe\telex-bridge-<sessionId>` on Windows; a unix domain socket under a
  user-only dir on POSIX), authorized by filesystem/pipe ACL to the current
  user.
- **Why:** (a) no token at rest in a plaintext file; the trust boundary is the
  same-user OS ACL, which is what loopback was approximating anyway. (b) No TCP
  port to collide or to be reachable by other local users. (c) The endpoint is
  derivable from the session id and stable across `/clear` reloads, so there is
  no random-port/token rewrite race when the bridge reloads. The registry then
  carries liveness/pid, not a secret.

### 7. displayPrompt label

- **Change:** the bridge sends with `displayPrompt` so the timeline shows a
  clean `[telex] from <addr> (<attention>)` label instead of the raw injected prompt.
- **Why:** operator legibility. `displayPrompt` is a first-class send option in
  the current SDK (`MessageOptions.displayPrompt`, 1.0.66) and is preserved on
  the underlying session RPC; the bridge uses the path that preserves it. (An
  earlier build's high-level wrapper dropped it; the bridge does not rely on that
  wrapper.) Confirmed rendering in a live run: the timeline showed the
  `[telex] ...` label.

## Lifecycle: load on bind, unload on detach

Load-on-bind needs a symmetric **unload-on-detach**, because a session-scoped
extension is durable. The bridge `extension.mjs` lives in
`session-state/<sessionId>/extensions/telex-bridge/`, and that directory
**persists across session resume**. So a bridge left there reloads every time
the session is resumed -- not just for the run that bound it. Without an unbind
step, a session that used telex once keeps forking the bridge forever.

The full lifecycle:

- **Bind** -- `telex --address <addr> copilot attach --copilot-bridge` writes the embedded
  `extension.mjs` into the session extension dir and registers the daemon
  on-deliver handler. The agent calls `extensions_reload` to load it.
- **Unbind** -- `telex --address <addr> copilot detach` deregisters the handler and **removes the
  `extension.mjs`** (so it will not reload on a later resume); the agent calls
  `extensions_reload` to unload the live process now. Removal is **ref-counted
  to the session's last telex binding**: one bridge serves all of a session's
  addresses, so it is removed only when the final binding for that session goes
  away. Both steps live in the `copilot.rs` boundary, so the daemon stays
  agnostic.
- **No elevated permission** -- the bridge requests no `skipPermission` and
  needs no agent tool for delivery (the pipe is the interface). So a (re)load is
  **silent** -- no permission prompt. This is what makes the orphan case below
  painless. (Observed: a `skipPermission` debug tool triggered an
  elevated-permission prompt on every resume; dropping it removes the prompt.)
- **Orphan safety (closed without detach)** -- if a session is killed or closed
  before `telex --address <addr> copilot detach`, the file persists and the bridge
  reloads silently on the next resume. Mitigations, in order of cost: (a) silent load
  means it is harmless if unused; (b) a `telex copilot gc` (or an attach-time sweep)
  prunes session-bridge dirs whose session ids telex no longer binds; (c) optionally
  the bridge self-exits on load if telex shows no binding for its session id, keeping
  orphan memory near zero. **v1 ships (a) only** -- silent-load-is-harmless plus the
  explicit cleanup paths (`copilot detach`, Copilot `sessionEnd`, and attach-failure
  rollback each remove the extension dir / registry / bindings); (b) `telex copilot gc`
  / attach-time sweep and (c) self-exit are **deferred** (ADR 0039).

## Effect on sessions that do not use this

- **Non-telex sessions:** zero. No extension is written, nothing is loaded, no
  process is forked, no port or pipe is opened. This is the whole point of
  load-on-bind.
- **Relationship to `--ui-server`:** the original #53 framing leaned on the
  hidden `--ui-server` flag (an in-process JSON-RPC server an external process
  injects into). The bridge path does **not** require `--ui-server` and does not
  change its behavior. `--ui-server` remains a viable lighter-weight transport
  for flag-launched sessions (in-process, no extra process), and an agent that
  detects it was started with `--ui-server` could choose that path instead. The
  bridge is the general answer that works without the flag; `--ui-server` is an
  optional optimization for sessions that happen to have it. Both feed the same
  generic daemon on-deliver exec; only the handler differs.

## What is proven (spikes)

- **External injection into a live session** works end to end (first proven via
  `--ui-server`: an external same-user process connected to the loopback JSON-RPC
  server, with no token on loopback, and drove a full user-message ->
  assistant-message -> idle turn).
- **`joinSession()`** is a real SDK export; the returned session exposes
  `sessionId`, `send(...)` (returns a message id), and a raw `rpc` surface.
- **`session.send` enqueue mode is non-interrupting:** it queues behind the
  current turn rather than interrupting in-flight work.
- **Runtime self-load** of an extension into a running session works (change #1
  above).
- **Memory** numbers above are measured, not estimated.
- **Namra's bridge** is published and working; we are forking its transport, not
  reinventing it.

## Risks and mitigations

| Risk | Mitigation |
|---|---|
| Resident Node process (~25 MB) once loaded | Only for telex sessions, only after bind; non-telex pay zero |
| `/clear` reloads the bridge (in-memory state lost) | Endpoint is derived from session id (named pipe) so it is stable; the daemon registration is daemon-side and survives `/clear`; one idempotent re-load re-arms |
| Delivery racing a reload gap | Lazy endpoint resolution + daemon retry redelivers; named pipe keeps the endpoint stable so the window is tiny |
| Stale registry on hard kill (SIGKILL skips cleanup) | `telex copilot push` treats a dead endpoint as a failed delivery -> daemon retry; the bridge best-effort removes its registry entry on SIGTERM/SIGINT, and explicit `copilot detach` / `sessionEnd` clean up. A GC / health-probe pruner is deferred (ADR 0039) |
| `extensions_reload` is global (restarts all extensions) | Acceptable; reload is idempotent and infrequent (bind, `/clear`) |
| Address mapping | telex owns address -> session mapping via `attach`; the Copilot-side registry is keyed by session id and correlated by the handler |
| Bridge bytes drift from protocol | telex embeds the bridge (`include_str!`) so it is versioned with the daemon |

## Where this lives in the code

- `src/commands/copilot.rs` -- the harness boundary. Add `CopilotCmd::Push`
  (the `telex copilot push` handler) and the `--copilot-bridge` provisioning on
  the copilot bind path (write the embedded `extension.mjs`, register the
  handler).
- daemon -- the generic on-deliver exec primitive (register a handler command
  per address; exec on commit, capped, off the critical path, retried).
- embedded bridge -- `extension.mjs` bytes carried in the binary, written on
  bind. Prototyped under `copilot-bridge/` in this branch.
- `skills/telex/SKILL.md` -- a small **bootstrap** that points the agent at
  `telex copilot skill` (version-matched, binary-owned) and `--help` for syntax,
  rather than embedding the workflow. See
  [DECISIONS.md ADR 0040](DECISIONS.md#0040--copilot-skill-is-binary-owned-the-plugin-skill-is-a-bootstrap).
- `COPILOT.md` + `telex copilot skill` -- the binary-owned, version-matched Copilot
  workflow (bind, load bridge, pushed turns, disposition, teardown, fallback) with a
  plugin/binary compatibility header (`telex v..`, bridge protocol, minimum plugin).

## Open questions

> **Resolved as shipped in PR #55.** The normative answers live in
> [daemon.md sec.13.2](daemon.md#132-on-deliver-push-opt-in-harness-neutral) and
> [DECISIONS.md ADR 0039](DECISIONS.md#0039--push-delivery-via-a-generic-on-deliver-exec--copilot-session-bridge).
> For the record: daemon registration is `Register.on_deliver: Vec<String>` (an argv), exec'd
> off the ack path with capped concurrency/timeout and a bounded per-heartbeat retry sweep;
> `telex copilot push` does a bounded round-trip to the bridge but delivery stays
> **at-least-once** via the sweep (no ack-gating in v1); the POSIX endpoint is a per-session UDS
> under `~/.copilot/telex-bridge/` with a `0700` dir and fail-closed `0600` socket; and the
> copilot module ships in the binary (no feature gate).

The original open questions, retained for history:

- Exact name/shape of the daemon on-deliver registration (per-address handler
  argv; capping and retry policy; how it reads from unacked store state).
- Whether `telex copilot push` blocks for an inject receipt (future ack-gating)
  or is fire-and-forget with daemon retry (v1).
- POSIX endpoint path/permissions for the unix-socket equivalent.
- Whether to feature-gate the copilot module (`#[cfg(feature = "copilot")]`)
  given it is the primary use case (current lean: keep it in the binary, no
  extraction; the module is ~960 lines with only `anyhow` + `serde`).

## Post-review hardening (PR #55)

Namra's PR review flagged edge cases where "push registered" could still let a station go
silently deaf or strand a durable message. Fixed in this PR (see
[daemon.md sec.13.2](daemon.md#132-on-deliver-push-opt-in-harness-neutral)):

- **Push accept is not delivery.** A successful `session.send` only proves Copilot *accepted*
  the turn, so "pushed" is an **attempt record**, not terminal suppression: a still-unacked
  message is re-pushed on a per-message **backoff** (base doubling to a cap), with a **degraded**
  status after a ceiling. A crash/reload after accept-but-before-ack no longer strands it.
- **Handler survives a pull re-attach.** A generic refresh that re-registers with
  `on_deliver = None` **preserves** the existing bridge handler; only an explicit re-provision
  replaces it.
- **Guard is mixed-session aware.** The turn guard no longer suppresses coverage for a whole
  session when any member is push-registered: pull members still get waiter-coverage checks, and a
  push member with an **unacked backlog** is surfaced (`push_registered` is "handler registered",
  not "bridge live").
- **Compatibility gate.** The daemon advertises an `on_deliver_exec_v1` capability, and a
  `--copilot-bridge` bind **verifies `push_registered`** and fails closed against an older daemon
  that would silently ignore `on_deliver`.
- **Prompt fence.** The untrusted-content fence uses a per-message **nonce** so a sender cannot
  forge the `END TELEX MESSAGE` delimiter and smuggle instructions past it.
- **POSIX reload cleanup.** The bridge's unix-socket unlink is now inside the pid-ownership guard,
  so an old bridge's reload cannot unlink a newer bridge's socket.

Deferred as follow-up hardening (non-blocking): a larger/negotiated bridge frame cap with
prompt-wrapping headroom, per-`(member, message)` attempt surfacing / dead-letter, per-endpoint
ordering, `store_key` in the ack/handle hints for named-backend users, session-id token
validation in the JS bridge, and explicit Windows named-pipe ACL hardening.
