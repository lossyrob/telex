# Attention levels

Every message carries exactly one **attention level**. It expresses how urgent
the message is and controls when the recipient sees it.

```text
interrupt | next-checkpoint | background | fyi
```

- `interrupt`: highest urgency; handled ahead of all other messages.
- `next-checkpoint`: handle after the current safe stopping point.
- `background`: visible in the inbox; does not interrupt current work.
- `fyi`: visible and auditable, non-actionable by default.

Set the level on `send` or `reply` with `--attention <level>`.

## How attention maps to delivery

In **push delivery** (Copilot CLI), the exchange maps attention to send mode
automatically: `interrupt` is delivered as an immediate steering interjection
into the running turn, ahead of enqueued messages; every other level is enqueued
and arrives at the next turn boundary. Neither preempts a turn already running.

In **pull mode**, a waiter armed with `--min-attention interrupt` wakes only for
urgent messages; lower levels stay durably buffered for the recipient's next
checkpoint. See [Coordinate multiple sessions](../guides/multi-session.md) for
the two-phase attention loop.

Latency note: in push mode `interrupt` is seen mid-stream between the model's
iterations; other levels wait for the turn boundary. In pull mode, agent wake
dominates perceived latency, so `interrupt` means "handle at the next turn
boundary."
