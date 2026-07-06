# telex copilot bridge (prototype)

Proof-of-code for the **load-on-bind push delivery** path described in
`docs/design/copilot-bridge-push.md` (issue #53). It lets the telex daemon push
a message into a live Copilot CLI session as a real turn, with no agent-managed
`telex wait` waiter and no `--ui-server` flag.

## Files

- `extension.mjs` -- the in-session bridge. A Copilot CLI extension that
  `joinSession()`s, opens a per-session OS named pipe
  (`\\.\pipe\telex-bridge-<sessionId>` on Windows; unix socket on POSIX), and on
  each connection injects the supplied prompt via `session.send(...)`. Writes a
  registry entry at `~/.copilot/telex-bridge/<sessionId>.json`. This is the
  prototype of the bytes telex would embed (`include_str!`) and write on
  `telex --address <addr> copilot attach --copilot-bridge`.
- `push.mjs` -- a prototype/debug reference for the daemon on-deliver handler
  (`telex copilot push`). It connects to a session's bridge, hands off one message,
  and prints the result. The shipped Rust handler supersedes it and **derives** the
  endpoint from the session id (using the registry only for liveness / session
  ownership), so treat `push.mjs` as a wire-protocol reference, not an endpoint oracle.
- `peer-prompt.txt` -- seed prompt for a peer Copilot terminal in the
  cross-terminal proof.

## Wire protocol

One JSON request per connection, newline-terminated; one JSON response,
newline-terminated.

```
-> {"prompt":"...","displayPrompt":"[telex] from <addr> (<attention>)","mode":"enqueue"}
<- {"ok":true,"sessionId":"...","messageId":"...","mode":"enqueue"}
```

Trust boundary is the same-user OS pipe/socket ACL (no bearer token at rest).
The endpoint is derived from the session id, so it is stable across `/clear`
reloads.

## Reproduce the proofs

Self-push (inject into your own session):

```powershell
# 1. load the bridge into this session
$sid = $env:COPILOT_AGENT_SESSION_ID
$dst = Join-Path $HOME ".copilot\session-state\$sid\extensions\telex-bridge"
New-Item -ItemType Directory -Force -Path $dst | Out-Null
Copy-Item .\extension.mjs (Join-Path $dst "extension.mjs") -Force
# 2. call the extensions_reload tool from the agent, then:
node .\push.mjs --session $sid --prompt "hello from the bridge" --display "[telex] self test"
# the prompt arrives as your next turn
```

Cross-terminal (push from session A to session B):

```powershell
# in session B, load the bridge (copy extension.mjs into its session dir + extensions_reload),
# then from session A:
node .\push.mjs --latest --prompt "Write BRIDGE-PUSH-OK to .\peer-proof.txt" --display "[telex] cross-terminal"
```

## Findings from the live runs

- **Runtime self-load works.** Dropping `extension.mjs` into the session
  extension dir and calling `extensions_reload` forks the bridge live, same
  turn; non-telex sessions never load it.
- **Push delivers as a queued turn**, non-interrupting (enqueue mode).
- **A client-side push timeout does NOT imply non-delivery.** In an early run the
  push client timed out waiting for a response (a framing bug) yet the message
  was still injected. The real `telex copilot push` must be at-least-once and let
  the agent disposition; never ack on push. (Fixed here with newline-delimited
  framing where the server responds then closes.)
- **`displayPrompt`** renders a clean timeline label.

## Relationship to the shipped implementation

These are the reference bytes: `extension.mjs` is embedded in the telex binary
(`include_str!`) and written into the session extension dir on
`telex copilot attach --copilot-bridge`. The daemon's generic on-deliver exec
primitive and the `telex copilot push` Rust handler (which supersedes `push.mjs`)
drive delivery; `push.mjs` remains as a wire-protocol reference and debugging
tool. See `docs/design/copilot-bridge-push.md`.
