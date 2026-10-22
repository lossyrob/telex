//! Shared, fully isolated Telex process/install harness for the public
//! Application Client conformance battery, the external consumer fixture
//! probes, and the InstalledCurrent process proofs.
//!
//! Everything here is *setup* scaffolding, never an assertion surface. It
//! deliberately uses private/internal seams (`telex::install`, `telex::daemon`,
//! the `telex` CLI binary) only to build an isolated installed layout, to
//! induce faults, and to observe process-level state. The public contract that
//! the batteries assert on is `telex::application_client` alone.
//!
//! Isolation guarantees (never touches installed/user state):
//! - a unique temp root per harness instance,
//! - `TELEX_HOME`, `TELEX_RUN_DIR`, `TELEX_DB`, `TELEX_CONFIG`,
//!   `TELEX_INSTALL_ROOT` and the platform lock-state dir all point inside it,
//! - the installed layout is populated from the *branch* binary resolved
//!   through `CARGO_BIN_EXE_telex` (absolute path), so the daemon `HelloAck`
//!   `build_id` matches the strict manifest this harness writes.

#![allow(dead_code)]

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use telex::install::{self, InstallLayout};
use telex::model::now_ms;

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// Process-global environment lock.
///
/// `TELEX_*` selection is process-wide, so any test that installs an isolated
/// environment must hold this for its whole duration. It is an async mutex so
/// the guard can be held across `.await` inside multi-threaded tokio tests.
pub static ENV_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Absolute path of the branch `telex` binary under test.
pub fn branch_binary() -> PathBuf {
    if let Some(path) = option_env!("CARGO_BIN_EXE_telex") {
        let path = PathBuf::from(path);
        if path.is_file() {
            return std::fs::canonicalize(&path).unwrap_or(path);
        }
    }
    let exe = std::env::current_exe().expect("current test exe");
    let dir = exe.parent().expect("test exe dir");
    let target_dir = if dir.file_name().and_then(|n| n.to_str()) == Some("deps") {
        dir.parent().expect("target profile dir")
    } else {
        dir
    };
    let candidate = target_dir.join(format!("telex{}", std::env::consts::EXE_SUFFIX));
    assert!(
        candidate.is_file(),
        "branch telex binary not found at {}; build the workspace before running this suite",
        candidate.display()
    );
    std::fs::canonicalize(&candidate).unwrap_or(candidate)
}

#[cfg(windows)]
fn original_localappdata() -> Option<PathBuf> {
    use std::sync::OnceLock;
    static ORIGINAL: OnceLock<Option<PathBuf>> = OnceLock::new();
    ORIGINAL
        .get_or_init(|| std::env::var_os("LOCALAPPDATA").map(PathBuf::from))
        .clone()
}

fn harness_base_dir() -> PathBuf {
    if let Some(base) = std::env::var_os("TELEX_TEST_BASE_DIR") {
        return PathBuf::from(base);
    }
    #[cfg(windows)]
    {
        // Put each disposable authority root directly below the local volume.
        // Hosted profile/temp ancestors can carry foreign mutation ACEs that
        // the production authority-chain validator correctly rejects.
        original_localappdata()
            .unwrap_or_else(std::env::temp_dir)
            .ancestors()
            .last()
            .expect("Windows local path has a volume root")
            .to_path_buf()
    }
    #[cfg(not(windows))]
    {
        std::env::temp_dir().join("telex-app-client-tests")
    }
}

/// One fully isolated Telex environment: unique home/run/db/install roots plus
/// an installed layout whose `current` selector points at the branch binary.
pub struct Isolation {
    pub label: String,
    pub root: PathBuf,
    pub home: PathBuf,
    pub run_dir: PathBuf,
    pub state_dir: PathBuf,
    pub install_root: PathBuf,
    pub config_path: PathBuf,
    pub db_path: PathBuf,
    pub tag: String,
    binary: PathBuf,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DaemonIdentityEvidence {
    Absent,
    Exact { pid: u32, start_time: u64 },
    Unverifiable,
}

impl Isolation {
    /// Create the isolated tree and install the branch binary as `current`.
    pub fn new(label: &str) -> Self {
        Self::with_root(label, unique_root(label), true)
    }

    /// Create the isolated tree *without* installing anything, so a caller can
    /// build a deliberately broken or hand-written installed layout.
    pub fn new_empty(label: &str) -> Self {
        Self::with_root(label, unique_root(label), false)
    }

    fn with_root(label: &str, root: PathBuf, install_current: bool) -> Self {
        #[cfg(windows)]
        {
            create_owner_private_dir(&root);
        }
        #[cfg(not(windows))]
        std::fs::create_dir_all(&root).expect("create isolated harness root");
        let home = root.join("home");
        let run_dir = root.join("run");
        let state_dir = root.join("state");
        let install_root = root.join("install");
        create_owner_private_dir(&home);
        create_owner_private_dir(&run_dir);
        create_owner_private_dir(&install_root);
        std::fs::create_dir_all(&state_dir).expect("create lock state dir");
        let config_path = home.join("config.toml");
        std::fs::write(&config_path, "").expect("seed empty config");
        let db_path = root.join("telex.db");
        let tag = format!("v0.0.0-test-{}", now_ms());
        let harness = Self {
            label: label.to_string(),
            root,
            home,
            run_dir,
            state_dir,
            install_root,
            config_path,
            db_path,
            tag: tag.clone(),
            binary: branch_binary(),
        };
        if install_current {
            harness.install_tag(&tag, true);
        }
        harness
    }

    pub fn layout(&self) -> InstallLayout {
        install::layout_for_root(&self.install_root)
    }

    /// Install the branch binary under `tag`, optionally switching `current`.
    ///
    /// The manifest is written by the production installer, so its `build_id`
    /// is the branch build id the spawned daemon reports in `HelloAck`, and
    /// its `binary` field binds the exact versioned target.
    pub fn install_tag(&self, tag: &str, switch_current: bool) {
        let layout = self.layout();
        install::install_binary(
            &layout,
            tag,
            &self.binary,
            "conformance-harness",
            false,
            None,
        )
        .unwrap_or_else(|e| panic!("installing branch binary as {tag}: {e:#}"));
        if switch_current {
            install::switch_to(&layout, tag)
                .unwrap_or_else(|e| panic!("switching installed current to {tag}: {e:#}"));
        }
    }

    /// Absolute path of the versioned target the `current` selector resolves to.
    pub fn current_binary(&self) -> PathBuf {
        let layout = self.layout();
        install::current_binary(&layout)
            .expect("reading installed current binary")
            .expect("installed layout has a current binary")
    }

    /// Trusted root handed to `ApplicationDaemonBootstrap::InstalledCurrent`.
    pub fn trusted_root(&self) -> PathBuf {
        self.install_root.clone()
    }

    /// Apply this isolation to the current process environment.
    ///
    /// Callers must hold [`ENV_LOCK`] for as long as the environment is in use.
    pub fn apply_env(&self) -> EnvRestore {
        let mut restore = EnvRestore::default();
        restore.set("TELEX_HOME", &self.home);
        restore.set("TELEX_RUN_DIR", &self.run_dir);
        restore.set("TELEX_CONFIG", &self.config_path);
        restore.set("TELEX_DB", &self.db_path);
        restore.set(install::INSTALL_ROOT_ENV, &self.install_root);
        restore.set("TELEX_SESSION_ID", format!("{}-session", self.label));
        restore.set("TELEX_RECONNECT_GRACE_MS", "3000");
        restore.set("TELEX_LIVENESS_WINDOW_SECS", "0");
        #[cfg(windows)]
        restore.set("LOCALAPPDATA", &self.state_dir);
        #[cfg(not(windows))]
        restore.set("XDG_STATE_HOME", &self.state_dir);
        restore.unset("TELEX_BACKEND");
        restore.unset("TELEX_ADDRESS");
        restore.unset("TELEX_SESSION_PID");
        restore.unset(install::LAUNCHER_GUARD_ENV);
        restore
    }

    /// Write the backend profile config used by both the client and the daemon.
    pub fn write_config(&self, config: &telex::profiles::ConfigFile) {
        let text = toml::to_string_pretty(config).expect("serialize backend config");
        std::fs::write(&self.config_path, text).expect("write backend config");
    }

    /// Build a `Command` for the installed current binary with this isolation's
    /// environment applied explicitly (independent of process-global env).
    pub fn command(&self) -> Command {
        self.command_for(&self.current_binary())
    }

    pub fn command_for(&self, binary: &Path) -> Command {
        let mut cmd = Command::new(binary);
        cmd.env("TELEX_HOME", &self.home)
            .env("TELEX_RUN_DIR", &self.run_dir)
            .env("TELEX_CONFIG", &self.config_path)
            .env("TELEX_DB", &self.db_path)
            .env(install::INSTALL_ROOT_ENV, &self.install_root)
            .env("TELEX_SESSION_ID", format!("{}-session", self.label))
            .env("TELEX_RECONNECT_GRACE_MS", "3000")
            .env("TELEX_LIVENESS_WINDOW_SECS", "0")
            .env_remove("TELEX_BACKEND")
            .env_remove("TELEX_ADDRESS")
            .env_remove("TELEX_SESSION_PID")
            .env_remove(install::LAUNCHER_GUARD_ENV)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(windows)]
        cmd.env("LOCALAPPDATA", &self.state_dir);
        #[cfg(not(windows))]
        cmd.env("XDG_STATE_HOME", &self.state_dir);
        cmd
    }

    /// Path of the daemon capability file for this isolation, when present.
    pub fn cap_path(&self) -> Option<PathBuf> {
        std::fs::read_dir(&self.run_dir)
            .ok()?
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .find(|path| {
                path.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|name| name.starts_with("daemon-") && name.ends_with(".cap"))
            })
    }

    pub fn cap_json(&self) -> Option<serde_json::Value> {
        let path = self.cap_path()?;
        let text = std::fs::read_to_string(path).ok()?;
        serde_json::from_str(&text).ok()
    }

    /// Admin capability proof for privileged daemon setup/fault requests.
    pub fn admin_cap(&self) -> Option<String> {
        self.cap_json()?
            .get("admin_cap")
            .and_then(|v| v.as_str())
            .map(str::to_string)
    }

    fn daemon_identity_from_cap(cap: &serde_json::Value) -> DaemonIdentityEvidence {
        let Some(pid) = cap
            .get("server_pid")
            .and_then(|v| v.as_u64())
            .and_then(|pid| u32::try_from(pid).ok())
        else {
            return DaemonIdentityEvidence::Unverifiable;
        };
        let Some(start_time) = cap.get("server_start_time").and_then(|v| v.as_u64()) else {
            return DaemonIdentityEvidence::Unverifiable;
        };
        DaemonIdentityEvidence::Exact { pid, start_time }
    }

    fn daemon_identity(&self) -> DaemonIdentityEvidence {
        let Some(path) = self.cap_path() else {
            return DaemonIdentityEvidence::Absent;
        };
        let Ok(text) = std::fs::read_to_string(path) else {
            return DaemonIdentityEvidence::Unverifiable;
        };
        let Ok(cap) = serde_json::from_str(&text) else {
            return DaemonIdentityEvidence::Unverifiable;
        };
        Self::daemon_identity_from_cap(&cap)
    }

    pub fn daemon_pid(&self) -> Option<u32> {
        match self.daemon_identity() {
            DaemonIdentityEvidence::Exact { pid, .. } => Some(pid),
            DaemonIdentityEvidence::Absent | DaemonIdentityEvidence::Unverifiable => None,
        }
    }

    fn daemon_running_observation(&self) -> Option<bool> {
        let out = self.run_cli(["--json", "daemon", "status"], Duration::from_secs(20));
        (out.code == Some(0))
            .then(|| serde_json::from_str::<serde_json::Value>(&out.stdout).ok())
            .flatten()
            .and_then(|json| json.get("running").and_then(|v| v.as_bool()))
    }

    pub fn daemon_running(&self) -> bool {
        self.daemon_running_observation() == Some(true)
    }

    /// Stop the daemon and wait until it is no longer serving.
    pub fn stop_daemon(&self) {
        self.stop_daemon_with_timeout(Duration::from_secs(30));
    }

    pub fn stop_daemon_with_timeout(&self, timeout: Duration) {
        assert!(
            self.stop_daemon_with_executable_within(&self.current_binary(), timeout),
            "daemon for {} did not stop within the deadline",
            self.label
        );
    }

    pub fn stop_daemon_with_executable(&self, executable: &Path) {
        assert!(
            self.stop_daemon_with_executable_within(executable, Duration::from_secs(30)),
            "daemon for {} did not stop within the deadline",
            self.label
        );
    }

    pub fn stop_daemon_best_effort(&self) -> bool {
        let executable = match install::current_binary(&self.layout()) {
            Ok(Some(executable)) => executable,
            Ok(None) | Err(_) => return false,
        };
        self.stop_daemon_with_executable_within(&executable, Duration::from_secs(20))
    }

    fn stop_daemon_with_executable_within(&self, executable: &Path, timeout: Duration) -> bool {
        let identity = self.daemon_identity();
        let mut command = self.command_for(executable);
        command.args(["--json", "daemon", "stop", "--drain"]);
        let _ = run_with_timeout(command, timeout);
        self.wait_until_stopped(identity, timeout)
    }

    fn wait_until_stopped(&self, identity: DaemonIdentityEvidence, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            match identity {
                DaemonIdentityEvidence::Exact { pid, start_time } => {
                    if !telex::session_watch::process_alive_with_start_time(pid, Some(start_time)) {
                        return true;
                    }
                }
                DaemonIdentityEvidence::Absent => return false,
                DaemonIdentityEvidence::Unverifiable => return false,
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        false
    }

    pub fn run_cli<I, S>(&self, args: I, timeout: Duration) -> CliOutput
    where
        I: IntoIterator<Item = S>,
        S: AsRef<std::ffi::OsStr>,
    {
        let mut cmd = self.command();
        cmd.args(args);
        run_with_timeout(cmd, timeout)
    }

    /// Best-effort teardown. Roots are retained because endpoint absence cannot
    /// be proven from a missing or stale capability record during publication.
    pub fn cleanup(&self) {
        let _ = self.stop_daemon_best_effort();
    }
}

impl Drop for Isolation {
    /// Attempt graceful teardown on unwind. The isolated root is retained
    /// because endpoint absence is not independently provable during startup.
    fn drop(&mut self) {
        self.cleanup();
    }
}

fn unique_root(label: &str) -> PathBuf {
    let id = NEXT_ID.fetch_add(1, Ordering::SeqCst);
    #[cfg(windows)]
    let name = format!("telex-app-{label}-{}-{}-{id}", std::process::id(), now_ms());
    #[cfg(not(windows))]
    let name = format!("{label}-{}-{}-{id}", std::process::id(), now_ms());
    harness_base_dir().join(name)
}

/// Whether a process id still names a live process.
#[cfg(unix)]
pub fn process_alive(pid: u32) -> bool {
    unsafe { libc::kill(pid as libc::pid_t, 0) == 0 }
}

/// Whether a process id still names a live process.
#[cfg(windows)]
pub fn process_alive(pid: u32) -> bool {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{
        GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    const STILL_ACTIVE: u32 = 259;
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle == 0 {
            return false;
        }
        let mut code: u32 = 0;
        let ok = GetExitCodeProcess(handle, &mut code);
        CloseHandle(handle);
        ok != 0 && code == STILL_ACTIVE
    }
}

/// Terminate a process by id. Fault induction and teardown only.
///
/// Never terminates the calling process: a test that plants a *foreign*
/// capability record naming its own pid must not be able to kill itself.
#[cfg(unix)]
pub fn terminate_process(pid: u32) {
    if pid == 0 || pid == std::process::id() {
        return;
    }
    unsafe {
        libc::kill(pid as libc::pid_t, libc::SIGKILL);
    }
}

/// Terminate a process by id. Fault induction and teardown only.
///
/// Never terminates the calling process: a test that plants a *foreign*
/// capability record naming its own pid must not be able to kill itself.
#[cfg(windows)]
pub fn terminate_process(pid: u32) {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{OpenProcess, TerminateProcess, PROCESS_TERMINATE};
    if pid == 0 || pid == std::process::id() {
        return;
    }
    unsafe {
        let handle = OpenProcess(PROCESS_TERMINATE, 0, pid);
        if handle != 0 {
            TerminateProcess(handle, 1);
            CloseHandle(handle);
        }
    }
}

#[derive(Debug)]
pub struct CliOutput {
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub timed_out: bool,
}

impl CliOutput {
    pub fn assert_success(&self, context: &str) {
        assert!(
            !self.timed_out && self.code == Some(0),
            "{context} failed: code={:?} timed_out={} stdout={} stderr={}",
            self.code,
            self.timed_out,
            self.stdout,
            self.stderr
        );
    }

    pub fn assert_failure(&self, context: &str) {
        assert!(
            self.timed_out || self.code != Some(0),
            "{context} unexpectedly succeeded: stdout={} stderr={}",
            self.stdout,
            self.stderr
        );
    }
}

pub fn run_with_timeout(mut cmd: Command, timeout: Duration) -> CliOutput {
    let mut child = match cmd.spawn() {
        Ok(child) => child,
        Err(e) => {
            return CliOutput {
                code: None,
                stdout: String::new(),
                stderr: format!("spawn failed: {e}"),
                timed_out: false,
            }
        }
    };
    let stdout = child.stdout.take().map(drain_pipe);
    let stderr = child.stderr.take().map(drain_pipe);
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return CliOutput {
                        code: None,
                        stdout: drained_text(&stdout),
                        stderr: drained_text(&stderr),
                        timed_out: true,
                    };
                }
                std::thread::sleep(Duration::from_millis(25));
            }
            Err(e) => {
                return CliOutput {
                    code: None,
                    stdout: String::new(),
                    stderr: format!("wait failed: {e}"),
                    timed_out: false,
                }
            }
        }
    }
    let status = child.wait().expect("collect child status");
    CliOutput {
        code: status.code(),
        stdout: drained_text(&stdout),
        stderr: drained_text(&stderr),
        timed_out: false,
    }
}

struct DrainedPipe {
    bytes: Arc<Mutex<Vec<u8>>>,
    done: Arc<AtomicBool>,
}

fn drain_pipe<R: std::io::Read + Send + 'static>(mut pipe: R) -> DrainedPipe {
    let bytes = Arc::new(Mutex::new(Vec::new()));
    let sink = bytes.clone();
    let done = Arc::new(AtomicBool::new(false));
    let finished = done.clone();
    std::thread::spawn(move || {
        let mut chunk = [0u8; 8192];
        loop {
            match pipe.read(&mut chunk) {
                Ok(0) => break,
                Ok(read) => sink.lock().unwrap().extend_from_slice(&chunk[..read]),
                Err(_) => break,
            }
        }
        finished.store(true, Ordering::Release);
    });
    DrainedPipe { bytes, done }
}

fn drained_text(output: &Option<DrainedPipe>) -> String {
    output
        .as_ref()
        .map(|output| {
            let deadline = Instant::now() + Duration::from_secs(1);
            while !output.done.load(Ordering::Acquire) && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(5));
            }
            String::from_utf8_lossy(&output.bytes.lock().unwrap())
                .trim()
                .to_string()
        })
        .unwrap_or_default()
}

/// Restores every environment variable this harness changed.
#[derive(Default)]
pub struct EnvRestore {
    previous: Vec<(String, Option<OsString>)>,
}

impl EnvRestore {
    pub fn set(&mut self, key: &str, value: impl AsRef<std::ffi::OsStr>) {
        self.previous.push((key.to_string(), std::env::var_os(key)));
        std::env::set_var(key, value);
    }

    pub fn unset(&mut self, key: &str) {
        self.previous.push((key.to_string(), std::env::var_os(key)));
        std::env::remove_var(key);
    }
}

impl Drop for EnvRestore {
    fn drop(&mut self) {
        for (key, value) in self.previous.drain(..).rev() {
            match value {
                Some(value) => std::env::set_var(&key, value),
                None => std::env::remove_var(&key),
            }
        }
    }
}

/// Create a directory owned by the current principal with an explicit,
/// non-inherited authority DACL on Windows (and 0o700 on Unix), matching what
/// the production install-authority and owner-private runtime checks demand.
pub fn create_owner_private_dir(path: &Path) {
    #[cfg(not(windows))]
    {
        use std::os::unix::fs::DirBuilderExt;
        if path.exists() {
            return;
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(path)
            .unwrap_or_else(|e| panic!("creating owner-private dir {}: {e}", path.display()));
    }
    #[cfg(windows)]
    {
        use std::ffi::{c_void, OsStr};
        use std::mem::{size_of, MaybeUninit};
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, LocalFree};
        use windows_sys::Win32::Security::Authorization::{
            ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
            SDDL_REVISION_1,
        };
        use windows_sys::Win32::Security::{
            GetTokenInformation, TokenUser, SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER,
        };
        use windows_sys::Win32::Storage::FileSystem::CreateDirectoryW;
        use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).ok();
        }

        fn wide_null(value: &OsStr) -> Vec<u16> {
            value.encode_wide().chain(std::iter::once(0)).collect()
        }

        unsafe fn wide_ptr_to_string(ptr: *const u16) -> String {
            let mut len = 0usize;
            while *ptr.add(len) != 0 {
                len += 1;
            }
            String::from_utf16_lossy(std::slice::from_raw_parts(ptr, len))
        }

        let sid = {
            let mut token = 0;
            let ok = unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) };
            assert_ne!(ok, 0, "opening current process token");
            let mut needed = 0u32;
            unsafe {
                GetTokenInformation(token, TokenUser, std::ptr::null_mut(), 0, &mut needed);
            }
            assert!(needed > 0, "querying token user buffer length");
            let mut buf = vec![
                MaybeUninit::<TOKEN_USER>::uninit();
                (needed as usize).div_ceil(size_of::<TOKEN_USER>())
            ];
            let ok = unsafe {
                GetTokenInformation(
                    token,
                    TokenUser,
                    buf.as_mut_ptr().cast::<c_void>(),
                    needed,
                    &mut needed,
                )
            };
            assert_ne!(ok, 0, "reading current token user");
            let token_user = buf.as_ptr().cast::<TOKEN_USER>();
            assert!(
                token_user.is_aligned(),
                "TOKEN_USER pointer before dereference"
            );
            let token_user = unsafe { &*token_user };
            let mut sid_ptr: *mut u16 = std::ptr::null_mut();
            let ok = unsafe { ConvertSidToStringSidW(token_user.User.Sid, &mut sid_ptr) };
            unsafe {
                CloseHandle(token);
            }
            assert_ne!(ok, 0, "converting current SID to string");
            let sid = unsafe { wide_ptr_to_string(sid_ptr) };
            unsafe {
                LocalFree(sid_ptr as *mut c_void);
            }
            sid
        };

        // Protected DACL, inheritable (OICI) so every artifact the installer
        // writes below the trusted root also carries an owner-only descriptor.
        let sddl = format!("O:{sid}G:{sid}D:P(A;OICI;GA;;;{sid})");
        let mut descriptor: *mut c_void = std::ptr::null_mut();
        let sddl_wide = wide_null(OsStr::new(&sddl));
        let ok = unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl_wide.as_ptr(),
                SDDL_REVISION_1,
                &mut descriptor,
                std::ptr::null_mut(),
            )
        };
        assert_ne!(ok, 0, "building owner-only security descriptor");
        let attrs = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: descriptor,
            bInheritHandle: 0,
        };
        let path_wide = wide_null(path.as_os_str());
        let ok = unsafe { CreateDirectoryW(path_wide.as_ptr(), &attrs) };
        unsafe {
            LocalFree(descriptor);
        }
        assert_ne!(
            ok,
            0,
            "creating exclusive owner-private dir {} failed with native error {}: {}",
            path.display(),
            unsafe { GetLastError() },
            std::io::Error::last_os_error()
        );
    }
}

/// Whether a live credentialed Postgres environment is configured.
///
/// `TELEX_PG_REQUIRE=1` turns a missing/empty `TELEX_PG_URL` into a hard
/// failure so an authoritative CI job can never pass by silently skipping.
pub fn postgres_url_or_fail_closed(context: &str) -> Option<String> {
    let require = std::env::var("TELEX_PG_REQUIRE")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    match std::env::var("TELEX_PG_URL") {
        Ok(url) if !url.trim().is_empty() => Some(url),
        _ => {
            assert!(
                !require,
                "TELEX_PG_REQUIRE is set but TELEX_PG_URL is unset/empty; \
                 refusing to skip {context}."
            );
            eprintln!("[{context}] TELEX_PG_URL not set; Postgres leg not executed here.");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn owner_private_directory_creation_refuses_an_existing_path() {
        let root = unique_root("exclusive-existing");
        std::fs::create_dir(&root).expect("create pre-existing proof root");
        let refused = std::panic::catch_unwind(|| create_owner_private_dir(&root));
        assert!(refused.is_err(), "existing proof root must not be claimed");
        assert!(root.is_dir(), "refusal must not modify the existing root");
        std::fs::remove_dir(&root).expect("remove exact pre-existing proof root");
    }

    #[test]
    #[ignore = "subprocess role used by the pipe-drain regression"]
    fn large_output_fixture() {
        if std::env::var_os("TELEX_LARGE_OUTPUT_FIXTURE").is_some() {
            println!("{}", "x".repeat(131_072));
            eprintln!("{}", "y".repeat(131_072));
        }
    }

    #[test]
    fn run_with_timeout_drains_large_piped_output() {
        let mut command = Command::new(std::env::current_exe().expect("current test executable"));
        command
            .env("TELEX_LARGE_OUTPUT_FIXTURE", "1")
            .args([
                "--ignored",
                "--exact",
                "isolation::tests::large_output_fixture",
                "--nocapture",
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let output = run_with_timeout(command, Duration::from_secs(30));
        output.assert_success("large piped output fixture");
        assert!(output.stdout.contains(&"x".repeat(1024)));
        assert!(output.stderr.contains(&"y".repeat(1024)));
    }

    #[test]
    fn forced_teardown_identity_requires_pid_and_generation() {
        assert_eq!(
            Isolation::daemon_identity_from_cap(&serde_json::json!({
                "server_pid": 42,
                "server_start_time": 99
            })),
            DaemonIdentityEvidence::Exact {
                pid: 42,
                start_time: 99
            }
        );
        assert_eq!(
            Isolation::daemon_identity_from_cap(&serde_json::json!({"server_pid": 42})),
            DaemonIdentityEvidence::Unverifiable
        );
        assert_eq!(
            Isolation::daemon_identity_from_cap(&serde_json::json!({"server_start_time": 99})),
            DaemonIdentityEvidence::Unverifiable
        );
    }
}
