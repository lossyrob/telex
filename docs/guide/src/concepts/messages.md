# Messages and threads

A telex message is a typed operational message with a sender (`from`), a
recipient (`to`), an optional subject, and a body. Messages can carry CC
recipients (visible observers), a `kind` label, an
[attention level](attention.md), and arbitrary metadata.

## Sending

```sh
telex send --to <addr> --subject "<subject>" --body "<body>"
telex send --to <addr> --subject "<subject>" --body-file <path>   # UTF-8 file; - for stdin
```

`--body` and `--body-file` are mutually exclusive and exactly one is required.
Prefer `--body-file` for multiline or structured content (Markdown, code blocks,
JSON) to avoid shell quoting limits. The file is read as UTF-8 and sent exactly
as written.

`send` prints a receipt: `delivered`, `queued-unoccupied`, or `rejected-retired`,
plus the new message id. A `queued-unoccupied` receipt is durable: the message is
persisted and delivered when a station next attends the address.

## Threads and replies

Reply under a parent message; the reply threads under it and routes to the
parent's sender:

```sh
telex reply --to-message <message-id> --body "<body>"
```

## Reading

```sh
telex inbox --address <addr>              # actionable and recent messages
telex inbox --address <addr> --all --limit N
telex read --id <message-id> --thread     # a message with compact thread context
telex read --id <message-id> --full       # full history
```
