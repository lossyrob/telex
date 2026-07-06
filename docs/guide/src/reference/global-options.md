# Global options

Global options apply to all subcommands and are set before the subcommand, for
example `telex --backend prod inbox`.

| Option | Purpose |
|---|---|
| `--backend <name>` | Use a configured backend by name (or `$TELEX_BACKEND`). Defaults to the configured default backend, or an implicit `default` SQLite store. |
| `--db <path>` | Override the SQLite path for this invocation (SQLite backends only; or `$TELEX_DB`). |
| `--address <addr>` | Default address (or `$TELEX_ADDRESS`) for commands that act on one address; also a `from` fallback for `send` and `reply`. |
| `--json` / `--text` | Output format. Defaults to JSON when stdout is not a TTY, text when interactive. |

## Relevant environment variables

- `TELEX_SESSION_ID` — stable session identity for daemon membership.
- `TELEX_ADDRESS` — default address.
- `TELEX_BACKEND` — default backend by name.
- `TELEX_DB` — SQLite path override.

Postgres connections are configured once as named backends with
`telex backend add` (see [Backends](../concepts/backends.md)), not through
per-call environment variables.

For the full, version-matched flag set of any command, run its `--help`; see the
[CLI reference](cli.md).
