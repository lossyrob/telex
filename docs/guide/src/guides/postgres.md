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

### Credential commands are one-shot

Use `--password-command` for a command that prints a complete UTF-8 password or
token and finishes. Telex trims that output. If `--password-env` is also set,
the environment reference takes precedence.

Telex owns the invocation and its supported in-scope helpers. It cleans up
remaining owned helpers after successful status plus complete output, or on
error/cancellation. A shell that exits before a finite helper finishes writing
does not cause partial credential output to be returned. Do not use this option
to start a persistent refresh daemon or detached/broker-owned background job.
Querying an existing credential agent is supported without making that agent a
cleanup target. Windows jobs and Unix process groups have different escape
limits; Telex does not claim universal descendant containment.

On Windows, completion means checked termination of the private job, zero active
job processes, completed launched-shell wait, and completion of Telex's own I/O,
handle release, and worker. It does not promise that every former helper's
process handle is already signaled or all kernel/driver rundown and prior external
I/O has finished. The invocation limit below bounds Telex-owned work, not every
residual Windows kernel object. Cleanup API failures are still failures, not
successful receipts.

Each owning process admits at most two credential invocations, and equal
configured command sources do not overlap during cleanup. Waiting for admission
spends the original caller budget. Cancellation returns control on that budget
while the native owner finishes cleanup independently of the async runtime.

Normal owning-host exit can add up to three seconds for one concurrent cleanup
drain. If cleanup cannot be confirmed, Telex reports a named `FAILED_HELD`
obligation and holds the host rather than silently leaking ownership or claiming
clean exit. The exceptional hold can exceed three seconds and requires operator
intervention. A waiter client does not drain work owned by its daemon. Cleanup
errors report lifecycle stage/status without echoing the command or helper stderr.

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
