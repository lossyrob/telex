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
# observer/table seats that want live CC turns opt in:
telex --address <addr> copilot attach --copilot-bridge --wake-on-cc
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
   needs the bridge to be running yet. `--wake-on-cc` is an additional per-bind opt-in:
   the daemon records a CC lower bound and pushes only live CC observer messages after
   that point.
2. **Load (once).** The agent calls `extensions_reload` (a first-party Copilot
   CLI tool available to every agent). Copilot forks the bridge as a child
   process; it calls `joinSession()` to attach to the live foreground session,
   opens a private local endpoint, and writes a registry entry keyed by the
   Copilot session id.
3. **Deliver (per message).** When a durable message is committed for the
   address (or, with `--wake-on-cc`, live CC traffic is committed for the address),
   the daemon execs the registered handler argv. `telex copilot push`
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
  clean `[telex] FROM: <addr> SUBJECT: <subject>` label instead of the raw injected prompt.
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
  bind. Lives under `copilot/bridge/` in this repository.
- `copilot/plugin/skills/telex/SKILL.md` -- a small **bootstrap** that points the agent at
  `telex copilot skill` (version-matched, binary-owned) and `--help` for syntax,
  rather than embedding the workflow. See
  [DECISIONS.md ADR 0040](DECISIONS.md#0040--copilot-skill-is-binary-owned-the-plugin-skill-is-a-bootstrap).
- `copilot/COPILOT.md` + `telex copilot skill` -- the binary-owned, version-matched Copilot
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

- **Push accept is not delivery.** The bridge waits briefly for the SDK `session.send` RPC ack, then
  replies success once the enqueue is confirmed or still pending behind a busy current turn; it does
  **not** wait for the injected turn to be processed. "Pushed" is an **attempt record**, not terminal
  suppression: the durable ack fence remains authoritative, and a crash/reload after
  accept-but-before-ack no longer strands it.
- **Handler survives a pull re-attach.** A generic refresh that re-registers with
  `on_deliver = None` **preserves** the existing bridge handler; only an explicit re-provision
  replaces it.
- **Guard is mixed-session aware.** The turn guard no longer suppresses coverage for a whole
  session when any member is push-registered: pull members still get waiter-coverage checks, and a
  push member whose bridge heartbeat is stale is surfaced (`push_registered` is "handler
  registered", not "bridge live"). A live push member's unacked backlog is **not** surfaced by the
  guard, because enqueue-mode turns may already be queued behind the current turn.
- **Compatibility gate.** The daemon advertises an `on_deliver_exec_v1` capability, and a
  `--copilot-bridge` bind **verifies `push_registered`** and fails closed against an older daemon
  that would silently ignore `on_deliver`.
- **Prompt fence.** The untrusted-content fence uses a per-message **nonce** so a sender cannot
  forge the `END TELEX MESSAGE` delimiter and smuggle instructions past it.
- **POSIX reload cleanup.** The bridge's unix-socket unlink is now inside the pid-ownership guard,
  so an old bridge's reload cannot unlink a newer bridge's socket.

Deferred as follow-up hardening (non-blocking): per-endpoint push ordering when queued-turn
order matters, and session-id token validation in the JS bridge.

## Further hardening (PR #55)

A second review pass and builder-directed follow-ups added:

- **Oversize dead-letter.** `telex copilot push` preflights the fully-encoded request against the
  bridge frame cap; a message that cannot fit returns a permanent exit and the daemon
  **dead-letters** it (skips further pushes, surfaces a status) instead of retrying forever -- it
  stays durably queued and readable via `telex inbox`.
- **Negotiated / larger frame cap.** The bridge advertises its `maxRequestBytes` in the registry
  and the cap is raised (8 MiB) so realistic large messages push as turns; dead-letter is only a
  backstop for anything larger than the negotiated cap.
- **Atomic bridge bindings.** The per-session bridge ref-count uses a lock + temp-file-rename and
  distinguishes an absent bindings file (empty) from a corrupt one (error), so teardown never
  removes a bridge another address still shares; a failed bind rolls back.
- **Per-session pipe secret.** The bridge mints a secret into its owner-only registry and rejects
  any push whose secret does not match (the default Windows named-pipe DACL grants Everyone READ,
  so the OS ACL alone does not restrict the pipe to the owner).
- **Store-correct disposition hints.** The on-deliver handler argv carries the session's
  `--backend` / `--db` selection, so the `ack` / `handle` hints in the pushed turn target the exact
  store even for named-backend / profile users.
- **Direct bridge-liveness signal.** The bridge heartbeats into its registry; the turn guard treats
  a push member whose registry heartbeat is stale as uncovered (bridge not loaded / live) and
  nudges to `extensions_reload`, rather than only inferring deafness from unacked backlog.
- **CI JS gate.** `node --check copilot/bridge/extension.mjs` runs in CI so a broken embedded
  bridge cannot ship baked into the binary.
- **Re-delivery is re-provision-triggered, not timer-churned.** An **accepted** push (already queued
  in the live session) is no longer re-pushed on the fast failure backoff; while the same session
  stays continuously attached it is not re-sent on that cadence -- only a long backstop may re-check
  it every few minutes if it stays unacked. Un-acked messages are re-delivered when the
  **attachment changes** -- a reattach, a
  `/clear` bridge-reload re-provision, or a new session taking the address -- which resets the
  attempt map and rescans `fetch_undelivered`. A **failed** push (dead / absent bridge) still retries
  on the fast backoff, and a long backstop covers the rare silent in-session drop of a queued turn.
  This removes the redelivery amplification a busy / slow recipient hit under the old fixed backoff
  (each re-push was a duplicate turn the agent had to dedupe) while preserving at-least-once: nothing
  is dropped, and the durable+ack fence is unchanged.
- **Bridge success is a bounded enqueue acknowledgement.** The bridge waits a short window for
  `session.send(...)` to return its message id (idle path), but if that promise is still blocked
  behind a long-running current turn, it writes `{ok:true, accepted:"pending"}` before the Rust
  handler's timeout. Without this, the daemon sees a false failed push, retries on the fast failure
  backoff, and a late SDK enqueue plus fast retry inject duplicate turns. The bridge **retains and
  observes** the pending `session.send(...)` promise until it settles, so the SDK request remains
  live after the socket response. Synchronous / quick `session.send` failures still return failure;
  asynchronous rejection after the pending ack is logged by the bridge and the durable long-backstop
  / re-provision path remains the recovery route.
- **Inbox is recovery, not the live push receive path.** With a fresh bridge heartbeat, unacked
  backlog can simply mean enqueue-mode turns are already queued behind the current turn. Agents
  should not proactively `inbox`+ack unseen messages in that state; doing so can consume the message
  before the already-queued turn arrives, making that later turn look like a duplicate. `telex inbox`
  remains the diagnostic/recovery path for stale bridge, reload/re-provision, degraded/backstop, or
  explicit operator intervention.

## Liveness / self-stop hardening (issue #66; folds in #62/#64/#67)

A later node hardened the liveness and stop edges of this push path (see DECISIONS ADR 0042):

- **A live push bridge reports live, not `unattended`/`deaf`.** Station health for a registered
  push station is derived from the daemon's own push-attempt outcomes (harness-neutral — the daemon
  never reads the bridge registry): a recent accepted push -> `attended_push` (structured
  `push_delivery: delivering`), a backlog with no attempt yet (e.g. post-restart) -> `probing`, an
  accepted push whose 300s backstop elapsed with no fresh accept -> `stale_accepted` (an
  earlier-than-deaf hint), and only actually-**failing** pushes -> `unattended_with_backlog`/`deaf`.
  A successful push is answerback that clears a stale deaf state. This fixes #64 (a live bridge was
  called `unattended`) and the persistent false-deaf of #66 without coupling the daemon to Copilot.
- **Actionable-inbound is reported distinctly from raw pending** (`inbound_actionable_count` vs
  `pending_unconsumed_count`), so a station whose only "pending" is no-disposition notes or
  shared-address traffic is not mistaken for having actionable backlog.
- **The re-push pool is bounded.** No-disposition notes are delivered once and skipped forever after
  accept; a still-unacked disposition-required message is re-pushed until a hard cap
  (`ON_DELIVER_MAX_REPUSH`) then suppressed (durable/readable, surfaced via `push_suppressed_count`).
  Consumed / terminally-dispositioned messages were already excluded from re-push.
- **Self-stop is durable and honored by the push helper.** A deliberate `telex copilot detach` (and
  `station stop`) records a **durable** detach tombstone, written atomically with the lease release
  (no separate follow-up write that could race a re-attach's clear). `telex copilot push` preflights
  the tombstone (via its baked `--backend`/`--db` selector) and refuses with the permanent exit code
  if the session was detached, so delivery stops and sticks — across a daemon restart and against a
  push racing member removal. The check is **fail-open** on a transient backend error (defense-in-depth;
  member removal is the primary steady-state stop), so the honored guarantee is weaker under backend
  faults: a push that raced member removal can still be delivered once if the tombstone lookup itself
  fails. `station stop` does **not** unload the in-session bridge extension, so its response reports
  `push_registered` and the CLI warns, pointing the operator at `telex copilot detach`.

## Idle drain: defer non-interrupt pushes until turn-stop (issue #65)

> This section is the authoritative narrative for the push scheduling state machine (deferred /
> accepted / failed). [DECISIONS.md ADR 0043](DECISIONS.md#0043--copilot-bridge-defers-non-interrupt-pushes-until-turn-stop-drained-by-an-ungated-agentstop-hook)
> is the decision record; it supersedes-and-links ADR 0041 where the busy path now differs.

ADR 0041's `accepted:"pending"` busy path still queued a non-`interrupt` turn behind the current
one. If the agent inspected Telex and acked/handled the message manually during that turn, the
queued turn arrived later as stale, already-handled work. Issue #65 replaces "queue while busy" with
"defer until the turn stops, then revalidate durable state and push."

- **Busy = the root-agent turn boundary.** The bridge tracks busy from `assistant.turn_start` /
  `assistant.turn_end`, **only for root-agent events** (`agentId` absent). A sub-agent's inner
  `turn_end` must not clear the gate while the parent turn runs. Full `session.idle` is intentionally
  not used: it also waits for background shells/sub-agents, which is stronger than #65 needs and can
  starve delivery after a long tool run. The bridge defaults to busy and self-heals to not-busy after
  a bound of no activity, so a missed `turn_end` (crash/abort, or a load outside any turn) does not
  defer forever.
- **Defer, don't send.** A non-`interrupt` push arriving while busy returns `deferred_until_idle`
  **without** calling `session.send`. `interrupt` (`immediate`) still sends immediately. A one-tick
  yield before answering lets a just-arrived `turn_end` settle, collapsing the drain-vs-`turn_end`
  race into a single non-deferred attempt.
- **Deferred is a distinct daemon outcome.** `telex copilot push` maps `deferred_until_idle` to
  `PUSH_EXIT_DEFERRED`; the daemon records a **deferred** attempt that is neither accepted (not
  queued, no CC lower-bound advance) nor a transient failure (no fast re-push, no error log), does
  not count toward the degraded-status threshold, and is held for `ON_DELIVER_DEFERRED_BACKSTOP`
  (invariant `HEARTBEAT_INTERVAL <= deferred < accepted backstop`). `telex status` reports a
  member's `push_deferred_count`, so deferred is diagnosable distinctly from accepted-unacked and
  failed-transient.
- **Turn-stop drains it.** A dedicated `agentStop` hook entry runs `telex copilot drain --session
  <id>`, independent of `TELEX_TURN_GUARD` / its nudge cap, with its own `TELEX_COPILOT_DRAIN`
  off-switch. It sends `DrainDeferred`; the daemon clears the deferred skip for the session's
  on-deliver members (leaving accepted attempts untouched, so a genuinely queued turn is not
  duplicated) and re-runs the on-deliver sweep. The sweep re-fetches `fetch_wait_candidates`, so a
  message acked before the drain is no longer a candidate and is skipped — the repro guarantee. The
  drain re-sweeps **every** on-deliver member of the session (matched by `session_id` across stores,
  so a named-`--backend`/`--db` session still drains), which closes a deferred-vs-drain inflight race
  and opportunistically re-attempts messages whose backstop elapsed; the only zero-work fast path is
  client-side (`telex copilot drain` skips the daemon round-trip when the session has no bridge
  registry). The drain returns before the sweeps complete and has a client-side deadline below the
  hook timeout, so it never blocks turn-stop. The daemon stays harness-neutral: it re-runs a generic
  sweep on request; "busy/idle" lives only in the bridge.
- **No loss.** If the drain hook is missed or races a still-busy bridge, the deferred backstop +
  heartbeat sweep re-attempt within a bounded delay (a re-defer while busy is cheap and injects no
  stale turn); re-provision (reattach / `/clear` reload) still re-delivers unacked backlog. Durable
  Telex state remains authoritative throughout.
