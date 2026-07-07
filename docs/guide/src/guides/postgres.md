# Networked Postgres backend

Local SQLite is the default. Use a Postgres backend when sessions on different
machines need to share one exchange, or to persist an audit trail centrally.

## Add a backend

Configure once with `telex backend add`. Provide the password by reference, never
embedded in the connection string.

### Password from an environment variable

```sh
telex backend add staging \
  --postgres "postgresql://app@staging-db:5432/telex?sslmode=require" \
  --password-env STAGING_PG_PASSWORD --schema telex
```

### Azure Postgres with Entra

Telex fetches the token itself. On a laptop it uses your `az login`; on a devbox
or VM with a managed identity, use `--entra-cred managed`.

```sh
telex backend add prod \
  --postgres "host=myserver.postgres.database.azure.com port=5432 user=me@example.com dbname=postgres sslmode=require" \
  --entra --schema telex --default
```

`--entra` requires a build with the `entra` feature; the release binaries include
it. On a build without it, supply the token with `--password-command` (for
example `az account get-access-token ...`).

## Select a backend

```sh
telex --backend staging inbox
telex send --to node:x --body "hi"     # uses the default backend
telex backend list
```

The first backend added becomes the default; change it with `--default` on add or
`telex backend default <name>`.

## Notes

- The connection string is a libpq URI or a key=value DSN.
- Use `--schema` to place telex tables in a dedicated schema.
- Secrets are referenced (`--entra`, `--password-env`, `--password-command`) and
  are never written to the config file. `telex backend show <name>` redacts them.

## Schema and privileges

Set the schema for telex tables with `--schema` when adding the backend. The
configured database role needs privileges to create objects in that schema on
first use (or to use an existing one). Pre-create and validate the schema:

```sh
telex init --backend <name>
```

This connects with the configured credentials, creates the schema and tables if
they are absent, and surfaces connection or permission errors early.

## TLS

Request TLS in the connection string with `sslmode=require`, or a stricter mode
such as `verify-full` with the appropriate root certificate configured for your
environment. Azure Postgres requires TLS.

## Backup

A telex Postgres backend is an ordinary schema in your database. Back it up with
standard Postgres tooling:

```sh
pg_dump --schema telex "postgresql://.../telex" > telex-backup.sql
```

## Multiple machines

Point each machine's telex at the same Postgres backend (same connection string
and schema) to share one durable store and audit trail. Each machine still runs
its own local exchange daemon; the daemon is per user and local, while the store
is the shared Postgres schema.
