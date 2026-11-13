use super::{DaemonBootstrapFailure, MacosExecutableWitness};
use std::ffi::CString;
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::ffi::OsStrExt;
use std::process::Command;
use std::sync::{Mutex, MutexGuard, TryLockError};
use std::time::{Duration, Instant};

struct Custody {
    pid: libc::pid_t,
    _witness: std::fs::File,
    _verified_start: Option<u64>,
    generation_known: bool,
}

static CUSTODY: Mutex<Option<Custody>> = Mutex::new(None);

#[cfg(test)]
thread_local! {
    static OBSERVE_ERROR: std::cell::Cell<Option<i32>> = const { std::cell::Cell::new(None) };
    static REAP_PENDING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static GROUP_SIGNALS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static CONTINUES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static OBSERVATIONS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
pub(super) fn take_observe_error() -> Option<i32> {
    OBSERVATIONS.with(|count| count.set(count.get() + 1));
    OBSERVE_ERROR.with(|error| error.take())
}

#[cfg(test)]
pub(super) fn take_reap_pending() -> bool {
    REAP_PENDING.with(|pending| pending.replace(false))
}

#[cfg(test)]
pub(super) fn record_group_signal() {
    GROUP_SIGNALS.with(|count| count.set(count.get() + 1));
}

#[cfg(test)]
pub(super) fn record_continue() {
    CONTINUES.with(|count| count.set(count.get() + 1));
}

fn unresolved_custody() -> DaemonBootstrapFailure {
    use std::io::Write;
    // A closed diagnostic sink must not panic during exact-child Drop.
    let _ = writeln!(std::io::stderr(), "telex: previous compatibility probe cleanup is unconfirmed; original custody and witness remain permanently quarantined without signals; restart the CLIENT process to retry native compatibility probes");
    DaemonBootstrapFailure::IncompatibleManifest
}

fn reserve_custody(
    deadline: Instant,
) -> Result<MutexGuard<'static, Option<Custody>>, DaemonBootstrapFailure> {
    loop {
        if Instant::now() >= deadline {
            return Err(DaemonBootstrapFailure::IncompatibleManifest);
        }
        let slot = match CUSTODY.try_lock() {
            Ok(slot) => slot,
            Err(TryLockError::WouldBlock) => {
                std::thread::sleep(Duration::from_millis(10));
                continue;
            }
            Err(TryLockError::Poisoned(_)) => return Err(unresolved_custody()),
        };
        // A same-process waiter could have consumed the original child and
        // recycled this numeric PID while parked. No later PID query or reap
        // can release a child-bearing record, even if its old flag says owned.
        if slot.is_some() {
            return Err(unresolved_custody());
        }
        return Ok(slot);
    }
}

pub(super) struct Child {
    pid: u32,
    pub(super) stdout: Option<std::io::PipeReader>,
    pub(super) stderr: Option<std::io::PipeReader>,
    pub(super) verified_start: Option<u64>,
    custody: Option<MutexGuard<'static, Option<Custody>>>,
}

impl Child {
    pub(super) fn id(&self) -> u32 {
        self.pid
    }

    pub(super) fn owns_generation(&self) -> bool {
        self.custody
            .as_ref()
            .and_then(|slot| slot.as_ref())
            .is_some_and(|owner| owner.generation_known)
    }

    pub(super) fn lose_generation(&mut self) {
        if let Some(owner) = self.custody.as_mut().and_then(|slot| slot.as_mut()) {
            owner.generation_known = false;
        }
    }

    pub(super) fn mark_reaped(&mut self) {
        if let Some(mut slot) = self.custody.take() {
            slot.take();
        }
    }

    pub(super) fn spawn(
        command: &Command,
        witness: &MacosExecutableWitness,
        deadline: Instant,
    ) -> Result<Self, DaemonBootstrapFailure> {
        let failure = || DaemonBootstrapFailure::IncompatibleManifest;
        if !std::path::Path::new(command.get_program()).is_absolute()
            || command.get_current_dir().is_some()
        {
            return Err(failure());
        }
        let program = CString::new(command.get_program().as_bytes()).map_err(|_| failure())?;
        let arguments = std::iter::once(command.get_program())
            .chain(command.get_args())
            .map(|arg| CString::new(arg.as_bytes()).map_err(|_| failure()))
            .collect::<Result<Vec<_>, _>>()?;
        // Only explicitly supplied values survive; never inherit the host's
        // credentials, loader configuration, signal policy or runtime state.
        let environment = command
            .get_envs()
            .filter_map(|(key, value)| value.map(|value| (key, value)))
            .map(|(key, value)| {
                let mut entry = key.as_bytes().to_vec();
                entry.push(b'=');
                entry.extend_from_slice(value.as_bytes());
                CString::new(entry).map_err(|_| failure())
            })
            .collect::<Result<Vec<_>, _>>()?;
        let pointers = |strings: &[CString]| {
            strings
                .iter()
                .map(|value| value.as_ptr().cast_mut())
                .chain(std::iter::once(std::ptr::null_mut()))
                .collect::<Vec<_>>()
        };
        let argv = pointers(&arguments);
        let envp = pointers(&environment);
        let input = std::fs::File::open("/dev/null").map_err(|_| failure())?;
        let (stdout, stdout_write) = std::io::pipe().map_err(|_| failure())?;
        let (stderr, stderr_write) = std::io::pipe().map_err(|_| failure())?;
        let descriptors = [
            input.as_raw_fd(),
            stdout.as_raw_fd(),
            stdout_write.as_raw_fd(),
            stderr.as_raw_fd(),
            stderr_write.as_raw_fd(),
        ];
        if descriptors.iter().any(|fd| *fd <= libc::STDERR_FILENO) {
            return Err(failure());
        }
        let mut actions = FileActions::new()?;
        for (source, target) in [
            (input.as_raw_fd(), libc::STDIN_FILENO),
            (stdout_write.as_raw_fd(), libc::STDOUT_FILENO),
            (stderr_write.as_raw_fd(), libc::STDERR_FILENO),
        ] {
            if unsafe { libc::posix_spawn_file_actions_adddup2(&mut actions.0, source, target) }
                != 0
            {
                return Err(failure());
            }
        }
        for descriptor in descriptors {
            if unsafe { libc::posix_spawn_file_actions_addclose(&mut actions.0, descriptor) } != 0 {
                return Err(failure());
            }
        }
        let mut attributes = Attributes::new()?;
        let mut mask = unsafe { std::mem::zeroed::<libc::sigset_t>() };
        let mut defaults = unsafe { std::mem::zeroed::<libc::sigset_t>() };
        let flags = libc::POSIX_SPAWN_START_SUSPENDED
            | libc::POSIX_SPAWN_SETPGROUP
            | libc::POSIX_SPAWN_CLOEXEC_DEFAULT
            | libc::POSIX_SPAWN_SETSIGDEF
            | libc::POSIX_SPAWN_SETSIGMASK;
        if unsafe {
            libc::sigemptyset(&mut mask) != 0
                || libc::sigfillset(&mut defaults) != 0
                || libc::sigdelset(&mut defaults, libc::SIGKILL) != 0
                || libc::sigdelset(&mut defaults, libc::SIGSTOP) != 0
                || libc::posix_spawnattr_setsigmask(&mut attributes.0, &mask) != 0
                || libc::posix_spawnattr_setsigdefault(&mut attributes.0, &defaults) != 0
                || libc::posix_spawnattr_setpgroup(&mut attributes.0, 0) != 0
                || libc::posix_spawnattr_setflags(&mut attributes.0, flags as libc::c_short) != 0
        } {
            return Err(failure());
        }
        let slot = reserve_custody(deadline)?;
        let witness_fd = unsafe { libc::fcntl(witness.file.as_raw_fd(), libc::F_DUPFD_CLOEXEC, 3) };
        if witness_fd < 0 {
            return Err(failure());
        }
        let held_witness = unsafe { std::fs::File::from_raw_fd(witness_fd) };
        let mut child = Self {
            pid: 0,
            stdout: Some(stdout),
            stderr: Some(stderr),
            verified_start: None,
            custody: Some(slot),
        };
        let Some(slot) = child.custody.as_mut() else {
            return Err(failure());
        };
        let owner = slot.insert(Custody {
            pid: 0,
            _witness: held_witness,
            _verified_start: None,
            generation_known: true,
        });
        // The global slot and witness are armed before spawn, and the kernel
        // writes its PID directly into that custody. Nothing fallible allocates
        // or transfers ownership after successful spawn.
        let result = unsafe {
            libc::posix_spawn(
                &mut owner.pid,
                program.as_ptr(),
                &actions.0,
                &attributes.0,
                argv.as_ptr(),
                envp.as_ptr(),
            )
        };
        if result != 0 {
            slot.take();
            return Err(failure());
        }
        child.pid = owner.pid as u32;
        Ok(child)
    }
}

impl Drop for Child {
    fn drop(&mut self) {
        if let Some(owner) = self.custody.as_mut().and_then(|slot| slot.as_mut()) {
            owner._verified_start = self.verified_start;
            owner.generation_known = false;
            // The slot retains original custody and the CLOEXEC witness after
            // unlocking, never future wait authority over the stored PID.
            unresolved_custody();
        }
    }
}

struct FileActions(libc::posix_spawn_file_actions_t);

impl FileActions {
    fn new() -> Result<Self, DaemonBootstrapFailure> {
        let mut actions = std::mem::MaybeUninit::uninit();
        if unsafe { libc::posix_spawn_file_actions_init(actions.as_mut_ptr()) } != 0 {
            return Err(DaemonBootstrapFailure::IncompatibleManifest);
        }
        Ok(Self(unsafe { actions.assume_init() }))
    }
}

impl Drop for FileActions {
    fn drop(&mut self) {
        unsafe {
            libc::posix_spawn_file_actions_destroy(&mut self.0);
        }
    }
}

struct Attributes(libc::posix_spawnattr_t);

impl Attributes {
    fn new() -> Result<Self, DaemonBootstrapFailure> {
        let mut attributes = std::mem::MaybeUninit::uninit();
        if unsafe { libc::posix_spawnattr_init(attributes.as_mut_ptr()) } != 0 {
            return Err(DaemonBootstrapFailure::IncompatibleManifest);
        }
        Ok(Self(unsafe { attributes.assume_init() }))
    }
}

impl Drop for Attributes {
    fn drop(&mut self) {
        unsafe {
            libc::posix_spawnattr_destroy(&mut self.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{DirBuilderExt, MetadataExt};

    #[test]
    #[ignore = "subprocess role used by the real native custody-retention regression"]
    fn custody_fixture() {
        let Some(case) = std::env::var_os("TELEX_TEST_PROBE_CUSTODY_CASE") else {
            return;
        };
        let root = std::env::temp_dir().join(format!(
            "telex-custody-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&root)
            .unwrap();
        let marker = root.join("entry");
        let executable = std::env::current_exe().unwrap();
        let witness = super::super::open_macos_witness(&executable).unwrap();
        let identity = witness.identity;
        let command = || {
            let mut command = Command::new(&executable);
            command
                .env_clear()
                .env("TELEX_TEST_PROBE_ENTRY_MARKER", &marker)
                .args([
                    "--exact",
                    "daemon_bootstrap::tests::macos_admission_marker_fixture",
                    "--ignored",
                ]);
            command
        };
        if case == "deadline" {
            let output = super::super::run_bounded_probe_process(
                command(),
                Duration::from_secs(10),
                4096,
                &witness,
            )
            .expect("normal authenticated probe before quarantine");
            assert!(output.success && marker.is_file());
            assert!(CUSTODY.try_lock().unwrap().is_none());
            std::fs::remove_file(&marker).unwrap();
            GROUP_SIGNALS.with(|count| count.set(0));
            CONTINUES.with(|count| count.set(0));
        }
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut child = Child::spawn(&command(), &witness, deadline).unwrap();
        let mut group = super::super::ProbeProcessGroup::attach(&child).unwrap();
        let fd = child
            .custody
            .as_ref()
            .unwrap()
            .as_ref()
            .unwrap()
            ._witness
            .as_raw_fd();
        assert_ne!(
            unsafe { libc::fcntl(fd, libc::F_GETFD) } & libc::FD_CLOEXEC,
            0
        );
        let pid = child.id() as libc::pid_t;
        group.signal_and_seal(&mut child).unwrap();
        drop(witness);

        if case == "deadline" {
            REAP_PENDING.with(|pending| pending.set(true));
            assert_eq!(
                super::super::finalize_probe_process(&mut child, &mut group, Instant::now()),
                Err(DaemonBootstrapFailure::IncompatibleManifest)
            );
            assert!(child.owns_generation());
        } else {
            // A real competing waiter consumes the terminal child. The
            // production adapter must retain, never guess at a recycled PID.
            loop {
                let mut status = 0;
                let result = unsafe { libc::waitpid(pid, &mut status, libc::WNOHANG) };
                if result == pid {
                    break;
                }
                assert!(result >= 0 && Instant::now() < deadline);
                std::thread::sleep(Duration::from_millis(10));
            }
            if case == "reused" {
                assert!(
                    child.owns_generation(),
                    "competing reap was not observed by the adapter"
                );
            } else {
                if case == "eintr" {
                    OBSERVE_ERROR.with(|error| error.set(Some(libc::EINTR)));
                } else {
                    assert_eq!(case, "echild");
                }
                assert!(super::super::observe_probe_leader(&mut child).is_err());
                assert!(!child.owns_generation());
                assert!(super::super::resume_probe_process(&mut child, deadline).is_err());
                assert_eq!(
                    super::super::fail_probe_process(
                        &mut child,
                        &mut group,
                        deadline,
                        DaemonBootstrapFailure::ExecutableIdentityMismatch
                    ),
                    DaemonBootstrapFailure::IncompatibleManifest
                );
            }
        }
        assert_eq!(GROUP_SIGNALS.with(|count| count.get()), 1);
        assert_eq!(CONTINUES.with(|count| count.get()), 0);
        drop(child);
        {
            let slot = CUSTODY.try_lock().unwrap();
            let owner = slot
                .as_ref()
                .expect("unconfirmed cleanup retains exact custody");
            let metadata = owner._witness.metadata().unwrap();
            assert_eq!(
                (metadata.dev(), metadata.ino()),
                (identity.high, identity.low)
            );
            assert_eq!(owner.pid, pid);
        }
        let mut unrelated = if case == "reused" {
            let mut command = command();
            command
                .env(
                    "TELEX_TEST_PROBE_ENTRY_MARKER",
                    root.join("unrelated-entry"),
                )
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null());
            let other = command.spawn().expect("different real child");
            loop {
                match super::super::observe_probe_pid(other.id()) {
                    Ok(super::super::ProbeLeaderState::Exited(_)) => break,
                    Ok(super::super::ProbeLeaderState::Running) => {
                        assert!(Instant::now() < deadline);
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    Err(error) => panic!("unrelated child observation: {error:?}"),
                }
            }
            // Simulate numeric-PID reuse after an UNOBSERVED competing reap.
            // The new PID belongs to a different REAL terminal child; a stale
            // flag/start must never authorize stealing its consuming wait.
            let mut slot = CUSTODY.try_lock().unwrap();
            let owner = slot.as_mut().unwrap();
            owner.pid = other.id() as libc::pid_t;
            owner._verified_start = Some(u64::MAX);
            owner.generation_known = true;
            Some(other)
        } else {
            None
        };
        let observations = OBSERVATIONS.with(|count| count.get());
        let witness = super::super::open_macos_witness(&executable).unwrap();
        assert!(
            Child::spawn(&command(), &witness, deadline).is_err(),
            "ANY parked child-bearing record refuses admission"
        );
        assert_eq!(
            OBSERVATIONS.with(|count| count.get()),
            observations,
            "parked admission performs no PID observation"
        );
        {
            let slot = CUSTODY.try_lock().unwrap();
            let owner = slot.as_ref().expect("no parked slot/witness release");
            let metadata = owner._witness.metadata().unwrap();
            assert_eq!(
                (metadata.dev(), metadata.ino()),
                (identity.high, identity.low)
            );
            assert_eq!(owner._witness.as_raw_fd(), fd);
        }
        if let Some(other) = unrelated.as_mut() {
            assert!(other
                .wait()
                .expect("unrelated terminal status was NOT reaped")
                .success());
        }
        assert_eq!(GROUP_SIGNALS.with(|count| count.get()), 1);
        assert_eq!(CONTINUES.with(|count| count.get()), 0);
        assert!(!marker.exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn native_returned_errors_permanently_retain_parked_custody_without_reaping_reused_pid() {
        for case in ["deadline", "echild", "eintr", "reused"] {
            let output = Command::new(std::env::current_exe().unwrap())
                .env("TELEX_TEST_PROBE_CUSTODY_CASE", case)
                .args([
                    "--exact",
                    "daemon_bootstrap::macos_probe::tests::custody_fixture",
                    "--ignored",
                    "--nocapture",
                ])
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{case}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }
}
