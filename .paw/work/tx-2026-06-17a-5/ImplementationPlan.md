# ImplementationPlan — Issue #5

## Outcome anchor
A holder bound with `--session-pid <pid>` (or `$TELEX_SESSION_PID`) releases its lease and exits
through the normal shutdown path within the liveness window when that pid dies — on both Windows
and Unix — with `--no-session-bind` as the documented persistent-holder escape hatch and SKILL.md
updated. Satisfying this → PR uses `Closes #5`.

## Approach
Add a small cross-platform liveness primitive and a holder-side watch task that funnels session
death into the **existing** `state.shutdown` → `release_lease` path. No change to lease,
heartbeat, delivery, or messaging semantics. New behavior is entirely opt-in; default is
unchanged.

## Key decisions (spread noted)
1. **Binding is explicit, via `--session-pid`/`$TELEX_SESSION_PID`.** *(low spread)* The primary
   contract from the issue.
2. **No ppid-default (and no ppid opt-in in this PR).** *(HIGH spread — flagged to human +
   DECISIONS 0010)* Spawn-and-return launchers (common for async/background launches) would leave
   the holder with a dead/reparented ppid and make it wrongly self-exit, breaking the primary use
   case. The issue's own risk section calls ppid "the central risk." Default stays *no binding*
   (backward compatible). If the human wants a best-effort ppid opt-in later, it can be added
   behind an explicit flag with the Windows ppid-walk.
3. **`--session-fd` pipe path deferred.** *(low spread)* Explicitly optional; pid-watch meets all
   acceptance criteria; avoids new cross-platform IPC surface. Carry-forward.
4. **Reuse `state.shutdown` rather than a second release path.** *(low spread)* Guarantees
   criterion 4 (identical to detach/ctrl-c) by construction.
5. **Define `SYNCHRONIZE`/`WAIT_*` constants locally on Windows.** *(low spread)* Avoids
   import-path churn across windows-sys versions; the values are stable Win32 ABI constants.
6. **"access-denied ⇒ alive" failure direction.** *(low spread)* If we cannot positively confirm
   death we treat the process as alive, biasing away from premature lease release; only a
   definite "no such process" releases.

## Work items (single sequential implementation — tightly coupled, 6 files)

### W1 — Platform deps (`Cargo.toml`)
Add target-gated dependencies (both already in the lock tree):
```toml
[target.'cfg(unix)'.dependencies]
libc = "0.2"

[target.'cfg(windows)'.dependencies]
windows-sys = { version = "0.52", features = ["Win32_Foundation", "Win32_System_Threading"] }
```

### W2 — `src/session_watch.rs` (new) + register in `src/lib.rs`
- `pub fn process_alive(pid: u32) -> bool` with `#[cfg(unix)]` and `#[cfg(windows)]` impls per
  CodeResearch API notes.
- `#[cfg(test)] mod tests`: (a) `process_alive(std::process::id())` is true; (b)
  `process_alive(2_000_000_000)` is false; (c) spawn a trivial child (`sh -c 'exit 0'` /
  `cmd /C exit`), `wait()` to reap, assert `!process_alive(pid)`.
- `src/lib.rs`: add `pub mod session_watch;`.

### W3 — Holder flags (`src/cli.rs`, `AttachArgs`)
- `--session-pid <PID>`: `#[arg(long, env = "TELEX_SESSION_PID")] pub session_pid: Option<u32>`.
- `--no-session-bind`: `#[arg(long)] pub no_session_bind: bool`. **No clap `conflicts_with`** — clap
  4 treats env-sourced values as "present" for conflict checks, so `conflicts_with = "session_pid"`
  would reject `TELEX_SESSION_PID=… telex attach --no-session-bind` *before* the holder runs,
  breaking the escape hatch (AC3). Precedence is resolved purely at runtime instead (see W4).
- `--session-poll-secs <N>`: `#[arg(long, default_value_t = 2)] pub session_poll_secs: u64`.
- Doc comments explaining the belt-and-suspenders intent and the persistent-holder escape hatch.

### W4 — Watch wiring (`src/commands/attach.rs`)
- Resolve the watched pid via the pure helper `session_watch::resolve_session_pid(no_session_bind,
  session_pid)`: `--no-session-bind` wins (→ `None`); a `0` pid is an unbound sentinel (→ `None`);
  otherwise `Some(pid)`. Keeping precedence in a pure function makes it testable and independent of
  clap.
- If `Some(pid)`: clamp the poll interval to the lease liveness window (so the address always frees
  within it), log `[holder] session-bound to pid <pid> ...`, and spawn a watch task:
  ```rust
  let window = ctx.cfg.liveness_window_secs.max(1) as u64;
  let interval = args.session_poll_secs.max(1).min(window);
  let st = state.clone();
  tokio::spawn(async move {
      let mut tick = tokio::time::interval(Duration::from_secs(interval));
      loop {
          tick.tick().await;
          if !crate::session_watch::process_alive(pid) {
              eprintln!("[holder] session {pid} gone; releasing lease and exiting");
              st.shutdown.notify_one();
              break;
          }
      }
  });
  ```
  (`process_alive` is a cheap non-blocking syscall. The first `interval` tick is immediate, but
  `process_alive` is conservative, so a live pid never triggers a spurious early release; a pid that
  is genuinely dead at startup releases promptly.)
- If `None` but a pid was supplied alongside `--no-session-bind`, log that the pid is ignored.

> **Planning-docs review (gpt-5.5 + claude-opus-4.8) outcome:** both flagged the clap
> `conflicts_with` env-conflict as the sole MUST-FIX; opus empirically confirmed clap 4.6.1 rejects
> `TELEX_SESSION_PID=… --no-session-bind`. The implementation already uses pure runtime precedence
> (no `conflicts_with`), so the MUST-FIX is satisfied by construction. Applied NICE-TO-HAVE items:
> `0` pid sentinel, poll-interval clamp to the liveness window, a live-other-process liveness test,
> and removal of the unused `WAIT_TIMEOUT` const. Both reviews confirmed AC coverage, `Closes #5`,
> the ppid-decline (HIGH-spread → human), the cross-platform liveness logic, and the
> `state.shutdown` reuse as sound.

### W5 — Docs (`SKILL.md`, `DECISIONS.md`)
- `SKILL.md`: in "The core loop" holder section, add `--session-pid` as the in-binary
  belt-and-suspenders companion to launching background + session-bound; mention
  `$TELEX_SESSION_PID` and `--no-session-bind`. Add the flags to the `telex attach` rows in the
  optional-flags line and the PRESENCE command-reference table.
- `DECISIONS.md`: append **0010 — Holder self-binds to its launching session (pid-watch);
  ppid-default declined, fd path deferred.** Status `Accepted (pending validation)`. Note it
  makes 0004 enforceable; cross-reference 0004/0005.

## Validation
- `cargo build` (default features); `cargo test`; `cargo clippy --all-targets`.
- Feature-gated: `cargo build --no-default-features --features sqlite` and `--features postgres`.
- Manual Windows e2e (acceptance criterion 1): start a dummy long-lived process, capture its pid;
  `telex attach --backend <temp> --address <temp> --session-pid <dummy>`; `telex status` shows
  occupied; kill the dummy; within the window `telex status` shows `occupied=false` and the holder
  process has exited; holder log shows the `session ... gone` line.

## Acceptance-criteria mapping
- AC1 → W2+W4 + manual e2e. AC2 → W4 (binding independent of how the holder was launched).
- AC3 → W3 (`--no-session-bind`) + W4 resolution + W5 docs. AC4 → W4 routes through
  `state.shutdown` (the detach/ctrl-c path). AC5 → W2 cfg-gated impls; fd path declared deferred.
- AC6 → W5 SKILL.md.

## Rollback / risk
- Pid reuse: a recycled pid could keep the holder alive past the real session; documented caveat
  (DECISIONS 0010). The 2s poll + small reuse window makes this rare; fd path (deferred) is the
  reuse-immune upgrade.
- All new code is opt-in; if the watch misbehaves, omitting the flag fully restores prior
  behavior.
