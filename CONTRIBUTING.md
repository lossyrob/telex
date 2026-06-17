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
