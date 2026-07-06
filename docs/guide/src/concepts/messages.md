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

`send` prints a receipt: `delivered` or `queued-unoccupied`, plus the new message
id. A `queued-unoccupied` receipt is durable: the message is persisted and
delivered when a station next attends the address. Sending to a retired address
is an error (`address <addr> is retired`), not a receipt.

## Threads and replies

Reply under a parent message; the reply threads under it and routes to the
parent's sender:

```sh
telex reply --to-message <message-id> --body "<body>"
```

## CC (observers)

Add `--cc <addr>` to copy observer addresses on a message. Each recipient gets
its own delivery of the same message id, with a `delivery_role` of `to` or `cc`:

```sh
telex send --to node:worker --cc node:lead --subject "Status" --body "..."
```

The primary recipient and each CC observer see the same `id`, `thread_id`, and
`primary_to`, but their own `delivery_role` and `delivered_to`:

| Recipient | delivery_role | delivered_to | requires disposition |
|---|---|---|---|
| `node:worker` (to) | `to` | `node:worker` | set when the sender passes `--requires-disposition` |
| `node:lead` (cc) | `cc` | `node:lead` | no |

`--cc` may be repeated and accepts comma-separated values. Ack and disposition
are per recipient: acking for `node:lead` does not consume the copy for
`node:worker`. An observer reads its copy with `telex inbox --all` or
`telex read`, and acks its own `(message_id, address)`.

## Reading

```sh
telex inbox --address <addr>              # actionable messages (add --all for recent)
telex inbox --address <addr> --all --limit N
telex read --id <message-id> --thread     # a message with compact thread context
telex read --id <message-id> --full       # full history
```

---

Next: [Attention levels](attention.md)
