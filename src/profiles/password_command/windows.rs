//! Windows completion is checked private-job termination, zero active accounting,
//! exact leader signaling, closed owned I/O, checked finalization, and owner join
//! before publication. It does not promise every former descendant is signaled,
//! complete kernel/driver/external-I/O rundown, or a bound on those residuals.
//! Admission capacity bounds invocation owners, not residual kernel resources.

use super::Failure;
use std::io;
use std::mem::{size_of, zeroed};
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::os::windows::io::{AsRawHandle, FromRawHandle, IntoRawHandle, OwnedHandle};
use std::path::Path;
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicBool, Ordering};
use windows_sys::Win32::Foundation::{
    CloseHandle, GetHandleInformation, SetHandleInformation, ERROR_BROKEN_PIPE,
    ERROR_INSUFFICIENT_BUFFER, ERROR_INVALID_HANDLE, GENERIC_READ, HANDLE, HANDLE_FLAG_INHERIT,
    HANDLE_FLAG_PROTECT_FROM_CLOSE, INVALID_HANDLE_VALUE, WAIT_OBJECT_0, WAIT_TIMEOUT,
};
use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, GetFileAttributesW, ReadFile, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ,
    FILE_SHARE_WRITE, INVALID_FILE_ATTRIBUTES, OPEN_EXISTING,
};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectBasicAccountingInformation,
    JobObjectExtendedLimitInformation, QueryInformationJobObject, SetInformationJobObject,
    TerminateJobObject, JOBOBJECT_BASIC_ACCOUNTING_INFORMATION,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};
use windows_sys::Win32::System::Pipes::{CreatePipe, PeekNamedPipe};
use windows_sys::Win32::System::SystemInformation::{GetSystemDirectoryW, GetWindowsDirectoryW};
use windows_sys::Win32::System::Threading::{
    CreateProcessW, DeleteProcThreadAttributeList, GetExitCodeProcess,
    InitializeProcThreadAttributeList, ResumeThread, TerminateProcess, UpdateProcThreadAttribute,
    WaitForSingleObject, CREATE_SUSPENDED, EXTENDED_STARTUPINFO_PRESENT, PROCESS_INFORMATION,
    PROC_THREAD_ATTRIBUTE_HANDLE_LIST, STARTF_USESTDHANDLES, STARTUPINFOEXW,
};

const READ_QUANTUM: usize = 16 * 1024;

// The native owner transfers this entire value to the registry on failure. In
// particular, an error must not drop the job (kill-on-close is not a receipt).
pub(super) struct Process {
    job: Option<OwnedHandle>,
    process: Option<OwnedHandle>,
    thread: Option<OwnedHandle>,
    stdin: Option<OwnedHandle>,
    stdout: Option<OwnedHandle>,
    stderr: Option<OwnedHandle>,
    stdout_writer: Option<OwnedHandle>,
    stderr_writer: Option<OwnedHandle>,
    attempted: bool,
    assigned: bool,
    cleanup_started: bool,
    cleanup_error: Option<Failure>,
    cleanup_observed: bool,
    close_error: Option<Failure>,
    #[cfg(test)]
    fault: Option<SpawnFault>,
    #[cfg(test)]
    withheld_receipt: Option<ReceiptPart>,
}

impl Process {
    pub(super) fn new() -> Self {
        Self {
            job: None,
            process: None,
            thread: None,
            stdin: None,
            stdout: None,
            stderr: None,
            stdout_writer: None,
            stderr_writer: None,
            attempted: false,
            assigned: false,
            cleanup_started: false,
            cleanup_error: None,
            cleanup_observed: false,
            close_error: None,
            #[cfg(test)]
            fault: None,
            #[cfg(test)]
            withheld_receipt: None,
        }
    }

    pub(super) fn spawn(&mut self, command: &str, cancelled: &AtomicBool) -> Result<(), Failure> {
        if self.attempted || self.cleanup_started {
            return Err(Failure::at("spawn state"));
        }
        self.attempted = true;
        check_cancelled(cancelled)?;
        let mut command_line = command_line(command)?;
        let application = command_interpreter(cancelled)?;
        check_cancelled(cancelled)?;

        // Null security attributes make the unnamed, invocation-private job
        // noninheritable. Neither this handle nor the reader handles is passed on.
        self.job = Some(owned(
            unsafe { CreateJobObjectW(null(), null()) },
            "job creation",
        )?);
        check_cancelled(cancelled)?;
        let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { zeroed() };
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        checked(
            unsafe {
                SetInformationJobObject(
                    raw(&self.job),
                    JobObjectExtendedLimitInformation,
                    (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                    size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                )
            },
            "job limits",
        )?;
        check_cancelled(cancelled)?;

        let security = SECURITY_ATTRIBUTES {
            nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: null_mut(),
            bInheritHandle: 1,
        };
        pipe(
            &mut self.stdout,
            &mut self.stdout_writer,
            &security,
            cancelled,
        )?;
        #[cfg(test)]
        if self.fault == Some(SpawnFault::Setup) {
            return Err(Failure::at("injected pipe setup"));
        }
        pipe(
            &mut self.stderr,
            &mut self.stderr_writer,
            &security,
            cancelled,
        )?;
        self.stdin = Some(owned(
            unsafe {
                CreateFileW(
                    [b'N' as u16, b'U' as u16, b'L' as u16, 0].as_ptr(),
                    GENERIC_READ,
                    FILE_SHARE_READ | FILE_SHARE_WRITE,
                    &security,
                    OPEN_EXISTING,
                    FILE_ATTRIBUTE_NORMAL,
                    0,
                )
            },
            "stdin creation",
        )?);
        check_cancelled(cancelled)?;

        let handles = [
            raw(&self.stdin),
            raw(&self.stdout_writer),
            raw(&self.stderr_writer),
        ];
        let mut attributes = Attributes::new(cancelled)?;
        checked(
            unsafe {
                UpdateProcThreadAttribute(
                    attributes.pointer(),
                    0,
                    PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize,
                    handles.as_ptr().cast(),
                    size_of::<[HANDLE; 3]>(),
                    null_mut(),
                    null(),
                )
            },
            "stdio inheritance",
        )?;
        check_cancelled(cancelled)?;
        let mut startup: STARTUPINFOEXW = unsafe { zeroed() };
        startup.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as u32;
        startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
        startup.StartupInfo.hStdInput = handles[0];
        startup.StartupInfo.hStdOutput = handles[1];
        startup.StartupInfo.hStdError = handles[2];
        startup.lpAttributeList = attributes.pointer();
        let mut information: PROCESS_INFORMATION = unsafe { zeroed() };
        checked(
            unsafe {
                CreateProcessW(
                    application.as_ptr(),
                    command_line.as_mut_ptr(),
                    null(),
                    null(),
                    1,
                    CREATE_SUSPENDED | EXTENDED_STARTUPINFO_PRESENT,
                    null(),
                    null(),
                    &startup.StartupInfo,
                    &mut information,
                )
            },
            "process creation",
        )?;
        // CreateProcessW succeeded, so both returned handles are valid and must
        // become owned before any further fallible step or cancellation check.
        self.process = Some(unsafe { OwnedHandle::from_raw_handle(information.hProcess as _) });
        self.thread = Some(unsafe { OwnedHandle::from_raw_handle(information.hThread as _) });
        drop(attributes);
        self.close_child_stdio()?;
        check_cancelled(cancelled)?;

        let job = raw(&self.job);
        #[cfg(test)]
        let job = if self.fault == Some(SpawnFault::Assignment) {
            0
        } else {
            job
        };
        checked(
            unsafe { AssignProcessToJobObject(job, raw(&self.process)) },
            "job assignment",
        )?;
        self.assigned = true;
        check_cancelled(cancelled)?;
        #[cfg(test)]
        if self.fault == Some(SpawnFault::CancelBeforeResume) {
            cancelled.store(true, Ordering::Release);
        }
        check_cancelled(cancelled)?;
        let thread = raw(&self.thread);
        #[cfg(test)]
        let thread = if self.fault == Some(SpawnFault::Resume) {
            0
        } else {
            thread
        };
        let previous_count = unsafe { ResumeThread(thread) };
        if previous_count == u32::MAX {
            return Err(last_error("process resume"));
        }
        if previous_count != 1 {
            return Err(Failure::at("process suspension state"));
        }
        check_cancelled(cancelled)?;
        Ok(())
    }

    pub(super) fn read_available(&mut self, stdout: &mut Vec<u8>) -> Result<(), Failure> {
        // One bounded quantum per stream: a continuously writable stdout cannot
        // starve stderr drainage or the owner's next cancellation/deadline check.
        read_pipe(
            &mut self.stdout,
            Some(stdout),
            &mut self.close_error,
            "stdout handle closure",
        )?;
        read_pipe(
            &mut self.stderr,
            None,
            &mut self.close_error,
            "stderr handle closure",
        )
    }

    pub(super) fn output_closed(&self) -> bool {
        self.stdout.is_none() && self.stderr.is_none()
    }

    pub(super) fn exit_status(&mut self) -> Result<Option<i32>, Failure> {
        if self.process.is_none() || !self.process_terminated()? {
            return Ok(None);
        }
        let mut code = 0;
        checked(
            unsafe { GetExitCodeProcess(raw(&self.process), &mut code) },
            "process exit status",
        )?;
        Ok(Some(code as i32))
    }

    pub(super) fn begin_cleanup(&mut self) -> Result<(), Failure> {
        if self.cleanup_started {
            return self.cleanup_error.clone().map_or(Ok(()), Err);
        }
        self.cleanup_started = true;
        let mut error = self.close_io().err();
        if self.job.is_some() && unsafe { TerminateJobObject(raw(&self.job), 1) } == 0 {
            error.get_or_insert_with(|| last_error("job termination"));
        }
        // Assignment failure leaves a suspended child outside our job. Only its
        // exact retained handle authorizes this separate termination request.
        if self.process.is_some() && !self.assigned {
            match self.process_terminated() {
                Ok(true) => {}
                Ok(false) => {
                    if unsafe { TerminateProcess(raw(&self.process), 1) } == 0 {
                        let failure = last_error("unassigned process termination");
                        if !matches!(self.process_terminated(), Ok(true)) {
                            error.get_or_insert(failure);
                        }
                    }
                }
                Err(failure) => {
                    error.get_or_insert(failure);
                }
            }
        }
        self.cleanup_error = error;
        self.cleanup_error.clone().map_or(Ok(()), Err)
    }

    pub(super) fn poll_cleanup(&mut self) -> Result<bool, Failure> {
        if !self.cleanup_started {
            return Err(Failure::at("cleanup state"));
        }
        if let Some(error) = &self.cleanup_error {
            return Err(error.clone());
        }
        if let Some(error) = &self.close_error {
            return Err(error.clone());
        }
        let terminated = self.process_terminated()?;
        let empty = self.job_empty()?;
        let io_closed = self.output_closed()
            && self.stdin.is_none()
            && self.stdout_writer.is_none()
            && self.stderr_writer.is_none();
        #[cfg(test)]
        let (terminated, empty, io_closed) = (
            terminated && self.withheld_receipt != Some(ReceiptPart::Process),
            empty && self.withheld_receipt != Some(ReceiptPart::Job),
            io_closed && self.withheld_receipt != Some(ReceiptPart::Io),
        );
        // Do not close job/process/thread handles here: the parent still owes
        // native worker completion and join before releasing the owned record.
        let complete = terminated && empty && io_closed;
        self.cleanup_observed |= complete;
        Ok(complete)
    }

    // Run in the native owner after cleanup receipt, without a registry lock.
    // Publication still requires owner completion/join after this checked close.
    pub(super) fn finalize(&mut self) -> Result<(), Failure> {
        self.finalize_with(close_handle)
    }

    fn finalize_with(
        &mut self,
        mut close: impl FnMut(HANDLE) -> io::Result<()>,
    ) -> Result<(), Failure> {
        if let Some(error) = self.close_error.as_ref().or(self.cleanup_error.as_ref()) {
            return Err(error.clone());
        }
        if !self.cleanup_observed {
            let error = Failure::at("finalization before cleanup receipt");
            self.close_error = Some(error.clone());
            return Err(error);
        }
        for (handle, stage) in [
            (&mut self.thread, "primary thread handle finalization"),
            (&mut self.process, "process handle finalization"),
            (&mut self.job, "job handle finalization"),
        ] {
            if let Err(error) = close_owned_with(handle, stage, &mut close) {
                self.close_error = Some(error.clone());
                return Err(error);
            }
        }
        Ok(())
    }

    fn close_io(&mut self) -> Result<(), Failure> {
        close_recorded(
            &mut self.stdout,
            &mut self.close_error,
            "stdout handle closure",
        )?;
        close_recorded(
            &mut self.stderr,
            &mut self.close_error,
            "stderr handle closure",
        )?;
        self.close_child_stdio()
    }

    fn close_child_stdio(&mut self) -> Result<(), Failure> {
        close_recorded(
            &mut self.stdin,
            &mut self.close_error,
            "stdin handle closure",
        )?;
        close_recorded(
            &mut self.stdout_writer,
            &mut self.close_error,
            "stdout writer handle closure",
        )?;
        close_recorded(
            &mut self.stderr_writer,
            &mut self.close_error,
            "stderr writer handle closure",
        )
    }

    fn process_terminated(&self) -> Result<bool, Failure> {
        if self.process.is_none() {
            return Ok(true);
        }
        match unsafe { WaitForSingleObject(raw(&self.process), 0) } {
            WAIT_OBJECT_0 => Ok(true),
            WAIT_TIMEOUT => Ok(false),
            _ => Err(last_error("process wait")),
        }
    }

    fn job_empty(&self) -> Result<bool, Failure> {
        if self.job.is_none() {
            return Ok(true);
        }
        let mut accounting: JOBOBJECT_BASIC_ACCOUNTING_INFORMATION = unsafe { zeroed() };
        checked(
            unsafe {
                QueryInformationJobObject(
                    raw(&self.job),
                    JobObjectBasicAccountingInformation,
                    (&mut accounting as *mut JOBOBJECT_BASIC_ACCOUNTING_INFORMATION).cast(),
                    size_of::<JOBOBJECT_BASIC_ACCOUNTING_INFORMATION>() as u32,
                    null_mut(),
                )
            },
            "job completion observation",
        )?;
        Ok(accounting.ActiveProcesses == 0)
    }
}

fn check_cancelled(cancelled: &AtomicBool) -> Result<(), Failure> {
    if cancelled.load(Ordering::Acquire) {
        Err(Failure::at("cancelled"))
    } else {
        Ok(())
    }
}

fn raw(handle: &Option<OwnedHandle>) -> HANDLE {
    handle.as_ref().map_or(0, |h| h.as_raw_handle() as HANDLE)
}

fn owned(handle: HANDLE, stage: &'static str) -> Result<OwnedHandle, Failure> {
    if handle == 0 || handle == INVALID_HANDLE_VALUE {
        Err(last_error(stage))
    } else {
        Ok(unsafe { OwnedHandle::from_raw_handle(handle as _) })
    }
}

fn checked(result: i32, stage: &'static str) -> Result<(), Failure> {
    if result == 0 {
        Err(last_error(stage))
    } else {
        Ok(())
    }
}

fn last_error(stage: &'static str) -> Failure {
    Failure::io(stage, io::Error::last_os_error())
}

fn close_handle(handle: HANDLE) -> io::Result<()> {
    if unsafe { CloseHandle(handle) } == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

fn close_owned_with(
    slot: &mut Option<OwnedHandle>,
    stage: &'static str,
    close: &mut impl FnMut(HANDLE) -> io::Result<()>,
) -> Result<(), Failure> {
    let Some(handle) = slot.take() else {
        return Ok(());
    };
    let value = handle.as_raw_handle() as HANDLE;
    let mut flags = 0;
    if unsafe { GetHandleInformation(value, &mut flags) } == 0 {
        let error = last_error(stage);
        // Ownership is lost or unverified: do not let Drop retry this value.
        let _ = handle.into_raw_handle();
        return Err(error);
    }
    match close(value) {
        Ok(()) => {
            let _ = handle.into_raw_handle();
            Ok(())
        }
        Err(error) => {
            // A protected handle remains valid even if CloseHandle reports
            // ERROR_INVALID_HANDLE. Otherwise that error loses close authority.
            if error.raw_os_error() == Some(ERROR_INVALID_HANDLE as i32)
                && flags & HANDLE_FLAG_PROTECT_FROM_CLOSE == 0
            {
                let _ = handle.into_raw_handle();
            } else {
                *slot = Some(handle);
            }
            Err(Failure::io(stage, error))
        }
    }
}

fn close_recorded(
    slot: &mut Option<OwnedHandle>,
    failure: &mut Option<Failure>,
    stage: &'static str,
) -> Result<(), Failure> {
    if let Some(error) = failure {
        return Err(error.clone());
    }
    let result = close_owned_with(slot, stage, &mut close_handle);
    if let Err(error) = &result {
        *failure = Some(error.clone());
    }
    result
}

fn pipe(
    reader: &mut Option<OwnedHandle>,
    writer: &mut Option<OwnedHandle>,
    security: &SECURITY_ATTRIBUTES,
    cancelled: &AtomicBool,
) -> Result<(), Failure> {
    let mut read = 0;
    let mut write = 0;
    checked(
        unsafe { CreatePipe(&mut read, &mut write, security, 0) },
        "pipe creation",
    )?;
    *reader = Some(unsafe { OwnedHandle::from_raw_handle(read as _) });
    *writer = Some(unsafe { OwnedHandle::from_raw_handle(write as _) });
    check_cancelled(cancelled)?;
    checked(
        unsafe { SetHandleInformation(read, HANDLE_FLAG_INHERIT, 0) },
        "pipe inheritance",
    )?;
    check_cancelled(cancelled)
}

fn read_pipe(
    pipe: &mut Option<OwnedHandle>,
    output: Option<&mut Vec<u8>>,
    close_error: &mut Option<Failure>,
    close_stage: &'static str,
) -> Result<(), Failure> {
    if let Some(error) = close_error {
        return Err(error.clone());
    }
    if pipe.is_none() {
        return Ok(());
    }
    let mut available = 0;
    if unsafe {
        PeekNamedPipe(
            raw(pipe),
            null_mut(),
            0,
            null_mut(),
            &mut available,
            null_mut(),
        )
    } == 0
    {
        let error = io::Error::last_os_error();
        if error.raw_os_error() == Some(ERROR_BROKEN_PIPE as i32) {
            return close_recorded(pipe, close_error, close_stage);
        }
        return Err(Failure::io("pipe availability", error));
    }
    if available == 0 {
        return Ok(());
    }
    let mut buffer = [0_u8; READ_QUANTUM];
    let mut count = 0;
    // This owner is the sole reader, so bytes reported available cannot be
    // consumed by another reader before this bounded synchronous ReadFile.
    checked(
        unsafe {
            ReadFile(
                raw(pipe),
                buffer.as_mut_ptr(),
                available.min(READ_QUANTUM as u32),
                &mut count,
                null_mut(),
            )
        },
        "pipe read",
    )?;
    if let Some(output) = output {
        output.extend_from_slice(&buffer[..count as usize]);
    }
    Ok(())
}

struct Attributes {
    storage: Vec<usize>,
    initialized: bool,
}

impl Attributes {
    fn new(cancelled: &AtomicBool) -> Result<Self, Failure> {
        let mut bytes = 0;
        let result = unsafe { InitializeProcThreadAttributeList(null_mut(), 1, 0, &mut bytes) };
        let error = io::Error::last_os_error();
        if result != 0
            || error.raw_os_error() != Some(ERROR_INSUFFICIENT_BUFFER as i32)
            || bytes == 0
        {
            return Err(Failure::io("inheritance allocation size", error));
        }
        check_cancelled(cancelled)?;
        let mut attributes = Self {
            storage: vec![0; bytes.div_ceil(size_of::<usize>())],
            initialized: false,
        };
        checked(
            unsafe { InitializeProcThreadAttributeList(attributes.pointer(), 1, 0, &mut bytes) },
            "inheritance initialization",
        )?;
        attributes.initialized = true;
        check_cancelled(cancelled)?;
        Ok(attributes)
    }

    fn pointer(&mut self) -> windows_sys::Win32::System::Threading::LPPROC_THREAD_ATTRIBUTE_LIST {
        self.storage.as_mut_ptr().cast()
    }
}

impl Drop for Attributes {
    fn drop(&mut self) {
        if self.initialized {
            unsafe { DeleteProcThreadAttributeList(self.pointer()) };
        }
    }
}

fn command_interpreter(cancelled: &AtomicBool) -> Result<Vec<u16>, Failure> {
    // Match std Command's search for a bare "cmd", with unchanged environment:
    // application directory, system directory, Windows directory, then PATH.
    // COMSPEC and the working directory are not extra search locations.
    if let Ok(executable) = std::env::current_exe() {
        if let Some(directory) = executable.parent() {
            if let Some(path) = interpreter_at(directory, cancelled)? {
                return Ok(path);
            }
        }
    }
    for get_directory in [GetSystemDirectoryW, GetWindowsDirectoryW] {
        check_cancelled(cancelled)?;
        let mut directory = vec![0_u16; 32768];
        let length = unsafe { get_directory(directory.as_mut_ptr(), directory.len() as u32) };
        check_cancelled(cancelled)?;
        if length != 0 && (length as usize) < directory.len() {
            let directory = std::ffi::OsString::from_wide(&directory[..length as usize]);
            if let Some(path) = interpreter_at(Path::new(&directory), cancelled)? {
                return Ok(path);
            }
        }
    }
    if let Some(paths) = std::env::var_os("PATH") {
        for directory in std::env::split_paths(&paths).filter(|path| !path.as_os_str().is_empty()) {
            if let Some(path) = interpreter_at(&directory, cancelled)? {
                return Ok(path);
            }
        }
    }
    Err(Failure::at("command interpreter not found"))
}

fn interpreter_at(directory: &Path, cancelled: &AtomicBool) -> Result<Option<Vec<u16>>, Failure> {
    check_cancelled(cancelled)?;
    let path = directory.join("cmd.exe");
    let mut wide: Vec<_> = path.as_os_str().encode_wide().collect();
    if wide.contains(&0) {
        return Ok(None);
    }
    wide.push(0);
    let exists = unsafe { GetFileAttributesW(wide.as_ptr()) } != INVALID_FILE_ATTRIBUTES;
    check_cancelled(cancelled)?;
    Ok(exists.then_some(wide))
}

fn command_line(command: &str) -> Result<Vec<u16>, Failure> {
    if command.contains('\0') {
        return Err(Failure::at("command argument"));
    }
    // Match Command::new("cmd").arg("/C").arg(command), not raw_arg:
    // ordinary Windows argument quoting escapes embedded quotes and doubles
    // the preceding/trailing backslashes only where quoting requires it.
    let mut line: Vec<u16> = "\"cmd\" /C ".encode_utf16().collect();
    let quote = command.is_empty() || command.contains([' ', '\t']);
    if quote {
        line.push(b'"' as u16);
    }
    let mut backslashes = 0;
    for character in command.encode_utf16() {
        if character == b'\\' as u16 {
            backslashes += 1;
        } else {
            if character == b'"' as u16 {
                line.extend(std::iter::repeat_n(b'\\' as u16, backslashes + 1));
            }
            backslashes = 0;
        }
        line.push(character);
    }
    if quote {
        line.extend(std::iter::repeat_n(b'\\' as u16, backslashes));
        line.push(b'"' as u16);
    }
    line.push(0);
    Ok(line)
}

#[cfg(test)]
#[derive(Clone, Copy, PartialEq, Eq)]
enum SpawnFault {
    Setup,
    Assignment,
    Resume,
    CancelBeforeResume,
}

#[cfg(test)]
#[derive(Clone, Copy, PartialEq, Eq)]
enum ReceiptPart {
    Process,
    Job,
    Io,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::io::{BufRead, Write};
    use std::ops::{Deref, DerefMut};
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
    use windows_sys::Win32::Foundation::FILETIME;
    use windows_sys::Win32::System::JobObjects::IsProcessInJob;
    use windows_sys::Win32::System::Threading::{
        CreateEventW, GetProcessTimes, OpenEventW, OpenProcess, SetEvent,
        PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE, SYNCHRONIZATION_SYNCHRONIZE,
    };

    const FIXTURE: &str = "profiles::password_command::windows::tests::fixture";
    const MODE: &str = "TELEX_M2_WINDOWS_UNIT_FIXTURE";
    const GATE: &str = "TELEX_M2_WINDOWS_UNIT_GATE";
    const CREATION_WINDOW_CHILDREN: usize = 8;

    struct Invocation {
        scope: Process,
        fixture_job: Option<OwnedHandle>,
        fixture_root: Option<OwnedHandle>,
        receipt_child: Option<OwnedHandle>,
        fixture_children: Vec<OwnedHandle>,
        helper_wait_at_receipt: Option<u32>,
        receipt_at: Option<Instant>,
        fixture_cleanup_observed: bool,
        fixture_observation_error: Cell<Option<&'static str>>,
    }

    impl Invocation {
        fn new() -> Self {
            Self {
                scope: Process::new(),
                fixture_job: None,
                fixture_root: None,
                receipt_child: None,
                fixture_children: Vec::new(),
                helper_wait_at_receipt: None,
                receipt_at: None,
                fixture_cleanup_observed: false,
                fixture_observation_error: Cell::new(None),
            }
        }

        fn spawn(command: &str) -> Self {
            let mut invocation = Self::new();
            invocation.spawn(command, &AtomicBool::new(false)).unwrap();
            invocation.fixture_job = Some(invocation.job.as_ref().unwrap().try_clone().unwrap());
            invocation.fixture_root =
                Some(invocation.process.as_ref().unwrap().try_clone().unwrap());
            invocation
        }

        fn collect(&mut self) -> (i32, Vec<u8>) {
            let deadline = Instant::now() + Duration::from_secs(10);
            let mut output = Vec::new();
            loop {
                self.read_available(&mut output).unwrap();
                if let Some(status) = self.exit_status().unwrap() {
                    if self.output_closed() {
                        return (status, output);
                    }
                }
                assert!(Instant::now() < deadline, "output deadline");
                std::thread::sleep(Duration::from_millis(1));
            }
        }

        fn clean(&mut self) {
            self.observe_cleanup();
            self.finalize().unwrap();
            self.finish_fixture_cleanup();
        }

        fn observe_cleanup(&mut self) {
            self.begin_cleanup().unwrap();
            let deadline = Instant::now() + Duration::from_secs(3);
            while !self.poll_cleanup().unwrap() {
                assert!(Instant::now() < deadline, "cleanup deadline");
                std::thread::sleep(Duration::from_millis(1));
            }
            self.receipt_at = Some(Instant::now());
            if let Some(helper) = &self.receipt_child {
                let observed = unsafe { WaitForSingleObject(helper.as_raw_handle() as HANDLE, 0) };
                assert!(matches!(observed, WAIT_OBJECT_0 | WAIT_TIMEOUT));
                self.helper_wait_at_receipt = Some(observed);
            }
            assert!(self.process_terminated().unwrap());
            assert!(self.job_empty().unwrap());
            assert!(self.output_closed());
        }

        fn fixture_finished(&self) -> bool {
            let mut finished = true;
            if let Some(job) = self.fixture_job.as_ref().or(self.scope.job.as_ref()) {
                let mut accounting: JOBOBJECT_BASIC_ACCOUNTING_INFORMATION = unsafe { zeroed() };
                let result = unsafe {
                    QueryInformationJobObject(
                        job.as_raw_handle() as HANDLE,
                        JobObjectBasicAccountingInformation,
                        (&mut accounting as *mut JOBOBJECT_BASIC_ACCOUNTING_INFORMATION).cast(),
                        size_of::<JOBOBJECT_BASIC_ACCOUNTING_INFORMATION>() as u32,
                        null_mut(),
                    )
                };
                if result == 0 {
                    self.fixture_observation_error
                        .set(Some("fixture accounting query failed"));
                    return false;
                }
                if accounting.ActiveProcesses != 0 {
                    if self.receipt_at.is_some() {
                        self.fixture_observation_error
                            .set(Some("active job scope after receipt"));
                    }
                    finished = false;
                }
            }
            for handle in [
                self.fixture_root.as_ref().or(self.scope.process.as_ref()),
                self.receipt_child.as_ref(),
            ]
            .into_iter()
            .flatten()
            .chain(self.fixture_children.iter())
            {
                let result = unsafe { WaitForSingleObject(handle.as_raw_handle() as HANDLE, 0) };
                if !matches!(result, WAIT_OBJECT_0 | WAIT_TIMEOUT) {
                    self.fixture_observation_error
                        .set(Some("fixture process wait failed"));
                }
                finished &= result == WAIT_OBJECT_0;
            }
            finished
        }

        fn finish_fixture_cleanup(&mut self) {
            let deadline = Instant::now() + Duration::from_secs(3);
            while !self.fixture_finished() {
                assert!(
                    Instant::now() < deadline,
                    "independent fixture cleanup bound"
                );
                std::thread::yield_now();
            }
            self.fixture_cleanup_observed = true;
            assert_eq!(self.fixture_observation_error.get(), None);
            if let Some(observed) = self.helper_wait_at_receipt {
                eprintln!(
                    "job-terminal receipt observed; independent helper wait={observed}; \
                     later fixture signal observed after {:?}; timeout at receipt is an allowed limit",
                    self.receipt_at.unwrap().elapsed()
                );
            }
        }
    }

    impl Deref for Invocation {
        type Target = Process;
        fn deref(&self) -> &Process {
            &self.scope
        }
    }

    impl DerefMut for Invocation {
        fn deref_mut(&mut self) -> &mut Process {
            &mut self.scope
        }
    }

    impl Drop for Invocation {
        fn drop(&mut self) {
            if self.fixture_cleanup_observed {
                return;
            }
            // A panic retains independent exact handles even after invocation
            // finalization. A missing fixture receipt cannot release a live scope.
            unsafe {
                if let Some(job) = self.fixture_job.as_ref().or(self.scope.job.as_ref()) {
                    let result = TerminateJobObject(job.as_raw_handle() as HANDLE, 1);
                    if result == 0 {
                        eprintln!(
                            "fixture job termination failed: {:?}",
                            io::Error::last_os_error().raw_os_error()
                        );
                    }
                }
                if !self.scope.assigned {
                    if let Some(root) = self.fixture_root.as_ref().or(self.scope.process.as_ref()) {
                        let _ = TerminateProcess(root.as_raw_handle() as HANDLE, 1);
                    }
                }
            }
            let deadline = Instant::now() + Duration::from_secs(3);
            while !self.fixture_finished() {
                if Instant::now() >= deadline {
                    eprintln!("FAILED_HELD: current test owner retains unresolved fixture handles");
                    loop {
                        std::thread::park();
                    }
                }
                std::thread::yield_now();
            }
            self.fixture_cleanup_observed = true;
        }
    }

    fn fixture_command(mode: &str) -> String {
        let executable = std::env::current_exe().unwrap();
        fixture_command_for(mode, &executable)
    }

    fn fixture_command_for(mode: &str, executable: &Path) -> String {
        // cmd receives exactly the same .arg serialization as the old path.
        // The fixture path in this checkout has no shell-special characters.
        let executable = executable.to_str().unwrap();
        assert!(!executable.contains([' ', '&', '(', ')', '"']));
        format!(
            "set {MODE}={mode}&& {executable} --exact {FIXTURE} --ignored --nocapture --test-threads=1"
        )
    }

    fn read_until(invocation: &mut Invocation, marker: &[u8]) -> Vec<u8> {
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut output = Vec::new();
        loop {
            invocation.read_available(&mut output).unwrap();
            if output.windows(marker.len()).any(|part| part == marker) {
                return output;
            }
            assert!(Instant::now() < deadline, "readiness deadline");
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    fn retain_ready_helper(invocation: &mut Invocation, output: &[u8]) -> u32 {
        let output = std::str::from_utf8(output).unwrap();
        let pid = output
            .split("PARENT-READY ")
            .nth(1)
            .unwrap()
            .split_whitespace()
            .next()
            .unwrap()
            .parse()
            .unwrap();
        invocation.receipt_child = Some(retain_live_fixture_member(invocation, pid));
        pid
    }

    fn retain_live_fixture_member(invocation: &Invocation, pid: u32) -> OwnedHandle {
        let child = owned(
            unsafe {
                OpenProcess(
                    PROCESS_SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION,
                    0,
                    pid,
                )
            },
            "test descendant observation",
        )
        .unwrap();
        let mut member = 0;
        assert_ne!(
            unsafe {
                IsProcessInJob(
                    child.as_raw_handle() as HANDLE,
                    raw(&invocation.job),
                    &mut member,
                )
            },
            0
        );
        assert_ne!(member, 0, "ready helper must belong to this invocation");
        assert_eq!(
            unsafe { WaitForSingleObject(child.as_raw_handle() as HANDLE, 0) },
            WAIT_TIMEOUT,
            "retain a live helper before cleanup"
        );
        child
    }

    fn process_times(handle: HANDLE) -> (u64, u64) {
        let [mut created, mut exited, mut kernel, mut user]: [FILETIME; 4] = unsafe { zeroed() };
        assert_ne!(
            unsafe { GetProcessTimes(handle, &mut created, &mut exited, &mut kernel, &mut user) },
            0
        );
        let ticks =
            |time: FILETIME| (u64::from(time.dwHighDateTime) << 32) | u64::from(time.dwLowDateTime);
        (ticks(created), ticks(exited))
    }

    fn finalization_fixture() -> Process {
        let mut process = Process::new();
        // Private events exercise CloseHandle ownership only, without processes
        // or job operations. The receipt state is a pure finalization test seam.
        for slot in [&mut process.thread, &mut process.process, &mut process.job] {
            *slot = Some(
                owned(
                    unsafe { CreateEventW(null(), 1, 0, null()) },
                    "test finalization event",
                )
                .unwrap(),
            );
        }
        process.cleanup_started = true;
        process.cleanup_observed = true;
        process
    }

    struct ProtectedFixture(Process);

    impl Drop for ProtectedFixture {
        fn drop(&mut self) {
            for handle in [&self.0.process, &self.0.stdout].into_iter().flatten() {
                unsafe {
                    SetHandleInformation(
                        handle.as_raw_handle() as HANDLE,
                        HANDLE_FLAG_PROTECT_FROM_CLOSE,
                        0,
                    );
                }
            }
        }
    }

    #[test]
    fn finalize_in_native_owner_before_join_checks_real_owned_handle_release() {
        let native_owner = std::thread::spawn(|| {
            let mut invocation = Invocation::spawn("exit /b 0");
            let (status, _) = invocation.collect();
            assert_eq!(status, 0);
            invocation.observe_cleanup();
            assert!(invocation.thread.is_some());
            assert!(invocation.process.is_some());
            assert!(invocation.job.is_some());
            invocation.finalize().unwrap();
            assert!(invocation.thread.is_none());
            assert!(invocation.process.is_none());
            assert!(invocation.job.is_none());
            invocation.finish_fixture_cleanup();
            invocation
        });
        let invocation = native_owner.join().unwrap();
        assert!(invocation.thread.is_none());
        assert!(invocation.process.is_none());
        assert!(invocation.job.is_none());
        assert!(invocation.close_error.is_none());
    }

    #[test]
    fn finalize_requires_observed_cleanup_and_keeps_failure_sticky() {
        let mut process = finalization_fixture();
        process.cleanup_observed = false;
        let error = process
            .finalize_with(|_| panic!("no close before receipt"))
            .unwrap_err();
        assert_eq!(error.stage, "finalization before cleanup receipt");
        process.cleanup_observed = true;
        assert_eq!(
            process
                .finalize_with(|_| panic!("no close after failed finalization"))
                .unwrap_err(),
            error
        );
        assert!(process.thread.is_some());
        assert!(process.process.is_some());
        assert!(process.job.is_some());
    }

    #[test]
    fn finalize_success_is_idempotent_without_reclosing_values() {
        let mut process = finalization_fixture();
        let mut calls = 0;
        process
            .finalize_with(|handle| {
                calls += 1;
                close_handle(handle)
            })
            .unwrap();
        assert_eq!(calls, 3);
        assert!(process.thread.is_none());
        assert!(process.process.is_none());
        assert!(process.job.is_none());
        process
            .finalize_with(|_| panic!("closed values must not be retried"))
            .unwrap();
    }

    #[test]
    fn finalize_partial_failure_retains_only_unclosed_valid_handles() {
        let mut process = finalization_fixture();
        let mut calls = 0;
        let error = process
            .finalize_with(|handle| {
                calls += 1;
                if calls == 2 {
                    Err(io::Error::from_raw_os_error(5))
                } else {
                    close_handle(handle)
                }
            })
            .unwrap_err();
        assert_eq!(calls, 2);
        assert_eq!(error.stage, "process handle finalization");
        assert_eq!(error.os_code, Some(5));
        assert!(process.thread.is_none());
        for slot in [&process.process, &process.job] {
            let mut flags = 0;
            assert_ne!(unsafe { GetHandleInformation(raw(slot), &mut flags) }, 0);
        }
        assert_eq!(
            process
                .finalize_with(|_| panic!("failed finalization must not retry"))
                .unwrap_err(),
            error
        );
        assert_eq!(process.poll_cleanup().unwrap_err(), error);
    }

    #[test]
    fn finalize_lost_ownership_never_reconstructs_or_retries_closed_handles() {
        let mut process = finalization_fixture();
        let mut calls = 0;
        let error = process
            .finalize_with(|handle| {
                calls += 1;
                close_handle(handle)?;
                if calls == 2 {
                    // Simulate ownership loss with one close of a real private
                    // event; never call a native API on the resulting stale value.
                    Err(io::Error::from_raw_os_error(ERROR_INVALID_HANDLE as i32))
                } else {
                    Ok(())
                }
            })
            .unwrap_err();
        assert_eq!(calls, 2);
        assert_eq!(error.os_code, Some(ERROR_INVALID_HANDLE as i32));
        assert!(process.thread.is_none());
        assert!(process.process.is_none());
        assert!(process.job.is_some());
        assert_eq!(
            process
                .finalize_with(|_| panic!("lost handle authority must not be restored"))
                .unwrap_err(),
            error
        );
        assert_eq!(process.poll_cleanup().unwrap_err(), error);
    }

    #[test]
    fn finalize_protected_private_handle_failure_retains_original_ownership() {
        let mut fixture = ProtectedFixture(finalization_fixture());
        assert_ne!(
            unsafe {
                SetHandleInformation(
                    raw(&fixture.0.process),
                    HANDLE_FLAG_PROTECT_FROM_CLOSE,
                    HANDLE_FLAG_PROTECT_FROM_CLOSE,
                )
            },
            0
        );
        let error = fixture.0.finalize().unwrap_err();
        assert_eq!(error.stage, "process handle finalization");
        assert!(error.os_code.is_some());
        assert!(fixture.0.thread.is_none());
        let mut flags = 0;
        assert_ne!(
            unsafe { GetHandleInformation(raw(&fixture.0.process), &mut flags) },
            0
        );
        assert_ne!(flags & HANDLE_FLAG_PROTECT_FROM_CLOSE, 0);
        assert!(fixture.0.job.is_some());
        assert_eq!(fixture.0.finalize().unwrap_err(), error);
    }

    #[test]
    fn finalize_cannot_turn_failed_io_closure_into_receipt() {
        let mut process = Process::new();
        process.stdout = Some(
            owned(
                unsafe { CreateEventW(null(), 1, 0, null()) },
                "test protected IO event",
            )
            .unwrap(),
        );
        let mut fixture = ProtectedFixture(process);
        assert_ne!(
            unsafe {
                SetHandleInformation(
                    raw(&fixture.0.stdout),
                    HANDLE_FLAG_PROTECT_FROM_CLOSE,
                    HANDLE_FLAG_PROTECT_FROM_CLOSE,
                )
            },
            0
        );
        let error = fixture.0.begin_cleanup().unwrap_err();
        assert_eq!(error.stage, "stdout handle closure");
        assert!(fixture.0.stdout.is_some());
        assert!(!fixture.0.cleanup_observed);
        assert_eq!(fixture.0.poll_cleanup().unwrap_err(), error);
        assert_eq!(fixture.0.finalize().unwrap_err(), error);
    }

    #[test]
    fn success_collects_full_output_and_drains_stderr() {
        let command = "(for /l %i in (1,1,16384) do @echo 0123456789abcdef) & (for /l %i in (1,1,16384) do @echo discarded) 1>&2";
        let mut invocation = Invocation::spawn(command);
        let (status, output) = invocation.collect();
        assert_eq!(status, 0);
        assert_eq!(output, b"0123456789abcdef\r\n".repeat(16384));
        invocation.clean();
    }

    #[test]
    fn command_parsing_environment_cwd_and_nonzero_match_previous_path() {
        for command in [
            "echo plain",
            "echo \"quoted value\"",
            "echo trailing\\",
            "echo \"backslash\\\"quote\"",
            "echo %SystemRoot% & cd",
            "echo redirected 1>&2 & echo stdout",
            "chcp 65001 >nul & echo caf\u{e9} \u{4e16}\u{754c}",
            "exit /b 7",
            "",
        ] {
            let baseline = Command::new("cmd").arg("/C").arg(command).output().unwrap();
            let mut invocation = Invocation::spawn(command);
            let (status, output) = invocation.collect();
            assert_eq!(status, baseline.status.code().unwrap());
            assert_eq!(output, baseline.stdout);
            invocation.clean();
        }
    }

    #[test]
    fn shell_selection_matches_application_directory_override() {
        struct FixtureDirectory(std::path::PathBuf);
        impl Drop for FixtureDirectory {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let directory = std::env::current_dir()
            .unwrap()
            .join("target")
            .join(format!(
                "windows-shell-selection-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
        std::fs::create_dir_all(&directory).unwrap();
        let directory = FixtureDirectory(directory);
        let executable = std::env::current_exe().unwrap();
        let host = directory.0.join("selection-host.exe");
        std::fs::hard_link(&executable, &host).unwrap();
        std::fs::hard_link(&executable, directory.0.join("cmd.exe")).unwrap();
        let mut invocation = Invocation::spawn(&fixture_command_for("selection-host", &host));
        let (status, _) = invocation.collect();
        assert_eq!(status, 0, "isolated shell selection parity");
        invocation.clean();
        drop(invocation);
        std::fs::remove_dir_all(&directory.0).unwrap();
    }

    #[test]
    fn empty_and_pre_cancelled_scopes_have_no_child_receipt() {
        let mut empty = Invocation::new();
        empty.clean();
        let mut cancelled = Invocation::new();
        let error = cancelled
            .spawn("echo must-not-run", &AtomicBool::new(true))
            .unwrap_err();
        assert_eq!(error.stage, "cancelled");
        assert!(cancelled.process.is_none());
        assert!(cancelled.job.is_none());
        cancelled.clean();
    }

    #[test]
    fn setup_assignment_resume_and_pre_resume_cancellation_retain_ownership() {
        for fault in [
            SpawnFault::Setup,
            SpawnFault::Assignment,
            SpawnFault::Resume,
            SpawnFault::CancelBeforeResume,
        ] {
            let mut invocation = Invocation::new();
            invocation.fault = Some(fault);
            let error = invocation
                .spawn("echo must-not-run", &AtomicBool::new(false))
                .unwrap_err();
            assert!(invocation.job.is_some());
            if fault != SpawnFault::Setup {
                assert!(invocation.process.is_some());
                assert!(invocation.thread.is_some());
                assert!(!invocation.process_terminated().unwrap());
                let mut output = Vec::new();
                invocation.read_available(&mut output).unwrap();
                assert!(output.is_empty(), "suspended child executed");
            }
            if fault == SpawnFault::Assignment || fault == SpawnFault::Resume {
                assert!(error.os_code.is_some(), "real API failure expected");
            }
            invocation.clean();
        }
    }

    #[test]
    fn receipt_requires_each_observed_component_and_retains_handles() {
        let mut invocation = Invocation::spawn("echo complete");
        invocation.collect();
        assert!(invocation.poll_cleanup().is_err());
        invocation.observe_cleanup();
        for withheld in [ReceiptPart::Process, ReceiptPart::Job, ReceiptPart::Io] {
            invocation.withheld_receipt = Some(withheld);
            assert!(!invocation.poll_cleanup().unwrap());
            assert!(invocation.process.is_some());
            assert!(invocation.job.is_some());
            assert!(invocation.thread.is_some());
        }
        invocation.withheld_receipt = None;
        assert!(invocation.poll_cleanup().unwrap());
        invocation.finalize().unwrap();
        invocation.finish_fixture_cleanup();
    }

    #[test]
    fn job_terminal_query_failure_cannot_use_zero_initialized_accounting() {
        let mut process = Process::new();
        // A valid private event is deliberately the wrong kernel object type.
        process.job = Some(
            owned(
                unsafe { CreateEventW(null(), 1, 0, null()) },
                "test accounting failure event",
            )
            .unwrap(),
        );
        process.cleanup_started = true;
        let error = process.poll_cleanup().unwrap_err();
        assert_eq!(error.stage, "job completion observation");
        assert!(error.os_code.is_some());
        assert!(!process.cleanup_observed);
        assert!(process.job.is_some());
        assert!(process.finalize().is_err());
    }

    #[test]
    fn job_terminal_termination_failure_cannot_publish_a_receipt() {
        let mut process = Process::new();
        process.job = Some(
            owned(
                unsafe { CreateEventW(null(), 1, 0, null()) },
                "test termination failure event",
            )
            .unwrap(),
        );
        let error = process.begin_cleanup().unwrap_err();
        assert_eq!(error.stage, "job termination");
        assert!(error.os_code.is_some());
        assert_eq!(process.poll_cleanup().unwrap_err(), error);
        assert_eq!(process.finalize().unwrap_err(), error);
        assert!(process.job.is_some());
    }

    #[test]
    fn inherited_writer_output_is_collected_after_shell_exit() {
        let name = format!(
            "Local\\telex-m2-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let wide_name: Vec<_> = name.encode_utf16().chain(Some(0)).collect();
        let gate = owned(
            unsafe { CreateEventW(null(), 1, 0, wide_name.as_ptr()) },
            "test output gate",
        )
        .unwrap();
        let mut invocation = Invocation::spawn(&format!(
            "set {GATE}={name}&& {}",
            fixture_command("inherited-parent")
        ));
        let mut output = read_until(&mut invocation, b"CHILD-READY");
        let deadline = Instant::now() + Duration::from_secs(5);
        while invocation.exit_status().unwrap().is_none() {
            assert!(Instant::now() < deadline, "shell exit deadline");
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(
            !invocation.job_empty().unwrap(),
            "delayed helper must be alive"
        );
        assert!(!invocation.output_closed());
        assert!(!output.windows(12).any(|part| part == b"DELAYED-FULL"));
        assert_ne!(unsafe { SetEvent(gate.as_raw_handle() as HANDLE) }, 0);
        let (status, tail) = invocation.collect();
        output.extend_from_slice(&tail);
        assert_eq!(status, 0);
        assert!(output.windows(12).any(|part| part == b"DELAYED-FULL"));
        invocation.clean();
    }

    #[test]
    fn normal_completion_cleans_redirected_live_helper() {
        let mut invocation = Invocation::spawn(&fixture_command("redirected-parent"));
        let (status, output) = invocation.collect();
        assert_eq!(status, 0);
        assert!(output.windows(12).any(|part| part == b"PARENT-READY"));
        assert!(invocation.process_terminated().unwrap());
        assert!(!invocation.job_empty().unwrap());
        assert!(invocation.output_closed());
        retain_ready_helper(&mut invocation, &output);
        invocation.clean();
    }

    #[test]
    fn cancellation_cleans_ready_shell_and_delayed_helper_with_job_receipt() {
        let mut invocation = Invocation::spawn(&fixture_command("cancel-parent"));
        let output = read_until(&mut invocation, b"END-READY");
        retain_ready_helper(&mut invocation, &output);
        assert!(!invocation.process_terminated().unwrap());
        assert!(!invocation.job_empty().unwrap());
        let cancelled_at = Instant::now();
        invocation.clean();
        assert!(cancelled_at.elapsed() < Duration::from_secs(3));
    }

    #[test]
    fn bounded_ordinary_creation_window_then_job_cancellation() {
        let name = format!(
            "Local\\telex-create-window-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let wide_name: Vec<_> = name.encode_utf16().chain(Some(0)).collect();
        let permit = owned(
            unsafe { CreateEventW(null(), 1, 0, wide_name.as_ptr()) },
            "test creation permit",
        )
        .unwrap();
        let mut invocation = Invocation::spawn(&format!(
            "set {GATE}={name}&& {}",
            fixture_command("creation-window-parent")
        ));
        let ready = read_until(&mut invocation, b"END-READY");
        let creator_id = retain_ready_helper(&mut invocation, &ready);
        assert!(!invocation.process_terminated().unwrap());

        let window_started = Instant::now();
        assert_ne!(unsafe { SetEvent(permit.as_raw_handle() as HANDLE) }, 0);
        let reports = read_until(&mut invocation, b"CREATION-WINDOW-END");
        let reports = std::str::from_utf8(&reports).unwrap();
        let mut identities = Vec::new();
        for record in reports
            .lines()
            .filter_map(|line| line.strip_prefix("CREATED "))
        {
            let mut fields = record.split_whitespace();
            let index: usize = fields.next().unwrap().parse().unwrap();
            let child_id: u32 = fields.next().unwrap().parse().unwrap();
            assert!(fields.next().is_none(), "unmatched creation report");
            assert_eq!(index, invocation.fixture_children.len());
            assert!(index < CREATION_WINDOW_CHILDREN);
            assert_ne!(child_id, creator_id);
            assert!(
                !identities.contains(&child_id),
                "duplicate fixture identity"
            );
            let handle = retain_live_fixture_member(&invocation, child_id);
            invocation.fixture_children.push(handle);
            identities.push(child_id);
        }
        assert_eq!(invocation.fixture_children.len(), CREATION_WINDOW_CHILDREN);
        let mut accounting: JOBOBJECT_BASIC_ACCOUNTING_INFORMATION = unsafe { zeroed() };
        assert_ne!(
            unsafe {
                QueryInformationJobObject(
                    raw(&invocation.fixture_job),
                    JobObjectBasicAccountingInformation,
                    (&mut accounting as *mut JOBOBJECT_BASIC_ACCOUNTING_INFORMATION).cast(),
                    size_of::<JOBOBJECT_BASIC_ACCOUNTING_INFORMATION>() as u32,
                    null_mut(),
                )
            },
            0,
            "creation-window accounting failed"
        );
        let expected = CREATION_WINDOW_CHILDREN as u32 + 2;
        assert_eq!(accounting.TotalProcesses, expected, "unmatched job member");
        assert_eq!(
            accounting.ActiveProcesses, expected,
            "unexpected early fixture exit"
        );
        let creation_window = window_started.elapsed();
        let cancelled_at = Instant::now();
        invocation.observe_cleanup();
        let receipt_after_cancel = cancelled_at.elapsed();
        let mut pending_at_receipt = 0;
        for handle in &invocation.fixture_children {
            let result = unsafe { WaitForSingleObject(handle.as_raw_handle() as HANDLE, 0) };
            assert!(matches!(result, WAIT_OBJECT_0 | WAIT_TIMEOUT));
            pending_at_receipt += usize::from(result == WAIT_TIMEOUT);
        }
        invocation.finalize().unwrap();
        assert!(invocation.thread.is_none());
        assert!(invocation.process.is_none());
        assert!(invocation.job.is_none());
        invocation.finish_fixture_cleanup();
        for handle in &invocation.fixture_children {
            assert_eq!(
                unsafe { WaitForSingleObject(handle.as_raw_handle() as HANDLE, 0) },
                WAIT_OBJECT_0
            );
        }
        eprintln!(
            "bounded ordinary creation: children={CREATION_WINDOW_CHILDREN}; \
             total_members={expected}; creation_window={creation_window:?}; \
             receipt_after_cancel={receipt_after_cancel:?}; \
             children_pending_at_receipt={pending_at_receipt}; \
             known_fixture_handles_signaled=true; post_receipt_scope_revival=false; \
             in_flight_CreateProcess_overlap=not_claimed"
        );
    }

    #[test]
    fn job_terminal_receipt_records_allowed_descendant_signal_lag() {
        let mut pending_observations = 0;
        for iteration in 0..16 {
            let mut invocation = Invocation::spawn(&fixture_command("cancel-parent"));
            let output = read_until(&mut invocation, b"END-READY");
            retain_ready_helper(&mut invocation, &output);
            let before = process_times(raw(&invocation.receipt_child));
            invocation.observe_cleanup();
            let at_receipt = invocation.helper_wait_at_receipt.unwrap();
            if at_receipt == WAIT_TIMEOUT {
                pending_observations += 1;
            }
            assert!(matches!(at_receipt, WAIT_OBJECT_0 | WAIT_TIMEOUT));
            invocation.finalize().unwrap();
            assert!(invocation.thread.is_none());
            assert!(invocation.process.is_none());
            assert!(invocation.job.is_none());
            // This separate fixture obligation cannot gate or retroactively
            // strengthen the already recorded product receipt.
            invocation.finish_fixture_cleanup();
            let after = process_times(raw(&invocation.receipt_child));
            assert_eq!(after.0, before.0);
            assert_ne!(after.1, 0);
            let handle = raw(&invocation.receipt_child);
            assert_eq!(unsafe { WaitForSingleObject(handle, 0) }, WAIT_OBJECT_0);
            let mut exit_code = 0;
            assert_ne!(unsafe { GetExitCodeProcess(handle, &mut exit_code) }, 0);
            assert_eq!(exit_code, 1);
            eprintln!(
                "allowed-limit observation iteration={iteration}; helper_wait_at_receipt={at_receipt}; \
                 checked_finalization=true; independent_fixture_signal=true"
            );
        }
        eprintln!(
            "allowed-limit observations: {pending_observations}/16 helpers unsignaled at receipt"
        );
    }

    #[test]
    #[ignore = "disposable child entrypoint, invoked by the lifecycle tests"]
    fn fixture() {
        let mode = std::env::var(MODE).unwrap();
        if mode == "selection-host" {
            let command = "echo SYSTEM-SHELL-MUST-NOT-BE-SELECTED";
            let baseline = Command::new("cmd").args(["/C", command]).output().unwrap();
            assert!(!baseline.stdout.windows(8).any(|part| part == b"SELECTED"));
            let mut invocation = Invocation::spawn(command);
            let (status, output) = invocation.collect();
            assert_eq!(status, baseline.status.code().unwrap());
            assert_eq!(output, baseline.stdout);
            invocation.clean();
        } else if mode == "inherited-child" {
            let name: Vec<_> = std::env::var(GATE)
                .unwrap()
                .encode_utf16()
                .chain(Some(0))
                .collect();
            let gate = owned(
                unsafe { OpenEventW(SYNCHRONIZATION_SYNCHRONIZE, 0, name.as_ptr()) },
                "test output gate",
            )
            .unwrap();
            println!("CHILD-READY");
            io::stdout().flush().unwrap();
            assert_eq!(
                unsafe { WaitForSingleObject(gate.as_raw_handle() as HANDLE, 8000) },
                WAIT_OBJECT_0
            );
            println!("DELAYED-FULL");
        } else if mode == "creation-window-parent" {
            for handle in [
                io::stdin().as_raw_handle(),
                io::stdout().as_raw_handle(),
                io::stderr().as_raw_handle(),
            ] {
                assert_ne!(
                    unsafe { SetHandleInformation(handle as HANDLE, HANDLE_FLAG_INHERIT, 0) },
                    0
                );
            }
            let name: Vec<_> = std::env::var(GATE)
                .unwrap()
                .encode_utf16()
                .chain(Some(0))
                .collect();
            let permit = owned(
                unsafe { OpenEventW(SYNCHRONIZATION_SYNCHRONIZE, 0, name.as_ptr()) },
                "test creation permit",
            )
            .unwrap();
            println!("PARENT-READY {} END-READY", std::process::id());
            io::stdout().flush().unwrap();
            assert_eq!(
                unsafe { WaitForSingleObject(permit.as_raw_handle() as HANDLE, 8000) },
                WAIT_OBJECT_0
            );
            let mut children = Vec::with_capacity(CREATION_WINDOW_CHILDREN);
            for index in 0..CREATION_WINDOW_CHILDREN {
                let child = Command::new(std::env::current_exe().unwrap())
                    .args([
                        "--exact",
                        FIXTURE,
                        "--ignored",
                        "--nocapture",
                        "--test-threads=1",
                    ])
                    .env(MODE, "waiting-child")
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn()
                    .unwrap();
                println!("CREATED {index} {}", child.id());
                children.push(child);
            }
            // Close the bounded creation window before cancellation so every
            // created process has an identity available for an exact handle.
            println!("CREATION-WINDOW-END");
            io::stdout().flush().unwrap();
            for mut child in children {
                child.wait().unwrap();
            }
        } else if mode == "waiting-child" {
            println!("CHILD-READY");
            io::stdout().flush().unwrap();
            std::thread::sleep(Duration::from_secs(30));
        } else {
            let inherited = mode == "inherited-parent";
            if !inherited {
                // Redirection alone need not clear the originals' inheritable
                // flags in a process that received inheritable standard handles.
                for handle in [
                    io::stdin().as_raw_handle(),
                    io::stdout().as_raw_handle(),
                    io::stderr().as_raw_handle(),
                ] {
                    assert_ne!(
                        unsafe { SetHandleInformation(handle as HANDLE, HANDLE_FLAG_INHERIT, 0) },
                        0
                    );
                }
            }
            let mut child = Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    FIXTURE,
                    "--ignored",
                    "--nocapture",
                    "--test-threads=1",
                ])
                .env(
                    MODE,
                    if inherited {
                        "inherited-child"
                    } else {
                        "waiting-child"
                    },
                )
                .stdin(Stdio::null())
                .stdout(if inherited {
                    Stdio::inherit()
                } else {
                    Stdio::piped()
                })
                .stderr(if inherited {
                    Stdio::inherit()
                } else {
                    Stdio::null()
                })
                .spawn()
                .unwrap();
            if !inherited {
                let reader = io::BufReader::new(child.stdout.take().unwrap());
                for line in reader.lines() {
                    if line.unwrap().contains("CHILD-READY") {
                        break;
                    }
                }
                println!("PARENT-READY {} END-READY", child.id());
                io::stdout().flush().unwrap();
                if mode == "cancel-parent" {
                    std::thread::sleep(Duration::from_secs(30));
                }
            }
        }
        io::stdout().flush().unwrap();
        std::process::exit(0);
    }
}
