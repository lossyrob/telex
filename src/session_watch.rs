//! Cross-platform liveness check for a watched session/launcher pid.
//!
//! The resident holder (`telex attach`) uses this for *session binding* (issue #5): when a
//! caller passes `--session-pid <pid>` (or sets `$TELEX_SESSION_PID`), the holder polls that pid
//! and, the moment it stops existing, releases its lease and exits through the normal shutdown
//! path. This is defense-in-depth: even a mis-launched *detached* holder cannot outlive the
//! session that spawned it (Decision 0004 made enforceable rather than advisory; Decision 0010).
//!
//! [`process_alive`] is intentionally conservative: if it cannot positively determine that a
//! process is gone (e.g. it exists but we lack the rights to query it), it reports the process as
//! **alive** so the holder never releases a lease prematurely. Only a definite "no such process"
//! reports dead.
//!
//! Caveat — pid reuse: a raw pid can be recycled by the OS after the original process exits, so a
//! freshly-allocated unrelated process reusing the pid would read as "still alive." The window is
//! small at a few-second poll cadence; a reuse-immune inherited-fd path is a documented future
//! upgrade (issue #5, deferred).

/// Why the holder is running unbound (persistent). Carried so the holder can log a clear,
/// always-visible reason rather than silently running with no session binding.
#[derive(Debug, PartialEq, Eq)]
pub enum UnboundReason {
    /// No `--session-pid` flag and no `$TELEX_SESSION_PID` — the legacy default (logged quietly).
    NotRequested,
    /// `--no-session-bind` was passed (the persistent-holder escape hatch).
    OptedOut,
    /// A `0` pid was supplied (flag or env) — the "no binding" sentinel.
    ZeroSentinel,
    /// `$TELEX_SESSION_PID` was set to something that is not a valid pid; ignored, holder runs
    /// persistent. Carries the raw value for diagnostics.
    MalformedEnv(String),
}

/// The resolved session-binding decision for the holder.
#[derive(Debug, PartialEq, Eq)]
pub enum SessionBinding {
    /// Watch this pid; release the lease and exit when it dies.
    Bound(u32),
    /// Run a persistent holder (no binding), for the given reason.
    Unbound(UnboundReason),
}

/// Resolve whether and to which pid the holder should bind its lifetime, from the parsed flags and
/// the raw `$TELEX_SESSION_PID` value.
///
/// Precedence is resolved here at runtime rather than via clap `conflicts_with` *and* the env value
/// is parsed here rather than by clap, so that `--no-session-bind` always wins **without ever
/// erroring** — even when `$TELEX_SESSION_PID` is unset, malformed, or conflicting. (clap 4 treats
/// env-sourced values as "present" for conflict checks and would parse `TELEX_SESSION_PID` at parse
/// time, so a malformed env would otherwise fail every `attach`, including `--no-session-bind`.)
///
/// Order: `--no-session-bind` → explicit `--session-pid` flag → `$TELEX_SESSION_PID`. A `0` from
/// either source is the "unbound" sentinel; a non-numeric env value is reported as malformed and
/// the holder runs persistent.
pub fn resolve_session_pid(
    no_session_bind: bool,
    flag_pid: Option<u32>,
    env_pid: Option<&str>,
) -> SessionBinding {
    if no_session_bind {
        return SessionBinding::Unbound(UnboundReason::OptedOut);
    }
    if let Some(p) = flag_pid {
        return bind_or_sentinel(p);
    }
    match env_pid.map(str::trim) {
        None | Some("") => SessionBinding::Unbound(UnboundReason::NotRequested),
        Some(s) => match s.parse::<u32>() {
            Ok(p) => bind_or_sentinel(p),
            Err(_) => SessionBinding::Unbound(UnboundReason::MalformedEnv(s.to_string())),
        },
    }
}

fn bind_or_sentinel(pid: u32) -> SessionBinding {
    if pid == 0 {
        SessionBinding::Unbound(UnboundReason::ZeroSentinel)
    } else {
        SessionBinding::Bound(pid)
    }
}

/// Returns `true` if a process with `pid` currently exists, `false` if it definitely does not.
///
/// Best-effort and conservative: an existing-but-not-queryable process counts as alive, and any
/// ambiguous probe error also counts as alive, so the holder never releases a lease unless it can
/// positively confirm the watched session is gone.
pub fn process_alive(pid: u32) -> bool {
    // pid 0 is never a valid session/launcher pid to bind to (it is the kernel idle/swapper
    // "process"); treat it as not-a-session so a bogus `--session-pid 0` self-releases promptly.
    if pid == 0 {
        return false;
    }
    platform::process_alive(pid)
}

#[cfg(unix)]
mod platform {
    /// Probe via `kill(pid, 0)`, which sends no signal but performs the same
    /// permission/existence checks as a real signal.
    ///
    /// - returns 0            → the process exists and we may signal it → alive.
    /// - `errno == EPERM`     → the process exists but we may not signal it → alive.
    /// - `errno == ESRCH`     → no such process → dead.
    pub fn process_alive(pid: u32) -> bool {
        // Guard against pid values that would become negative as `pid_t` (i32): a negative pid
        // makes `kill` address a process group / broadcast, which is never what we want here.
        if pid > i32::MAX as u32 {
            return false;
        }
        // SAFETY: `kill` with signal 0 performs only existence/permission checks and delivers no
        // signal; it has no memory effects.
        let rc = unsafe { libc::kill(pid as libc::pid_t, 0) };
        if rc == 0 {
            return true;
        }
        match std::io::Error::last_os_error().raw_os_error() {
            Some(e) if e == libc::ESRCH => false, // no such process → gone
            _ => true,                            // EPERM (exists) or any ambiguous error → alive
        }
    }
}

#[cfg(windows)]
mod platform {
    use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, ERROR_INVALID_PARAMETER};
    use windows_sys::Win32::System::Threading::{OpenProcess, WaitForSingleObject};

    // Stable Win32 ABI constants, defined locally so this code does not depend on which module a
    // given `windows-sys` version exposes them from.
    const SYNCHRONIZE: u32 = 0x0010_0000;
    const WAIT_OBJECT_0: u32 = 0x0000_0000; // wait satisfied: the process object is signaled (exited)

    /// Probe via `OpenProcess` + `WaitForSingleObject`.
    ///
    /// - `OpenProcess` succeeds and `WaitForSingleObject(h, 0)`:
    ///   - `WAIT_TIMEOUT` (anything but `WAIT_OBJECT_0`) → not signaled → still running → alive.
    ///   - `WAIT_OBJECT_0` → signaled → the process has exited → dead.
    /// - `OpenProcess` fails:
    ///   - `ERROR_INVALID_PARAMETER` → no process with that pid → dead.
    ///   - any other error (e.g. `ERROR_ACCESS_DENIED`, transient failures) → conservatively
    ///     treated as alive, so we never release a lease on an ambiguous probe error.
    pub fn process_alive(pid: u32) -> bool {
        // SAFETY: FFI to Win32. `OpenProcess` returns a process handle or null; we only pass the
        // handle to `WaitForSingleObject`/`CloseHandle` and never dereference it.
        unsafe {
            let handle = OpenProcess(SYNCHRONIZE, 0, pid);
            if handle == 0 {
                // Only a definitively-unknown pid is "dead"; everything else stays alive.
                return GetLastError() != ERROR_INVALID_PARAMETER;
            }
            let wait = WaitForSingleObject(handle, 0);
            CloseHandle(handle);
            // WAIT_OBJECT_0 means the object is signaled → the process has exited. Anything else
            // (notably WAIT_TIMEOUT, or an ambiguous WAIT_FAILED) means it has not confirmed exit
            // → still running.
            wait != WAIT_OBJECT_0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{process_alive, resolve_session_pid, SessionBinding, UnboundReason};

    #[test]
    fn resolve_no_bind_wins_over_pid_and_env() {
        // `--no-session-bind` opts out even with a flag pid or a (valid or malformed) env value,
        // and must never error — protecting the persistent-holder escape hatch (AC3).
        assert_eq!(
            resolve_session_pid(true, Some(1234), Some("5678")),
            SessionBinding::Unbound(UnboundReason::OptedOut)
        );
        assert_eq!(
            resolve_session_pid(true, None, Some("not-a-pid")),
            SessionBinding::Unbound(UnboundReason::OptedOut)
        );
    }

    #[test]
    fn resolve_binds_to_flag_pid() {
        assert_eq!(
            resolve_session_pid(false, Some(1234), None),
            SessionBinding::Bound(1234)
        );
    }

    #[test]
    fn resolve_binds_to_env_pid() {
        assert_eq!(
            resolve_session_pid(false, None, Some("4321")),
            SessionBinding::Bound(4321)
        );
    }

    #[test]
    fn resolve_flag_beats_env() {
        assert_eq!(
            resolve_session_pid(false, Some(11), Some("22")),
            SessionBinding::Bound(11)
        );
    }

    #[test]
    fn resolve_default_is_unbound() {
        // No flag, no env → legacy behavior: persistent holder, no binding.
        assert_eq!(
            resolve_session_pid(false, None, None),
            SessionBinding::Unbound(UnboundReason::NotRequested)
        );
        assert_eq!(
            resolve_session_pid(false, None, Some("  ")),
            SessionBinding::Unbound(UnboundReason::NotRequested)
        );
    }

    #[test]
    fn resolve_zero_is_unbound_sentinel() {
        // `--session-pid 0` / `TELEX_SESSION_PID=0` means "no binding", not "watch pid 0".
        assert_eq!(
            resolve_session_pid(false, Some(0), None),
            SessionBinding::Unbound(UnboundReason::ZeroSentinel)
        );
        assert_eq!(
            resolve_session_pid(false, None, Some("0")),
            SessionBinding::Unbound(UnboundReason::ZeroSentinel)
        );
    }

    #[test]
    fn resolve_malformed_env_is_unbound_not_error() {
        // A malformed env (not no-bind) must not crash attach: report it and run persistent.
        assert_eq!(
            resolve_session_pid(false, None, Some("not-a-pid")),
            SessionBinding::Unbound(UnboundReason::MalformedEnv("not-a-pid".to_string()))
        );
    }

    #[test]
    fn self_is_alive() {
        assert!(process_alive(std::process::id()));
    }

    #[test]
    fn never_allocated_pid_is_dead() {
        // Far above any real `pid_max` on Linux (max 2^22) and any practical Windows pid; chosen
        // to stay positive as a 32-bit signed pid_t so the Unix path never broadcasts.
        assert!(!process_alive(2_000_000_000));
    }

    #[test]
    fn zero_pid_is_dead() {
        assert!(!process_alive(0));
    }

    #[test]
    fn reaped_child_is_dead() {
        // Spawn a trivial child, reap it, then confirm it reads as gone. After `wait()` the
        // process has exited; on Windows it is signaled (WAIT_OBJECT_0) even before pid recycle,
        // on Unix it is reaped (ESRCH), so both paths report dead.
        let mut child = spawn_trivial();
        let pid = child.id();
        child.wait().expect("reap child");
        assert!(!process_alive(pid));
    }

    #[test]
    fn live_child_is_alive_then_dead() {
        // Covers the "another process is alive" path (distinct from self): a running child reads
        // alive; once killed and reaped it reads dead.
        let mut child = spawn_sleeper();
        let pid = child.id();
        assert!(process_alive(pid), "running child should read alive");
        child.kill().expect("kill child");
        child.wait().expect("reap child");
        assert!(!process_alive(pid), "killed child should read dead");
    }

    fn spawn_trivial() -> std::process::Child {
        #[cfg(windows)]
        {
            std::process::Command::new("cmd")
                .args(["/C", "exit"])
                .spawn()
                .expect("spawn cmd")
        }
        #[cfg(unix)]
        {
            std::process::Command::new("sh")
                .args(["-c", "exit 0"])
                .spawn()
                .expect("spawn sh")
        }
    }

    fn spawn_sleeper() -> std::process::Child {
        #[cfg(windows)]
        {
            // ping loops ~30s without needing a console/stdin, unlike `timeout`.
            std::process::Command::new("cmd")
                .args(["/C", "ping -n 30 127.0.0.1 >NUL"])
                .spawn()
                .expect("spawn sleeper")
        }
        #[cfg(unix)]
        {
            std::process::Command::new("sh")
                .args(["-c", "sleep 30"])
                .spawn()
                .expect("spawn sleeper")
        }
    }
}
