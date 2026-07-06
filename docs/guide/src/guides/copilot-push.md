# Copilot CLI push delivery

In GitHub Copilot CLI, telex delivers messages to the agent as **turns**. The
agent does not run or re-arm a waiter. The full, version-matched Copilot workflow
is printed by the installed binary:

```sh
telex copilot skill
```

That command is the source of truth for the Copilot path. This guide is the
operator's overview.

## Install the plugin

```sh
copilot plugin marketplace add lossyrob/telex
copilot plugin install telex@telex
```

The plugin contributes session lifecycle hooks and provisions the push bridge. It
maps `$COPILOT_AGENT_SESSION_ID` to the generic telex session id and
`$COPILOT_LOADER_PID` to a loader watch-pid.

## Bind and provision the bridge

```sh
telex --address <addr> copilot attach --copilot-bridge --description "<work>"
```

Then run the `extensions_reload` tool once (the agent does this; telex cannot
trigger a reload). After that, delivered telex messages arrive as new turns
labelled `[telex] from <addr> (<attention>)`.

## Receive and disposition

A pushed turn includes the `ack` and `handle` commands with the address, id, and
session filled in. The generic verbs do not read Copilot env vars, so they need
`--session "$COPILOT_AGENT_SESSION_ID"`:

```sh
telex ack --address <addr> --id <message-id> --session "$COPILOT_AGENT_SESSION_ID"
telex handle --address <addr> --id <message-id> --session "$COPILOT_AGENT_SESSION_ID" --note "completed"
```

Sending is not push: `telex send` and `telex reply` also need `--session`.

## Tear down

```sh
telex --address <addr> copilot detach
```

This detaches the address and, when it was the last binding, removes the bridge
files so nothing reloads on a later resume. Session end also removes them.

## Fallback

If the bridge cannot load (extensions disabled), push is unavailable. Surface that
plainly or fall back to generic [pull mode](agent-pull.md); do not silently spin a
waiter.

## Compatibility

`telex copilot skill` prints the installed version, the bridge protocol version,
and the minimum compatible plugin version, and warns if the plugin is older than
the binary supports. Release install scripts pin the plugin and binary to the same
tag. The plugin shape is validated against a specific Copilot CLI version; see the
[acceptance matrix](https://github.com/lossyrob/telex/blob/main/docs/design/copilot-plugin-validation.md).
