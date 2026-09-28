# Contributing to telex

## Adding a backend

Telex keeps a single semantic core and treats storage as a pluggable axis: the
[`Backend`](src/backend/mod.rs) trait is the extension point (see DECISIONS 0008 for the
modular-backend direction). To add a new backend (e.g. AWS RDS Postgres, GCP Cloud SQL,
DynamoDB, Firestore):

1. **Implement `Backend`.** Add a module under `src/backend/` and implement every method of
   the `Backend` trait. Gate it behind a Cargo feature so users only compile what they need
   (`optional` dependency + a `[features]` entry), and add a `#[cfg(feature = "...")]` arm to
   the factory in [`src/profiles.rs`](src/profiles.rs).
2. **Add a conformance fixture.** Wire your backend into the shared conformance battery in
   [`tests/conformance.rs`](tests/conformance.rs) by providing a `Store` factory that yields a
   fresh, empty store on demand and can open multiple independent connections to it. Use the
   existing SQLite and Postgres fixtures as templates. Skip cleanly (don't fail) when the
   backend's server isn't configured, so default CI stays green.
3. **Run the conformance suite.** `cargo test` runs the full battery against your backend and
   proves it honours the trait's contract — schema idempotency, address/directory semantics,
   lease liveness and TTL occupancy, cursor delivery, message threading, inbox derivation,
   disposition terminality, export filters, and concurrent-insert id monotonicity.

If `cargo test` is green, your backend behaves like every other telex backend.

## Running the conformance suite

```sh
# SQLite runs by default (fresh temp-file database per scenario):
cargo test

# Postgres runs the same battery against an isolated schema when configured, and is
# skipped cleanly otherwise:
TELEX_PG_URL='postgresql://user@host:5432/telex?sslmode=disable' \
  TELEX_PG_PASSWORD=secret \
  TELEX_PG_SCHEMA=telex_conformance \
  cargo test
```

- `TELEX_PG_URL` — libpq URI or `key=value` DSN. When unset, the Postgres suite is skipped.
- `TELEX_PG_PASSWORD` — optional; applied to the connection if the URL omits a password.
- `TELEX_PG_SCHEMA` — optional schema prefix (default `telex_conformance`); the suite
  creates a per-run schema under it and drops it when finished.
- `TELEX_PG_REQUIRE=1` — fail instead of skipping when `TELEX_PG_URL` is unset/empty, so a CI
  job that intends to exercise the Postgres leg can't pass by silently skipping it.

The live LISTEN/NOTIFY proof observes subscription and waiter readiness, then
disables only the test waiter's polling fallback. A committed row without a
notification must time out; a real notification must deliver the exact row even
when its epoch proof is deliberately delayed. This distinguishes the wake path
from end-to-end SQL/scheduler latency. A separate virtual-time test proves that
notification selection does not wait for the normal polling deadline. Production
poll intervals remain unchanged.

## Credential-command lifecycle tests

Run the real OS lifecycle targets on Windows, Linux, and macOS:

```text
cargo test --no-default-features --features postgres --test credential_command --test credential_command_process -- --test-threads=1
cargo test --no-default-features --features postgres --lib profiles::password_command -- --test-threads=1
```

These tests use non-secret fixture commands and disposable roots, not operator
credential commands or a shared database. The macOS credential job is separate
from SQLite Copilot fallback coverage. Ignored subprocess entrypoints are
invoked by their parent proofs; an ignored or filtered target alone is not runtime
evidence. Delayed credential fixtures publish their native host/owned-child
identities only after a child-ready barrier, without CIM or parent-process
discovery. Acquisition errors are observed directly instead of being reported
as an indistinguishable readiness timeout. Deterministic error/identity seams
supplement, rather than replace, real platform scope/termination/reap and pipe tests.

## Windows token-user regression coverage

On Windows, run `cargo test --lib windows_token_user_alignment` to exercise the
SQLite current-user SID lookup and daemon peer identity lookup. Both paths check
the allocation element's alignment guarantee and the returned pointer's alignment
before dereferencing `TOKEN_USER`. A `Vec<u8>` regression fails the element check
even if the allocator happens to return an aligned address. The daemon tests also
check that an invalid token retains the existing sizing error.

The `windows-token-user` CI job runs this proof with default features, SQLite-only,
PostgreSQL-only, Entra-only, SQLite+PostgreSQL, SQLite+Entra, SQLite+self-update, and
all features. SQLite-disabled profiles exercise the daemon path. The existing
Windows process suite also exercises the aligned owner-private fixture helper in
`tests/daemon_process_sqlite.rs`.

The CI toolchain action sets `RUSTFLAGS=-D warnings`. Reproduce that policy
locally rather than running with unset flags. For the SQLite-disabled profiles
in PowerShell:

```powershell
$env:RUSTFLAGS = "-D warnings"
cargo test --lib --no-default-features --features postgres windows_token_user_alignment
cargo test --lib --no-default-features --features entra windows_token_user_alignment
```

Each command must execute two daemon tests; SQLite-enabled profiles execute
three tests across both production paths. SQLite-only test helpers must carry
the same feature guards as their callers so these configurations compile with
warnings denied.

These checks establish token-buffer alignment and preserve identity/error
behavior. They do not establish that alignment caused historical heap-corruption
crashes.

## Releasing

Maintainers cut public releases by pushing a `vX.Y.Z` tag, which triggers the
release workflow to build, checksum, and publish platform assets. The full
procedure — tag/version conventions, the pre-cut checklist, first-release notes,
post-cut verification, and rollback — is in
[docs/developing/releasing.md](docs/developing/releasing.md).
