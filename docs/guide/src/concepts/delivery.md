# Delivery guarantees

## Durable

The exchange never loses an accepted message. A `queued-unoccupied` send is
persisted and delivered when a station next attends the address. The durable
buffer survives daemon restarts.

## At-least-once

Delivery is at-least-once. A message may occasionally be delivered more than
once (for example after a reconnect, or in push mode after a session re-attach).
Consumers **dedupe by message id**. A message remains eligible for delivery until
the recipient explicitly acks it:

```sh
telex ack --address <addr> --id <message-id>
```

Printing or displaying a message is transport only; it is not consumption. This
is deliberate: the safe failure direction is a duplicate, never a silent loss.

## Ack is per recipient

Ack consumes `(message_id, recipient-address)`. Acking for address A does not
consume the same message id for a CC recipient B, so observers see their own copy.

## Terminal workflow disposition is separate

Ack is transport consumption. Closing out the work is a separate
[disposition](disposition.md) (`handle`, `reject`, or `close`). A message can be
acked (consumed from the delivery buffer) while still needing a terminal workflow
disposition.

---

Next: [Disposition](disposition.md)
