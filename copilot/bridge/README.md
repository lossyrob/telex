# telex copilot bridge

> **Status: shipped.** `extension.mjs` is the canonical bridge source embedded into the
> `telex` binary via `include_str!` in `src/commands/copilot.rs` and written into a
> session's extension dir on `telex copilot attach --copilot-bridge`. This directory
> (`copilot/bridge/`) is binary-embedded source, not a marketplace-distributed plugin
> file. `push.mjs` remains a wire-protocol reference/debug tool; the shipped Rust
> `telex copilot push` handler supersedes it. See `docs/design/copilot-bridge-push.md`.

Describes the **load-on-bind push delivery** path in
`docs/design/copilot-bridge-push.md` (issue #53). It lets the telex daemon push
a message through an already-loaded, live bridge in Copilot CLI or Copilot App as a real turn, with no agent-managed
`telex wait` waiter and no `--ui-server` flag.

First-time loading uses a supported live bridge loader or startup extension
discovery. App's `install_extension` is also supported: install this folder
from the published commit matching the installed binary **before**
`copilot attach --copilot-bridge`, with `scope: "session"` and
`name: "telex-bridge"`. `telex copilot skill` prints pinned tool arguments.
The installer reloads extensions but refuses an existing extension name.
Never use floating `main` or a second bridge name. Keep an already-live bridge;
for an unloaded existing copy, use supported reload/startup discovery or the
[explicit binding teardown procedure](../COPILOT.md#app-session-installer).
Do not delete shared bridge files to force installation.

For App setup without a usable installer or advertised live loader, provision with
`telex --address <addr> copilot attach --copilot-bridge`, restart the App session,
then run `telex --address <addr> copilot resume`. Verify
`telex --address <addr> --json status` reports `station_health: attended_push` and
`push_registered: true`; where available, call `telex_bridge_info` for supporting
bridge evidence. Keep push when the bridge is already live.

Provisioning files is not loading proof, and a restart is not proof by itself.
[Issue #230](https://github.com/lossyrob/telex/issues/230) reports startup loading
and pushed delivery on one Windows App build, not every App build or a plain
conversation clear. Liveness and end-to-end delivery are separate checks.
When a live bridge is unavailable and a restart is unacceptable or liveness does
not verify, follow `telex copilot skill` and `copilot fallback prepare` for the
Telex-owned single-shot pull path. Fallback preparation is not wakeup proof;
report setup/backend failures.

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

Access uses the OS pipe/socket permissions and a bearer secret stored in the
registry. Unix protects that file; Windows uses the profile policy described below.
The endpoint is derived from the session id, so it is stable across `/clear`
reloads.

### Windows trust boundary

The bridge is a same-user integration, not a sandbox boundary. On Windows,
Node inherits the surrounding profile's permissions for the bridge directory,
registry secret and binding metadata. Telex does not inspect or repair their
owner/DACL. Directory redirects are allowed; credential readers still enforce
regular non-reparse files, a size cap and same-handle metadata, with containment
checked during daemon recovery. Daemon-owned capabilities/intents and Unix
producer permissions retain their strict checks.

A principal that can read the registry learns its bearer secret. If that
principal can also write to the session's named pipe, it can inject prompts into
that Copilot session. Profile ACLs and Windows pipe access control determine who
can do that; this release does not promise credential secrecy from foreign
readers, isolation from a compromised same-user process, or explicit rejection of
remote pipe clients. Rust verifies the endpoint's user, PID, start time and
executable before sending. Those checks prevent a different-user server from
receiving a credential, but are not authentication of incoming prompt senders.
A writer that can replace the registry can disrupt service; combined with control
of a same-user server at the session-derived endpoint it can impersonate the
producer. Containment and shape checks do not prevent that.

The three bridge modules are byte-identical to RC1: there is no additional helper
process, native compilation, ACL pinning or module-staging/reload requirement.
An already-loaded RC1 bridge can remain loaded. Upgrade/restart the owning
daemon as well: an old daemon retains its old reader policy.

## Reproduce the proofs

These CLI proof recipes require the `extensions_reload` tool. On Copilot CLI,
enable Copilot Extensions under `/experimental` if needed to expose it. They are
not the App startup-discovery procedure described above.

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

- **Runtime self-load works on the tested loader-capable CLI host.** Dropping `extension.mjs` into the session
  extension dir and calling `extensions_reload` forks the bridge live, same
  turn; non-telex sessions never load it.
- **The same session-scoped extension supports Copilot App.** A separate
  App-only extension would duplicate the pipe, authentication, and turn-injection
  protocol without adding lifecycle information. App compatibility instead lives
  at the hook boundary: `sessionEnd(reason=complete)` preserves attendance while
  this extension's heartbeat remains live, then binds daemon membership to the
  extension host PID. The host remains stable across extension reloads and dies
  on a true App or one-shot CLI exit. These lifecycle semantics assume a loaded
  bridge; they do not establish that every App session can load one.
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
