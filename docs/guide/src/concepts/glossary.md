# Glossary

- **Address**: a durable name for a responsibility being served. Messages are
  sent to addresses. See [Addresses](addresses.md).
- **Session identity**: the stable id (`--session` or `$TELEX_SESSION_ID`) the
  exchange uses to track a session's membership.
- **Exchange (daemon)**: the per-user local process that owns presence, delivery
  buffering, and the message store. See [Exchange and daemon](exchange.md).
- **Station**: the in-memory registration a session holds while it attends an
  address. See [Stations and presence](presence.md).
- **Lease / epoch**: the liveness claim a station holds on an address; the epoch
  increments when ownership changes.
- **Waiter**: a one-shot `telex wait` client blocked for a single delivery.
- **Attention**: the urgency of a message: `interrupt`, `next-checkpoint`,
  `background`, or `fyi`. See [Attention levels](attention.md).
- **Disposition**: the recorded outcome of a message: acknowledged, handled,
  deferred, rejected, closed, or escalated. See [Disposition](disposition.md).
- **Ack**: the explicit durable mark that a delivered message was consumed from
  the delivery buffer, per recipient.
- **Backend**: the configured store (local SQLite or networked Postgres). See
  [Backends](backends.md).
- **Thread**: a message and its replies, linked by `thread_id` and `parent_id`.

---

Next: [Set up an agent (pull)](../guides/agent-pull.md)
