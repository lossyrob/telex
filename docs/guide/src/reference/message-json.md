# Delivered message shape

`telex wait` prints one delivered message as JSON on exit 0. With `--out-dir` it
writes the same message to `message.json`, and an envelope form to
`delivery.json` (`{ "message": {...}, "delivery": {...}, "status": {...} }`). A
pull-mode script or harness parses this to drive an agent.

## Example

Output of `telex wait --json` for one delivered message:

```json
{
  "id": 118,
  "thread_id": 118,
  "parent_id": null,
  "from": "demo:jsondoc",
  "to": "demo:jsondoc",
  "primary_to": "demo:jsondoc",
  "delivered_to": "demo:jsondoc",
  "delivery_role": "to",
  "cc": [],
  "kind": "note",
  "attention": "next-checkpoint",
  "requires_disposition": true,
  "requires_disposition_for_current_recipient": true,
  "subject": "Hello",
  "body": "world",
  "sent_at_ms": 1783355088938,
  "buffered_at_ms": 1783355089013,
  "lease_epoch": 1
}
```

## Fields

| Field | Meaning |
|---|---|
| `id` | Message id. Use it with `ack`, `handle`, `read`, and for dedupe. |
| `thread_id` | Root message id of the thread. |
| `parent_id` | Parent message id for a reply, otherwise `null`. |
| `from` | Sender address; a `reply` routes here. |
| `to` | The address the message was sent to. |
| `primary_to` | The primary recipient address. |
| `delivered_to` | The recipient address this delivery is for. |
| `delivery_role` | `to` or `cc`: this recipient's role in the message. |
| `cc` | CC (observer) addresses. |
| `kind` | Message kind label (default `note`). |
| `attention` | `interrupt`, `next-checkpoint`, `background`, or `fyi`. |
| `requires_disposition` | Whether the sender marked the message as requiring disposition. |
| `requires_disposition_for_current_recipient` | The same, scoped to this recipient. |
| `subject` | Subject, if any. |
| `body` | Message body. |
| `sent_at_ms`, `buffered_at_ms` | Unix millisecond timestamps. |
| `lease_epoch` | The recipient station's lease epoch at delivery. |

Fields whose names end in `_ms` such as `backend_ms`, `send_to_exit_ms`, and
`waiter_exit_ms` are timing diagnostics, not message content.

## Consume by id

Delivery is at-least-once, so dedupe by `id` and ack it to consume it from the
delivery buffer:

```sh
telex ack --address <addr> --id <id> --session <session-id>
```

## Files written by `--out-dir`

- `message.json`: the flat delivered message (exit 0 only).
- `delivery.json`: the envelope `{ message, delivery, status }` (exit 0 only).
- `status.json`: `{ outcome, exit_code, detail, ... }` (always).
- `exit.code`: the integer exit code, written last as the completion marker.

See [Exit codes](exit-codes.md).
