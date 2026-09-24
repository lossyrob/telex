# Historical waiter behavior fixture

This is source-pinned historical CLI error/out-dir behavior in an authenticated
same-image subprocess fixture, paired with actual current CLI process proof and
legacy wire decoding. It is not a released old executable paired with a new
daemon. Existing executable-identity authentication remains a separate
prerequisite for deployment interoperability.

## Source and extraction

The immutable source is commit
`7ed886b07620e8ab8adba68249ab84f96ea26013`. `provenance.json` records Git blob
identities, extraction boundaries, and SHA-256 digests. Extraction used `git show`
and verified each resulting byte sequence occurred unchanged in its source blob.
The offline test verifies the digests, allowing only checkout LF/CRLF conversion.
No Git history, network request, or historical binary is needed at test time.

| Fixture | Unchanged source |
|---|---|
| `historical_wait.rs` | All production code in `src/commands/wait.rs`, before its `#[cfg(test)]` section; blob `c3526da3a5f24cff9b00585f9998879cfd15fdfe`. |
| `historical_hello.rs` | `daemon_capabilities`, `daemon_required_capabilities`, and `client_hello`; `src/daemon_ipc.rs` blob `96a1bb3b9c9abcdede113781899c5b28c4a9fdbe`. |
| `historical_outcome.rs` | The exact closed `WaiterOutcome` enum and derives from that same IPC blob. |
| `historical_result.rs` | Final `match result` in `cli::run`; `src/cli.rs` blob `e5a84b065b8947f7ea2d5f39bb7fe1a451eef95a`. |

The historical production waiter includes its generic `Response::Error` branch,
run-level artifact/error propagation, `WaitOutcome::error`, startup and terminal
artifact writers, and ordinary non-error outcomes. The fixture does not replace
the error path with `emit_outcome` or hardcode a test exit code.

## Adapters and evidence

`tests/wait_recovery_process.rs` supplies only these context adapters:

- Module imports bind the unchanged waiter to existing CLI/model types and a
  connector wrapper. The wrapper selects the pinned historical Hello, records
  the actual response, and uses production endpoint connection, capability-file
  PID/start-time, same-user and canonical-image verification.
- The test executable has daemon-host and historical-frontend subprocess roles.
  The daemon runs production `serve` and dispatch. There is no release CLI mode
  or production environment-controlled downgrade.
- The entry point supplies parsed fixture arguments and passes the historical
  result mapping to `std::process::exit`, matching the final exit operation in
  historical `src/main.rs`, blob `e03dd7ee9b15e150183d6a9080b60ee51e0708e1`.
  Launcher dispatch is outside this test; no launcher identity is substituted.

Both processes use the same unchanged test executable image. Genuine
PostgreSQL candidate-fetch failures drive production bounded recovery; the
fixture never injects a final response or process exit. Assertions cover
historical exit 1 and exact artifacts/detail, startup PID, stale-payload removal,
no-out-dir errors, a Timeout/exit-2 control, actual wire Hello/Status behavior,
and preserved daemon classification 7 for the same exhausted member. A separate
matching current CLI/daemon pair proves real process/out-dir exit 7.

Each pair has isolated config, runtime, install, and database paths plus a unique
schema. Commands have deadlines; daemons are drained and reaped. Failed fixture
evidence is retained; successful fixture roots and schemas are removed.

Run with a disposable PostgreSQL server and `TELEX_PG_REQUIRE=1`:

```text
cargo test --all-features --test wait_recovery_process
```

This #155 evidence strategy does not waive #157's genuine v0.1.2 upgrade and
fresh-install proof. That release work must exercise supported ordered upgrade
and daemon replacement in disposable roots, rather than directly pairing an old
client with a different executable image.
