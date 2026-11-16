# Claude Code channel spike

Question: can Telex push messages into a *running* Claude Code session the way the
Copilot bridge does? Answer being tested: yes, via a Claude Code
[Channel](https://code.claude.com/docs/en/channels-reference), an MCP stdio server that
declares `experimental['claude/channel']` and emits `notifications/claude/channel`.

This is a throwaway spike, not a product path. No Telex daemon integration yet.

## Pieces

| File | Role |
|---|---|
| `channel.mjs` | Channel MCP server. Claude Code spawns it. Listens on `~/.telex/claude-channel/<id>.sock` (0600, dir 0700) and forwards each JSON line as a channel event. |
| `push.mjs` | Stand-in for `telex claude push`: writes one message to the socket. |
| `handshake.test.mjs` | Fake Claude Code speaking raw JSON-RPC. Asserts capability + push -> notification. |
| `run-cli.sh` | Generates an MCP config and launches Claude Code with the dev-channel flag. |

Real design would be: daemon on-deliver exec -> `telex claude push` -> socket -> channel.
Disposition stays with the agent (`telex ack`/`handle`); the channel never acks.

## Run

```sh
npm install
node handshake.test.mjs                      # protocol check, no Claude needed

# Live. Channels need Claude Code >= 2.1.80. Use the Desktop-bundled binary if PATH `claude` is old.
TELEX_CHANNEL_HELLO=1 CLAUDE_BIN=/path/to/claude ./run-cli.sh
# accept "I am using this for local development"; a self-test message arrives after ~1.5s.

node push.mjs --from team/alice --subject "build" "CI is red on main"   # second message
```

The flag is interactive-only (ignored with `-p` / Agent SDK) and Pro/Max accounts need no
org setting; Team/Enterprise need `channelsEnabled`. If the server is not loaded as a
channel, events are dropped **silently**.

## Desktop / Code tab

Docs only describe the terminal CLI. To test Desktop: add the server to a project
`.mcp.json` (`node <abs path>/channel.mjs`) and check whether the Code tab offers any way
to pass `--dangerously-load-development-channels`. If it does not, channels are CLI-only
for now and Desktop would need a different path.

## Findings

Live run, 2026-11-15, Claude Code 2.1.325 (Desktop-bundled binary), macOS:

- **Works:** the dev-channel flag loads the server. Both the self-test message and a
  message pushed with `push.mjs` were accepted by the socket, rendered in the TUI as
  `← telex: <body>`, and each started a new turn in the idle session.
- **Not verified:** whether the model saw the `meta` attributes and followed the
  instructions, and the reply/ack loop. Both turns failed with "Login expired / Not logged
  in": the Desktop-bundled binary run from a shell does not share the Desktop app's auth.
  Run `/login` in the session and re-push to close this gap.
- **After `/login`:** a third message (m3, asking the model to state the `id`/`from`/
  `subject` attributes and whether it treats the body as a user instruction) woke the
  session and the model began working. Its first action was a read-only
  `telex status` shell command, so the channel `instructions` (which mention the `telex`
  CLI) steer behaviour as intended. Its final answer (pasted by the operator) confirmed:
  - all three queued messages arrived with `id`, `from` and `subject` readable as
    attributes (the `meta` mapping works);
  - it treated the bodies as untrusted data from another sender, not user instructions,
    and acted on none of them (including m3, which asked it that question directly);
  - it did **not** ack or reply, because the channel event carries no destination
    address and the session had no default one. **Design implication:** each event must
    include `to` (the Telex address) and ready-to-run `ack`/`handle` commands, as the
    Copilot pushed turn does, or the agent cannot disposition correctly.
  - events that arrived while the session was unauthenticated were queued and delivered
    once it could run a turn, so nothing was lost across the login gap.
- **PATH `claude` was 2.1.31**, too old for channels. Version skew would be a real
  failure mode for `telex claude ...` guidance; the binary should check the version.
- **Desktop Code tab: very likely not usable today (probed, not run end to end).**
  - Desktop spawns the same bundled binary (2.1.325) but in SDK mode:
    `--output-format stream-json --input-format stream-json --permission-prompt-tool=stdio
    --await-initialize ...`. None of the 10 Desktop-launched processes seen carried any
    `--channels` / `--dangerously-load-development-channels` flag.
  - Channels are enabled only by a per-launch flag; there is no settings key that opts a
    session in (`channelsEnabled` / `allowedChannelPlugins` are org policy gates).
  - The docs say the dev flag is **ignored under `-p` and the Agent SDK**, which is the
    mode Desktop uses, and Desktop exposes no UI to pass it.
  - So a Telex channel would only work via the **terminal CLI** unless Anthropic adds Desktop
    support or Telex is on the approved allowlist and Desktop passes `--channels`.
    Re-check on newer Desktop builds.
  - Not tried: unpacking `app.asar` to read Desktop's launch code (blocked by the
    permission classifier; not needed for this conclusion).
