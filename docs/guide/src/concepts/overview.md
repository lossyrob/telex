# Concepts overview

Telex has a small set of concepts. This page names them; the following pages
cover each in detail.

- **[Address](addresses.md)**: a durable name for a responsibility. Sessions
  attach to addresses; messages are sent to addresses.
- **[Exchange and daemon](exchange.md)**: a per-user local process that owns
  presence, delivery buffering, and the message store, reached over local IPC.
- **[Stations and presence](presence.md)**: the in-memory registration a session
  holds while it attends an address, with a liveness lease.
- **[Messages and threads](messages.md)**: typed messages with a sender, body,
  and thread context; replies thread under a parent.
- **[Attention](attention.md)**: how urgent a message is, from `interrupt` to
  `fyi`, which controls when the recipient sees it.
- **[Disposition](disposition.md)**: the auditable outcome recorded for a
  message: acknowledged, handled, deferred, rejected, closed, or escalated.
- **[Delivery guarantees](delivery.md)**: messages are durable and delivered
  at-least-once; consumption is an explicit ack.
- **[Backends](backends.md)**: the configured store: local SQLite by default, or
  networked Postgres.

## The shape of a session

A session has a stable identity (`--session` or `$TELEX_SESSION_ID`). It
`attach`es to one or more addresses, `send`s and `receive`s messages, records
`disposition` by message id, and detaches or stops its station when done. The
exchange retains the durable message buffer across daemon restarts.
