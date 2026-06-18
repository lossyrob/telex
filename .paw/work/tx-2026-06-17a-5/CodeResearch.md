# CodeResearch — Issue #5

## Holder shutdown & lease-release path (the reuse target)
`src/commands/attach.rs`:
- The holder's main loop is a `tokio::select!` over three arms (attach.rs:186-210):
  - `listener.accept()` — serve a new waiter connection.
  - `state.shutdown.notified()` — break the loop (attach.rs:201-204).
  - `tokio::signal::ctrl_c()` — break the loop (attach.rs:205-208).
- After the loop breaks, the holder calls `backend.release_lease(&address, &occupant)` and logs
  `[holder] lease released=...; exiting` (attach.rs:212-217). **This is the single release path.**
- `state.shutdown` is a `tokio::sync::Notify` (attach.rs:33,121). The IPC `Shutdown` request
  handler calls `st.shutdown.notify_one()` (attach.rs:318-322). So a watch task can trigger the
  **identical** release path simply by calling `state.shutdown.notify_one()`.
- `Notify::notify_one()` stores a permit if no waiter is currently parked, and the main loop is
  continuously re-registered in the select, so the signal is never lost (same mechanism already
  used by the IPC Shutdown handler).
- Background tasks are spawned with `tokio::spawn` capturing `state.clone()` (heartbeat task
  attach.rs:132-148, poll task attach.rs:150-163) — the watch task follows the same shape.

## detach / ctrl-c equivalence
`src/commands/detach.rs` sends an IPC `Shutdown` (detach.rs:10-33) which lands in the holder's
`Request::Shutdown` handler → `st.shutdown.notify_one()`. So "release on session-death identical
to detach/ctrl-c" (criterion 4) is satisfied by routing the watch through `state.shutdown`.

## Liveness window
- Lease claimed with `ctx.cfg.liveness_window_secs` (attach.rs:91-93); `telex status` reports
  `liveness_window_secs: 15` (observed) and `occupancy.occupied`. After `release_lease`, status
  shows `occupied=false`. Detection at a 2s poll → release well inside the 15s window.

## CLI args (where new flags go)
`src/cli.rs`:
- `AttachArgs` (cli.rs:101-126) holds existing `--description/--scope/--tags/--heartbeat-secs/
  --poll-secs/--keepalive-secs/--occupant/--push`. Uses clap derive with `#[arg(long, ...)]`,
  `default_value_t`, and `clap` already has the `env` feature (Cargo.toml:31) so
  `#[arg(long, env = "TELEX_SESSION_PID")]` is supported.
- Conflicts expressed via `#[arg(long, conflicts_with = "session_pid")]`.

## Cross-platform precedent
`src/ipc.rs` already does `#[cfg(windows)]` / `#[cfg(unix)]` module split (ipc.rs:77-185) — the
established pattern for platform-specific code. The new `session_watch` module mirrors it.
- `tokio` Cargo features include `process`/`signal` (Cargo.toml:25) but those don't help watch an
  arbitrary external pid — raw syscalls are needed.
- `libc` is already in the lock tree (Cargo.lock:1106, via tokio/rusqlite). `windows-sys` is
  present at several versions incl. 0.52.0 (Cargo.lock:2752). Adding direct platform-gated deps
  pins the API surface without growing the tree.
- Existing target-gated dep block precedent: `[target.'cfg(target_os = "linux")'.dependencies]`
  for vendored openssl (Cargo.toml:49-50).

## Module registration & test conventions
- `src/lib.rs:3-12` lists `pub mod ...`; a new `pub mod session_watch;` slots in.
- Repo convention is **inline `#[cfg(test)] mod tests`** in the module under test (e.g. model.rs,
  ipc-adjacent); no dev-dependencies in Cargo.toml, so tests use only std (`std::process::Command`
  for spawn/reap). The `tests/conformance.rs` battery is **backend-trait** coverage only and is
  not relevant to this holder/CLI change.

## Cross-platform liveness API notes
- **Unix** (`libc`): `kill(pid as pid_t, 0)` → 0 means exists; on -1 inspect errno: `ESRCH` =
  no such process (dead), `EPERM` = exists but not permitted (alive). Must avoid pid values that
  become negative as `pid_t` (i32) — a negative pid broadcasts. Tests use a large positive pid
  (2_000_000_000) far above any real `pid_max`.
- **Windows** (`windows-sys` 0.52, features `Win32_Foundation` + `Win32_System_Threading`):
  `OpenProcess(SYNCHRONIZE, FALSE, pid)`; if handle is null inspect `GetLastError()` —
  `ERROR_ACCESS_DENIED (5)` = exists (alive), else dead. With a handle,
  `WaitForSingleObject(h, 0)` → `WAIT_OBJECT_0 (0)` = signaled/exited (dead),
  `WAIT_TIMEOUT (0x102)` = still running (alive); then `CloseHandle`. In windows-sys 0.52
  `HANDLE = isize` (null == 0), `BOOL = i32`. SYNCHRONIZE (0x0010_0000) defined locally to avoid
  import-path churn across windows-sys versions.

## DECISIONS.md
- Append-only, sequentially numbered; supersede rather than rewrite (DECISIONS.md:22-31). Last
  entry is **0009** (DECISIONS.md:358) → new entry is **0010**.
- Relevant context: 0004 (holder must track session lifetime, "not a fully detached daemon",
  DECISIONS.md:174-185) and 0005 (TTL/poll baseline, DECISIONS.md:198-227). This change makes
  0004 *enforceable* rather than advisory.

## Open-PR overlap
PRs #15 (issue #10) and #16 (issue #4) are open and also touch holder / `attach.rs` / lease code.
This change adds a watch task + a `state.shutdown` trigger inside `attach.rs::run` and new
`AttachArgs` fields in `cli.rs`. Overlap is in the holder select-loop / shutdown region and the
args struct — note for builder rebase ordering.
