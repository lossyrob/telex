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
- `--password-command <cmd>`: run a one-shot command that prints the password.
  Telex owns supported invocation helpers through completion/cancellation and
  cleanup; persistent or escaped background work is outside this contract.
  See the [credential lifecycle](../guides/postgres.md#credential-commands-are-one-shot).

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

### A private CA for one PostgreSQL profile

Supply the public CA certificate in PEM format and set `sslmode=require`. For example,
from PowerShell:

```powershell
telex backend add personal --postgres 'host=db.home.test port=5432 dbname=telex user=telex sslmode=require' --tls-ca-file '.\home-ca.pem' --password-env TELEX_PG_PASSWORD
```

Telex saves the absolute certificate path as `tls_ca_file` in the backend profile.
When editing the profile by hand, use an absolute path on the client machine. Each
new connection uses this file as its trust anchor, including daemon notification and
recovery connections. The server certificate must be valid for the hostname or IP
address in the connection string. The CA stays local to this profile and the OS
trust store is untouched. Existing profiles without this option use system trust.
The same TLS option works with password or Entra authentication.

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

---

Next: [Security and data](security.md)
