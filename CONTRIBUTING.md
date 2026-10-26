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

## Station-intent boundary coverage

The `station-intent-lock` CI job runs the complete station-intent and
owner-private filesystem modules on Linux, macOS, and Windows:

```text
cargo test --locked --no-default-features --features sqlite --lib station_intent::tests:: -- --nocapture
cargo test --locked --no-default-features --features sqlite --lib platform_fs::tests:: -- --nocapture
```

This dedicated SQLite-only job verifies the checked-out source head separately
from the provider event SHA and uses isolated runtime paths. It retains the
ordinary test profile, default parallel scheduling, and hard failures. The
broader `build-test` jobs remain separate because their default-feature
workspace tests and provider-selected checkout cover a different configuration.

The full station-intent module includes
`a_cooperatively_truncated_phase_still_returns_its_partial_page`, which requires
`Completed`, truthful truncation, and exactly one manifest. Its private blocking
slot and paused Tokio clock make the result controlled; it does not prove that
a first filesystem read completes within a real 200ms grace period. The
separate `cooperative-partial-page` matrix and its ten extra invocations per OS
were retired as duplicate execution. This intentionally reduces repeated-run
exposure without removing the positive case or neighboring deadline, admission,
cancellation, and permit-retention checks. Historical ten-run results are not
evidence for a newer source head.

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

## Stall-detector fixture isolation

On Unix, workspace tests require `TELEX_TEST_HOME_ROOT` for the binary-owned
stall-detector fixture. It must name an existing absolute directory outside the
real user home, owned by the current user with mode 0700. Neither that directory
nor any ancestor may be a symlink. Ancestors must be owned by root or the
current user and must not be group- or world-writable. The complete management
socket path must fit the platform limit.

The test validates these conditions before creating a unique child home. It
fails with setup instructions if the variable is missing; it never falls back
to the real home or adjusts an existing directory's permissions. Teardown
removes each child on normal return or unwind. An abrupt process exit can leave
only a fixture outside the real home.

Where a short user-owned runtime directory already qualifies, create a private
child there without sudo. For example, `/run/user/$(id -u)` can qualify on Linux
when its ancestors satisfy the conditions above:

```sh
export TELEX_TEST_HOME_ROOT="$(mktemp -d "/run/user/$(id -u)/XXX")"
chmod 0700 "$TELEX_TEST_HOME_ROOT"
cargo test --workspace
rmdir "$TELEX_TEST_HOME_ROOT"
```

The eight-character fixture child plus `/.telex-watcher/` and the 64-character
socket key add 89 bytes to the root path. The final path must be shorter than
`sockaddr_un.sun_path` (typically 108 bytes on Linux and 104 on macOS). A
shorter setup root may therefore be necessary even with safe permissions.
Ordinary `/tmp` roots are rejected because their ancestry is world-writable.

The hosted Linux workspace job creates a short `/t.XXXXXXXX` root. Only setup
uses sudo, to create the new directory and transfer its ownership to the runner;
the runner applies mode 0700 and all tests run unprivileged. The job checks that
the setup root is empty after tests; the root itself is discarded with the
hosted runner. Windows uses a home inside the existing isolated fixture root
and needs no additional setup.

The Windows scheduled-host stall gate is a separate validation target:

```text
cargo build --locked --workspace --bins
cargo test --locked -p telex-watcher --test stall_gate
```

It uses the shared isolated installed-current process harness, temporary
LOCALAPPDATA and synthetic session logs. It covers the detector calling its
own running host's public management endpoint, suppression, durable episode
delivery and long history. The harness retains its isolated root after daemon
cleanup for process-identity safety. This automated target does not replace the
real App root confirm/ring and background-shell cases; see the
[Windows gate evidence](docs/copilot-stall-gate-report.md).

## Windows CI bounds and execution evidence

The Windows `build-test` job has independent hosted ceilings around whole steps:

| Scope | Ceiling |
| --- | --- |
| Workspace `Test` | 30 minutes |
| Complete SQLite Application Client suite | 15 minutes |
| Credential invocation lifecycle | 5 minutes |
| Entire Windows workspace job, including builds and evidence collection | 60 minutes |

These are failure-cost limits chosen from whole-step receipts, not product
deadlines or latency percentiles; see [#209](https://github.com/lossyrob/telex/issues/209).
Other OS bounds, test commands, profiles, PageHeap checks, teardown and artifact
definitions are unchanged. Existing failure collection still runs where its
conditions permit; a job cancellation can preempt cleanup or artifact upload.

`Record Windows execution context` emits one `TELEX_CI_CONTEXT` JSON record in
the job log, under its own two-minute ceiling. It records the run and attempt,
event/provider/checkout identities, image, exposed logical CPU count and actual
Rust toolchain. Missing image environment values are unavailable metadata.
GitHub step timestamps supply whole-step durations. These context fields do not
measure CPU contention, disk latency, fsync or Defender activity.

Record the original run, attempt, job and test before rerunning a failed job at
the unchanged head. A green exact-head rerun can satisfy CI; retain the first
failure, and treat repeated failure of the same test as a finding.

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
