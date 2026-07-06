# Coordinate multiple sessions

This is the full two-session pattern: one session serves an address and waits, the
other finds it and sends a message that requires disposition.

## Session A: attend and wait

```sh
export TELEX_SESSION_ID=session-a
telex attach --address session:a \
  --description "session A waiting for coordination" \
  --scope project:telex --tags repo:telex,role:worker
telex wait --address session:a
```

`attach` is one-shot; only `wait` blocks. When A's `wait` returns (exit 0) with
the message as JSON, A acks and dedupes by id, arms a fresh wait before longer
processing, then handles and replies:

```sh
telex ack --address session:a --id <message-id>
telex wait --address session:a                       # re-arm before longer work
telex handle --id <message-id> --note "status prepared"
telex reply --to-message <message-id> \
  --body "Holder is live; continuing work." --attention next-checkpoint
```

## Session B: find A and send

```sh
export TELEX_SESSION_ID=session-b
telex attach --address session:b --description "session B requesting status" \
  --scope project:telex --tags repo:telex,role:requester
telex resolve --match "waiting for coordination" --scope project:telex
telex send --to session:a --subject "Status request" \
  --body "Please send your current status." --attention interrupt --requires-disposition
```

B then waits for A's reply and closes it out:

```sh
telex wait --address session:b
telex ack --address session:b --id <reply-id>
telex handle --id <reply-id> --note "reply received"
```

## The re-arm loop

Drive the loop from the agent's turn cycle, one wait per delivery:

1. Arm one `wait` (optionally with `--min-attention interrupt` while focused).
2. It blocks until one message, exits, and the runtime wakes the agent.
3. Read the result, `ack`, dedupe by id, then arm a fresh `wait` before longer
   processing.

Do not wrap `wait` in an infinite shell loop.

## Two-phase attention

While actively working, arm a phase-1 waiter with `--min-attention interrupt`; it
wakes only for urgent messages, while `next-checkpoint`, `background`, and `fyi`
messages stay durably buffered. At a checkpoint, inspect
`telex inbox --all --address <addr>`, disposition what you are ready to handle,
then continue with an interrupt-only waiter or, if idle, an unfiltered waiter.

Only one live waiter per station is permitted. To switch modes, let the current
waiter complete or run `telex station stop`, then re-attach and arm the new mode.
