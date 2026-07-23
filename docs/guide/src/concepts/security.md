# Security and data

## Where data lives

- **Local SQLite store:** `~/.telex/telex.db` (override with `--db` or `$TELEX_DB`).
- **Config (named backends):** `~/.telex/config.toml`.
- **Runtime directory (daemon IPC and lease state):** a per-user runtime directory
  (on Windows under `%LOCALAPPDATA%\telex\run`; on Unix a per-user socket/runtime
  directory).
- **Copilot bridge extension:** in the Copilot session's extension directory.
- **Copilot bridge registry and bindings:** under the Copilot home `telex-bridge`
  directory. Durable files are retained across resumable session end and removed by
  final-binding `telex copilot detach`, fallback/rollback cleanup, or `telex copilot gc`.

## Trust model

The local exchange serves a single operating-system user over local IPC. A
same-user process operates under the current trust model. Do not place a telex
store or socket where other users can read it if the messages are sensitive.

## Secrets

Postgres passwords are referenced, never written to the config file: use
`--entra`, `--password-env`, or `--password-command`. `telex backend show`
redacts secrets.

## Message content

Message bodies, subjects, metadata, and disposition history are stored in the
selected backend and are readable by anyone with access to that store (the local
file, or the Postgres schema). Treat the store as sensitive if the messages are.

## Postgres sharing

A Postgres backend is shared across the machines and users configured to use it.
Scope access with database and schema grants, and use a dedicated `--schema`.

## Reporting a vulnerability

Report security issues privately, not through public issues or pull requests. See
the repository
[security policy](https://github.com/lossyrob/telex/blob/main/SECURITY.md) for the
private reporting channel and what to expect.

---

Next: [Glossary](glossary.md)
