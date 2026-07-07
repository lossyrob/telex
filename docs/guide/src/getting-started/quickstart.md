# Quickstart

No server setup and no configuration are required. The first daemon-backed
command auto-spawns a per-user local exchange over a SQLite store at
`~/.telex/telex.db`.

## Send and read a message

Set a stable session id once, then send a message to yourself and read it back:

```sh
export TELEX_SESSION_ID=quickstart   # PowerShell: $env:TELEX_SESSION_ID = "quickstart"
telex --address me send --to me --body "hello"
telex --address me inbox --all
```

`send` needs a session id (from `--session` or `$TELEX_SESSION_ID`) and a sender
address. Here `--to me` is the recipient and the global `--address me` supplies
the sender. `inbox --all` lists recent messages; the default `inbox` lists only
actionable ones (messages requiring disposition).

## Print the agent usage guide

The binary carries its own runtime instructions for agents. Print them with:

```sh
telex skill
```

This is the generic (pull) workflow. In Copilot CLI, run `telex copilot skill`
for the push-delivery workflow instead.

## A two-session taste

One session attaches to an address and waits; another finds it and sends. Run
these in two terminals.

Session A registers an address and waits for one message:

```sh
export TELEX_SESSION_ID=session-a   # PowerShell: $env:TELEX_SESSION_ID = "session-a"
telex attach --address session:a --description "session A waiting for coordination"
telex wait --address session:a
```

Session B registers, finds A by its description, and sends:

```sh
export TELEX_SESSION_ID=session-b   # PowerShell: $env:TELEX_SESSION_ID = "session-b"
telex attach --address session:b --description "session B requesting status"
telex resolve --match "waiting for coordination"
telex send --to session:a --subject "Status request" --body "Please send status." --attention interrupt
```

Session A's `wait` returns the message as JSON. A then acks it, dispositions it,
and can reply in the same thread:

```sh
telex ack --address session:a --id <message-id>
telex handle --id <message-id> --note "status prepared"
telex reply --to-message <message-id> --body "Holder is live; continuing work."
```

See [Coordinate multiple sessions](../guides/multi-session.md) for the full
pattern, including the re-arm loop and attention phases.
