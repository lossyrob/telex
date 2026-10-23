//! Bounded execution of trusted, same-user commands, not a sandbox.
//!
//! Windows assigns the suspended process to a kill-on-close job *at creation*.
//! Linux uses a separately executable, pipe-owned subreaper. The launch gate is
//! released only after the caller durably records the containment identity.
//! Unsupported platforms fail closed rather than substituting process groups
//! (which do not provide runtime-death containment).

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::sync::oneshot;

pub const MAX_STDIN_BYTES: usize = 1024 * 1024;
pub const MAX_STDOUT_BYTES: usize = 256 * 1024;
pub const MAX_STDERR_BYTES: usize = 64 * 1024;
const CLEANUP_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone)]
pub struct ProcessSpec {
    pub command: Vec<String>,
    pub working_directory: PathBuf,
    pub environment_allowlist: Vec<String>,
    pub timeout: Duration,
    /// Registry-owned persistent storage, not a temporary directory. Keep
    /// receipts while their corresponding interrupted attempts may be probed.
    pub containment_directory: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "platform", rename_all = "kebab-case", deny_unknown_fields)]
pub enum ContainmentIdentity {
    WindowsJob {
        name: String,
        owner_pid: u32,
        owner_created: u64,
    },
    LinuxGuard {
        pid: u32,
        start_ticks: u64,
        boot_id: String,
        receipt: PathBuf,
        nonce: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContainmentStatus {
    Ended,
    Unproven,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProcessFailureKind {
    InvalidCommand,
    WorkingDirectory,
    Spawn,
    UnsupportedPlatform,
    InputLimit,
    StdoutLimit,
    StderrLimit,
    Io,
    Timeout,
    Cancelled,
    NonzeroExit,
    ContainmentRecording,
    ContainmentUnproven,
}

#[derive(Debug, Clone)]
pub struct ProcessFailure {
    pub kind: ProcessFailureKind,
    pub message: String,
    /// Present when an attempt may need restart reconciliation.
    pub containment: Option<Box<ContainmentIdentity>>,
}

impl std::fmt::Display for ProcessFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}: {}", self.kind, self.message)
    }
}

impl std::error::Error for ProcessFailure {}

impl ProcessFailure {
    pub fn category(&self) -> &'static str {
        match self.kind {
            ProcessFailureKind::Timeout => "timeout",
            ProcessFailureKind::InputLimit
            | ProcessFailureKind::StdoutLimit
            | ProcessFailureKind::StderrLimit => "output-limit",
            ProcessFailureKind::Cancelled => "runtime-interrupted",
            ProcessFailureKind::ContainmentUnproven => "orphan-containment-unproven",
            _ => "process-failed",
        }
    }

    fn new(kind: ProcessFailureKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            containment: None,
        }
    }

    fn io(error: impl std::fmt::Display) -> Self {
        Self::new(ProcessFailureKind::Io, error.to_string())
    }
}

#[derive(Debug)]
pub struct ProcessOutput {
    pub stdout: Vec<u8>,
    /// UTF-8 lossily decoded, with sensitive allowlist values redacted.
    pub stderr: String,
    pub exit_code: Option<i32>,
    pub containment: ContainmentIdentity,
}

/// Cancellation includes dropping the sender. Dropping this future also closes
/// the platform's lifetime capability, so aborting a runtime task cannot detach
/// its detector tree.
pub async fn execute<F, Fut>(
    spec: &ProcessSpec,
    request: Vec<u8>,
    mut cancellation: oneshot::Receiver<()>,
    record_containment: F,
) -> Result<ProcessOutput, ProcessFailure>
where
    F: FnOnce(ContainmentIdentity) -> Fut,
    Fut: Future<Output = Result<(), String>>,
{
    if request.len() > MAX_STDIN_BYTES {
        return Err(ProcessFailure::new(
            ProcessFailureKind::InputLimit,
            "detector request exceeds 1 MiB",
        ));
    }
    if spec.command.is_empty() || spec.command[0].is_empty() || spec.timeout.is_zero() {
        return Err(ProcessFailure::new(
            ProcessFailureKind::InvalidCommand,
            "nonempty argv and a positive timeout are required",
        ));
    }
    let cwd = std::fs::canonicalize(&spec.working_directory).map_err(|error| {
        ProcessFailure::new(ProcessFailureKind::WorkingDirectory, error.to_string())
    })?;
    if !spec.working_directory.is_absolute() || !cwd.is_dir() {
        return Err(ProcessFailure::new(
            ProcessFailureKind::WorkingDirectory,
            "working directory must be an existing absolute directory",
        ));
    }
    let (environment, sensitive) = launch_environment(&spec.environment_allowlist)?;
    let deadline = tokio::time::Instant::now() + spec.timeout;
    let mut child = tokio::select! {
        biased;
        _ = &mut cancellation => return Err(ProcessFailure::new(ProcessFailureKind::Cancelled, "cancelled before launch")),
        _ = tokio::time::sleep_until(deadline) => return Err(ProcessFailure::new(ProcessFailureKind::Timeout, "timed out preparing containment")),
        result = platform::prepare(spec, &cwd, &environment) => result?,
    };
    let identity = child.identity().clone();
    let recording = tokio::select! {
        biased;
        _ = &mut cancellation => Err(ProcessFailure::new(ProcessFailureKind::Cancelled, "cancelled before launch gate")),
        _ = tokio::time::sleep_until(deadline) => Err(ProcessFailure::new(ProcessFailureKind::Timeout, "timed out before launch gate")),
        result = record_containment(identity.clone()) => result.map_err(|error| ProcessFailure::new(ProcessFailureKind::ContainmentRecording, error)),
    };
    let result = match recording {
        Err(error) => Err(error),
        Ok(()) => match child.start(request).await {
            Err(error) => Err(error),
            Ok(()) => tokio::select! {
                biased;
                _ = &mut cancellation => Err(ProcessFailure::new(ProcessFailureKind::Cancelled, "runtime shutdown")),
                _ = tokio::time::sleep_until(deadline) => Err(ProcessFailure::new(ProcessFailureKind::Timeout, "detector timed out")),
                result = child.complete() => result,
            },
        },
    };
    // Always terminate descendants, including after a successful direct-child
    // exit. A descendant retaining a pipe must not postpone normal cleanup.
    if child.cleanup().await != ContainmentStatus::Ended {
        return Err(ProcessFailure {
            kind: ProcessFailureKind::ContainmentUnproven,
            message: "could not prove the detector containment ended".into(),
            containment: Some(Box::new(identity)),
        });
    }
    match result {
        Ok((stdout, stderr, exit_code)) => {
            let stderr = redact(&stderr, &sensitive);
            if exit_code != Some(0) {
                return Err(ProcessFailure {
                    kind: ProcessFailureKind::NonzeroExit,
                    message: format!("detector exited with {exit_code:?}; stderr: {stderr}"),
                    containment: Some(Box::new(identity)),
                });
            }
            Ok(ProcessOutput {
                stdout,
                stderr,
                exit_code,
                containment: identity,
            })
        }
        Err(mut error) => {
            error.message = redact(error.message.as_bytes(), &sensitive);
            error.containment = Some(Box::new(identity));
            Err(error)
        }
    }
}

/// Evidence, never elapsed time or an absent/reused PID, decides recovery.
/// A bounded wait lets an EOF-triggered guardian finish. Missing/corrupt proof
/// and inaccessible OS state remain unproven.
pub async fn probe_previous(identity: &ContainmentIdentity) -> ContainmentStatus {
    platform::probe(identity).await
}

type Environment = BTreeMap<OsString, OsString>;

fn launch_environment(allowlist: &[String]) -> Result<(Environment, Vec<String>), ProcessFailure> {
    let mut environment = Environment::new();
    // These are launch mechanics, not arbitrary application configuration.
    let baseline = [
        "PATH",
        "PATHEXT",
        "SystemRoot",
        "WINDIR",
        "HOME",
        "USERPROFILE",
        "HOMEDRIVE",
        "HOMEPATH",
        "TMPDIR",
        "TMP",
        "TEMP",
        "LANG",
        "LANGUAGE",
        "LC_ALL",
        "LC_CTYPE",
        "LC_COLLATE",
        "LC_MESSAGES",
        "LC_MONETARY",
        "LC_NUMERIC",
        "LC_TIME",
    ];
    for name in baseline {
        if let Some(value) = std::env::var_os(name) {
            environment.insert(environment_key(name), value);
        }
    }
    let mut sensitive = Vec::new();
    for name in allowlist {
        if name.is_empty() || name.contains(['=', '\0']) {
            return Err(ProcessFailure::new(
                ProcessFailureKind::InvalidCommand,
                "invalid environment variable name",
            ));
        }
        if let Some(value) = std::env::var_os(name) {
            let lower = name.to_ascii_lowercase();
            if ["token", "pat", "key", "secret", "password", "credential"]
                .iter()
                .any(|part| lower.contains(part))
            {
                if let Some(value) = value.to_str().filter(|value| !value.is_empty()) {
                    sensitive.push(value.to_owned());
                }
            }
            environment.insert(environment_key(name), value);
        }
    }
    sensitive.sort_by_key(|value| std::cmp::Reverse(value.len()));
    Ok((environment, sensitive))
}

fn environment_key(name: &str) -> OsString {
    if cfg!(windows) {
        name.to_ascii_uppercase().into()
    } else {
        name.into()
    }
}

fn redact(bytes: &[u8], sensitive: &[String]) -> String {
    let mut text = String::from_utf8_lossy(bytes).into_owned();
    for value in sensitive {
        text = text.replace(value, "[REDACTED]");
    }
    // Redaction and lossy UTF-8 conversion must not increase diagnostic bounds.
    if text.len() > MAX_STDERR_BYTES {
        let mut end = MAX_STDERR_BYTES;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
    }
    text
}

type Completion = (Vec<u8>, Vec<u8>, Option<i32>);

#[cfg(target_os = "linux")]
async fn read_capped<R: tokio::io::AsyncRead + Unpin>(
    mut reader: R,
    limit: usize,
    kind: ProcessFailureKind,
) -> Result<Vec<u8>, ProcessFailure> {
    use tokio::io::AsyncReadExt;
    let mut output = Vec::new();
    let mut buffer = [0; 8192];
    loop {
        let read = reader.read(&mut buffer).await.map_err(ProcessFailure::io)?;
        if read == 0 {
            return Ok(output);
        }
        if output.len() + read > limit {
            return Err(ProcessFailure::new(
                kind,
                "detector output exceeded byte cap",
            ));
        }
        output.extend_from_slice(&buffer[..read]);
    }
}

#[cfg(windows)]
mod platform {
    use super::*;
    use std::fs::File;
    use std::io::{Read, Write};
    use std::mem::{size_of, zeroed};
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::io::{FromRawHandle, RawHandle};
    use std::ptr::{null, null_mut};
    use windows_sys::Win32::Foundation::*;
    use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;
    use windows_sys::Win32::System::JobObjects::*;
    use windows_sys::Win32::System::Pipes::CreatePipe;
    use windows_sys::Win32::System::Threading::*;

    struct Handle(HANDLE);
    impl Drop for Handle {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
    impl Handle {
        fn into_file(self) -> File {
            let raw = self.0;
            std::mem::forget(self);
            unsafe { File::from_raw_handle(raw as RawHandle) }
        }
    }

    struct Attributes {
        // usize backing gives the alignment required by the Windows ABI.
        _storage: Vec<usize>,
        pointer: LPPROC_THREAD_ATTRIBUTE_LIST,
    }
    // The attribute list is used synchronously only during CreateProcessW.
    unsafe impl Send for Attributes {}
    impl Drop for Attributes {
        fn drop(&mut self) {
            unsafe {
                DeleteProcThreadAttributeList(self.pointer);
            }
        }
    }
    impl Attributes {
        fn new() -> Result<Self, ProcessFailure> {
            let mut bytes = 0;
            unsafe {
                InitializeProcThreadAttributeList(null_mut(), 2, 0, &mut bytes);
            }
            if bytes == 0 {
                return Err(ProcessFailure::io(std::io::Error::last_os_error()));
            }
            let mut storage = vec![0usize; bytes.div_ceil(size_of::<usize>())];
            let pointer = storage.as_mut_ptr() as LPPROC_THREAD_ATTRIBUTE_LIST;
            if unsafe { InitializeProcThreadAttributeList(pointer, 2, 0, &mut bytes) } == 0 {
                return Err(ProcessFailure::io(std::io::Error::last_os_error()));
            }
            Ok(Self {
                _storage: storage,
                pointer,
            })
        }
        fn set(&self, attribute: usize, handles: &[HANDLE]) -> Result<(), ProcessFailure> {
            if unsafe {
                UpdateProcThreadAttribute(
                    self.pointer,
                    0,
                    attribute,
                    handles.as_ptr() as *const _,
                    std::mem::size_of_val(handles),
                    null_mut(),
                    null(),
                )
            } == 0
            {
                return Err(ProcessFailure::new(
                    ProcessFailureKind::Spawn,
                    format!(
                        "atomic containment attributes unavailable: {}",
                        std::io::Error::last_os_error()
                    ),
                ));
            }
            Ok(())
        }
    }

    pub(super) struct Prepared {
        // Drop closes the only lifetime handle, killing all associated tasks.
        job: Handle,
        process: Handle,
        thread: Handle,
        identity: ContainmentIdentity,
        stdin: Option<File>,
        stdout: Option<File>,
        stderr: Option<File>,
        writer: Option<tokio::task::JoinHandle<std::io::Result<()>>>,
    }

    fn wide(value: &OsStr) -> Result<Vec<u16>, ProcessFailure> {
        let mut value: Vec<_> = value.encode_wide().collect();
        if value.contains(&0) {
            return Err(ProcessFailure::new(
                ProcessFailureKind::InvalidCommand,
                "embedded NUL in launch value",
            ));
        }
        value.push(0);
        Ok(value)
    }

    fn pipe(parent_reads: bool) -> Result<(File, Handle), ProcessFailure> {
        let mut read = 0;
        let mut write = 0;
        let security = SECURITY_ATTRIBUTES {
            nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: null_mut(),
            bInheritHandle: 1,
        };
        if unsafe { CreatePipe(&mut read, &mut write, &security, 0) } == 0 {
            return Err(ProcessFailure::io(std::io::Error::last_os_error()));
        }
        let (parent, child) = if parent_reads {
            (Handle(read), Handle(write))
        } else {
            (Handle(write), Handle(read))
        };
        if unsafe { SetHandleInformation(parent.0, HANDLE_FLAG_INHERIT, 0) } == 0 {
            return Err(ProcessFailure::io(std::io::Error::last_os_error()));
        }
        Ok((parent.into_file(), child))
    }

    fn quote(argument: &str) -> String {
        let mut quoted = String::from("\"");
        let mut slashes = 0;
        for ch in argument.chars() {
            if ch == '\\' {
                slashes += 1;
            } else {
                if ch == '"' {
                    quoted.extend(std::iter::repeat_n('\\', slashes * 2 + 1));
                } else {
                    quoted.extend(std::iter::repeat_n('\\', slashes));
                }
                slashes = 0;
                quoted.push(ch);
            }
        }
        quoted.extend(std::iter::repeat_n('\\', slashes * 2));
        quoted.push('"');
        quoted
    }

    fn executable(
        program: &str,
        cwd: &Path,
        environment: &Environment,
    ) -> Result<PathBuf, ProcessFailure> {
        let program = Path::new(program);
        let candidates: Vec<PathBuf> = if program.is_absolute() {
            vec![program.to_path_buf()]
        } else if program.components().count() > 1 {
            vec![cwd.join(program)]
        } else {
            environment
                .get(OsStr::new("PATH"))
                .map(|path| {
                    std::env::split_paths(path)
                        .map(|dir| {
                            if dir.is_absolute() {
                                dir.join(program)
                            } else {
                                cwd.join(dir).join(program)
                            }
                        })
                        .collect()
                })
                .unwrap_or_default()
        };
        for candidate in candidates {
            if candidate.is_file() {
                return Ok(candidate);
            }
            if candidate.extension().is_none() {
                let executable = candidate.with_extension("exe");
                if executable.is_file() {
                    return Ok(executable);
                }
            }
        }
        Err(ProcessFailure::new(
            ProcessFailureKind::Spawn,
            "executable not found in the controlled launch path",
        ))
    }

    fn created(handle: HANDLE) -> Result<u64, ProcessFailure> {
        let mut creation = unsafe { zeroed::<FILETIME>() };
        let mut exit = unsafe { zeroed::<FILETIME>() };
        let mut kernel = unsafe { zeroed::<FILETIME>() };
        let mut user = unsafe { zeroed::<FILETIME>() };
        if unsafe { GetProcessTimes(handle, &mut creation, &mut exit, &mut kernel, &mut user) } == 0
        {
            return Err(ProcessFailure::io(std::io::Error::last_os_error()));
        }
        Ok((u64::from(creation.dwHighDateTime) << 32) | u64::from(creation.dwLowDateTime))
    }

    pub(super) async fn prepare(
        spec: &ProcessSpec,
        cwd: &Path,
        environment: &Environment,
    ) -> Result<Prepared, ProcessFailure> {
        let name = format!("Local\\telex-watcher-{}", uuid::Uuid::new_v4());
        let encoded_name = wide(OsStr::new(&name))?;
        let job_raw = unsafe { CreateJobObjectW(null(), encoded_name.as_ptr()) };
        if job_raw == 0 {
            return Err(ProcessFailure::io(std::io::Error::last_os_error()));
        }
        let job = Handle(job_raw);
        if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
            return Err(ProcessFailure::new(
                ProcessFailureKind::Spawn,
                "job identity collision",
            ));
        }
        let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { zeroed() };
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        if unsafe {
            SetInformationJobObject(
                job.0,
                JobObjectExtendedLimitInformation,
                &limits as *const _ as *const _,
                size_of_val(&limits) as u32,
            )
        } == 0
        {
            return Err(ProcessFailure::io(std::io::Error::last_os_error()));
        }
        let identity = ContainmentIdentity::WindowsJob {
            name,
            owner_pid: std::process::id(),
            owner_created: created(unsafe { GetCurrentProcess() })?,
        };
        let (stdin, child_stdin) = pipe(false)?;
        let (stdout, child_stdout) = pipe(true)?;
        let (stderr, child_stderr) = pipe(true)?;
        let attributes = Attributes::new()?;
        let inherited = [child_stdin.0, child_stdout.0, child_stderr.0];
        let jobs = [job.0];
        attributes.set(PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize, &inherited)?;
        attributes.set(PROC_THREAD_ATTRIBUTE_JOB_LIST as usize, &jobs)?;
        let mut startup: STARTUPINFOEXW = unsafe { zeroed() };
        startup.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as u32;
        startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
        startup.StartupInfo.hStdInput = child_stdin.0;
        startup.StartupInfo.hStdOutput = child_stdout.0;
        startup.StartupInfo.hStdError = child_stderr.0;
        startup.lpAttributeList = attributes.pointer;
        let application = wide(executable(&spec.command[0], cwd, environment)?.as_os_str())?;
        let mut command_line = wide(OsStr::new(
            &spec
                .command
                .iter()
                .map(|arg| quote(arg))
                .collect::<Vec<_>>()
                .join(" "),
        ))?;
        let cwd = wide(cwd.as_os_str())?;
        let mut environment_block = Vec::<u16>::new();
        for (name, value) in environment {
            let mut entry = OsString::from(name);
            entry.push("=");
            entry.push(value);
            environment_block.extend(wide(&entry)?);
        }
        environment_block.push(0);
        if environment_block.len() == 1 {
            environment_block.push(0);
        }
        let mut information: PROCESS_INFORMATION = unsafe { zeroed() };
        // The job-list attribute is atomic with process creation. A separate
        // AssignProcessToJobObject call has a crash window even when suspended.
        if unsafe {
            CreateProcessW(
                application.as_ptr(),
                command_line.as_mut_ptr(),
                null(),
                null(),
                1,
                CREATE_SUSPENDED
                    | CREATE_UNICODE_ENVIRONMENT
                    | EXTENDED_STARTUPINFO_PRESENT
                    | CREATE_NO_WINDOW,
                environment_block.as_ptr() as *const _,
                cwd.as_ptr(),
                &startup.StartupInfo,
                &mut information,
            )
        } == 0
        {
            return Err(ProcessFailure::new(
                ProcessFailureKind::Spawn,
                std::io::Error::last_os_error().to_string(),
            ));
        }
        Ok(Prepared {
            job,
            process: Handle(information.hProcess),
            thread: Handle(information.hThread),
            identity,
            stdin: Some(stdin),
            stdout: Some(stdout),
            stderr: Some(stderr),
            writer: None,
        })
    }

    fn active(job: HANDLE) -> Result<u32, ProcessFailure> {
        let mut accounting: JOBOBJECT_BASIC_ACCOUNTING_INFORMATION = unsafe { zeroed() };
        if unsafe {
            QueryInformationJobObject(
                job,
                JobObjectBasicAccountingInformation,
                &mut accounting as *mut _ as *mut _,
                size_of_val(&accounting) as u32,
                null_mut(),
            )
        } == 0
        {
            return Err(ProcessFailure::io(std::io::Error::last_os_error()));
        }
        Ok(accounting.ActiveProcesses)
    }

    async fn empty_job(job: HANDLE) -> ContainmentStatus {
        let deadline = tokio::time::Instant::now() + CLEANUP_TIMEOUT;
        loop {
            match active(job) {
                Ok(0) => return ContainmentStatus::Ended,
                Err(_) => return ContainmentStatus::Unproven,
                _ => {}
            }
            if tokio::time::Instant::now() >= deadline {
                return ContainmentStatus::Unproven;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    fn read_blocking(
        mut file: File,
        limit: usize,
        kind: ProcessFailureKind,
    ) -> Result<Vec<u8>, ProcessFailure> {
        let mut bytes = Vec::new();
        let mut buffer = [0; 8192];
        loop {
            let count = file.read(&mut buffer).map_err(ProcessFailure::io)?;
            if count == 0 {
                return Ok(bytes);
            }
            if bytes.len() + count > limit {
                return Err(ProcessFailure::new(
                    kind,
                    "detector output exceeded byte cap",
                ));
            }
            bytes.extend_from_slice(&buffer[..count]);
        }
    }

    impl Prepared {
        pub(super) fn identity(&self) -> &ContainmentIdentity {
            &self.identity
        }

        pub(super) async fn start(&mut self, request: Vec<u8>) -> Result<(), ProcessFailure> {
            if unsafe { ResumeThread(self.thread.0) } != 1 {
                return Err(ProcessFailure::new(
                    ProcessFailureKind::Spawn,
                    "initial thread could not be resumed exactly once",
                ));
            }
            let mut stdin = self
                .stdin
                .take()
                .ok_or_else(|| ProcessFailure::io("missing stdin"))?;
            self.writer = Some(tokio::task::spawn_blocking(move || {
                stdin.write_all(&request)?;
                drop(stdin);
                Ok(())
            }));
            Ok(())
        }

        pub(super) async fn complete(&mut self) -> Result<Completion, ProcessFailure> {
            let stdout = self
                .stdout
                .take()
                .ok_or_else(|| ProcessFailure::io("missing stdout"))?;
            let stderr = self
                .stderr
                .take()
                .ok_or_else(|| ProcessFailure::io("missing stderr"))?;
            let stdout = tokio::task::spawn_blocking(move || {
                read_blocking(stdout, MAX_STDOUT_BYTES, ProcessFailureKind::StdoutLimit)
            });
            let stderr = tokio::task::spawn_blocking(move || {
                read_blocking(stderr, MAX_STDERR_BYTES, ProcessFailureKind::StderrLimit)
            });
            let wait = async {
                loop {
                    match unsafe { WaitForSingleObject(self.process.0, 0) } {
                        WAIT_OBJECT_0 => break,
                        WAIT_TIMEOUT => tokio::time::sleep(Duration::from_millis(5)).await,
                        _ => return Err(ProcessFailure::io(std::io::Error::last_os_error())),
                    }
                }
                let mut code = 0;
                if unsafe { GetExitCodeProcess(self.process.0, &mut code) } == 0 {
                    return Err(ProcessFailure::io(std::io::Error::last_os_error()));
                }
                // Do not wait for pipe EOF before killing leftover descendants.
                if unsafe { TerminateJobObject(self.job.0, 1) } == 0 {
                    return Err(ProcessFailure::io(std::io::Error::last_os_error()));
                }
                Ok(Some(code as i32))
            };
            let (stdout, stderr, code) = tokio::try_join!(
                async { stdout.await.map_err(ProcessFailure::io)? },
                async { stderr.await.map_err(ProcessFailure::io)? },
                wait,
            )?;
            if let Some(writer) = self.writer.take() {
                writer
                    .await
                    .map_err(ProcessFailure::io)?
                    .map_err(ProcessFailure::io)?;
            }
            Ok((stdout, stderr, code))
        }

        pub(super) async fn cleanup(&mut self) -> ContainmentStatus {
            unsafe {
                TerminateJobObject(self.job.0, 1);
            }
            self.stdin.take();
            empty_job(self.job.0).await
        }
    }

    pub(super) async fn probe(identity: &ContainmentIdentity) -> ContainmentStatus {
        let ContainmentIdentity::WindowsJob {
            name,
            owner_pid,
            owner_created,
        } = identity
        else {
            return ContainmentStatus::Unproven;
        };
        let Some(nonce) = name.strip_prefix("Local\\telex-watcher-") else {
            return ContainmentStatus::Unproven;
        };
        if uuid::Uuid::parse_str(nonce).is_err() {
            return ContainmentStatus::Unproven;
        }
        let Ok(name) = wide(OsStr::new(name)) else {
            return ContainmentStatus::Unproven;
        };
        // JOB_OBJECT_QUERY (0x0004) | JOB_OBJECT_TERMINATE (0x0008).
        let raw = unsafe { OpenJobObjectW(0x000c, 0, name.as_ptr()) };
        if raw == 0 {
            // A job is destroyed only after its last handle closes AND all
            // associated processes terminate. Access denied is not absence.
            return if unsafe { GetLastError() } == ERROR_FILE_NOT_FOUND {
                ContainmentStatus::Ended
            } else {
                ContainmentStatus::Unproven
            };
        }
        let job = Handle(raw);
        if active(job.0).ok() == Some(0) {
            return ContainmentStatus::Ended;
        }
        let owner = unsafe {
            OpenProcess(
                PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
                0,
                *owner_pid,
            )
        };
        if owner != 0 {
            let owner = Handle(owner);
            match created(owner.0) {
                Ok(time) if time != *owner_created => {}
                Ok(_) if unsafe { WaitForSingleObject(owner.0, 0) } == WAIT_OBJECT_0 => {}
                _ => return ContainmentStatus::Unproven,
            }
        } else if unsafe { GetLastError() } != ERROR_INVALID_PARAMETER {
            return ContainmentStatus::Unproven;
        }
        // Only a uniquely named job whose exact owner has ended is safe to kill.
        if unsafe { TerminateJobObject(job.0, 1) } == 0 {
            return ContainmentStatus::Unproven;
        }
        empty_job(job.0).await
    }
}

#[cfg(target_os = "linux")]
mod platform {
    use super::*;
    use std::io::{BufRead, Read, Write};
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::net::UnixStream;
    use std::os::unix::process::CommandExt;
    use std::process::Stdio;
    use tokio::process::{Child, Command};

    #[derive(Serialize, Deserialize)]
    struct Launch {
        command: Vec<String>,
        cwd: PathBuf,
        receipt: PathBuf,
        nonce: String,
    }

    #[derive(Serialize, Deserialize)]
    struct Receipt {
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
        if !spec.containment_directory.is_absolute() {
            return Err(ProcessFailure::new(
                ProcessFailureKind::Spawn,
                "containment directory must be absolute",
            ));
        }
        std::fs::create_dir_all(&spec.containment_directory).map_err(ProcessFailure::io)?;
        let root =
            std::fs::canonicalize(&spec.containment_directory).map_err(ProcessFailure::io)?;
        let nonce = uuid::Uuid::new_v4().to_string();
        let launch = Launch {
            command: spec.command.clone(),
            cwd: cwd.to_path_buf(),
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
                // The guardian must survive a signal directed at the
                // runtime's foreground process group as well as its PID.
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
        let identity = tokio::task::spawn_blocking(move || {
            let mut bytes = Vec::new();
            std::io::BufReader::new(handshake)
                .take(8192)
                .read_until(b'\n', &mut bytes)?;
            serde_json::from_slice::<ContainmentIdentity>(&bytes).map_err(std::io::Error::other)
        })
        .await
        .map_err(ProcessFailure::io)?
        .map_err(ProcessFailure::io)?;
        match &identity {
            ContainmentIdentity::LinuxGuard {
                pid,
                start_ticks,
                boot_id,
                receipt,
                nonce,
            } if Some(*pid) == child.id()
                && *start_ticks > 0
                && !boot_id.is_empty()
                && receipt == &launch.receipt
                && nonce == &launch.nonce => {}
            _ => {
                return Err(ProcessFailure::io(
                    "process guardian returned an invalid identity",
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

        pub(super) async fn complete(&mut self) -> Result<Completion, ProcessFailure> {
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
            let wait = async {
                let status = self.child.wait().await.map_err(ProcessFailure::io)?;
                if !status.success() {
                    return Err(ProcessFailure::new(
                        ProcessFailureKind::ContainmentUnproven,
                        "process guard failed",
                    ));
                }
                let receipt = read_receipt(&self.identity).ok_or_else(|| {
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
            let (stdout, stderr, code) = tokio::try_join!(
                read_capped(stdout, MAX_STDOUT_BYTES, ProcessFailureKind::StdoutLimit),
                read_capped(stderr, MAX_STDERR_BYTES, ProcessFailureKind::StderrLimit),
                wait,
            )?;
            if let Some(writer) = self.writer.take() {
                writer
                    .await
                    .map_err(ProcessFailure::io)?
                    .map_err(ProcessFailure::io)?;
            }
            Ok((stdout, stderr, code))
        }

        pub(super) async fn cleanup(&mut self) -> ContainmentStatus {
            let _ = self.control.write_all(b"K");
            let _ = self.control.shutdown(std::net::Shutdown::Both);
            if let Some(writer) = self.writer.take() {
                writer.abort();
            }
            let _ = tokio::time::timeout(CLEANUP_TIMEOUT, self.child.wait()).await;
            if read_receipt(&self.identity).is_some() {
                ContainmentStatus::Ended
            } else {
                ContainmentStatus::Unproven
            }
        }
    }

    fn boot_id() -> std::io::Result<String> {
        Ok(std::fs::read_to_string("/proc/sys/kernel/random/boot_id")?
            .trim()
            .to_owned())
    }

    fn start_ticks() -> std::io::Result<u64> {
        let stat = std::fs::read_to_string("/proc/self/stat")?;
        stat.rsplit_once(')')
            .and_then(|(_, tail)| tail.split_whitespace().nth(19))
            .and_then(|value| value.parse().ok())
            .ok_or_else(|| std::io::Error::other("invalid Linux process identity"))
    }

    fn read_receipt(identity: &ContainmentIdentity) -> Option<Receipt> {
        let ContainmentIdentity::LinuxGuard { receipt, .. } = identity else {
            return None;
        };
        let mut bytes = Vec::new();
        std::fs::File::open(receipt)
            .ok()?
            .take(8192)
            .read_to_end(&mut bytes)
            .ok()?;
        let proof: Receipt = serde_json::from_slice(&bytes).ok()?;
        (proof.ended && &proof.identity == identity).then_some(proof)
    }

    pub(super) async fn probe(identity: &ContainmentIdentity) -> ContainmentStatus {
        let ContainmentIdentity::LinuxGuard {
            boot_id: previous_boot,
            ..
        } = identity
        else {
            return ContainmentStatus::Unproven;
        };
        if let Ok(current) = boot_id() {
            if !previous_boot.is_empty() && &current != previous_boot {
                return ContainmentStatus::Ended;
            }
        }
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

    pub(super) fn guard_main() -> std::io::Result<()> {
        let fd: i32 = std::env::args()
            .nth(1)
            .ok_or_else(|| std::io::Error::other("missing control descriptor"))?
            .parse()
            .map_err(std::io::Error::other)?;
        let mut control = unsafe { UnixStream::from_raw_fd(fd) };
        if unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) } < 0
            || unsafe { libc::prctl(libc::PR_SET_CHILD_SUBREAPER, 1, 0, 0, 0) } < 0
        {
            return Err(std::io::Error::last_os_error());
        }
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
        let identity = ContainmentIdentity::LinuxGuard {
            pid: std::process::id(),
            start_ticks: start_ticks()?,
            boot_id: boot_id()?,
            receipt: launch.receipt.clone(),
            nonce: launch.nonce,
        };
        let mut ready = serde_json::to_vec(&identity).map_err(std::io::Error::other)?;
        ready.push(b'\n');
        // No detector exists before both this handshake and the durable gate.
        if control.write_all(&ready).is_err() {
            return write_receipt(identity, None, None);
        }
        let mut gate = [0];
        if control.read(&mut gate).unwrap_or(0) != 1 || gate[0] != b'S' {
            return write_receipt(identity, None, None);
        }
        control.set_nonblocking(true)?;
        let mut command = std::process::Command::new(&launch.command[0]);
        command.args(&launch.command[1..]).current_dir(launch.cwd);
        let guardian_pid = std::process::id();
        unsafe {
            command.pre_exec(move || {
                if libc::setpgid(0, 0) < 0 || libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL) < 0
                {
                    return Err(std::io::Error::last_os_error());
                }
                if libc::getppid() != guardian_pid as i32 {
                    return Err(std::io::Error::other("guardian ended before exec"));
                }
                Ok(())
            });
        }
        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(error) => return write_receipt(identity, None, Some(error.to_string())),
        };
        let mut code = None;
        loop {
            match control.read(&mut gate) {
                Ok(_) => break,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(_) => break,
            }
            match child.try_wait() {
                Ok(Some(status)) => {
                    code = status.code();
                    break;
                }
                Ok(None) => {}
                Err(_) => break,
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        // Descendants that change sessions/groups are still adopted by the
        // subreaper. Kill direct children, reap, repeat until ECHILD. Never
        // infer completion from the original leader's PID disappearing.
        cleanup_children()?;
        write_receipt(identity, code, None)
    }

    fn cleanup_children() -> std::io::Result<()> {
        let children_file = format!("/proc/self/task/{}/children", std::process::id());
        let deadline = std::time::Instant::now() + CLEANUP_TIMEOUT;
        loop {
            let children = std::fs::read_to_string(&children_file)?;
            for pid in children
                .split_whitespace()
                .filter_map(|pid| pid.parse::<i32>().ok())
            {
                // These are exclusively our own unreaped children: their PIDs
                // cannot be reused by unrelated processes before waitpid.
                unsafe {
                    libc::kill(pid, libc::SIGKILL);
                }
            }
            loop {
                let mut status = 0;
                let reaped = unsafe { libc::waitpid(-1, &mut status, libc::WNOHANG) };
                if reaped > 0 {
                    continue;
                }
                if reaped < 0 {
                    let error = std::io::Error::last_os_error();
                    if error.raw_os_error() == Some(libc::ECHILD) {
                        return Ok(());
                    }
                    if error.kind() == std::io::ErrorKind::Interrupted {
                        continue;
                    }
                    return Err(error);
                }
                break;
            }
            if std::time::Instant::now() >= deadline {
                // Continue owning the descendants instead of exiting and
                // abandoning them. No receipt means restart remains blocked.
                std::thread::sleep(Duration::from_millis(100));
            } else {
                std::thread::sleep(Duration::from_millis(5));
            }
        }
    }

    fn write_receipt(
        identity: ContainmentIdentity,
        exit_code: Option<i32>,
        spawn_error: Option<String>,
    ) -> std::io::Result<()> {
        let ContainmentIdentity::LinuxGuard { receipt, .. } = &identity else {
            unreachable!()
        };
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(receipt)?;
        let proof = Receipt {
            identity: identity.clone(),
            ended: true,
            exit_code,
            spawn_error,
        };
        file.write_all(&serde_json::to_vec(&proof).map_err(std::io::Error::other)?)?;
        file.sync_all()?;
        if let Some(parent) = receipt.parent() {
            std::fs::File::open(parent)?.sync_all()?;
        }
        Ok(())
    }
}

/// Entrypoint for the private helper binary; not a Telex client.
#[cfg(target_os = "linux")]
pub fn guard_main() -> std::io::Result<()> {
    platform::guard_main()
}

#[cfg(not(target_os = "linux"))]
pub fn guard_main() -> std::io::Result<()> {
    Err(std::io::Error::other("the process guardian is Linux-only"))
}

#[cfg(all(not(target_os = "linux"), not(windows)))]
mod platform {
    use super::*;
    pub(super) struct Prepared;
    pub(super) async fn prepare(
        _: &ProcessSpec,
        _: &Path,
        _: &Environment,
    ) -> Result<Prepared, ProcessFailure> {
        Err(ProcessFailure::new(
            ProcessFailureKind::UnsupportedPlatform,
            "verified containment requires Windows jobs or a Linux subreaper",
        ))
    }
    impl Prepared {
        pub(super) fn identity(&self) -> &ContainmentIdentity {
            unreachable!()
        }
        pub(super) async fn start(&mut self, _: Vec<u8>) -> Result<(), ProcessFailure> {
            unreachable!()
        }
        pub(super) async fn complete(&mut self) -> Result<Completion, ProcessFailure> {
            unreachable!()
        }
        pub(super) async fn cleanup(&mut self) -> ContainmentStatus {
            ContainmentStatus::Unproven
        }
    }
    pub(super) async fn probe(_: &ContainmentIdentity) -> ContainmentStatus {
        ContainmentStatus::Unproven
    }
}
