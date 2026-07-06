# Backends

A **backend** is a named, configured store. Selection is by name:
`--backend <name>`, then `$TELEX_BACKEND`, then the configured `default`, then an
implicit `default` SQLite store at `~/.telex/telex.db`. With no setup, telex works
on local SQLite.

## SQLite (default)

Local, zero-config, single-user. Nothing to configure; the implicit default store
is created on first use. Override the path for one invocation with `--db <path>`
or `$TELEX_DB`.

## Postgres (networked)

Configure a Postgres backend once, then select it by name or make it the default.
The connection string is a libpq URI or a key=value DSN. Provide the password by
reference, never embedded in the config:

- `--entra`: Azure Entra; telex fetches the token itself (uses `az login`, or
  `--entra-cred managed` on a devbox or VM with a managed identity). Requires a
  build with the `entra` feature, which the release binaries include.
- `--password-env <VAR>`: read the password from an environment variable.
- `--password-command <cmd>`: run a command that prints the password.

```sh
# Postgres with a password from an env var:
telex backend add staging \
  --postgres "postgresql://app@staging-db:5432/telex?sslmode=require" \
  --password-env STAGING_PG_PASSWORD --schema telex

# Azure Postgres with Entra:
telex backend add prod \
  --postgres "host=myserver.postgres.database.azure.com port=5432 user=me@example.com dbname=postgres sslmode=require" \
  --entra --schema telex --default
```

## Managing backends

```sh
telex backend list
telex backend show <name>
telex backend default <name>
telex backend remove <name>
telex backend kinds       # backend kinds compiled into this build
```

The first backend added becomes the default; `--default` or
`telex backend default <name>` changes it. See the
[Networked Postgres backend](../guides/postgres.md) guide.
