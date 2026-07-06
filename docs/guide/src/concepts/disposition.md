# Disposition

**Disposition** is the auditable outcome recorded for a message. It is separate
from transport: receiving a message does not disposition it, and dispositioning a
message is an explicit, recorded act.

## Transport consumption: ack

`ack` marks the delivered `(message_id, recipient-address)` consumed in the
exchange delivery buffer. Ack is per recipient, so acking a message for address A
never consumes the same message id for a CC recipient B.

```sh
telex ack --address <addr> --id <message-id>
```

## Workflow disposition states

```text
acknowledged | handled | deferred | rejected | closed | escalated
```

- **Terminal** (removed from the actionable inbox): `handled`, `rejected`,
  `closed`.
- **Non-terminal** (still needing final disposition): `acknowledged`,
  `deferred`, `escalated`.

Record a workflow disposition with the matching verb, optionally with a note:

```sh
telex handle --id <message-id> --note "completed"
telex defer --id <message-id> --note "waiting on input"
telex reject --id <message-id> --note "out of scope"
telex close --id <message-id>
telex escalate --id <message-id> --note "needs operator"
```

Dispositions default to the current `--address` recipient. Pass `--recipient`
only to record for another recipient intentionally.

## Ordering

Ack first (transport consumption), then apply the workflow disposition that
reflects the actual outcome. A message that requires disposition
(`--requires-disposition` on send) stays actionable until it reaches a terminal
state.

---

Next: [Backends](backends.md)
