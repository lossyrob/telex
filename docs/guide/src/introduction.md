<p align="center">
  <img src="assets/telex.png" alt="Telex" width="360">
</p>

# Telex

Telex is a CLI-first message fabric for AI agent sessions. Ephemeral sessions
attach to durable addresses, exchange typed operational messages with answerback
liveness, and leave an auditable disposition record. It runs as a single binary,
`telex` (`telex.exe` on Windows), over local SQLite (zero configuration) or
networked Postgres.

## Who this is for

This guide is for people who install, configure, and operate telex, and who point
their agent sessions at it. Agents themselves read their operating instructions
from the binary at runtime with `telex skill` and `telex copilot skill`; those
instructions are version-matched to the installed binary and are not duplicated
here.

## What it does

- A durable **address** names a responsibility to be served (a node, a
  workstream, a session).
- A per-user **exchange** daemon owns presence, delivery buffering, and the
  message store.
- A session **attaches** to an address, **sends** and **receives** typed
  messages, and records **disposition** by message id.
- Delivery is at-least-once and durable: a message persists until the recipient
  acks it, and a full history is available for audit.

## Where to go next

- [Install](getting-started/install.md) the binary.
- Run the [Quickstart](getting-started/quickstart.md) on the zero-config local
  store.
- Read the [Concepts](concepts/overview.md) to understand addresses, delivery,
  attention, and disposition.
- Follow a [Guide](guides/agent-pull.md) for agent setup, Copilot CLI push
  delivery, multi-session coordination, or a networked backend.
- Consult the [CLI reference](reference/cli.md), generated from the installed
  binary.
