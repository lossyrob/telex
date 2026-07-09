# Plan: macOS Daemon Cap Identity

## Approach

Implement the macOS equivalents of the existing Linux daemon-authentication primitives so the daemon can publish a complete PID/start-time identity and the client can verify the connected server without weakening the fail-closed trust boundary.

## Work Items

1. Add macOS process start-time capture in `src/session_watch.rs` using `proc_pidinfo(PROC_PIDTBSDINFO)`.
2. Add macOS Unix-socket peer PID/UID lookup, executable-path lookup, and start-time verification in `src/daemon.rs`.
3. Add unit coverage in `src/session_watch.rs` and process integration coverage in `tests/daemon_process_sqlite.rs` that proves a spawned macOS daemon publishes `server_start_time` and accepts a verified client connection.
4. Expand the test-fixture scope to keep daemon, registry, and process-integration Unix socket paths within macOS `SUN_LEN`, including when the repository worktree path is long.
5. Run formatting, focused daemon tests, Clippy, and the workspace test suite.

## Key Decisions

- Preserve the requirement that both `server_pid` and `server_start_time` exist in the cap file.
- Use `getpeereid` for the peer effective UID and `LOCAL_PEERPID` for the peer PID on macOS.
- Use `proc_pidpath` for canonical executable verification and `proc_pidinfo(PROC_PIDTBSDINFO)` for a stable process-start identity.
- Encode macOS start time as microseconds from the `proc_bsdinfo` seconds/microseconds fields.
- Abort daemon startup before writing the capability file if macOS cannot capture its own start time.
- Keep Linux and Windows behavior unchanged and continue rejecting unverifiable or mismatched identities before sending `Hello`.

## Scope Exclusions

- Do not make either identity field optional during authentication.
- Do not change protocol versions or the cap-file schema.
- Do not add new dependencies or alter production behavior on other platforms.
- Do not update user-facing documentation; this restores documented supported-platform behavior without changing the interface.

## Success Criteria

- A macOS daemon cap file includes a non-null `server_start_time`.
- A macOS client authenticates and connects to the daemon it spawned.
- PID, start-time, UID, and executable mismatches still fail closed.
- Existing supported-platform tests pass, apart from independently reproduced baseline failures.
