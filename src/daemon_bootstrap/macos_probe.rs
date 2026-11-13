use super::DaemonBootstrapFailure;
use std::ffi::CString;
use std::os::fd::AsRawFd;
use std::os::unix::ffi::OsStrExt;
use std::process::Command;

pub(super) struct Child {
    pid: u32,
    pub(super) stdout: Option<std::io::PipeReader>,
    pub(super) stderr: Option<std::io::PipeReader>,
    pub(super) verified_start: Option<u64>,
}

impl Child {
    pub(super) fn id(&self) -> u32 {
        self.pid
    }

    pub(super) fn spawn(command: &Command) -> Result<Self, DaemonBootstrapFailure> {
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
        let mut pid = 0;
        // All buffers and descriptor owners outlive the native call. On success
        // ownership passes directly into Child, still suspended, with no reaper.
        let result = unsafe {
            libc::posix_spawn(
                &mut pid,
                program.as_ptr(),
                &actions.0,
                &attributes.0,
                argv.as_ptr(),
                envp.as_ptr(),
            )
        };
        if result != 0 {
            return Err(failure());
        }
        Ok(Self {
            pid: pid as u32,
            stdout: Some(stdout),
            stderr: Some(stderr),
            verified_start: None,
        })
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
