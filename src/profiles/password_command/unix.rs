#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(super) use supported::Process;

#[cfg(any(target_os = "linux", target_os = "macos"))]
mod supported {
    use super::super::Failure;
    use std::io;
    use std::os::fd::{AsRawFd, IntoRawFd, RawFd};
    use std::os::unix::process::CommandExt;
    use std::process::{Child, ChildStderr, ChildStdout, Command, Stdio};
    use std::sync::atomic::{AtomicBool, Ordering};

    const READ_SIZE: usize = 4096;
    const READS_PER_PIPE: usize = 4;

    // These are deliberately different types: an observation key cannot be
    // passed to the only mutating group operation.
    #[derive(Debug)]
    struct Anchor(libc::pid_t);
    #[derive(Debug)]
    struct SealedLeader(libc::pid_t);
    #[derive(Debug)]
    struct FormerGroup(libc::pid_t);

    #[derive(Debug)]
    enum Phase {
        Empty,
        Anchored(Anchor),
        SignalsSealed(SealedLeader),
        LeaderReaped(FormerGroup),
        Receipt,
        Finalized,
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    enum LeaderState {
        Running,
        Exited(i32),
        Interrupted,
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    enum GroupState {
        Present,
        Absent,
        Interrupted,
    }

    #[derive(Debug, PartialEq, Eq)]
    enum FinalSignalAttempt {
        Accepted,
        Failed(Failure),
    }

    trait Operations {
        fn check_sigchld(&mut self) -> Result<(), Failure>;
        fn observe(&mut self, pid: libc::pid_t) -> Result<LeaderState, Failure>;
        fn final_signal(&mut self, anchor: &Anchor) -> Result<(), Failure>;
        fn reap(&mut self, leader: &SealedLeader) -> Result<bool, Failure>;
        fn group(&mut self, key: &FormerGroup) -> Result<GroupState, Failure>;
    }

    struct Lifecycle {
        phase: Phase,
        status: Option<i32>,
        final_attempt: Option<FinalSignalAttempt>,
        failure: Option<Failure>,
        #[cfg(test)]
        signals: usize,
        #[cfg(test)]
        reaps: usize,
    }

    impl Lifecycle {
        fn new() -> Self {
            Self {
                phase: Phase::Empty,
                status: None,
                final_attempt: None,
                failure: None,
                #[cfg(test)]
                signals: 0,
                #[cfg(test)]
                reaps: 0,
            }
        }

        fn anchor(&mut self, id: u32) -> Result<(), Failure> {
            match libc::pid_t::try_from(id) {
                Ok(pid) if pid > 1 => {
                    self.phase = Phase::Anchored(Anchor(pid));
                    Ok(())
                }
                _ => self.hold(Failure::at("invalid child identity")),
            }
        }

        fn hold<T>(&mut self, failure: Failure) -> Result<T, Failure> {
            self.failure = Some(failure.clone());
            Err(failure)
        }

        fn check_failure(&self) -> Result<(), Failure> {
            match &self.failure {
                Some(failure) => Err(failure.clone()),
                None => Ok(()),
            }
        }

        fn exit_status(&mut self, ops: &mut impl Operations) -> Result<Option<i32>, Failure> {
            self.check_failure()?;
            if let Phase::Anchored(anchor) = &self.phase {
                let observed = ops.check_sigchld().and_then(|()| ops.observe(anchor.0));
                match observed {
                    Ok(LeaderState::Exited(status)) => self.status = Some(status),
                    Ok(LeaderState::Running | LeaderState::Interrupted) => {}
                    Err(failure) => return self.hold(failure),
                }
            }
            Ok(self.status)
        }

        fn cleanup(&mut self, ops: &mut impl Operations) -> Result<bool, Failure> {
            self.check_failure()?;
            match self.cleanup_step(ops) {
                Ok(complete) => Ok(complete),
                Err(failure) => self.hold(failure),
            }
        }

        fn cleanup_step(&mut self, ops: &mut impl Operations) -> Result<bool, Failure> {
            if let Phase::Empty = self.phase {
                self.phase = Phase::Receipt;
            }
            if let Phase::Anchored(anchor) = &self.phase {
                ops.check_sigchld()?;
                // A cached exit status is not a fresh assertion of exclusive
                // wait authority. In particular ECHILD must prevent signaling.
                match ops.observe(anchor.0)? {
                    LeaderState::Interrupted => return Ok(false),
                    LeaderState::Exited(status) => self.status = Some(status),
                    LeaderState::Running => {}
                }
                #[cfg(test)]
                {
                    self.signals += 1;
                }
                let attempt = match ops.final_signal(anchor) {
                    Ok(()) => FinalSignalAttempt::Accepted,
                    Err(failure) => FinalSignalAttempt::Failed(failure),
                };
                self.phase = Phase::SignalsSealed(SealedLeader(anchor.0));
                // Darwin may deny a nonzero signal to a zombie-only group.
                // Retain that FAILED attempt, not a successful kill or absence.
                // Only this anchored final-attempt EPERM may continue to the
                // independent receipt checks; no path regains signal authority.
                let continuation = match &attempt {
                    FinalSignalAttempt::Accepted => Ok(()),
                    FinalSignalAttempt::Failed(failure)
                        if cfg!(target_os = "macos") && failure.os_code == Some(libc::EPERM) =>
                    {
                        Ok(())
                    }
                    FinalSignalAttempt::Failed(failure) => Err(failure.clone()),
                };
                self.final_attempt = Some(attempt);
                continuation?;
            }
            if let Phase::SignalsSealed(leader) = &self.phase {
                ops.check_sigchld()?;
                if !ops.reap(leader)? {
                    return Ok(false);
                }
                #[cfg(test)]
                {
                    self.reaps += 1;
                }
                self.phase = Phase::LeaderReaped(FormerGroup(leader.0));
            }
            if let Phase::LeaderReaped(key) = &self.phase {
                match ops.group(key)? {
                    GroupState::Absent => self.phase = Phase::Receipt,
                    GroupState::Present | GroupState::Interrupted => return Ok(false),
                }
            }
            Ok(matches!(self.phase, Phase::Receipt | Phase::Finalized))
        }
    }

    #[derive(Debug, PartialEq, Eq)]
    enum CloseState {
        NotAttempted,
        Confirmed,
        Unconfirmed {
            // Audit identity only; never reused as an owned descriptor.
            _descriptor: RawFd,
            failure: Failure,
        },
    }

    impl CloseState {
        fn check(&self) -> Result<(), Failure> {
            match self {
                Self::NotAttempted | Self::Confirmed => Ok(()),
                Self::Unconfirmed { failure, .. } => Err(failure.clone()),
            }
        }
    }

    fn close_pipe<T: IntoRawFd>(
        pipe: &mut Option<T>,
        state: &mut CloseState,
        stage: &'static str,
    ) -> Result<(), Failure> {
        close_pipe_with(pipe, state, stage, |fd| {
            if unsafe { libc::close(fd) } == 0 {
                Ok(())
            } else {
                Err(io::Error::last_os_error())
            }
        })
    }

    fn close_pipe_with<T: IntoRawFd>(
        pipe: &mut Option<T>,
        state: &mut CloseState,
        stage: &'static str,
        close: impl FnOnce(RawFd) -> io::Result<()>,
    ) -> Result<(), Failure> {
        state.check()?;
        if let Some(pipe) = pipe.take() {
            // close(EINTR) has platform-dependent ownership semantics. Consume
            // the Rust owner once, retain uncertainty even on unwind, and never
            // retry/reconstruct a descriptor that another thread could reuse.
            let fd = pipe.into_raw_fd();
            *state = CloseState::Unconfirmed {
                _descriptor: fd,
                failure: Failure::at(stage),
            };
            *state = match close(fd) {
                Ok(()) => CloseState::Confirmed,
                Err(error) => CloseState::Unconfirmed {
                    _descriptor: fd,
                    failure: Failure::io(stage, error),
                },
            };
        }
        state.check()
    }

    pub(crate) struct Process {
        // std::process::Child has no signal/reap-on-drop behavior. Never call
        // its try_wait/wait: waitid and exact waitpid below are the sole waiter.
        child: Option<Child>,
        stdout: Option<ChildStdout>,
        stderr: Option<ChildStderr>,
        stdout_close: CloseState,
        stderr_close: CloseState,
        lifecycle: Lifecycle,
        started: bool,
        io_ready: bool,
        stopping: bool,
    }

    impl Process {
        pub(crate) fn new() -> Self {
            Self {
                child: None,
                stdout: None,
                stderr: None,
                stdout_close: CloseState::NotAttempted,
                stderr_close: CloseState::NotAttempted,
                lifecycle: Lifecycle::new(),
                started: false,
                io_ready: false,
                stopping: false,
            }
        }

        pub(crate) fn spawn(
            &mut self,
            command: &str,
            cancelled: &AtomicBool,
        ) -> Result<(), Failure> {
            if self.started || self.stopping {
                return Err(Failure::at("invalid spawn phase"));
            }
            self.started = true;
            check_cancelled(cancelled)?;
            Native.check_sigchld()?;
            check_cancelled(cancelled)?;
            let child = Command::new("sh")
                .arg("-c")
                .arg(command)
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                // std checks child-side setpgid(0, 0) before exec succeeds.
                .process_group(0)
                .spawn()
                .map_err(|error| Failure::io("spawn", error))?;

            // Install ALL resources before the next fallible operation.
            self.child = Some(child);
            let child = self.child.as_mut().expect("child just installed");
            self.stdout = child.stdout.take();
            self.stderr = child.stderr.take();
            self.lifecycle.anchor(child.id())?;
            check_cancelled(cancelled)?;
            Native.check_sigchld()?;
            if self.stdout.is_none() || self.stderr.is_none() {
                return Err(Failure::at("missing output pipe"));
            }
            nonblocking(self.stdout.as_ref().expect("checked stdout").as_raw_fd())?;
            check_cancelled(cancelled)?;
            nonblocking(self.stderr.as_ref().expect("checked stderr").as_raw_fd())?;
            check_cancelled(cancelled)?;
            self.io_ready = true;
            Ok(())
        }

        pub(crate) fn read_available(&mut self, stdout: &mut Vec<u8>) -> Result<(), Failure> {
            if self.stopping {
                return Err(Failure::at("output collection after cleanup"));
            }
            if !self.io_ready {
                return Err(Failure::at("output setup incomplete"));
            }
            self.lifecycle.check_failure()?;
            self.stdout_close.check()?;
            self.stderr_close.check()?;
            let mut fds = [poll_fd(self.stdout.as_ref()), poll_fd(self.stderr.as_ref())];
            // No blocking read or independent reader; both pipes get a bounded
            // quantum on each owner turn. The parent checks cancellation/budget.
            let result = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as _, 0) };
            if result < 0 {
                let error = io::Error::last_os_error();
                return if error.raw_os_error() == Some(libc::EINTR) {
                    Ok(())
                } else {
                    Err(Failure::io("output readiness", error))
                };
            }
            for (index, fd) in fds.iter().enumerate() {
                if fd.revents & libc::POLLNVAL != 0 {
                    return Err(Failure::at("invalid output pipe"));
                }
                if fd.revents & (libc::POLLIN | libc::POLLHUP | libc::POLLERR) == 0 {
                    continue;
                }
                if index == 0 {
                    if drain(fd.fd, Some(&mut *stdout))? {
                        close_pipe(&mut self.stdout, &mut self.stdout_close, "stdout close")?;
                    }
                } else if drain(fd.fd, None)? {
                    close_pipe(&mut self.stderr, &mut self.stderr_close, "stderr close")?;
                }
            }
            Ok(())
        }

        pub(crate) fn output_closed(&self) -> bool {
            self.stdout.is_none()
                && self.stderr.is_none()
                && self.stdout_close.check().is_ok()
                && self.stderr_close.check().is_ok()
        }

        pub(crate) fn exit_status(&mut self) -> Result<Option<i32>, Failure> {
            self.lifecycle.exit_status(&mut Native)
        }

        pub(crate) fn begin_cleanup(&mut self) -> Result<(), Failure> {
            self.stopping = true;
            // The parent selects normal cleanup only after status zero AND EOF.
            // Error/cancellation may abandon I/O; no partial result is published.
            close_pipe(&mut self.stdout, &mut self.stdout_close, "stdout close")?;
            close_pipe(&mut self.stderr, &mut self.stderr_close, "stderr close")?;
            self.lifecycle.cleanup(&mut Native).map(|_| ())
        }

        pub(crate) fn poll_cleanup(&mut self) -> Result<bool, Failure> {
            if !self.stopping {
                return Err(Failure::at("cleanup not started"));
            }
            self.stdout_close.check()?;
            self.stderr_close.check()?;
            Ok(self.lifecycle.cleanup(&mut Native)? && self.output_closed())
        }

        // The native owner calls after cleanup receipt, before returning, without
        // a registry lock. The collector must still join before publication/release.
        pub(crate) fn finalize(&mut self) -> Result<(), Failure> {
            self.lifecycle.check_failure()?;
            self.stdout_close.check()?;
            self.stderr_close.check()?;
            if !self.stopping
                || !matches!(self.lifecycle.phase, Phase::Receipt | Phase::Finalized)
                || !self.output_closed()
            {
                return Err(Failure::at("finalization before cleanup receipt"));
            }
            if let Some(child) = &self.child {
                if child.stdin.is_some() || child.stdout.is_some() || child.stderr.is_some() {
                    return Err(Failure::at("unaccounted child I/O at finalization"));
                }
            }
            // Exact reap was already observed. Unix Child has no remaining
            // native handle or wait obligation, and all pipe closes are checked.
            self.child = None;
            self.lifecycle.phase = Phase::Finalized;
            Ok(())
        }
    }

    // No custom Drop: in particular it must never signal a historical group.
    // The native owner/registry must retain this entire value on missing receipt.

    fn check_cancelled(cancelled: &AtomicBool) -> Result<(), Failure> {
        if cancelled.load(Ordering::Acquire) {
            Err(Failure::at("cancelled"))
        } else {
            Ok(())
        }
    }

    fn nonblocking(fd: RawFd) -> Result<(), Failure> {
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
        if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
            return Err(Failure::io(
                "nonblocking output",
                io::Error::last_os_error(),
            ));
        }
        Ok(())
    }

    fn poll_fd<T: AsRawFd>(pipe: Option<&T>) -> libc::pollfd {
        libc::pollfd {
            fd: pipe.map_or(-1, AsRawFd::as_raw_fd),
            events: libc::POLLIN,
            revents: 0,
        }
    }

    fn drain(fd: RawFd, mut output: Option<&mut Vec<u8>>) -> Result<bool, Failure> {
        let mut buffer = [0; READ_SIZE];
        for _ in 0..READS_PER_PIPE {
            let count = unsafe { libc::read(fd, buffer.as_mut_ptr().cast(), buffer.len()) };
            if count == 0 {
                return Ok(true);
            }
            if count < 0 {
                let error = io::Error::last_os_error();
                if error.kind() == io::ErrorKind::WouldBlock
                    || error.raw_os_error() == Some(libc::EINTR)
                {
                    return Ok(false);
                }
                return Err(Failure::io("output read", error));
            }
            if let Some(output) = output.as_mut() {
                output
                    .try_reserve(count as usize)
                    .map_err(|_| Failure::at("output allocation"))?;
                output.extend_from_slice(&buffer[..count as usize]);
            }
        }
        Ok(false)
    }

    struct Native;

    fn sigchld_compatible(handler: libc::sighandler_t, flags: libc::c_int) -> bool {
        handler != libc::SIG_IGN && flags & libc::SA_NOCLDWAIT as libc::c_int == 0
    }

    impl Operations for Native {
        fn check_sigchld(&mut self) -> Result<(), Failure> {
            let mut action = unsafe { std::mem::zeroed::<libc::sigaction>() };
            // Query only: never install/repair a host signal handler. A host
            // must also honor exclusive wait ownership, including in handlers.
            if unsafe { libc::sigaction(libc::SIGCHLD, std::ptr::null(), &mut action) } != 0 {
                return Err(Failure::io("SIGCHLD query", io::Error::last_os_error()));
            }
            if !sigchld_compatible(action.sa_sigaction, action.sa_flags as libc::c_int) {
                return Err(Failure::at("SIGCHLD automatic reaping"));
            }
            Ok(())
        }

        fn observe(&mut self, pid: libc::pid_t) -> Result<LeaderState, Failure> {
            let mut info = unsafe { std::mem::zeroed::<libc::siginfo_t>() };
            let result = unsafe {
                libc::waitid(
                    libc::P_PID,
                    pid as libc::id_t,
                    &mut info,
                    libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
                )
            };
            if result != 0 {
                let error = io::Error::last_os_error();
                return if error.raw_os_error() == Some(libc::EINTR) {
                    Ok(LeaderState::Interrupted)
                } else {
                    Err(Failure::io("child ownership observation", error))
                };
            }
            let observed_pid = unsafe { info.si_pid() };
            if observed_pid == 0 {
                return Ok(LeaderState::Running);
            }
            if observed_pid != pid {
                return Err(Failure::at("child observation identity"));
            }
            match info.si_code {
                libc::CLD_EXITED => Ok(LeaderState::Exited(unsafe { info.si_status() })),
                libc::CLD_KILLED | libc::CLD_DUMPED => {
                    Ok(LeaderState::Exited(128 + unsafe { info.si_status() }))
                }
                _ => Err(Failure::at("unexpected child observation")),
            }
        }

        fn final_signal(&mut self, anchor: &Anchor) -> Result<(), Failure> {
            // The sole nonzero group signal in this module. Anchor is never
            // constructible from SealedLeader or FormerGroup.
            if unsafe { libc::kill(-anchor.0, libc::SIGKILL) } != 0 {
                return Err(Failure::io(
                    "final group signal",
                    io::Error::last_os_error(),
                ));
            }
            Ok(())
        }

        fn reap(&mut self, leader: &SealedLeader) -> Result<bool, Failure> {
            let mut status = 0;
            let result = unsafe { libc::waitpid(leader.0, &mut status, libc::WNOHANG) };
            if result < 0 {
                let error = io::Error::last_os_error();
                return if error.raw_os_error() == Some(libc::EINTR) {
                    Ok(false)
                } else {
                    Err(Failure::io("exact child reap", error))
                };
            }
            if result == 0 {
                return Ok(false);
            }
            if result != leader.0 || !(libc::WIFEXITED(status) || libc::WIFSIGNALED(status)) {
                return Err(Failure::at("exact child reap evidence"));
            }
            Ok(true)
        }

        fn group(&mut self, key: &FormerGroup) -> Result<GroupState, Failure> {
            group_state(key)
        }
    }

    #[cfg(target_os = "linux")]
    fn group_state(key: &FormerGroup) -> Result<GroupState, Failure> {
        // libc's supported per-thread errno binding, not a process-global cell.
        let (value, errno) = unsafe {
            let errno = libc::__errno_location();
            *errno = 0;
            let value = libc::getpriority(libc::PRIO_PGRP, key.0 as libc::id_t);
            (value, *errno)
        };
        linux_group_result(value, errno)
    }

    #[cfg(any(target_os = "linux", test))]
    fn linux_group_result(value: libc::c_int, errno: libc::c_int) -> Result<GroupState, Failure> {
        if value != -1 || errno == 0 {
            return Ok(GroupState::Present);
        }
        match errno {
            libc::ESRCH => Ok(GroupState::Absent),
            libc::EINTR => Ok(GroupState::Interrupted),
            _ => Err(Failure::io(
                "group absence observation",
                io::Error::from_raw_os_error(errno),
            )),
        }
    }

    #[cfg(target_os = "macos")]
    fn group_state(key: &FormerGroup) -> Result<GroupState, Failure> {
        // Audited libc 0.2.186 src/unix/mod.rs:1169-1173 binds x86 macOS
        // kill to kill$UNIX2003. x86_64/aarch64 use Darwin's default POSIX ABI.
        // src/unix/bsd/apple/mod.rs:4689-4693 does the same for x86 waitid.
        // This is source-binding evidence, NOT macOS denial/zombie runtime proof.
        #[cfg(any(target_arch = "x86", target_arch = "x86_64", target_arch = "aarch64"))]
        {
            let (result, errno) = unsafe {
                let errno = libc::__error();
                *errno = 0;
                let result = libc::kill(-key.0, 0);
                (result, *errno)
            };
            macos_group_result(result, errno)
        }
        #[cfg(not(any(target_arch = "x86", target_arch = "x86_64", target_arch = "aarch64")))]
        {
            let _ = key;
            Err(Failure::at("unsupported group observation ABI"))
        }
    }

    #[cfg(any(target_os = "macos", test))]
    fn macos_group_result(value: libc::c_int, errno: libc::c_int) -> Result<GroupState, Failure> {
        match (value, errno) {
            (0, _) | (-1, libc::EPERM) => Ok(GroupState::Present),
            (-1, libc::ESRCH) => Ok(GroupState::Absent),
            (-1, libc::EINTR) => Ok(GroupState::Interrupted),
            _ => Err(Failure::io(
                "group absence observation",
                io::Error::from_raw_os_error(errno),
            )),
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::io::Read;
        use std::os::unix::net::UnixStream;
        use std::path::{Path, PathBuf};
        use std::sync::atomic::AtomicU64;
        use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

        struct Fake {
            events: Vec<&'static str>,
            failure_at: Option<&'static str>,
            failure_code: i32,
            observed: LeaderState,
            reaped: bool,
            group: GroupState,
        }

        impl Fake {
            fn new() -> Self {
                Self {
                    events: Vec::new(),
                    failure_at: None,
                    failure_code: libc::EIO,
                    observed: LeaderState::Exited(0),
                    reaped: true,
                    group: GroupState::Absent,
                }
            }

            fn call(&mut self, stage: &'static str) -> Result<(), Failure> {
                self.events.push(stage);
                if self.failure_at == Some(stage) {
                    Err(Failure {
                        stage,
                        os_code: Some(self.failure_code),
                    })
                } else {
                    Ok(())
                }
            }
        }

        impl Operations for Fake {
            fn check_sigchld(&mut self) -> Result<(), Failure> {
                self.call("policy")
            }
            fn observe(&mut self, _pid: libc::pid_t) -> Result<LeaderState, Failure> {
                self.call("observe")?;
                Ok(self.observed.clone())
            }
            fn final_signal(&mut self, anchor: &Anchor) -> Result<(), Failure> {
                assert!(anchor.0 > 1);
                self.call("signal")
            }
            fn reap(&mut self, _leader: &SealedLeader) -> Result<bool, Failure> {
                self.call("reap")?;
                Ok(self.reaped)
            }
            fn group(&mut self, _key: &FormerGroup) -> Result<GroupState, Failure> {
                self.call("group")?;
                Ok(self.group.clone())
            }
        }

        fn lifecycle() -> Lifecycle {
            let mut lifecycle = Lifecycle::new();
            // This label is used ONLY by Fake; never passed to a native call.
            lifecycle.anchor(123).unwrap();
            lifecycle
        }

        #[test]
        fn signal_seal_reap_absence_order_is_irreversible() {
            let mut owner = lifecycle();
            let mut ops = Fake::new();
            assert!(owner.cleanup(&mut ops).unwrap());
            assert_eq!(
                ops.events,
                ["policy", "observe", "signal", "policy", "reap", "group"]
            );
            assert_eq!((owner.signals, owner.reaps), (1, 1));
            let calls = ops.events.len();
            for _ in 0..3 {
                assert!(owner.cleanup(&mut ops).unwrap());
                assert_eq!(owner.exit_status(&mut ops).unwrap(), Some(0));
            }
            assert_eq!(ops.events.len(), calls);
        }

        #[test]
        fn every_ownership_error_retains_phase_and_forbids_retry_signals() {
            for stage in ["policy", "observe", "signal", "reap", "group"] {
                let mut owner = lifecycle();
                let mut ops = Fake::new();
                ops.failure_at = Some(stage);
                let failure = owner.cleanup(&mut ops).unwrap_err();
                assert_eq!(failure.stage, stage);
                match stage {
                    "policy" | "observe" => assert!(matches!(owner.phase, Phase::Anchored(_))),
                    "signal" | "reap" => assert!(matches!(owner.phase, Phase::SignalsSealed(_))),
                    "group" => assert!(matches!(owner.phase, Phase::LeaderReaped(_))),
                    _ => unreachable!(),
                }
                let calls = ops.events.len();
                for _ in 0..3 {
                    assert_eq!(owner.cleanup(&mut ops).unwrap_err(), failure);
                    assert_eq!(owner.exit_status(&mut ops).unwrap_err(), failure);
                }
                assert_eq!(ops.events.len(), calls, "{stage}");
            }
        }

        #[test]
        fn signal_and_leader_reap_are_not_group_receipts() {
            let mut owner = lifecycle();
            let mut ops = Fake::new();
            ops.reaped = false;
            assert!(!owner.cleanup(&mut ops).unwrap());
            assert!(matches!(owner.phase, Phase::SignalsSealed(_)));
            assert!(!ops.events.contains(&"group"));
            ops.reaped = true;
            ops.group = GroupState::Present;
            assert!(!owner.cleanup(&mut ops).unwrap());
            assert!(matches!(owner.phase, Phase::LeaderReaped(_)));
            // A surviving member, zombie, or reused label permits observation,
            // never resurrection of mutating authority.
            for state in [
                GroupState::Present,
                GroupState::Interrupted,
                GroupState::Present,
            ] {
                ops.group = state;
                assert!(!owner.cleanup(&mut ops).unwrap());
                assert_eq!((owner.signals, owner.reaps), (1, 1));
            }
            ops.group = GroupState::Absent;
            assert!(owner.cleanup(&mut ops).unwrap());
        }

        #[test]
        fn final_signal_esrch_is_held_not_an_absence_receipt() {
            let mut owner = lifecycle();
            let mut ops = Fake::new();
            ops.failure_at = Some("signal");
            ops.failure_code = libc::ESRCH;
            assert_eq!(
                owner.cleanup(&mut ops).unwrap_err().os_code,
                Some(libc::ESRCH)
            );
            assert!(matches!(owner.phase, Phase::SignalsSealed(_)));
            assert_eq!((owner.signals, owner.reaps), (1, 0));
            assert!(!ops.events.contains(&"group"));
            assert!(owner.cleanup(&mut ops).is_err());
            assert_eq!(owner.signals, 1);
        }

        #[test]
        fn final_signal_error_policy_is_platform_specific_and_retains_attempt_evidence() {
            for code in [
                libc::EPERM,
                libc::ECHILD,
                libc::ESRCH,
                libc::EACCES,
                libc::EINVAL,
                libc::ENOSYS,
                libc::EIO,
                libc::EINTR,
            ] {
                let mut owner = lifecycle();
                let mut ops = Fake::new();
                ops.failure_at = Some("signal");
                ops.failure_code = code;
                let failure = Failure {
                    stage: "signal",
                    os_code: Some(code),
                };
                let result = owner.cleanup(&mut ops);
                assert_eq!(
                    owner.final_attempt,
                    Some(FinalSignalAttempt::Failed(failure.clone()))
                );
                if cfg!(target_os = "macos") && code == libc::EPERM {
                    assert!(result.unwrap());
                    assert!(matches!(owner.phase, Phase::Receipt));
                    assert_eq!(owner.reaps, 1);
                } else {
                    assert_eq!(result.unwrap_err(), failure);
                    assert!(matches!(owner.phase, Phase::SignalsSealed(_)));
                    assert_eq!(owner.reaps, 0);
                    assert!(!ops.events.contains(&"group"));
                    assert!(owner.cleanup(&mut ops).is_err());
                }
                assert_eq!(owner.signals, 1);
            }
        }

        #[test]
        fn eperm_outside_the_final_signal_remains_an_ownership_failure() {
            for stage in ["policy", "observe", "reap", "group"] {
                let mut owner = lifecycle();
                let mut ops = Fake::new();
                ops.failure_at = Some(stage);
                ops.failure_code = libc::EPERM;
                let failure = owner.cleanup(&mut ops).unwrap_err();
                assert_eq!(failure.stage, stage);
                assert_eq!(failure.os_code, Some(libc::EPERM));
                let calls = ops.events.len();
                assert_eq!(owner.cleanup(&mut ops).unwrap_err(), failure);
                assert_eq!(ops.events.len(), calls);
                if matches!(stage, "policy" | "observe") {
                    assert_eq!(owner.final_attempt, None);
                    assert_eq!(owner.signals, 0);
                } else {
                    assert_eq!(owner.final_attempt, Some(FinalSignalAttempt::Accepted));
                    assert_eq!(owner.signals, 1);
                }
            }
        }

        #[cfg(target_os = "macos")]
        #[test]
        fn macos_failed_eperm_attempt_seals_then_requires_independent_reap_and_absence() {
            for (observed, status) in [
                (LeaderState::Exited(0), Some(0)),
                (LeaderState::Exited(7), Some(7)),
                (LeaderState::Running, None),
            ] {
                let mut owner = lifecycle();
                let mut ops = Fake::new();
                ops.observed = observed;
                ops.failure_at = Some("signal");
                ops.failure_code = libc::EPERM;
                ops.reaped = false;
                let evidence = Some(FinalSignalAttempt::Failed(Failure {
                    stage: "signal",
                    os_code: Some(libc::EPERM),
                }));
                assert!(!owner.cleanup(&mut ops).unwrap());
                assert!(matches!(owner.phase, Phase::SignalsSealed(_)));
                assert_eq!(owner.final_attempt, evidence);
                assert_eq!(owner.failure, None);
                assert_eq!(owner.status, status);
                assert_eq!((owner.signals, owner.reaps), (1, 0));
                assert!(!ops.events.contains(&"group"));

                ops.reaped = true;
                assert!(owner.cleanup(&mut ops).unwrap());
                assert_eq!(
                    ops.events,
                    ["policy", "observe", "signal", "policy", "reap", "policy", "reap", "group"]
                );
                assert_eq!(owner.final_attempt, evidence);
                assert_eq!((owner.signals, owner.reaps), (1, 1));
                let calls = ops.events.len();
                assert!(owner.cleanup(&mut ops).unwrap());
                assert_eq!(owner.exit_status(&mut ops).unwrap(), status);
                assert_eq!(ops.events.len(), calls);
            }
        }

        #[cfg(target_os = "macos")]
        #[test]
        fn macos_eperm_attempt_and_post_reap_query_denial_never_make_a_receipt() {
            let mut owner = lifecycle();
            let mut ops = Fake::new();
            ops.failure_at = Some("signal");
            ops.failure_code = libc::EPERM;
            ops.group = macos_group_result(-1, libc::EPERM).unwrap();
            for _ in 0..3 {
                assert!(!owner.cleanup(&mut ops).unwrap());
                assert!(matches!(owner.phase, Phase::LeaderReaped(_)));
                assert_eq!((owner.signals, owner.reaps), (1, 1));
            }
            assert_eq!(
                owner.final_attempt,
                Some(FinalSignalAttempt::Failed(Failure {
                    stage: "signal",
                    os_code: Some(libc::EPERM),
                }))
            );
            ops.group = macos_group_result(-1, libc::ESRCH).unwrap();
            assert!(owner.cleanup(&mut ops).unwrap());
            assert_eq!((owner.signals, owner.reaps), (1, 1));
        }

        #[cfg(target_os = "macos")]
        #[test]
        fn macos_allowed_eperm_attempt_does_not_mask_later_ownership_errors() {
            for (stage, code) in [
                ("policy", libc::EPERM),
                ("reap", libc::ECHILD),
                ("group", libc::ENOSYS),
            ] {
                let mut owner = lifecycle();
                let mut ops = Fake::new();
                ops.failure_at = Some("signal");
                ops.failure_code = libc::EPERM;
                ops.reaped = false;
                assert!(!owner.cleanup(&mut ops).unwrap());
                ops.reaped = true;
                ops.failure_at = Some(stage);
                ops.failure_code = code;
                let failure = owner.cleanup(&mut ops).unwrap_err();
                assert_eq!(failure.stage, stage);
                assert_eq!(failure.os_code, Some(code));
                assert_eq!(
                    owner.final_attempt,
                    Some(FinalSignalAttempt::Failed(Failure {
                        stage: "signal",
                        os_code: Some(libc::EPERM),
                    }))
                );
                let calls = ops.events.len();
                assert_eq!(owner.cleanup(&mut ops).unwrap_err(), failure);
                assert_eq!(owner.exit_status(&mut ops).unwrap_err(), failure);
                assert_eq!(ops.events.len(), calls);
                assert_eq!(owner.signals, 1);
            }
        }

        #[test]
        fn cached_exit_status_does_not_authorize_signaling_after_ownership_loss() {
            let mut owner = lifecycle();
            let mut ops = Fake::new();
            assert_eq!(owner.exit_status(&mut ops).unwrap(), Some(0));
            ops.failure_at = Some("observe");
            ops.failure_code = libc::ECHILD;
            assert_eq!(
                owner.cleanup(&mut ops).unwrap_err().os_code,
                Some(libc::ECHILD)
            );
            assert_eq!(owner.signals, 0);
            assert!(!ops.events.contains(&"reap"));
        }

        #[test]
        fn interrupted_anchor_observation_returns_to_the_parent_budget() {
            let mut owner = lifecycle();
            let mut ops = Fake::new();
            ops.observed = LeaderState::Interrupted;
            assert!(!owner.cleanup(&mut ops).unwrap());
            assert_eq!(owner.signals, 0);
            assert!(matches!(owner.phase, Phase::Anchored(_)));
            ops.observed = LeaderState::Running;
            assert!(owner.cleanup(&mut ops).unwrap());
            assert_eq!(owner.signals, 1);
        }

        #[test]
        fn invalid_identity_cannot_become_an_empty_receipt() {
            for pid in [0, 1, u32::MAX] {
                let mut owner = Lifecycle::new();
                assert!(owner.anchor(pid).is_err());
                let mut ops = Fake::new();
                assert!(owner.cleanup(&mut ops).is_err());
                assert!(ops.events.is_empty());
            }
        }

        #[test]
        fn linux_return_errno_pairs_do_not_confuse_negative_nice_with_absence() {
            assert_eq!(linux_group_result(-1, 0).unwrap(), GroupState::Present);
            for value in [-20, 0, 19] {
                assert_eq!(linux_group_result(value, 0).unwrap(), GroupState::Present);
            }
            assert_eq!(
                linux_group_result(-1, libc::ESRCH).unwrap(),
                GroupState::Absent
            );
            assert_eq!(
                linux_group_result(-1, libc::EINTR).unwrap(),
                GroupState::Interrupted
            );
            for error in [
                libc::EPERM,
                libc::EACCES,
                libc::EINVAL,
                libc::ENOSYS,
                libc::EIO,
            ] {
                assert_eq!(
                    linux_group_result(-1, error).unwrap_err().os_code,
                    Some(error)
                );
            }
        }

        #[test]
        fn macos_return_errno_pairs_never_accept_denial_as_absence() {
            assert_eq!(macos_group_result(0, 0).unwrap(), GroupState::Present);
            assert_eq!(
                macos_group_result(-1, libc::EPERM).unwrap(),
                GroupState::Present
            );
            assert_eq!(
                macos_group_result(-1, libc::ESRCH).unwrap(),
                GroupState::Absent
            );
            assert_eq!(
                macos_group_result(-1, libc::EINTR).unwrap(),
                GroupState::Interrupted
            );
            for error in [0, libc::EACCES, libc::EINVAL, libc::ENOSYS, libc::EIO] {
                assert!(macos_group_result(-1, error).is_err());
            }
            assert!(macos_group_result(1, 0).is_err());
        }

        #[test]
        fn known_sigchld_auto_reap_policies_are_rejected_without_installing_handlers() {
            assert!(sigchld_compatible(libc::SIG_DFL, 0));
            assert!(!sigchld_compatible(libc::SIG_IGN, 0));
            assert!(!sigchld_compatible(
                libc::SIG_DFL,
                libc::SA_NOCLDWAIT as libc::c_int
            ));
        }

        #[test]
        fn cancellation_before_spawn_acquires_nothing() {
            fn assert_send<T: Send>() {}
            assert_send::<Process>();
            let mut process = Process::new();
            assert_eq!(
                process
                    .spawn("exit 0", &AtomicBool::new(true))
                    .unwrap_err()
                    .stage,
                "cancelled"
            );
            assert!(process.child.is_none());
            assert!(process.output_closed());
            process.begin_cleanup().unwrap();
            assert!(process.poll_cleanup().unwrap());
            assert_eq!(process.lifecycle.signals, 0);
            assert!(process.spawn("exit 0", &AtomicBool::new(false)).is_err());
        }

        #[test]
        fn invalid_descriptor_operations_report_errors_without_blocking() {
            assert_eq!(nonblocking(-1).unwrap_err().os_code, Some(libc::EBADF));
            assert_eq!(drain(-1, None).unwrap_err().os_code, Some(libc::EBADF));
        }

        #[test]
        fn checked_close_confirms_owned_descriptor_and_never_closes_twice() {
            let (stream, mut peer) = UnixStream::pair().unwrap();
            peer.set_nonblocking(true).unwrap();
            let mut pipe = Some(stream);
            let mut state = CloseState::NotAttempted;
            close_pipe(&mut pipe, &mut state, "test close").unwrap();
            assert!(pipe.is_none());
            assert_eq!(state, CloseState::Confirmed);
            assert_eq!(peer.read(&mut [0]).unwrap(), 0);
            close_pipe_with(&mut pipe, &mut state, "test close", |_| {
                panic!("confirmed close must not repeat")
            })
            .unwrap();
        }

        #[test]
        fn injected_ambiguous_and_invalid_close_errors_are_retained_without_retry() {
            for code in [libc::EINTR, libc::EBADF, libc::EIO] {
                let (stream, mut peer) = UnixStream::pair().unwrap();
                peer.set_nonblocking(true).unwrap();
                let descriptor = stream.as_raw_fd();
                let mut pipe = Some(stream);
                let mut state = CloseState::NotAttempted;
                let mut attempts = 0;
                // Deterministic error seam: actually close this owned endpoint,
                // then report an injected error. No leaked or fabricated fd.
                let failure = close_pipe_with(&mut pipe, &mut state, "test close", |fd| {
                    attempts += 1;
                    assert_eq!(unsafe { libc::close(fd) }, 0);
                    Err(io::Error::from_raw_os_error(code))
                })
                .unwrap_err();
                assert_eq!(failure.os_code, Some(code));
                assert_eq!(
                    state,
                    CloseState::Unconfirmed {
                        _descriptor: descriptor,
                        failure: failure.clone(),
                    }
                );
                assert!(pipe.is_none());
                assert_eq!(peer.read(&mut [0]).unwrap(), 0);
                assert_eq!(
                    close_pipe_with(&mut pipe, &mut state, "test close", |_| {
                        attempts += 1;
                        Ok(())
                    })
                    .unwrap_err(),
                    failure
                );
                assert_eq!(attempts, 1);
            }
        }

        #[test]
        fn close_unwind_preserves_uncertainty_instead_of_a_drop_receipt() {
            let (stream, mut peer) = UnixStream::pair().unwrap();
            peer.set_nonblocking(true).unwrap();
            let mut pipe = Some(stream);
            let mut state = CloseState::NotAttempted;
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let _ = close_pipe_with(&mut pipe, &mut state, "test close unwind", |fd| {
                    assert_eq!(unsafe { libc::close(fd) }, 0);
                    panic!("injected close seam unwind");
                });
            }));
            assert!(result.is_err());
            assert!(pipe.is_none());
            assert_eq!(peer.read(&mut [0]).unwrap(), 0);
            assert_eq!(state.check().unwrap_err().stage, "test close unwind");
            assert!(
                close_pipe_with(&mut pipe, &mut state, "test close unwind", |_| {
                    panic!("uncertain close must not repeat")
                })
                .is_err()
            );
        }

        static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);

        struct Fixture {
            process: Process,
            // Independently owned exact child, NOT a remembered helper PID.
            member: Option<Child>,
            root: PathBuf,
        }

        impl Fixture {
            fn new() -> Self {
                let base = std::env::current_dir()
                    .unwrap()
                    .join("target")
                    .join("credential-unix-fixtures");
                std::fs::create_dir_all(&base).unwrap();
                let root = base.join(format!(
                    "{}-{}-{}",
                    std::process::id(),
                    SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap()
                        .as_nanos(),
                    NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed)
                ));
                std::fs::create_dir(&root).unwrap();
                Self {
                    process: Process::new(),
                    member: None,
                    root,
                }
            }

            fn spawn(&mut self, command: &str) {
                self.process
                    .spawn(command, &AtomicBool::new(false))
                    .unwrap();
            }

            fn collect(&mut self) -> (i32, Vec<u8>) {
                let mut output = Vec::new();
                let mut status = None;
                until(|| {
                    let previous = output.len();
                    self.process.read_available(&mut output).unwrap();
                    assert!(output.len() - previous <= READ_SIZE * READS_PER_PIPE);
                    status = self.process.exit_status().unwrap();
                    status.is_some() && self.process.output_closed()
                });
                (status.unwrap(), output)
            }

            fn receipt(&mut self) {
                self.process.begin_cleanup().unwrap();
                until(|| self.process.poll_cleanup().unwrap());
                assert!(self.process.output_closed());
                assert_eq!(
                    (self.process.lifecycle.signals, self.process.lifecycle.reaps),
                    (1, 1)
                );
                assert_eq!(self.process.stdout_close, CloseState::Confirmed);
                assert_eq!(self.process.stderr_close, CloseState::Confirmed);
                self.process.finalize().unwrap();
                assert!(self.process.child.is_none());
            }

            fn controlled_member(&mut self) {
                let shell_release = quoted(&self.root.join("shell-release"));
                self.spawn(&format!(
                    "while [ ! -f {shell_release} ]; do :; done; exit 0"
                ));
                let pid = self.process.child.as_ref().unwrap().id() as libc::pid_t;
                let member_release = quoted(&self.root.join("member-release"));
                let child = Command::new("sh")
                    .arg("-c")
                    .arg(format!(
                        "while [ ! -f {member_release} ]; do :; done; printf 'delayed credential'; printf 'discarded stderr' >&2"
                    ))
                    .stdin(Stdio::null())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .process_group(pid)
                    .spawn()
                    .unwrap();
                self.member = Some(child);
                let member = self.member.as_mut().unwrap();
                // Wire real inherited writer endpoints to the collector while
                // the independent guardian retains wait authority for the member.
                // Actual shell-fork ancestry is a separate integration fixture.
                close_pipe(
                    &mut self.process.stdout,
                    &mut self.process.stdout_close,
                    "fixture stdout replacement close",
                )
                .unwrap();
                close_pipe(
                    &mut self.process.stderr,
                    &mut self.process.stderr_close,
                    "fixture stderr replacement close",
                )
                .unwrap();
                self.process.stdout = member.stdout.take();
                self.process.stderr = member.stderr.take();
                self.process.stdout_close = CloseState::NotAttempted;
                self.process.stderr_close = CloseState::NotAttempted;
                nonblocking(self.process.stdout.as_ref().unwrap().as_raw_fd()).unwrap();
                nonblocking(self.process.stderr.as_ref().unwrap().as_raw_fd()).unwrap();
                std::fs::write(self.root.join("shell-release"), []).unwrap();
                until(|| self.process.exit_status().unwrap() == Some(0));
            }
        }

        impl Drop for Fixture {
            fn drop(&mut self) {
                if let Some(member) = &mut self.member {
                    if matches!(member.try_wait(), Ok(None)) {
                        let _ = member.kill();
                    }
                    let _ = member.wait();
                }
                if let Some(child) = &mut self.process.child {
                    // Never signal a consumed identifier, even on assertion
                    // failure. The fixture also respects exact wait ownership.
                    if matches!(
                        self.process.lifecycle.phase,
                        Phase::Anchored(_) | Phase::SignalsSealed(_)
                    ) {
                        match Native.observe(child.id() as libc::pid_t) {
                            Ok(LeaderState::Running) => {
                                let _ = child.kill();
                                let _ = child.wait();
                            }
                            Ok(LeaderState::Exited(_)) => {
                                let _ = child.wait();
                            }
                            _ => {}
                        }
                    }
                }
                let _ = std::fs::remove_dir_all(&self.root);
            }
        }

        fn quoted(path: &Path) -> String {
            format!("'{}'", path.to_str().unwrap().replace('\'', "'\\''"))
        }

        fn until(mut condition: impl FnMut() -> bool) {
            let deadline = Instant::now() + Duration::from_secs(3);
            while !condition() {
                assert!(
                    Instant::now() < deadline,
                    "owned Unix fixture exceeded its absolute budget"
                );
                std::thread::sleep(Duration::from_millis(2));
            }
        }

        #[test]
        fn normal_shell_and_finite_helper_collect_full_output_and_receipt() {
            for command in [
                "printf '  fixture credential\\n'; printf 'discarded' >&2",
                "sh -c 'printf \"  fixture credential\\n\"'; :",
            ] {
                let mut fixture = Fixture::new();
                fixture.spawn(command);
                assert_eq!(fixture.collect(), (0, b"  fixture credential\n".to_vec()));
                assert_eq!(fixture.process.lifecycle.signals, 0);
                fixture.receipt();
                fixture.process.begin_cleanup().unwrap();
                assert!(fixture.process.poll_cleanup().unwrap());
                assert_eq!(fixture.process.lifecycle.signals, 1);
            }
        }

        #[test]
        fn ordinary_finalization_requires_receipt_and_is_idempotent_without_signals() {
            let mut fixture = Fixture::new();
            fixture.spawn("printf 'fixture output'");
            assert!(fixture.process.finalize().is_err());
            assert!(fixture.process.child.is_some());
            assert_eq!(fixture.collect(), (0, b"fixture output".to_vec()));
            assert!(fixture.process.finalize().is_err());
            assert!(fixture.process.child.is_some());
            fixture.receipt();
            assert!(matches!(fixture.process.lifecycle.phase, Phase::Finalized));
            fixture.process.finalize().unwrap();
            assert!(fixture.process.poll_cleanup().unwrap());
            assert_eq!(
                (
                    fixture.process.lifecycle.signals,
                    fixture.process.lifecycle.reaps
                ),
                (1, 1)
            );
        }

        #[test]
        fn unconfirmed_output_close_holds_other_resources_and_blocks_finalization() {
            let mut fixture = Fixture::new();
            fixture.spawn("exit 0");
            until(|| fixture.process.exit_status().unwrap() == Some(0));
            // Close the owned fd exactly once; EINTR is injected, not a claim
            // about this kernel's close behavior.
            let failure = close_pipe_with(
                &mut fixture.process.stdout,
                &mut fixture.process.stdout_close,
                "stdout close",
                |fd| {
                    assert_eq!(unsafe { libc::close(fd) }, 0);
                    Err(io::Error::from_raw_os_error(libc::EINTR))
                },
            )
            .unwrap_err();
            assert!(!fixture.process.output_closed());
            assert_eq!(fixture.process.begin_cleanup().unwrap_err(), failure);
            assert!(fixture.process.stderr.is_some());
            assert!(fixture.process.child.is_some());
            assert!(matches!(
                fixture.process.lifecycle.phase,
                Phase::Anchored(_)
            ));
            assert_eq!(fixture.process.poll_cleanup().unwrap_err(), failure);
            assert_eq!(fixture.process.finalize().unwrap_err(), failure);
            assert_eq!(fixture.process.lifecycle.signals, 0);

            // Even independently observed scope completion cannot turn the
            // unconfirmed close into a receipt at the finalization boundary.
            assert!(fixture.process.lifecycle.cleanup(&mut Native).unwrap());
            assert_eq!(fixture.process.finalize().unwrap_err(), failure);
            assert!(fixture.process.child.is_some());
            assert!(fixture.process.stderr.is_some());
            assert_eq!(fixture.process.lifecycle.signals, 1);
        }

        #[test]
        fn stdout_and_stderr_pressure_are_fair_without_output_truncation() {
            let mut fixture = Fixture::new();
            let text = "0123456789abcdefghijklmnopqrstuv";
            fixture.spawn(&format!(
                "i=0; while [ \"$i\" -lt 8192 ]; do printf '{text}'; printf '{text}' >&2; i=$((i+1)); done"
            ));
            assert_eq!(fixture.collect(), (0, text.repeat(8192).into_bytes()));
            fixture.receipt();
        }

        #[test]
        fn cancellation_closes_output_and_receipts_the_exact_shell() {
            let mut fixture = Fixture::new();
            fixture.spawn("printf ready; while :; do :; done");
            let mut output = Vec::new();
            until(|| {
                fixture.process.read_available(&mut output).unwrap();
                output == b"ready"
            });
            assert_eq!(fixture.process.exit_status().unwrap(), None);
            fixture.receipt();
            assert!(fixture.process.read_available(&mut output).is_err());
        }

        #[test]
        fn incomplete_io_setup_retains_resources_and_never_enters_a_read() {
            let mut fixture = Fixture::new();
            fixture.spawn("while :; do :; done");
            fixture.process.io_ready = false;
            assert_eq!(
                fixture
                    .process
                    .read_available(&mut Vec::new())
                    .unwrap_err()
                    .stage,
                "output setup incomplete"
            );
            assert!(fixture.process.child.is_some());
            assert!(fixture.process.stdout.is_some());
            assert!(fixture.process.stderr.is_some());
            fixture.receipt();
        }

        #[test]
        fn eof_without_leader_exit_is_not_completion() {
            let mut fixture = Fixture::new();
            fixture.spawn("exec 1>&- 2>&-; while :; do :; done");
            until(|| {
                fixture.process.read_available(&mut Vec::new()).unwrap();
                fixture.process.output_closed()
            });
            assert_eq!(fixture.process.exit_status().unwrap(), None);
            assert!(fixture.process.poll_cleanup().is_err());
            fixture.receipt();
        }

        #[test]
        fn ordinary_nonzero_exit_preserves_status_without_helper_stderr() {
            let mut fixture = Fixture::new();
            fixture.spawn("printf partial; printf 'not diagnostic text' >&2; exit 7");
            assert_eq!(fixture.collect(), (7, b"partial".to_vec()));
            fixture.receipt();
        }

        #[test]
        fn finite_member_output_remains_collecting_after_leader_exit() {
            let mut fixture = Fixture::new();
            fixture.controlled_member();
            let mut output = Vec::new();
            fixture.process.read_available(&mut output).unwrap();
            assert!(output.is_empty());
            assert!(!fixture.process.output_closed());
            assert_eq!(fixture.process.lifecycle.signals, 0);
            assert!(fixture
                .member
                .as_mut()
                .unwrap()
                .try_wait()
                .unwrap()
                .is_none());
            std::fs::write(fixture.root.join("member-release"), []).unwrap();
            assert_eq!(fixture.collect(), (0, b"delayed credential".to_vec()));
            assert!(fixture.member.as_mut().unwrap().wait().unwrap().success());
            fixture.receipt();
        }

        #[test]
        fn group_receipt_waits_for_guardian_owned_zombie_after_leader_reap() {
            let mut fixture = Fixture::new();
            fixture.controlled_member();
            fixture.process.begin_cleanup().unwrap();
            until(|| {
                assert!(!fixture.process.poll_cleanup().unwrap());
                matches!(fixture.process.lifecycle.phase, Phase::LeaderReaped(_))
            });
            let member_pid = fixture.member.as_ref().unwrap().id() as libc::pid_t;
            until(|| matches!(Native.observe(member_pid).unwrap(), LeaderState::Exited(_)));
            // The member is deliberately unreaped by its independent guardian.
            // Neither killing nor reaping the leader was a whole-group receipt.
            assert!(!fixture.process.poll_cleanup().unwrap());
            assert_eq!(fixture.process.lifecycle.signals, 1);
            fixture.member.as_mut().unwrap().wait().unwrap();
            until(|| fixture.process.poll_cleanup().unwrap());
            assert_eq!(
                (
                    fixture.process.lifecycle.signals,
                    fixture.process.lifecycle.reaps
                ),
                (1, 1)
            );
        }

        #[test]
        fn competing_exact_reap_is_echild_not_receipt_or_signal_authority() {
            let mut fixture = Fixture::new();
            fixture.spawn("exit 0");
            until(|| fixture.process.exit_status().unwrap() == Some(0));
            fixture.process.child.as_mut().unwrap().wait().unwrap();
            assert_eq!(
                fixture.process.exit_status().unwrap_err().os_code,
                Some(libc::ECHILD)
            );
            assert!(fixture.process.begin_cleanup().is_err());
            assert!(fixture.process.poll_cleanup().is_err());
            assert_eq!(fixture.process.lifecycle.signals, 0);
        }

        #[test]
        fn dropping_process_never_signals_the_still_owned_child() {
            let mut fixture = Fixture::new();
            fixture.spawn("while :; do :; done");
            let mut process = std::mem::replace(&mut fixture.process, Process::new());
            fixture.member = process.child.take();
            drop(process);
            assert!(fixture
                .member
                .as_mut()
                .unwrap()
                .try_wait()
                .unwrap()
                .is_none());
        }

        #[cfg(target_os = "macos")]
        mod sandbox_null_query {
            use super::*;
            use std::io::Write;

            // Public (deprecated) SDK sandbox.h permits SANDBOX_NAMED only;
            // custom SBPL with flags=0 is reserved, so do not use it here.
            // kSBXProfilePureComputation prohibits OS services. Its effect on
            // signal zero is a runtime assertion, never assumed or skipped.
            // SDK 11.3 usr/lib/system/libsystem_sandbox.tbd exports these
            // public symbols under System; no production sandbox dependency.
            #[link(name = "System")]
            extern "C" {
                fn sandbox_init(
                    profile: *const libc::c_char,
                    flags: u64,
                    errorbuf: *mut *mut libc::c_char,
                ) -> libc::c_int;
                fn sandbox_free_error(errorbuf: *mut libc::c_char);
                static kSBXProfilePureComputation: libc::c_char;
            }

            fn null_query(group: libc::pid_t) -> (libc::c_int, libc::c_int) {
                unsafe {
                    let errno = libc::__error();
                    *errno = 0;
                    let result = libc::kill(-group, 0);
                    (result, *errno)
                }
            }

            fn child_result() -> i32 {
                let group = std::env::var("TELEX_MAC_NULL_QUERY_GROUP")
                    .ok()
                    .and_then(|value| value.parse::<libc::pid_t>().ok())
                    .filter(|value| *value > 1);
                let uid = std::env::var("TELEX_MAC_NULL_QUERY_UID")
                    .ok()
                    .and_then(|value| value.parse::<libc::uid_t>().ok());
                let (Some(group), Some(uid)) = (group, uid) else {
                    return 71;
                };
                if unsafe { libc::getuid() != uid || libc::geteuid() != uid } {
                    return 72;
                }
                if null_query(group) != (0, 0) {
                    return 73;
                }
                let mut error = std::ptr::null_mut();
                let result = unsafe {
                    sandbox_init(
                        std::ptr::addr_of!(kSBXProfilePureComputation),
                        1, // SANDBOX_NAMED, as declared by public sandbox.h.
                        &mut error,
                    )
                };
                if !error.is_null() {
                    // Never log the developer-oriented sandbox error buffer.
                    unsafe { sandbox_free_error(error) };
                }
                if result != 0 {
                    return 74;
                }
                // Pinned XNU kern_sig.c cansignal() passes signum (including 0)
                // to mac_proc_check_signal before the delivery-only !=0 check.
                if null_query(group) != (-1, libc::EPERM) {
                    return 75;
                }
                if !matches!(group_state(&FormerGroup(group)), Ok(GroupState::Present)) {
                    return 76;
                }
                // A normal libtest exit (including a zero-test filter mistake)
                // cannot be mistaken for an executed denial oracle.
                70
            }

            #[test]
            #[ignore = "isolated process-local sandbox entrypoint; guardian test executes it"]
            fn child() {
                // Sandbox restrictions never reach the test runner/guardian.
                // No post-sandbox allocation, reporting, or harness teardown is
                // needed to convey the fixed, non-secret oracle status.
                unsafe { libc::_exit(child_result()) };
            }

            #[test]
            fn same_user_named_sandbox_denies_null_query_without_hiding_owned_group() {
                let mut keeper = Fixture::new();
                keeper.member = Some(
                    Command::new("sh")
                        .args(["-c", "read -r token"])
                        .stdin(Stdio::piped())
                        .stdout(Stdio::null())
                        .stderr(Stdio::null())
                        .process_group(0)
                        .spawn()
                        .unwrap(),
                );
                let group = libc::pid_t::try_from(keeper.member.as_ref().unwrap().id()).unwrap();
                assert!(group > 1);
                assert_eq!(null_query(group), (0, 0));

                let mut query = Fixture::new();
                // libtest omits the crate prefix; this works both in the real
                // library namespace and in the standalone validation harness.
                let module = module_path!().split_once("::").unwrap().1;
                query.member = Some(
                    Command::new(std::env::current_exe().unwrap())
                        .args([
                            "--exact",
                            &format!("{module}::child"),
                            "--ignored",
                            "--nocapture",
                        ])
                        .env("TELEX_MAC_NULL_QUERY_GROUP", group.to_string())
                        .env(
                            "TELEX_MAC_NULL_QUERY_UID",
                            unsafe { libc::geteuid() }.to_string(),
                        )
                        .stdin(Stdio::null())
                        .stdout(Stdio::null())
                        .stderr(Stdio::null())
                        .process_group(0)
                        .spawn()
                        .unwrap(),
                );
                let mut status = None;
                until(|| {
                    status = query.member.as_mut().unwrap().try_wait().unwrap();
                    status.is_some()
                });
                assert!(
                    keeper
                        .member
                        .as_mut()
                        .unwrap()
                        .try_wait()
                        .unwrap()
                        .is_none(),
                    "independently owned group exited during the denial probe"
                );
                assert_eq!(null_query(group), (0, 0));
                assert_eq!(
                    status.unwrap().code(),
                    Some(70),
                    "macOS sandbox oracle failed: 71=input, 72=UID, 73=baseline, \
                     74=named sandbox unavailable/rejected, 75=null query was not EPERM, \
                     76=production query did not retain presence; signal exit also fails"
                );
                // Cooperative normal cleanup; Fixture owns the exact Children
                // and supplies checked-wait/kill fallback on any assertion.
                keeper
                    .member
                    .as_mut()
                    .unwrap()
                    .stdin
                    .as_mut()
                    .unwrap()
                    .write_all(b"release\n")
                    .unwrap();
                until(|| {
                    keeper
                        .member
                        .as_mut()
                        .unwrap()
                        .try_wait()
                        .unwrap()
                        .is_some()
                });
                keeper.member.as_mut().unwrap().wait().unwrap();
                query.member.as_mut().unwrap().wait().unwrap();
            }
        }
    }
}

// Other Unix targets must not borrow either platform's absence predicate.
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub(super) struct Process;

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
impl Process {
    pub(super) fn new() -> Self {
        Self
    }

    pub(super) fn spawn(
        &mut self,
        _command: &str,
        _cancelled: &std::sync::atomic::AtomicBool,
    ) -> Result<(), super::Failure> {
        Err(super::Failure::at("unsupported Unix credential lifecycle"))
    }

    pub(super) fn read_available(&mut self, _stdout: &mut Vec<u8>) -> Result<(), super::Failure> {
        Err(super::Failure::at("unsupported Unix credential lifecycle"))
    }

    pub(super) fn output_closed(&self) -> bool {
        true
    }

    pub(super) fn exit_status(&mut self) -> Result<Option<i32>, super::Failure> {
        Err(super::Failure::at("unsupported Unix credential lifecycle"))
    }

    pub(super) fn begin_cleanup(&mut self) -> Result<(), super::Failure> {
        Err(super::Failure::at("unsupported Unix credential lifecycle"))
    }

    pub(super) fn poll_cleanup(&mut self) -> Result<bool, super::Failure> {
        Err(super::Failure::at("unsupported Unix credential lifecycle"))
    }

    pub(super) fn finalize(&mut self) -> Result<(), super::Failure> {
        Err(super::Failure::at("unsupported Unix credential lifecycle"))
    }
}
