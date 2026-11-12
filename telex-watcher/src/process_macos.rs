//! macOS assigned-process-group containment (ADR 0054 as amended by ADR 0057).
//!
//! A private, per-attempt guard runs in its own session, outside the detector
//! group. It forks a gated trampoline that becomes the group leader and execs
//! the detector only after the runtime has durably recorded the complete
//! identity. The guard never waits for that leader until cleanup has finished:
//! the unreaped direct child is the PID/PGID-reuse-safe anchor for every
//! nonzero group signal.
//!
//! Completion is a fresh nonreaping terminal wait plus a two-slot group census
//! showing only the owned leader. The authority to signal is then consumed
//! (sealed) before the exact reap, and only `kill(-PGID, 0) == -1/ESRCH` may
//! establish that the former group is absent. An Ended receipt means this
//! predicate held, subject to ADR 0057's accepted transient-invisibility
//! residual. Detectors that leave the assigned group are outside the contract.

use super::*;
use std::io::{BufRead, Read, Write};
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::process::Stdio;
use tokio::process::{Child, Command};

pub(super) const SCOPE: &str = "assigned-process-group";
pub(super) const ANCHOR: &str = "unreaped-direct-child-leader";
pub(super) const IDENTITY_VERSION: u32 = 1;
const RECEIPT_LIMIT: u64 = 16 * 1024;
/// After the assigned group has verifiably ended, a process outside it may
/// still hold the output pipes. Wait this long for EOF, then stop reading.
const PIPE_DRAIN: Duration = Duration::from_secs(2);
/// Read-only observation of the former group after the reap is bounded; the
/// guard is not a persistent cleanup service.
const OBSERVE_LIMIT: Duration = Duration::from_secs(30);

// The census reads native `int` PIDs; both ABIs must agree at compile time.
const _: () = assert!(
    std::mem::size_of::<libc::c_int>() == 4
        && std::mem::size_of::<libc::pid_t>() == std::mem::size_of::<libc::c_int>()
);

/// Capability and safety admission. OS version numbers are deliberately not
/// an input: no minimum, maximum, per-major or per-build gate exists.
pub(super) mod admission {
    use std::ffi::CString;

    /// `Ok(None)` is ENOENT (the name does not exist); `Err` carries errno.
    pub(crate) type Fact<T> = Result<Option<T>, i32>;

    pub(crate) struct Facts {
        pub translated: Fact<i32>,
        pub hardware_arm64: Fact<i32>,
        pub kernel_version: Fact<String>,
    }

    fn sysctl_bytes(name: &str) -> Fact<Vec<u8>> {
        let name = CString::new(name).map_err(|_| libc::EINVAL)?;
        let mut size = 0usize;
        let errno = || std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
        if unsafe {
            libc::sysctlbyname(
                name.as_ptr(),
                std::ptr::null_mut(),
                &mut size,
                std::ptr::null_mut(),
                0,
            )
        } != 0
        {
            return match errno() {
                libc::ENOENT => Ok(None),
                other => Err(other),
            };
        }
        let mut bytes = vec![0u8; size];
        if unsafe {
            libc::sysctlbyname(
                name.as_ptr(),
                bytes.as_mut_ptr().cast(),
                &mut size,
                std::ptr::null_mut(),
                0,
            )
        } != 0
        {
            return Err(errno());
        }
        bytes.truncate(size);
        Ok(Some(bytes))
    }

    pub(crate) fn sysctl_i32(name: &str) -> Fact<i32> {
        match sysctl_bytes(name)? {
            None => Ok(None),
            Some(bytes) => <[u8; 4]>::try_from(bytes.as_slice())
                .map(|raw| Some(i32::from_ne_bytes(raw)))
                .map_err(|_| libc::EINVAL),
        }
    }

    pub(crate) fn sysctl_string(name: &str) -> Fact<String> {
        match sysctl_bytes(name)? {
            None => Ok(None),
            Some(mut bytes) => {
                while bytes.last() == Some(&0) {
                    bytes.pop();
                }
                String::from_utf8(bytes).map(Some).map_err(|_| libc::EINVAL)
            }
        }
    }

    pub(crate) fn collect() -> Facts {
        Facts {
            translated: sysctl_i32("sysctl.proc_translated"),
            hardware_arm64: sysctl_i32("hw.optional.arm64"),
            kernel_version: sysctl_string("kern.version"),
        }
    }

    /// Pure decision so every branch is testable on any host.
    pub(crate) fn evaluate(facts: &Facts, process_arm64: bool) -> Result<(), String> {
        // ENOENT is the native Intel case, not an indeterminate failure.
        match facts.translated {
            Ok(None) | Ok(Some(0)) => {}
            Ok(Some(1)) => {
                return Err(
                    "the process is Rosetta-translated; native execution is required".into(),
                )
            }
            Ok(Some(other)) => {
                return Err(format!(
                    "sysctl.proc_translated returned unexpected value {other}"
                ))
            }
            Err(errno) => {
                return Err(format!(
                    "cannot read sysctl.proc_translated (errno {errno})"
                ))
            }
        }
        let hardware_arm64 = match facts.hardware_arm64 {
            Ok(None) | Ok(Some(0)) => false,
            Ok(Some(1)) => true,
            Ok(Some(other)) => {
                return Err(format!(
                    "hw.optional.arm64 returned unexpected value {other}"
                ))
            }
            Err(errno) => return Err(format!("cannot read hw.optional.arm64 (errno {errno})")),
        };
        if hardware_arm64 != process_arm64 {
            return Err("process architecture does not match the hardware architecture".into());
        }
        // Darwin names the kernel configuration after the last '/', for
        // example `.../RELEASE_ARM64_T6020`. Development and KASAN kernels
        // (and any unreadable or unrecognized flavor) are refused.
        match &facts.kernel_version {
            Ok(Some(version)) => {
                let flavor = version.rsplit('/').next().unwrap_or("");
                if !version.contains('/') || !flavor.starts_with("RELEASE_") {
                    return Err(format!(
                        "kernel configuration `{flavor}` is not a release kernel"
                    ));
                }
            }
            Ok(None) => return Err("kern.version is unavailable".into()),
            Err(errno) => return Err(format!("cannot read kern.version (errno {errno})")),
        }
        Ok(())
    }

    pub(crate) fn check() -> Result<(), String> {
        if !cfg!(any(target_arch = "aarch64", target_arch = "x86_64")) {
            return Err("unsupported process architecture".into());
        }
        evaluate(&collect(), cfg!(target_arch = "aarch64"))
    }
}

/// The only code allowed to signal, wait for or reap the detector leader.
///
/// `Anchor` -> `Sealed` -> `FormerGroup` are consumed in order and are neither
/// `Clone` nor constructible from numbers outside this module. The one
/// nonzero `kill` lives on `Anchor`; nothing reachable from `Sealed` or
/// `FormerGroup` can signal.
pub(super) mod authority {
    use libc::{c_int, pid_t};
    use std::mem::ManuallyDrop;

    const PROC_PGRP_ONLY: u32 = 2;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub(crate) struct Terminal {
        /// `Some` only for an ordinary exit; a signalled leader has no code.
        pub exit_code: Option<i32>,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub(crate) enum Census {
        OnlyLeader,
        Rejected(&'static str),
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub(crate) enum Observation {
        Absent,
        Present,
        Uncertain(i32),
    }

    /// Classify one `proc_listpids(PROC_PGRP_ONLY)` result for an `int[2]`
    /// buffer. Only exactly one native `int` naming the owned leader passes.
    pub(crate) fn classify_census(bytes: c_int, slot0: c_int, leader: pid_t) -> Census {
        if bytes < 0 {
            Census::Rejected("census error")
        } else if bytes == 0 {
            Census::Rejected("census returned no member")
        } else if !(bytes as usize).is_multiple_of(std::mem::size_of::<c_int>()) {
            Census::Rejected("census returned a malformed size")
        } else if bytes as usize != std::mem::size_of::<c_int>() {
            Census::Rejected("census saturated or found another member")
        } else if slot0 != leader {
            Census::Rejected("census member is not the owned leader")
        } else {
            Census::OnlyLeader
        }
    }

    /// Classify `kill(-PGID, 0)`. Zero and EPERM are presence, never absence.
    pub(crate) fn classify_observation(result: c_int, errno: i32) -> Observation {
        match (result, errno) {
            (0, _) => Observation::Present,
            (_, libc::ESRCH) => Observation::Absent,
            (_, libc::EPERM) => Observation::Present,
            (_, other) => Observation::Uncertain(other),
        }
    }

    fn errno() -> i32 {
        std::io::Error::last_os_error().raw_os_error().unwrap_or(0)
    }

    /// Exclusive ownership of the unreaped direct-child group leader.
    pub(crate) struct Anchor {
        leader: pid_t,
    }

    impl Anchor {
        /// # Safety
        /// `leader` must be this process's own direct child that has never
        /// been waited for and is the leader of its own process group.
        pub(crate) unsafe fn from_owned_child(leader: pid_t) -> Self {
            Self { leader }
        }

        /// Fresh nonreaping wait. `Ok(None)` means no terminal event yet.
        pub(crate) fn terminal(&self) -> std::io::Result<Option<Terminal>> {
            loop {
                let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
                let result = unsafe {
                    libc::waitid(
                        libc::P_PID,
                        self.leader as libc::id_t,
                        &mut info,
                        libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
                    )
                };
                if result != 0 {
                    let error = std::io::Error::last_os_error();
                    if error.kind() == std::io::ErrorKind::Interrupted {
                        continue;
                    }
                    return Err(error);
                }
                if info.si_pid == 0 {
                    return Ok(None);
                }
                if info.si_pid != self.leader {
                    return Err(std::io::Error::other("wait returned another process"));
                }
                return match info.si_code {
                    libc::CLD_EXITED => Ok(Some(Terminal {
                        exit_code: Some(info.si_status),
                    })),
                    libc::CLD_KILLED | libc::CLD_DUMPED => Ok(Some(Terminal { exit_code: None })),
                    _ => Err(std::io::Error::other("wait returned a non-terminal event")),
                };
            }
        }

        /// Returned PIDs are evidence only; they are never signal targets.
        pub(crate) fn census(&self) -> Census {
            let mut pids: [c_int; 2] = [0; 2];
            let bytes = unsafe {
                libc::proc_listpids(
                    PROC_PGRP_ONLY,
                    self.leader as u32,
                    pids.as_mut_ptr().cast(),
                    std::mem::size_of_val(&pids) as c_int,
                )
            };
            classify_census(bytes, pids[0], self.leader)
        }

        /// The only nonzero signal. Valid because the leader is still an
        /// unreaped child, so neither its PID nor its PGID can be reused.
        /// Success, ESRCH or EPERM say nothing about completion.
        pub(crate) fn kill_group(&mut self) -> std::io::Result<()> {
            if unsafe { libc::kill(-self.leader, libc::SIGKILL) } == 0 {
                Ok(())
            } else {
                Err(std::io::Error::last_os_error())
            }
        }

        fn complete(&self) -> Option<Terminal> {
            let terminal = self.terminal().ok()??;
            (self.census() == Census::OnlyLeader).then_some(terminal)
        }

        /// Seal only on a positive terminal-wait + census pair. The pair is
        /// taken twice with no signal in between (defense in depth, not
        /// proof); the second, freshest pair is the one that seals.
        pub(crate) fn seal_if_complete(self) -> Result<Sealed, Self> {
            if self.complete().is_none() {
                return Err(self);
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
            let Some(terminal) = self.complete() else {
                return Err(self);
            };
            let anchor = ManuallyDrop::new(self);
            Ok(Sealed {
                leader: anchor.leader,
                terminal,
            })
        }
    }

    impl Drop for Anchor {
        /// An abandoned anchor (error or panic) is still valid here: the
        /// leader is unreaped until this process exits.
        fn drop(&mut self) {
            unsafe {
                libc::kill(-self.leader, libc::SIGKILL);
            }
        }
    }

    /// Signal authority is gone. Only the exact reap remains.
    pub(crate) struct Sealed {
        leader: pid_t,
        terminal: Terminal,
    }

    impl Sealed {
        pub(crate) fn reap(self) -> std::io::Result<(FormerGroup, Terminal)> {
            let mut status: c_int = 0;
            loop {
                let reaped = unsafe { libc::waitpid(self.leader, &mut status, 0) };
                if reaped == self.leader {
                    break;
                }
                if reaped < 0 && errno() == libc::EINTR {
                    continue;
                }
                return Err(std::io::Error::other("exact leader reap failed"));
            }
            let exit_code = libc::WIFEXITED(status).then(|| libc::WEXITSTATUS(status));
            if exit_code != self.terminal.exit_code
                || !(libc::WIFEXITED(status) || libc::WIFSIGNALED(status))
            {
                return Err(std::io::Error::other(
                    "reaped status does not match the terminal observation",
                ));
            }
            Ok((FormerGroup { pgid: self.leader }, self.terminal))
        }
    }

    /// A number that used to be ours. Read-only observation only.
    pub(crate) struct FormerGroup {
        pgid: pid_t,
    }

    impl FormerGroup {
        pub(crate) fn observe(&self) -> Observation {
            loop {
                let result = unsafe { libc::kill(-self.pgid, 0) };
                let errno = if result == 0 { 0 } else { errno() };
                if result != 0 && errno == libc::EINTR {
                    continue;
                }
                return classify_observation(result, errno);
            }
        }
    }
}

use authority::{Anchor, Observation};

#[derive(Serialize, Deserialize)]
struct Launch {
    command: Vec<String>,
    receipt: PathBuf,
    nonce: String,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "handshake", rename_all = "kebab-case")]
enum Handshake {
    Ready { identity: ContainmentIdentity },
    Refused { admission: bool, reason: String },
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Receipt {
    version: u32,
    identity: ContainmentIdentity,
    ended: bool,
    exit_code: Option<i32>,
    spawn_error: Option<String>,
}

pub(super) struct Prepared {
    child: Child,
    control: UnixStream,
    identity: ContainmentIdentity,
    writer: Option<tokio::task::JoinHandle<std::io::Result<()>>>,
}

impl Drop for Prepared {
    fn drop(&mut self) {
        let _ = self.control.shutdown(std::net::Shutdown::Both);
        if let Some(writer) = &self.writer {
            writer.abort();
        }
    }
}

pub(super) async fn prepare(
    spec: &ProcessSpec,
    cwd: &Path,
    environment: &Environment,
) -> Result<Prepared, ProcessFailure> {
    admission::check().map_err(|reason| {
        ProcessFailure::new(
            ProcessFailureKind::UnsupportedPlatform,
            format!("macOS containment admission refused: {reason}"),
        )
    })?;
    if !spec.containment_directory.is_absolute() {
        return Err(ProcessFailure::new(
            ProcessFailureKind::Spawn,
            "containment directory must be absolute",
        ));
    }
    std::fs::create_dir_all(&spec.containment_directory).map_err(ProcessFailure::io)?;
    let root = std::fs::canonicalize(&spec.containment_directory).map_err(ProcessFailure::io)?;
    let nonce = uuid::Uuid::new_v4().to_string();
    let launch = Launch {
        command: spec.command.clone(),
        receipt: root.join(format!("{nonce}.ended.json")),
        nonce,
    };
    let (mut control, guard_control) = UnixStream::pair().map_err(ProcessFailure::io)?;
    control
        .set_read_timeout(Some(CLEANUP_TIMEOUT))
        .map_err(ProcessFailure::io)?;
    control
        .set_write_timeout(Some(CLEANUP_TIMEOUT))
        .map_err(ProcessFailure::io)?;
    let fd = guard_control.as_raw_fd();
    let mut executable = std::env::current_exe().map_err(ProcessFailure::io)?;
    executable.pop();
    if executable.file_name() == Some(OsStr::new("deps")) {
        executable.pop();
    }
    executable.push("watcher_process_guard");
    let mut command = Command::new(executable);
    command
        .arg(fd.to_string())
        .current_dir(cwd)
        .env_clear()
        .envs(environment)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    unsafe {
        command.as_std_mut().pre_exec(move || {
            // The guard stays outside the detector group and must survive a
            // signal aimed at the runtime's foreground process group.
            if libc::setsid() < 0 || libc::fcntl(fd, libc::F_SETFD, 0) < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let child = command.spawn().map_err(|error| {
        ProcessFailure::new(
            ProcessFailureKind::Spawn,
            format!("start process guard: {error}"),
        )
    })?;
    drop(guard_control);
    let wire = serde_json::to_vec(&launch).map_err(ProcessFailure::io)?;
    if wire.len() > MAX_STDIN_BYTES {
        return Err(ProcessFailure::new(
            ProcessFailureKind::InvalidCommand,
            "launch specification exceeds 1 MiB",
        ));
    }
    control.write_all(&wire).map_err(ProcessFailure::io)?;
    control.write_all(b"\n").map_err(ProcessFailure::io)?;
    // The cloned stream exists only during the bounded startup handshake.
    // Cancellation closes the other endpoint too, even during this read.
    let handshake = control.try_clone().map_err(ProcessFailure::io)?;
    struct CloseOnDrop(Option<UnixStream>);
    impl Drop for CloseOnDrop {
        fn drop(&mut self) {
            if let Some(control) = &self.0 {
                let _ = control.shutdown(std::net::Shutdown::Both);
            }
        }
    }
    let mut close_on_drop = CloseOnDrop(Some(control.try_clone().map_err(ProcessFailure::io)?));
    let handshake = tokio::task::spawn_blocking(move || {
        let mut bytes = Vec::new();
        std::io::BufReader::new(handshake)
            .take(RECEIPT_LIMIT)
            .read_until(b'\n', &mut bytes)?;
        serde_json::from_slice::<Handshake>(&bytes).map_err(std::io::Error::other)
    })
    .await
    .map_err(ProcessFailure::io)?
    .map_err(ProcessFailure::io)?;
    let identity = match handshake {
        Handshake::Ready { identity } => identity,
        Handshake::Refused { admission, reason } => {
            return Err(ProcessFailure::new(
                if admission {
                    ProcessFailureKind::UnsupportedPlatform
                } else {
                    ProcessFailureKind::Spawn
                },
                format!("process guard refused before launch: {reason}"),
            ))
        }
    };
    match &identity {
        ContainmentIdentity::MacosGuard {
            version,
            scope,
            anchor,
            boot_session,
            guard_pid,
            guard_started_us,
            leader_pid,
            leader_started_us,
            pgid,
            receipt,
            nonce,
        } if *version == IDENTITY_VERSION
            && scope == SCOPE
            && anchor == ANCHOR
            && uuid::Uuid::parse_str(boot_session).is_ok()
            && Some(*guard_pid) == child.id()
            && *guard_started_us > 0
            && *leader_pid > 1
            && i32::try_from(*leader_pid).is_ok()
            && *leader_started_us > 0
            && pgid == leader_pid
            && leader_pid != guard_pid
            && receipt == &launch.receipt
            && nonce == &launch.nonce => {}
        _ => {
            return Err(ProcessFailure::io(
                "process guard returned an invalid identity",
            ))
        }
    }
    drop(close_on_drop.0.take());
    Ok(Prepared {
        child,
        control,
        identity,
        writer: None,
    })
}

impl Prepared {
    pub(super) fn identity(&self) -> &ContainmentIdentity {
        &self.identity
    }

    pub(super) async fn start(&mut self, request: Vec<u8>) -> Result<(), ProcessFailure> {
        use tokio::io::AsyncWriteExt;
        // One-use release. Once this write begins the detector may run.
        self.control.write_all(b"S").map_err(ProcessFailure::io)?;
        let mut stdin = self
            .child
            .stdin
            .take()
            .ok_or_else(|| ProcessFailure::io("missing stdin"))?;
        self.writer = Some(tokio::spawn(async move {
            stdin.write_all(&request).await?;
            stdin.shutdown().await
        }));
        Ok(())
    }

    pub(super) async fn complete(
        &mut self,
        sensitive: &[String],
    ) -> Result<Completion, ProcessFailure> {
        let stdout = self
            .child
            .stdout
            .take()
            .ok_or_else(|| ProcessFailure::io("missing stdout"))?;
        let stderr = self
            .child
            .stderr
            .take()
            .ok_or_else(|| ProcessFailure::io("missing stderr"))?;
        let identity = self.identity.clone();
        let child = &mut self.child;
        let wait = async {
            let status = child.wait().await.map_err(ProcessFailure::io)?;
            if !status.success() {
                return Err(ProcessFailure::new(
                    ProcessFailureKind::ContainmentUnproven,
                    "process guard failed",
                ));
            }
            let receipt = read_receipt(&identity).ok_or_else(|| {
                ProcessFailure::new(
                    ProcessFailureKind::ContainmentUnproven,
                    "guard completion receipt missing",
                )
            })?;
            if let Some(error) = receipt.spawn_error {
                return Err(ProcessFailure::new(ProcessFailureKind::Spawn, error));
            }
            Ok(receipt.exit_code)
        };
        let reads = async {
            tokio::try_join!(
                read_capped(stdout, MAX_STDOUT_BYTES, ProcessFailureKind::StdoutLimit),
                read_capped(stderr, MAX_STDERR_BYTES, ProcessFailureKind::StderrLimit),
            )
        };
        tokio::pin!(wait);
        tokio::pin!(reads);
        let (stdout, stderr, code) = tokio::select! {
            read = &mut reads => {
                let (stdout, stderr) = read?;
                (stdout, stderr, wait.await?)
            }
            code = &mut wait => {
                let code = code?;
                // The guard exits only after the assigned group ended. A pipe
                // still open now is held outside the group: the detector
                // broke the non-detaching contract. Stop reading; do not let
                // that process hold the attempt until its timeout.
                match tokio::time::timeout(PIPE_DRAIN, &mut reads).await {
                    Ok(read) => {
                        let (stdout, stderr) = read?;
                        (stdout, stderr, code)
                    }
                    Err(_) => return Err(ProcessFailure::new(
                        ProcessFailureKind::Io,
                        "detector output is still held open after the assigned process group ended; a process left the group (nonconforming detector)",
                    )),
                }
            }
        };
        finish_stdin(self.writer.take(), &stderr, sensitive).await?;
        Ok((stdout, stderr, code))
    }

    pub(super) async fn cleanup(&mut self) -> ContainmentStatus {
        let _ = self.control.write_all(b"K");
        let _ = self.control.shutdown(std::net::Shutdown::Both);
        if let Some(writer) = self.writer.take() {
            writer.abort();
        }
        // A timeout here is Unproven, never a forced seal: the guard keeps
        // its anchor and may still finalize a receipt that Resume accepts.
        let _ = tokio::time::timeout(CLEANUP_TIMEOUT, self.child.wait()).await;
        if read_receipt(&self.identity).is_some() {
            ContainmentStatus::Ended
        } else {
            ContainmentStatus::Unproven
        }
    }
}

fn boot_session() -> std::io::Result<String> {
    // The boot-session UUID, never the clock-adjustable kern.boottime.
    match admission::sysctl_string("kern.bootsessionuuid") {
        Ok(Some(value)) if uuid::Uuid::parse_str(&value).is_ok() => Ok(value),
        _ => Err(std::io::Error::other("invalid boot-session identity")),
    }
}

fn started_us(pid: libc::pid_t) -> std::io::Result<u64> {
    let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of::<libc::proc_bsdinfo>() as libc::c_int;
    let read = unsafe {
        libc::proc_pidinfo(
            pid,
            libc::PROC_PIDTBSDINFO,
            0,
            (&mut info as *mut libc::proc_bsdinfo).cast(),
            size,
        )
    };
    if read != size || info.pbi_pid != pid as u32 {
        return Err(std::io::Error::other("invalid process start identity"));
    }
    info.pbi_start_tvsec
        .checked_mul(1_000_000)
        .and_then(|seconds| seconds.checked_add(info.pbi_start_tvusec))
        .filter(|value| *value > 0)
        .ok_or_else(|| std::io::Error::other("invalid process start identity"))
}

fn read_receipt(identity: &ContainmentIdentity) -> Option<Receipt> {
    let ContainmentIdentity::MacosGuard { receipt, .. } = identity else {
        return None;
    };
    let mut bytes = Vec::new();
    std::fs::File::open(receipt)
        .ok()?
        .take(RECEIPT_LIMIT)
        .read_to_end(&mut bytes)
        .ok()?;
    let proof: Receipt = serde_json::from_slice(&bytes).ok()?;
    (proof.version == IDENTITY_VERSION && proof.ended && &proof.identity == identity)
        .then_some(proof)
}

pub(super) async fn probe(identity: &ContainmentIdentity) -> ContainmentStatus {
    let ContainmentIdentity::MacosGuard {
        version,
        scope,
        boot_session: previous_boot,
        ..
    } = identity
    else {
        return ContainmentStatus::Unproven;
    };
    if *version != IDENTITY_VERSION || scope != SCOPE {
        return ContainmentStatus::Unproven;
    }
    if let Ok(current) = boot_session() {
        if uuid::Uuid::parse_str(previous_boot).is_ok() && &current != previous_boot {
            return ContainmentStatus::Ended;
        }
    }
    // Never a stale numeric kill, enumeration or adoption: only the exact
    // finalized receipt. A bounded wait lets a still-running guard finish.
    let deadline = tokio::time::Instant::now() + CLEANUP_TIMEOUT;
    loop {
        if read_receipt(identity).is_some() {
            return ContainmentStatus::Ended;
        }
        if tokio::time::Instant::now() >= deadline {
            return ContainmentStatus::Unproven;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

fn last_error() -> std::io::Error {
    std::io::Error::last_os_error()
}

fn close_on_exec(fd: libc::c_int) -> std::io::Result<()> {
    if unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) } < 0 {
        return Err(last_error());
    }
    Ok(())
}

fn pipe() -> std::io::Result<(libc::c_int, libc::c_int)> {
    let mut fds = [0; 2];
    if unsafe { libc::pipe(fds.as_mut_ptr()) } != 0 {
        return Err(last_error());
    }
    close_on_exec(fds[0])?;
    close_on_exec(fds[1])?;
    Ok((fds[0], fds[1]))
}

fn descriptor_limit() -> libc::c_int {
    unsafe { libc::getdtablesize() }.clamp(256, 65_536)
}

/// Fork the gated trampoline. Only async-signal-safe calls run in the child.
fn spawn_trampoline(
    command: &[String],
    release_read: libc::c_int,
    status_write: libc::c_int,
) -> std::io::Result<libc::pid_t> {
    let arguments = command
        .iter()
        .map(|argument| std::ffi::CString::new(argument.as_bytes()))
        .collect::<Result<Vec<_>, _>>()
        .map_err(std::io::Error::other)?;
    let mut pointers: Vec<*const libc::c_char> =
        arguments.iter().map(|argument| argument.as_ptr()).collect();
    pointers.push(std::ptr::null());
    let limit = descriptor_limit();
    let pid = unsafe { libc::fork() };
    if pid < 0 {
        return Err(last_error());
    }
    if pid == 0 {
        unsafe {
            if libc::setpgid(0, 0) != 0 {
                libc::_exit(125);
            }
            for fd in 3..limit {
                if fd != release_read && fd != status_write {
                    libc::close(fd);
                }
            }
            // Rust ignores SIGPIPE; the detector must start with defaults.
            libc::signal(libc::SIGPIPE, libc::SIG_DFL);
            let mut mask: libc::sigset_t = std::mem::zeroed();
            libc::sigemptyset(&mut mask);
            libc::sigprocmask(libc::SIG_SETMASK, &mask, std::ptr::null_mut());
            let mut gate = 0u8;
            let read = loop {
                let read = libc::read(release_read, (&mut gate as *mut u8).cast(), 1);
                if read < 0 && *libc::__error() == libc::EINTR {
                    continue;
                }
                break read;
            };
            // EOF means the guard closed the gate or died: never exec.
            if read != 1 || gate != b'S' {
                libc::_exit(125);
            }
            libc::execvp(pointers[0], pointers.as_ptr());
            let errno: i32 = *libc::__error();
            libc::write(
                status_write,
                (&errno as *const i32).cast(),
                std::mem::size_of::<i32>(),
            );
            libc::_exit(127);
        }
    }
    // Both sides set the group so there is no window either way.
    unsafe {
        libc::setpgid(pid, pid);
    }
    Ok(pid)
}

fn refuse(control: &mut UnixStream, admission: bool, reason: String) -> std::io::Result<()> {
    let mut wire = serde_json::to_vec(&Handshake::Refused {
        admission,
        reason: reason.clone(),
    })
    .map_err(std::io::Error::other)?;
    wire.push(b'\n');
    let _ = control.write_all(&wire);
    Err(std::io::Error::other(reason))
}

/// Drive the anchor to a sealed, reaped, verifiably absent former group.
/// Returns only when the completion predicate held.
fn finish(mut anchor: Anchor) -> std::io::Result<()> {
    let started = std::time::Instant::now();
    let sealed = loop {
        anchor = match anchor.seal_if_complete() {
            Ok(sealed) => break sealed,
            Err(anchor) => anchor,
        };
        // Signal success, ESRCH, EPERM, repetition and quiet time are not
        // completion; only the next positive pair is.
        let _ = anchor.kill_group();
        // Past the runtime's bound the guard keeps owning the anchor rather
        // than sealing early or abandoning a still-visible group.
        std::thread::sleep(if started.elapsed() >= CLEANUP_TIMEOUT {
            Duration::from_millis(100)
        } else {
            Duration::from_millis(5)
        });
    };
    let (former, _) = sealed.reap()?;
    let started = std::time::Instant::now();
    let mut pause = Duration::from_millis(5);
    loop {
        match former.observe() {
            Observation::Absent => return Ok(()),
            Observation::Present | Observation::Uncertain(_) => {
                if started.elapsed() >= OBSERVE_LIMIT {
                    return Err(std::io::Error::other(
                        "former process group is still present; containment unproven",
                    ));
                }
                std::thread::sleep(pause);
                pause = (pause * 2).min(Duration::from_millis(250));
            }
        }
    }
}

pub(super) fn guard_main() -> std::io::Result<()> {
    let fd: i32 = std::env::args()
        .nth(1)
        .ok_or_else(|| std::io::Error::other("missing control descriptor"))?
        .parse()
        .map_err(std::io::Error::other)?;
    if fd < 3 {
        return Err(std::io::Error::other("invalid control descriptor"));
    }
    close_on_exec(fd)?;
    // An inherited SIGCHLD=SIG_IGN (or SA_NOCLDWAIT) would auto-reap the
    // leader and destroy the anchor. Restore the default before any child.
    unsafe {
        let mut action: libc::sigaction = std::mem::zeroed();
        action.sa_sigaction = libc::SIG_DFL;
        libc::sigemptyset(&mut action.sa_mask);
        if libc::sigaction(libc::SIGCHLD, &action, std::ptr::null_mut()) != 0 {
            return Err(last_error());
        }
        let mut mask: libc::sigset_t = std::mem::zeroed();
        libc::sigemptyset(&mut mask);
        libc::sigprocmask(libc::SIG_SETMASK, &mask, std::ptr::null_mut());
        // Another attempt's liveness or control handle must not survive here.
        for other in 3..descriptor_limit() {
            if other != fd {
                libc::close(other);
            }
        }
    }
    let mut control = unsafe { UnixStream::from_raw_fd(fd) };
    let mut wire = Vec::new();
    // Do not buffer beyond the newline: the next byte is the launch gate.
    loop {
        let mut byte = [0];
        if control.read(&mut byte)? == 0 {
            return Ok(());
        }
        if byte[0] == b'\n' {
            break;
        }
        wire.push(byte[0]);
        if wire.len() > MAX_STDIN_BYTES {
            return Err(std::io::Error::other("launch limit"));
        }
    }
    let launch: Launch = serde_json::from_slice(&wire).map_err(std::io::Error::other)?;
    if launch.command.is_empty() {
        return Err(std::io::Error::other("empty launch command"));
    }
    // The guard validates its own environment; it does not trust the
    // runtime's verdict.
    if let Err(reason) = admission::check() {
        return refuse(&mut control, true, reason);
    }
    let (boot, guard_started) = match (boot_session(), started_us(unsafe { libc::getpid() })) {
        (Ok(boot), Ok(started)) => (boot, started),
        (Err(error), _) | (_, Err(error)) => return refuse(&mut control, true, error.to_string()),
    };
    let (release_read, release_write) = pipe()?;
    let (status_read, status_write) = pipe()?;
    let leader = match spawn_trampoline(&launch.command, release_read, status_write) {
        Ok(leader) => leader,
        Err(error) => return refuse(&mut control, false, format!("fork trampoline: {error}")),
    };
    // SAFETY: `leader` was forked just above, has never been waited for, and
    // made itself (and was made) the leader of its own group.
    let anchor = unsafe { Anchor::from_owned_child(leader) };
    unsafe {
        libc::close(release_read);
        libc::close(status_write);
    }
    // Probe the required capabilities on the actual gated leader.
    let probe = (|| -> Result<u64, String> {
        if leader <= 1 || unsafe { libc::getpgid(leader) } != leader {
            return Err("the trampoline is not its own process-group leader".into());
        }
        match anchor.terminal() {
            Ok(None) => {}
            Ok(Some(_)) => return Err("the gated trampoline exited before release".into()),
            Err(error) => return Err(format!("nonreaping waitid probe failed: {error}")),
        }
        match anchor.census() {
            authority::Census::OnlyLeader => {}
            authority::Census::Rejected(reason) => {
                return Err(format!("process-group census probe failed: {reason}"))
            }
        }
        started_us(leader).map_err(|error| error.to_string())
    })();
    let leader_started = match probe {
        Ok(started) => started,
        Err(reason) => {
            // Never released: only our own trampoline code exists. Dropping
            // the anchor kills it; no group receipt is fabricated.
            unsafe { libc::close(release_write) };
            drop(anchor);
            return refuse(&mut control, true, reason);
        }
    };
    let identity = ContainmentIdentity::MacosGuard {
        version: IDENTITY_VERSION,
        scope: SCOPE.into(),
        anchor: ANCHOR.into(),
        boot_session: boot,
        guard_pid: std::process::id(),
        guard_started_us: guard_started,
        leader_pid: leader as u32,
        leader_started_us: leader_started,
        pgid: leader as u32,
        receipt: launch.receipt.clone(),
        nonce: launch.nonce,
    };
    let mut ready = serde_json::to_vec(&Handshake::Ready {
        identity: identity.clone(),
    })
    .map_err(std::io::Error::other)?;
    ready.push(b'\n');
    let mut gate = [0];
    // No detector runs before both this handshake and the durable gate.
    let released = control.write_all(&ready).is_ok()
        && control.read(&mut gate).unwrap_or(0) == 1
        && gate[0] == b'S'
        && unsafe { libc::write(release_write, b"S".as_ptr().cast(), 1) } == 1;
    // Closing the gate unreleased makes the trampoline exit without exec.
    unsafe { libc::close(release_write) };
    let mut code = None;
    let mut spawn_error = None;
    if released {
        control.set_nonblocking(true)?;
        unsafe {
            let flags = libc::fcntl(status_read, libc::F_GETFL);
            libc::fcntl(status_read, libc::F_SETFL, flags | libc::O_NONBLOCK);
        }
        loop {
            match control.read(&mut gate) {
                Ok(_) => break,
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
                    ) => {}
                Err(_) => break,
            }
            match anchor.terminal() {
                Ok(Some(terminal)) => {
                    code = terminal.exit_code;
                    break;
                }
                Ok(None) => {}
                Err(_) => break,
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        let mut errno = [0u8; 4];
        if unsafe { libc::read(status_read, errno.as_mut_ptr().cast(), errno.len()) } == 4 {
            spawn_error =
                Some(std::io::Error::from_raw_os_error(i32::from_ne_bytes(errno)).to_string());
            code = None;
        }
    }
    unsafe { libc::close(status_read) };
    finish(anchor)?;
    write_receipt(identity, code, spawn_error)
}

fn write_receipt(
    identity: ContainmentIdentity,
    exit_code: Option<i32>,
    spawn_error: Option<String>,
) -> std::io::Result<()> {
    let ContainmentIdentity::MacosGuard { receipt, .. } = &identity else {
        unreachable!()
    };
    let proof = Receipt {
        version: IDENTITY_VERSION,
        identity: identity.clone(),
        ended: true,
        exit_code,
        spawn_error,
    };
    let bytes = serde_json::to_vec(&proof).map_err(std::io::Error::other)?;
    // Readers never see a partial receipt: write and fully sync a private
    // temporary, then publish it without replacing any existing file.
    let mut partial = receipt.clone().into_os_string();
    partial.push(".partial");
    let partial = PathBuf::from(partial);
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&partial)?;
    file.write_all(&bytes)?;
    // On macOS `sync_all` is F_FULLFSYNC.
    file.sync_all()?;
    drop(file);
    std::fs::hard_link(&partial, receipt)?;
    std::fs::remove_file(&partial)?;
    if let Some(parent) = receipt.parent() {
        std::fs::File::open(parent)?.sync_all()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::admission::{evaluate, Facts};
    use super::authority::{classify_census, classify_observation, Census, Observation};

    fn facts(translated: super::admission::Fact<i32>, arm64: Option<i32>, kernel: &str) -> Facts {
        Facts {
            translated,
            hardware_arm64: Ok(arm64),
            kernel_version: Ok(Some(kernel.into())),
        }
    }

    const ARM: &str =
        "Darwin Kernel Version 25.6.0: Tue Aug 18 17:50:45 PDT 2026; root:xnu-1~1/RELEASE_ARM64_T6020";
    const INTEL: &str =
        "Darwin Kernel Version 24.6.0: Mon Jan  1 00:00:00 PST 2026; root:xnu-1~1/RELEASE_X86_64";

    #[test]
    fn admission_accepts_native_arm_and_intel_enoent() {
        assert!(evaluate(&facts(Ok(Some(0)), Some(1), ARM), true).is_ok());
        // Intel Macs have neither sysctl: ENOENT is native, not a failure.
        assert!(evaluate(&facts(Ok(None), None, INTEL), false).is_ok());
        assert!(evaluate(&facts(Ok(None), Some(0), INTEL), false).is_ok());
    }

    #[test]
    fn admission_refuses_translation_errors_and_architecture_mismatch() {
        assert!(evaluate(&facts(Ok(Some(1)), Some(1), ARM), false).is_err());
        // A translated process that hides the flag is still an x86_64
        // process on arm64 hardware.
        assert!(evaluate(&facts(Ok(Some(0)), Some(1), ARM), false).is_err());
        assert!(evaluate(&facts(Err(libc::EPERM), Some(1), ARM), true).is_err());
        assert!(evaluate(&facts(Ok(Some(7)), Some(1), ARM), true).is_err());
        let mut indeterminate = facts(Ok(Some(0)), Some(1), ARM);
        indeterminate.hardware_arm64 = Err(libc::EIO);
        assert!(evaluate(&indeterminate, true).is_err());
    }

    #[test]
    fn admission_refuses_development_kasan_and_unreadable_kernels() {
        for flavor in [
            "DEVELOPMENT_ARM64_T6020",
            "KASAN_ARM64_T6020",
            "DEBUG_X86_64",
            "",
        ] {
            let version = format!("Darwin Kernel Version 25.6.0: root:xnu-1~1/{flavor}");
            assert!(
                evaluate(&facts(Ok(Some(0)), Some(1), &version), true).is_err(),
                "{flavor}"
            );
        }
        assert!(evaluate(&facts(Ok(Some(0)), Some(1), "RELEASE_ARM64"), true).is_err());
        let mut missing = facts(Ok(Some(0)), Some(1), ARM);
        missing.kernel_version = Ok(None);
        assert!(evaluate(&missing, true).is_err());
        missing.kernel_version = Err(libc::EIO);
        assert!(evaluate(&missing, true).is_err());
    }

    #[test]
    fn version_identity_never_rejects() {
        // Old, new and unparseable Darwin versions are all admitted: only
        // the release flavor is read from this string.
        for version in [
            "19.6.0",
            "23.6.0",
            "24.6.0",
            "25.6.0",
            "99.0.0",
            "not-a-version",
        ] {
            let kernel =
                format!("Darwin Kernel Version {version}: root:xnu-1~1/RELEASE_ARM64_T6020");
            assert!(evaluate(&facts(Ok(Some(0)), Some(1), &kernel), true).is_ok());
        }
    }

    #[test]
    fn this_host_is_admitted_or_reports_a_concrete_reason() {
        match super::admission::check() {
            Ok(()) => {}
            Err(reason) => assert!(!reason.is_empty()),
        }
    }

    #[test]
    fn census_accepts_only_exactly_the_owned_leader() {
        assert_eq!(classify_census(4, 500, 500), Census::OnlyLeader);
        for (bytes, slot0) in [
            (0, 0),
            (-1, 0),
            (8, 500),
            (3, 500),
            (5, 500),
            (12, 500),
            (4, 501),
            (4, 0),
        ] {
            assert!(
                matches!(classify_census(bytes, slot0, 500), Census::Rejected(_)),
                "{bytes} {slot0}"
            );
        }
    }

    #[test]
    fn only_esrch_is_absence() {
        assert_eq!(classify_observation(-1, libc::ESRCH), Observation::Absent);
        assert_eq!(classify_observation(0, 0), Observation::Present);
        assert_eq!(classify_observation(0, libc::ESRCH), Observation::Present);
        assert_eq!(classify_observation(-1, libc::EPERM), Observation::Present);
        assert_eq!(
            classify_observation(-1, libc::EINVAL),
            Observation::Uncertain(libc::EINVAL)
        );
    }

    /// Source-boundary proof that no signal authority exists after the seal:
    /// the only nonzero signal is `Anchor::kill_group` (plus its `Drop`), the
    /// only other `kill` is signal zero, and no authority type can be cloned.
    #[test]
    fn signal_authority_is_confined_to_the_anchor() {
        let source = include_str!("process_macos.rs");
        let code: String = source
            .lines()
            .filter(|line| !line.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        // Product code only: everything before this test module.
        let code = &code[..code.find("#[cfg(test)]").unwrap()];
        let start = code.find("pub(super) mod authority {").unwrap();
        let end = code.find("use authority::{Anchor, Observation};").unwrap();
        let (authority, rest) = (&code[start..end], [&code[..start], &code[end..]].concat());
        let call = ["libc::", "kill("].concat();
        assert!(!rest.contains(&call), "kill outside authority");
        for forbidden in ["killpg(", "pthread_kill(", "libc::raise(", "posix_spawn"] {
            assert!(!code.contains(forbidden), "{forbidden}");
        }
        assert_eq!(authority.matches(&call).count(), 3);
        assert_eq!(authority.matches("libc::SIGKILL").count(), 2);
        assert_eq!(authority.matches("libc::kill(-self.pgid, 0)").count(), 1);
        let anchor = authority.find("impl Anchor {").unwrap();
        let sealed = authority.find("pub(crate) struct Sealed {").unwrap();
        assert!(anchor < sealed);
        // Everything from the sealed state onward is free of nonzero signals.
        assert!(!authority[sealed..].contains("SIGKILL"));
        for name in ["Anchor", "Sealed", "FormerGroup"] {
            let declaration = authority
                .find(&format!("pub(crate) struct {name} {{"))
                .unwrap();
            let attributes = authority[..declaration].rsplit("\n\n").next().unwrap();
            assert!(!attributes.contains("derive"), "{name} must not derive");
            assert!(!authority.contains(&format!("Clone for {name}")));
        }
        // One constructor, and it is only used where the leader is forked.
        assert_eq!(code.matches("Anchor::from_owned_child(").count(), 1);
        assert_eq!(authority.matches("fn from_owned_child(").count(), 1);
    }
}
