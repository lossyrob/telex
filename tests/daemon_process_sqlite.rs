#![cfg(feature = "sqlite")]

use serde_json::Value;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

static NEXT_ENV: AtomicUsize = AtomicUsize::new(1);
static NEXT_CAPTURE: AtomicUsize = AtomicUsize::new(1);

#[derive(Debug)]
struct CmdOutput {
    code: Option<i32>,
    stdout: String,
    stderr: String,
    timed_out: bool,
}

impl CmdOutput {
    fn assert_success(&self, context: &str) {
        assert!(
            !self.timed_out && self.code == Some(0),
            "{context} failed: code={:?} timed_out={} stdout={} stderr={}",
            self.code,
            self.timed_out,
            self.stdout,
            self.stderr
        );
    }

    fn assert_failure(&self, context: &str) {
        assert!(
            self.timed_out || self.code != Some(0),
            "{context} unexpectedly succeeded: stdout={} stderr={}",
            self.stdout,
            self.stderr
        );
    }

    fn json(&self, context: &str) -> Value {
        serde_json::from_str(&self.stdout).unwrap_or_else(|e| {
            panic!(
                "{context} did not emit JSON: {e}; code={:?} stdout={} stderr={}",
                self.code, self.stdout, self.stderr
            )
        })
    }
}

#[derive(Debug)]
struct ProcessEnv {
    bin: PathBuf,
    root: PathBuf,
    home: PathBuf,
    run_dir: PathBuf,
    db: PathBuf,
    state_dir: PathBuf,
    session_id: String,
    liveness_window_secs: i64,
}

impl ProcessEnv {
    fn new(name: &str) -> Self {
        let id = NEXT_ENV.fetch_add(1, Ordering::SeqCst);
        let root = process_test_root(id);
        let db = root.join("db.sqlite");
        let state_dir = root.join("state");
        Self::with_paths(name, root, db, state_dir)
    }

    fn with_shared_store(name: &str, db: PathBuf, state_dir: PathBuf) -> Self {
        let id = NEXT_ENV.fetch_add(1, Ordering::SeqCst);
        let root = process_test_root(id);
        Self::with_paths(name, root, db, state_dir)
    }

    fn with_paths(name: &str, root: PathBuf, db: PathBuf, state_dir: PathBuf) -> Self {
        let home = root.join("h");
        let run_dir = root.join("r");
        std::fs::create_dir_all(&root).expect("create test root");
        #[cfg(windows)]
        {
            create_owner_private_daemon_fixture_dir(&home);
            create_owner_private_daemon_fixture_dir(&run_dir);
        }
        std::fs::create_dir_all(&state_dir).expect("create lock state dir");
        Self {
            bin: telex_bin(),
            root,
            home,
            run_dir,
            db,
            state_dir,
            session_id: format!("{name}-session"),
            liveness_window_secs: 0,
        }
    }

    fn command_with_session(&self, session: &str) -> Command {
        let mut cmd = Command::new(&self.bin);
        cmd.env("TELEX_HOME", &self.home)
            .env("TELEX_RUN_DIR", &self.run_dir)
            .env("TELEX_DB", &self.db)
            .env("TELEX_CONFIG", self.home.join("config.toml"))
            .env("TELEX_SESSION_ID", session)
            .env("TELEX_RECONNECT_GRACE_MS", "3000")
            .env(
                "TELEX_LIVENESS_WINDOW_SECS",
                self.liveness_window_secs.to_string(),
            )
            .env_remove("TELEX_BACKEND")
            .env_remove("TELEX_ADDRESS")
            .env_remove("TELEX_SESSION_PID");
        #[cfg(windows)]
        {
            cmd.env("LOCALAPPDATA", &self.state_dir);
        }
        #[cfg(not(windows))]
        {
            cmd.env("XDG_STATE_HOME", &self.state_dir);
        }
        cmd
    }

    fn run<I, S>(&self, args: I, timeout: Duration) -> CmdOutput
    where
        I: IntoIterator<Item = S>,
        S: AsRef<std::ffi::OsStr>,
    {
        self.run_with_session(&self.session_id, args, timeout)
    }

    fn run_with_session<I, S>(&self, session: &str, args: I, timeout: Duration) -> CmdOutput
    where
        I: IntoIterator<Item = S>,
        S: AsRef<std::ffi::OsStr>,
    {
        let mut cmd = self.command_with_session(session);
        cmd.args(args);
        run_command_with_capture(cmd, &self.root, timeout)
    }

    fn attach(&self, session: &str, address: &str) -> Value {
        let out = self.run_with_session(
            session,
            [
                "--json",
                "--address",
                address,
                "attach",
                "--session",
                session,
                "--description",
                "process integration test",
            ],
            Duration::from_secs(8),
        );
        out.assert_success("attach");
        out.json("attach")
    }

    fn daemon_status(&self) -> CmdOutput {
        self.run(["--json", "daemon", "status"], Duration::from_secs(4))
    }

    fn stop_daemon_best_effort(&self) {
        let _ = self.run(
            ["--json", "daemon", "stop", "--drain"],
            Duration::from_secs(4),
        );
        let _ = self.wait_until_not_running(Duration::from_secs(3));
    }

    fn wait_until_not_running(&self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            let status = self.daemon_status();
            if status.code == Some(0) {
                let json = status.json("daemon status");
                if json.get("running").and_then(Value::as_bool) == Some(false) {
                    return true;
                }
            } else {
                return true;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        false
    }

    fn cap_json(&self) -> Value {
        let path = self.cap_path().unwrap_or_else(|| {
            panic!(
                "no daemon cap file found in run dir {}",
                self.run_dir.display()
            )
        });
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("reading cap file {}: {e}", path.display()));
        serde_json::from_str(&text)
            .unwrap_or_else(|e| panic!("parsing cap file {}: {e}", path.display()))
    }

    fn cap_path(&self) -> Option<PathBuf> {
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

    fn daemon_pid(&self) -> u32 {
        self.cap_json()
            .get("server_pid")
            .and_then(Value::as_u64)
            .and_then(|pid| u32::try_from(pid).ok())
            .expect("cap file contains server_pid")
    }
}

impl Drop for ProcessEnv {
    fn drop(&mut self) {
        self.stop_daemon_best_effort();
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn telex_bin() -> PathBuf {
    if let Some(path) = option_env!("CARGO_BIN_EXE_telex") {
        return PathBuf::from(path);
    }
    let exe = std::env::current_exe().expect("current test exe");
    let dir = exe.parent().expect("test exe dir");
    let target_dir = if dir.file_name().and_then(|n| n.to_str()) == Some("deps") {
        dir.parent().expect("target debug dir")
    } else {
        dir
    };
    target_dir.join(format!("telex{}", std::env::consts::EXE_SUFFIX))
}

#[cfg(windows)]
fn process_test_root(id: usize) -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("telex-process-tests")
        .join(format!("tx{}-{}", std::process::id(), id))
}

#[cfg(not(windows))]
fn process_test_root(id: usize) -> PathBuf {
    std::env::current_dir()
        .expect("current dir")
        .join("target")
        .join("t")
        .join(format!("tx{}-{}", std::process::id(), id))
}

#[cfg(windows)]
fn create_owner_private_daemon_fixture_dir(path: &Path) {
    use std::ffi::{c_void, OsStr};
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Foundation::{
        CloseHandle, GetLastError, LocalFree, ERROR_ALREADY_EXISTS,
    };
    use windows_sys::Win32::Security::Authorization::{
        ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
        SDDL_REVISION_1,
    };
    use windows_sys::Win32::Security::{
        GetTokenInformation, TokenUser, SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER,
    };
    use windows_sys::Win32::Storage::FileSystem::CreateDirectoryW;
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    let sid = current_user_sid_string();
    let sddl = format!("O:{sid}G:{sid}D:P(A;;GA;;;{sid})");
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
    assert_ne!(
        ok,
        0,
        "building owner-only test directory security descriptor: {}",
        std::io::Error::last_os_error()
    );

    let mut attrs = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor,
        bInheritHandle: 0,
    };
    let path_wide = wide_null(path.as_os_str());
    let ok = unsafe { CreateDirectoryW(path_wide.as_ptr(), &mut attrs) };
    unsafe {
        LocalFree(descriptor);
    }
    if ok == 0 {
        let err = unsafe { GetLastError() };
        assert_eq!(
            err,
            ERROR_ALREADY_EXISTS,
            "creating owner-only test daemon directory {}: {}",
            path.display(),
            std::io::Error::last_os_error()
        );
    }

    fn current_user_sid_string() -> String {
        let mut token = 0;
        let ok = unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) };
        assert_ne!(
            ok,
            0,
            "opening current process token: {}",
            std::io::Error::last_os_error()
        );
        let mut needed = 0u32;
        unsafe {
            GetTokenInformation(token, TokenUser, std::ptr::null_mut(), 0, &mut needed);
        }
        assert!(needed > 0, "querying token user buffer length");
        let mut buf = vec![0u8; needed as usize];
        let ok = unsafe {
            GetTokenInformation(
                token,
                TokenUser,
                buf.as_mut_ptr() as *mut c_void,
                needed,
                &mut needed,
            )
        };
        if ok == 0 {
            unsafe {
                CloseHandle(token);
            }
            panic!(
                "reading current token user: {}",
                std::io::Error::last_os_error()
            );
        }
        let token_user = unsafe { &*(buf.as_ptr() as *const TOKEN_USER) };
        let mut sid_ptr: *mut u16 = std::ptr::null_mut();
        let ok = unsafe { ConvertSidToStringSidW(token_user.User.Sid, &mut sid_ptr) };
        unsafe {
            CloseHandle(token);
        }
        assert_ne!(
            ok,
            0,
            "converting current SID to string: {}",
            std::io::Error::last_os_error()
        );
        let sid = unsafe { wide_ptr_to_string(sid_ptr) };
        unsafe {
            LocalFree(sid_ptr as *mut c_void);
        }
        sid
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
}

fn run_command_with_capture(mut cmd: Command, root: &Path, timeout: Duration) -> CmdOutput {
    let capture_id = NEXT_CAPTURE.fetch_add(1, Ordering::SeqCst);
    let capture_dir = root.join("cmd");
    std::fs::create_dir_all(&capture_dir).expect("create command capture dir");
    let stdout_path = capture_dir.join(format!("cmd-{capture_id}.out"));
    let stderr_path = capture_dir.join(format!("cmd-{capture_id}.err"));
    let mut stdout_file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .read(true)
        .write(true)
        .open(&stdout_path)
        .unwrap_or_else(|e| panic!("opening stdout capture {}: {e}", stdout_path.display()));
    let mut stderr_file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .read(true)
        .write(true)
        .open(&stderr_path)
        .unwrap_or_else(|e| panic!("opening stderr capture {}: {e}", stderr_path.display()));
    cmd.stdout(Stdio::from(
        stdout_file.try_clone().expect("clone stdout capture"),
    ))
    .stderr(Stdio::from(
        stderr_file.try_clone().expect("clone stderr capture"),
    ));

    let child = cmd.spawn().unwrap_or_else(|e| {
        panic!(
            "spawning command failed; stdout={} stderr={}: {e}",
            stdout_path.display(),
            stderr_path.display()
        )
    });
    let (code, timed_out) = wait_status_with_timeout(child, timeout);
    let stdout = read_capture(&mut stdout_file, &stdout_path);
    let stderr = read_capture(&mut stderr_file, &stderr_path);
    CmdOutput {
        code,
        stdout: stdout.trim().to_string(),
        stderr: stderr.trim().to_string(),
        timed_out,
    }
}

fn read_capture(file: &mut std::fs::File, path: &Path) -> String {
    file.seek(SeekFrom::Start(0))
        .unwrap_or_else(|e| panic!("seeking capture {}: {e}", path.display()));
    let mut text = String::new();
    file.read_to_string(&mut text)
        .unwrap_or_else(|e| panic!("reading capture {}: {e}", path.display()));
    text
}

fn wait_until_path_exists(path: &Path, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if path.exists() {
            return;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    panic!("timed out waiting for {}", path.display());
}

fn wait_until_daemon_lists_waiter(env: &ProcessEnv, waiter_pid: u32, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        let status = env.daemon_status();
        if status.code == Some(0) {
            let status_json = status.json("daemon status while waiting for waiter");
            if let Some(waiters) = status_json.get("live_waiters").and_then(Value::as_array) {
                if waiters
                    .iter()
                    .any(|w| w.get("pid").and_then(Value::as_u64) == Some(waiter_pid as u64))
                {
                    return;
                }
            }
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    panic!("timed out waiting for daemon to list waiter pid {waiter_pid}");
}

fn wait_status_with_timeout(
    mut child: std::process::Child,
    timeout: Duration,
) -> (Option<i32>, bool) {
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => {
                let status = child.wait().expect("collect process status");
                return (status.code(), false);
            }
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let status = child.wait().expect("collect killed status");
                return (status.code(), true);
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(e) => panic!("waiting for child process: {e}"),
        }
    }
}

fn assert_text_contains_any(text: &str, needles: &[&str], context: &str) {
    let lower = text.to_ascii_lowercase();
    assert!(
        needles
            .iter()
            .any(|needle| lower.contains(&needle.to_ascii_lowercase())),
        "{context}: expected one of {needles:?} in {text:?}"
    );
}

fn message_id(json: &Value) -> i64 {
    json.get("id")
        .and_then(Value::as_i64)
        .expect("message id in wait JSON")
}

fn wait_for_message(env: &ProcessEnv, session: &str, address: &str, body: &str) -> Value {
    let deadline = Instant::now() + Duration::from_secs(6);
    let mut last = None;
    while Instant::now() < deadline {
        let out = env.run_with_session(
            session,
            [
                "--json",
                "--address",
                address,
                "wait",
                "--session",
                session,
                "--timeout-ms",
                "1500",
                "--hang-ms",
                "1000",
                "--reconnect-grace-ms",
                "3000",
            ],
            Duration::from_secs(5),
        );
        if out.code == Some(0) && !out.timed_out {
            let json = out.json("wait");
            assert_eq!(json.get("body").and_then(Value::as_str), Some(body));
            return json;
        }
        last = Some(out);
        std::thread::sleep(Duration::from_millis(100));
    }
    let last = last.expect("wait attempted");
    panic!(
        "wait never delivered message: code={:?} timed_out={} stdout={} stderr={}",
        last.code, last.timed_out, last.stdout, last.stderr
    );
}

#[test]
fn real_process_01_concurrent_first_use() {
    let env = ProcessEnv::new("real01");
    let workers = 8;
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(workers));
    let handles = (0..workers)
        .map(|i| {
            let barrier = barrier.clone();
            let bin = env.bin.clone();
            let home = env.home.clone();
            let run_dir = env.run_dir.clone();
            let db = env.db.clone();
            let config = env.home.join("config.toml");
            let state_dir = env.state_dir.clone();
            let capture_root = env.root.clone();
            std::thread::spawn(move || {
                barrier.wait();
                let session = format!("real01-s{i}");
                let address = format!("addr:real01-{i}");
                let mut cmd = Command::new(bin);
                cmd.env("TELEX_HOME", home)
                    .env("TELEX_RUN_DIR", run_dir)
                    .env("TELEX_DB", db)
                    .env("TELEX_CONFIG", config)
                    .env("TELEX_SESSION_ID", &session)
                    .env("TELEX_RECONNECT_GRACE_MS", "3000")
                    .env("TELEX_LIVENESS_WINDOW_SECS", "0")
                    .env_remove("TELEX_BACKEND")
                    .env_remove("TELEX_ADDRESS")
                    .env_remove("TELEX_SESSION_PID")
                    .args([
                        "--json",
                        "--address",
                        &address,
                        "attach",
                        "--session",
                        &session,
                        "--description",
                        "herd worker",
                    ]);
                #[cfg(windows)]
                {
                    cmd.env("LOCALAPPDATA", state_dir);
                }
                #[cfg(not(windows))]
                {
                    cmd.env("XDG_STATE_HOME", state_dir);
                }
                run_command_with_capture(cmd, &capture_root, Duration::from_secs(8))
            })
        })
        .collect::<Vec<_>>();

    let mut owner_ids = Vec::new();
    for handle in handles {
        let out = handle.join().expect("attach worker thread");
        out.assert_success("concurrent attach");
        let json = out.json("concurrent attach");
        owner_ids.push(
            json.get("owner_instance_id")
                .and_then(Value::as_str)
                .expect("owner instance id")
                .to_string(),
        );
    }
    owner_ids.sort();
    owner_ids.dedup();
    assert_eq!(
        owner_ids.len(),
        1,
        "all workers should register with one daemon"
    );

    let status = env.daemon_status();
    status.assert_success("daemon status after herd");
    let status_json = status.json("daemon status after herd");
    assert_eq!(
        status_json.get("instance_id").and_then(Value::as_str),
        Some(owner_ids[0].as_str())
    );
    let cap = env.cap_json();
    assert_eq!(
        cap.get("instance_id").and_then(Value::as_str),
        Some(owner_ids[0].as_str())
    );
    assert_ne!(env.daemon_pid(), std::process::id());
}

#[test]
fn real_process_02_second_instance_and_store_lock() {
    let first = ProcessEnv::new("real02-a");
    first.attach("real02-a-session", "addr:real02-a");

    let second_serve = first.run(["daemon", "serve"], Duration::from_secs(3));
    second_serve.assert_failure("second daemon serve");
    assert!(
        !second_serve.timed_out,
        "second daemon serve should fail closed quickly"
    );
    assert_text_contains_any(
        &format!("{} {}", second_serve.stdout, second_serve.stderr),
        &["already", "live", "exists", "busy", "first instance"],
        "second daemon refusal",
    );

    let second =
        ProcessEnv::with_shared_store("real02-b", first.db.clone(), first.state_dir.clone());
    let locked = second.run_with_session(
        "real02-b-session",
        [
            "--json",
            "--address",
            "addr:real02-b",
            "attach",
            "--session",
            "real02-b-session",
            "--description",
            "store lock contender",
        ],
        Duration::from_secs(5),
    );
    locked.assert_failure("second config root same sqlite store");
    assert!(
        !locked.timed_out,
        "store-lock contention must fail closed rather than deadlock"
    );
    assert_text_contains_any(
        &format!("{} {}", locked.stdout, locked.stderr),
        &[
            "cannot acquire store lock",
            "lock",
            "another instance",
            "unsupported",
        ],
        "canonical store lock refusal",
    );
    second.stop_daemon_best_effort();
}

#[test]
fn real_process_14_os_trust_same_user_and_prebound() {
    let env = ProcessEnv::new("real14");
    let attached = env.attach("real14-session", "addr:real14");
    let owner = attached
        .get("owner_instance_id")
        .and_then(Value::as_str)
        .expect("owner instance");

    let status = env.daemon_status();
    status.assert_success("same-user daemon status");
    let status_json = status.json("same-user daemon status");
    assert_eq!(
        status_json.get("instance_id").and_then(Value::as_str),
        Some(owner)
    );
    env.stop_daemon_best_effort();

    #[cfg(target_os = "linux")]
    assert_hostile_prebound_endpoint_rejected_before_hello(&env);

    #[cfg(not(target_os = "linux"))]
    {
        // Different-OS-user and hostile pre-bound endpoint negatives depend on OS-specific
        // peer-credential primitives. Linux exercises the real pre-bound Unix socket path above;
        // Windows named-pipe owner/auth is covered here by the real same-user status path and by
        // the existing daemon helper tests for first-instance and verifier-before-Hello behavior.
    }
}

#[test]
fn real_process_idle_wait_timeout_is_not_hung() {
    let env = ProcessEnv::new("real-idle-wait");
    let session = "real-idle-wait-session";
    let address = "addr:real-idle-wait";
    env.attach(session, address);

    let idle = env.run_with_session(
        session,
        [
            "--json",
            "--address",
            address,
            "wait",
            "--session",
            session,
            "--timeout-ms",
            "250",
            "--hang-ms",
            "25",
        ],
        Duration::from_secs(3),
    );
    assert_eq!(
        idle.code,
        Some(2),
        "idle wait should timeout, not report daemon HUNG: stdout={} stderr={}",
        idle.stdout,
        idle.stderr
    );
    assert!(
        idle.stderr.contains("idle-timeout"),
        "timeout stderr should be explicit: {}",
        idle.stderr
    );
}

#[test]
fn real_process_wait_out_dir_writes_artifacts() {
    let env = ProcessEnv::new("real-out-dir");
    let session = "real-out-dir-session";
    let address = "addr:real-out-dir";
    env.attach(session, address);

    let out_dir = env.root.join("wait-out");
    let idle = env.run_with_session(
        session,
        [
            "--json",
            "--address",
            address,
            "wait",
            "--session",
            session,
            "--timeout-ms",
            "250",
            "--hang-ms",
            "25",
            "--out-dir",
            out_dir.to_str().expect("out dir is utf8"),
        ],
        Duration::from_secs(3),
    );
    assert_eq!(
        idle.code,
        Some(2),
        "idle wait should timeout: stdout={} stderr={}",
        idle.stdout,
        idle.stderr
    );

    let exit_code =
        std::fs::read_to_string(out_dir.join("exit.code")).expect("exit.code artifact written");
    assert_eq!(
        exit_code.trim(),
        "2",
        "artifact exit.code reflects the real wait exit, not the launcher"
    );

    let status: Value = serde_json::from_str(
        &std::fs::read_to_string(out_dir.join("status.json"))
            .expect("status.json artifact written"),
    )
    .expect("status.json parses");
    assert_eq!(
        status.get("outcome").and_then(Value::as_str),
        Some("idle-timeout")
    );
    assert_eq!(status.get("exit_code").and_then(Value::as_i64), Some(2));
    assert!(
        !out_dir.join("message.json").exists(),
        "no message.json on idle timeout"
    );
}

#[test]
fn real_process_attach_with_redirected_output_returns_after_daemon_spawn() {
    let env = ProcessEnv::new("real-attach-redirect");
    let session = "real-attach-redirect-session";
    let address = "addr:real-attach-redirect";
    let out = env.run_with_session(
        session,
        [
            "--json",
            "--address",
            address,
            "attach",
            "--session",
            session,
            "--description",
            "redirected attach should return",
        ],
        Duration::from_secs(8),
    );
    out.assert_success("redirected attach");
    let attached = out.json("redirected attach");
    assert_eq!(
        attached.get("address").and_then(Value::as_str),
        Some(address)
    );
}

#[test]
fn real_process_wait_without_daemon_does_not_spawn() {
    let env = ProcessEnv::new("real-wait-no-spawn");
    let session = "real-wait-no-spawn-session";
    let address = "addr:real-wait-no-spawn";
    let out_dir = env.root.join("wait-no-spawn");

    let wait = env.run_with_session(
        session,
        [
            "--json",
            "--address",
            address,
            "wait",
            "--session",
            session,
            "--timeout-ms",
            "250",
            "--out-dir",
            out_dir.to_str().expect("out dir is utf8"),
        ],
        Duration::from_secs(5),
    );
    assert_eq!(
        wait.code,
        Some(3),
        "wait should report daemon-gone rather than spawning a daemon: stdout={} stderr={}",
        wait.stdout,
        wait.stderr
    );
    assert!(
        wait.stderr.contains("daemon-gone"),
        "stderr should tell the agent to re-attach/recover: {}",
        wait.stderr
    );
    assert!(
        env.wait_until_not_running(Duration::from_millis(500)),
        "wait without a daemon must not auto-spawn one"
    );
}

#[test]
fn real_process_send_without_daemon_does_not_spawn() {
    let env = ProcessEnv::new("real-send-no-spawn");
    let sender = "real-send-no-spawn-session";
    let sender_addr = "addr:real-send-no-spawn-sender";
    let receiver_addr = "addr:real-send-no-spawn-receiver";

    let sent = env.run_with_session(
        sender,
        [
            "--json",
            "--address",
            sender_addr,
            "send",
            "--session",
            sender,
            "--from",
            sender_addr,
            "--to",
            receiver_addr,
            "--subject",
            "no daemon",
            "--body",
            "should not spawn",
        ],
        Duration::from_secs(5),
    );
    sent.assert_failure("send without daemon");
    assert!(
        env.wait_until_not_running(Duration::from_millis(500)),
        "send without a daemon must not auto-spawn one"
    );
}

#[test]
fn real_process_wait_out_dir_delivers_message_artifact() {
    let env = ProcessEnv::new("real-out-dir-msg");
    let receiver = "real-out-dir-msg-receiver";
    let sender = "real-out-dir-msg-sender";
    let receiver_addr = "addr:real-out-dir-msg-receiver";
    let sender_addr = "addr:real-out-dir-msg-sender";
    let body = "delivered through --out-dir artifacts";

    env.attach(receiver, receiver_addr);
    env.attach(sender, sender_addr);
    let sent = env.run_with_session(
        sender,
        [
            "--json",
            "--address",
            sender_addr,
            "send",
            "--session",
            sender,
            "--from",
            sender_addr,
            "--to",
            receiver_addr,
            "--subject",
            "out-dir delivery",
            "--body",
            body,
        ],
        Duration::from_secs(5),
    );
    sent.assert_success("send before out-dir wait");

    let out_dir = env.root.join("wait-out-msg");
    let delivered = env.run_with_session(
        receiver,
        [
            "--json",
            "--address",
            receiver_addr,
            "wait",
            "--session",
            receiver,
            "--timeout-ms",
            "4000",
            "--hang-ms",
            "1000",
            "--out-dir",
            out_dir.to_str().expect("out dir is utf8"),
        ],
        Duration::from_secs(6),
    );
    assert_eq!(
        delivered.code,
        Some(0),
        "buffered message should deliver: stdout={} stderr={}",
        delivered.stdout,
        delivered.stderr
    );

    assert_eq!(
        std::fs::read_to_string(out_dir.join("exit.code"))
            .expect("exit.code artifact written")
            .trim(),
        "0"
    );
    let message: Value = serde_json::from_str(
        &std::fs::read_to_string(out_dir.join("message.json"))
            .expect("message.json artifact written"),
    )
    .expect("message.json parses");
    assert_eq!(message.get("body").and_then(Value::as_str), Some(body));
    assert_eq!(
        message.get("to").and_then(Value::as_str),
        Some(receiver_addr)
    );

    let status: Value = serde_json::from_str(
        &std::fs::read_to_string(out_dir.join("status.json"))
            .expect("status.json artifact written"),
    )
    .expect("status.json parses");
    assert_eq!(
        status.get("outcome").and_then(Value::as_str),
        Some("message")
    );
    let delivery: Value = serde_json::from_str(
        &std::fs::read_to_string(out_dir.join("delivery.json"))
            .expect("delivery.json artifact written"),
    )
    .expect("delivery.json parses");
    assert_eq!(
        delivery
            .get("message")
            .and_then(|m| m.get("body"))
            .and_then(Value::as_str),
        Some(body)
    );
    assert_eq!(
        delivery
            .get("delivery")
            .and_then(|d| d.get("delivery_role"))
            .and_then(Value::as_str),
        Some("to")
    );
}

#[test]
fn real_process_wait_min_attention_delivers_interrupt_and_leaves_background() {
    let env = ProcessEnv::new("real-min-attention");
    let receiver = "real-min-attention-receiver";
    let sender = "real-min-attention-sender";
    let receiver_addr = "addr:real-min-attention-receiver";
    let sender_addr = "addr:real-min-attention-sender";

    env.attach(receiver, receiver_addr);
    env.attach(sender, sender_addr);
    let background = env.run_with_session(
        sender,
        [
            "--json",
            "--address",
            sender_addr,
            "send",
            "--session",
            sender,
            "--from",
            sender_addr,
            "--to",
            receiver_addr,
            "--subject",
            "background",
            "--body",
            "background body",
        ],
        Duration::from_secs(5),
    );
    background.assert_success("send background");
    let interrupt = env.run_with_session(
        sender,
        [
            "--json",
            "--address",
            sender_addr,
            "send",
            "--session",
            sender,
            "--from",
            sender_addr,
            "--to",
            receiver_addr,
            "--subject",
            "interrupt",
            "--body",
            "interrupt body",
            "--attention",
            "interrupt",
        ],
        Duration::from_secs(5),
    );
    interrupt.assert_success("send interrupt");

    let out_dir = env.root.join("wait-min-attention");
    let filtered = env.run_with_session(
        receiver,
        [
            "--json",
            "--address",
            receiver_addr,
            "wait",
            "--session",
            receiver,
            "--min-attention",
            "interrupt",
            "--timeout-ms",
            "4000",
            "--out-dir",
            out_dir.to_str().expect("out dir is utf8"),
        ],
        Duration::from_secs(6),
    );
    assert_eq!(
        filtered.code,
        Some(0),
        "interrupt-eligible message should deliver: stdout={} stderr={}",
        filtered.stdout,
        filtered.stderr
    );
    let delivered: Value = serde_json::from_str(
        &std::fs::read_to_string(out_dir.join("message.json"))
            .expect("message.json artifact written"),
    )
    .expect("message.json parses");
    assert_eq!(
        delivered.get("subject").and_then(Value::as_str),
        Some("interrupt")
    );
    let id = message_id(&delivered);
    let ack = env.run_with_session(
        receiver,
        [
            "--json",
            "--address",
            receiver_addr,
            "ack",
            "--session",
            receiver,
            "--id",
            &id.to_string(),
        ],
        Duration::from_secs(5),
    );
    ack.assert_success("ack interrupt");

    let background_delivered = wait_for_message(&env, receiver, receiver_addr, "background body");
    assert_eq!(
        background_delivered.get("subject").and_then(Value::as_str),
        Some("background")
    );
}

#[test]
fn real_process_delivery_role_metadata_for_primary_and_cc() {
    let env = ProcessEnv::new("real-delivery-role");
    let sender = "real-delivery-role-sender";
    let primary = "real-delivery-role-primary";
    let cc = "real-delivery-role-cc";
    let sender_addr = "addr:real-delivery-role-sender";
    let primary_addr = "addr:real-delivery-role-primary";
    let cc_addr = "addr:real-delivery-role-cc";

    env.attach(sender, sender_addr);
    env.attach(primary, primary_addr);
    env.attach(cc, cc_addr);
    let sent = env.run_with_session(
        sender,
        [
            "--json",
            "--address",
            sender_addr,
            "send",
            "--session",
            sender,
            "--from",
            sender_addr,
            "--to",
            primary_addr,
            "--cc",
            cc_addr,
            "--subject",
            "role metadata",
            "--body",
            "role body",
            "--requires-disposition",
        ],
        Duration::from_secs(5),
    );
    sent.assert_success("send role metadata");
    let sent_json = sent.json("send role metadata");
    let id = message_id(&sent_json);

    let cc_inbox = env.run_with_session(
        cc,
        ["--json", "--address", cc_addr, "inbox", "--all"],
        Duration::from_secs(5),
    );
    cc_inbox.assert_success("cc inbox");
    let cc_inbox_json = cc_inbox.json("cc inbox");
    let cc_item = cc_inbox_json
        .get("items")
        .and_then(Value::as_array)
        .unwrap()
        .iter()
        .find(|item| item.get("id").and_then(Value::as_i64) == Some(id))
        .expect("cc inbox item");
    assert_eq!(
        cc_item.get("delivery_role").and_then(Value::as_str),
        Some("cc")
    );
    assert_eq!(
        cc_item
            .get("requires_disposition_for_current_recipient")
            .and_then(Value::as_bool),
        Some(false)
    );

    let cc_read = env.run_with_session(
        cc,
        [
            "--json",
            "--address",
            cc_addr,
            "read",
            "--id",
            &id.to_string(),
        ],
        Duration::from_secs(5),
    );
    cc_read.assert_success("cc read");
    let cc_read_json = cc_read.json("cc read");
    assert_eq!(
        cc_read_json
            .get("delivery")
            .and_then(|d| d.get("delivery_role"))
            .and_then(Value::as_str),
        Some("cc")
    );

    let cc_wait = env.run_with_session(
        cc,
        [
            "--json",
            "--address",
            cc_addr,
            "wait",
            "--session",
            cc,
            "--timeout-ms",
            "250",
            "--hang-ms",
            "1000",
        ],
        Duration::from_secs(3),
    );
    assert_eq!(
        cc_wait.code,
        Some(2),
        "CC observer should not be woken/wedged by visibility-only delivery: stdout={} stderr={}",
        cc_wait.stdout,
        cc_wait.stderr
    );

    let primary_wait = wait_for_message(&env, primary, primary_addr, "role body");
    assert_eq!(
        primary_wait.get("delivery_role").and_then(Value::as_str),
        Some("to")
    );
    assert_eq!(
        primary_wait
            .get("requires_disposition_for_current_recipient")
            .and_then(Value::as_bool),
        Some(true)
    );
}

#[test]
fn real_process_send_accepts_repeated_cc_flags() {
    let env = ProcessEnv::new("real-repeat-cc");
    let sender = "real-repeat-cc-sender";
    let primary = "real-repeat-cc-primary";
    let cc_one = "real-repeat-cc-one";
    let cc_two = "real-repeat-cc-two";
    let sender_addr = "addr:real-repeat-cc-sender";
    let primary_addr = "addr:real-repeat-cc-primary";
    let cc_one_addr = "addr:real-repeat-cc-one";
    let cc_two_addr = "addr:real-repeat-cc-two";

    env.attach(sender, sender_addr);
    env.attach(primary, primary_addr);
    env.attach(cc_one, cc_one_addr);
    env.attach(cc_two, cc_two_addr);
    let sent = env.run_with_session(
        sender,
        [
            "--json",
            "--address",
            sender_addr,
            "send",
            "--session",
            sender,
            "--from",
            sender_addr,
            "--to",
            primary_addr,
            "--cc",
            cc_one_addr,
            "--cc",
            cc_two_addr,
            "--subject",
            "repeat cc",
            "--body",
            "repeat cc body",
        ],
        Duration::from_secs(5),
    );
    sent.assert_success("send repeated cc");
    let id = message_id(&sent.json("send repeated cc"));

    for (session, address) in [(cc_one, cc_one_addr), (cc_two, cc_two_addr)] {
        let inbox = env.run_with_session(
            session,
            ["--json", "--address", address, "inbox", "--all"],
            Duration::from_secs(5),
        );
        inbox.assert_success("cc inbox repeated");
        let inbox_json = inbox.json("cc inbox repeated");
        assert!(
            inbox_json
                .get("items")
                .and_then(Value::as_array)
                .unwrap()
                .iter()
                .any(|item| {
                    item.get("id").and_then(Value::as_i64) == Some(id)
                        && item.get("delivery_role").and_then(Value::as_str) == Some("cc")
                }),
            "cc recipient {address} should see repeated-cc message: {inbox_json}"
        );
    }
}

#[test]
fn real_process_reply_accepts_cc_and_preserves_thread() {
    let env = ProcessEnv::new("real-reply-cc");
    let origin = "real-reply-cc-origin";
    let replier = "real-reply-cc-replier";
    let observer = "real-reply-cc-observer";
    let origin_addr = "addr:real-reply-cc-origin";
    let replier_addr = "addr:real-reply-cc-replier";
    let observer_addr = "addr:real-reply-cc-observer";

    env.attach(origin, origin_addr);
    env.attach(replier, replier_addr);
    env.attach(observer, observer_addr);
    let sent = env.run_with_session(
        origin,
        [
            "--json",
            "--address",
            origin_addr,
            "send",
            "--session",
            origin,
            "--from",
            origin_addr,
            "--to",
            replier_addr,
            "--subject",
            "root",
            "--body",
            "root body",
        ],
        Duration::from_secs(5),
    );
    sent.assert_success("send root for reply");
    let root = sent.json("send root for reply");
    let root_id = message_id(&root);
    let root_thread = root
        .get("thread_id")
        .and_then(Value::as_i64)
        .expect("root thread id");

    let reply = env.run_with_session(
        replier,
        [
            "--json",
            "--address",
            replier_addr,
            "reply",
            "--session",
            replier,
            "--from",
            replier_addr,
            "--to-message",
            &root_id.to_string(),
            "--body",
            "reply body",
            "--cc",
            observer_addr,
        ],
        Duration::from_secs(5),
    );
    reply.assert_success("reply with cc");
    let reply_json = reply.json("reply with cc");
    let reply_id = message_id(&reply_json);
    assert_eq!(
        reply_json.get("parent_id").and_then(Value::as_i64),
        Some(root_id)
    );
    assert_eq!(
        reply_json.get("thread_id").and_then(Value::as_i64),
        Some(root_thread)
    );

    let observer_inbox = env.run_with_session(
        observer,
        ["--json", "--address", observer_addr, "inbox", "--all"],
        Duration::from_secs(5),
    );
    observer_inbox.assert_success("observer inbox for reply cc");
    let observer_json = observer_inbox.json("observer inbox for reply cc");
    assert!(
        observer_json
            .get("items")
            .and_then(Value::as_array)
            .unwrap()
            .iter()
            .any(|item| {
                item.get("id").and_then(Value::as_i64) == Some(reply_id)
                    && item.get("delivery_role").and_then(Value::as_str) == Some("cc")
            }),
        "observer should see cc reply in thread: {observer_json}"
    );
}

#[test]
fn real_process_disposition_defaults_to_current_recipient_not_primary() {
    let env = ProcessEnv::new("real-disposition-recipient");
    let sender = "real-disposition-recipient-sender";
    let primary = "real-disposition-recipient-primary";
    let cc = "real-disposition-recipient-cc";
    let sender_addr = "addr:real-disposition-recipient-sender";
    let primary_addr = "addr:real-disposition-recipient-primary";
    let cc_addr = "addr:real-disposition-recipient-cc";

    env.attach(sender, sender_addr);
    env.attach(primary, primary_addr);
    env.attach(cc, cc_addr);
    let sent = env.run_with_session(
        sender,
        [
            "--json",
            "--address",
            sender_addr,
            "send",
            "--session",
            sender,
            "--from",
            sender_addr,
            "--to",
            primary_addr,
            "--cc",
            cc_addr,
            "--subject",
            "recipient safety",
            "--body",
            "recipient body",
            "--requires-disposition",
        ],
        Duration::from_secs(5),
    );
    sent.assert_success("send recipient safety");
    let id = message_id(&sent.json("send recipient safety"));

    let no_address = env.run_with_session(
        cc,
        ["--json", "handle", "--id", &id.to_string()],
        Duration::from_secs(5),
    );
    no_address.assert_failure("handle without address should fail");
    assert!(
        no_address.stderr.contains("--address") || no_address.stderr.contains("--recipient"),
        "failure should tell caller how to disambiguate: {}",
        no_address.stderr
    );

    let cc_handle = env.run_with_session(
        cc,
        [
            "--json",
            "--address",
            cc_addr,
            "handle",
            "--id",
            &id.to_string(),
        ],
        Duration::from_secs(5),
    );
    cc_handle.assert_success("cc handle");
    let cc_handle_json = cc_handle.json("cc handle");
    assert_eq!(
        cc_handle_json.get("recipient").and_then(Value::as_str),
        Some(cc_addr),
        "cc handle should not default to primary recipient"
    );

    let primary_inbox = env.run_with_session(
        primary,
        ["--json", "--address", primary_addr, "inbox", "--all"],
        Duration::from_secs(5),
    );
    primary_inbox.assert_success("primary inbox after cc handle");
    let primary_inbox_json = primary_inbox.json("primary inbox after cc handle");
    let primary_item = primary_inbox_json
        .get("items")
        .and_then(Value::as_array)
        .unwrap()
        .iter()
        .find(|item| item.get("id").and_then(Value::as_i64) == Some(id))
        .expect("primary inbox item");
    assert_eq!(
        primary_item.get("delivery_role").and_then(Value::as_str),
        Some("to")
    );
    assert_eq!(
        primary_item
            .get("requires_disposition_for_current_recipient")
            .and_then(Value::as_bool),
        Some(true)
    );
    assert_eq!(
        primary_item.get("latest_disposition"),
        Some(&Value::Null),
        "cc disposition must not clobber primary disposition"
    );
}

#[test]
fn real_process_station_stop_drains_waiter_and_preserves_next_message() {
    let env = ProcessEnv::new("real-station-stop");
    let receiver = "real-station-stop-receiver";
    let sender = "real-station-stop-sender";
    let next_receiver = "real-station-stop-next";
    let receiver_addr = "addr:real-station-stop-receiver";
    let sender_addr = "addr:real-station-stop-sender";
    env.attach(receiver, receiver_addr);
    env.attach(sender, sender_addr);

    let out_dir = env.root.join("station-stop-wait");
    let mut wait_cmd = env.command_with_session(receiver);
    wait_cmd
        .args([
            "--json",
            "--address",
            receiver_addr,
            "wait",
            "--session",
            receiver,
            "--timeout-ms",
            "10000",
            "--out-dir",
            out_dir.to_str().expect("out dir is utf8"),
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let waiter = wait_cmd.spawn().expect("spawn waiter");
    wait_until_path_exists(&out_dir.join("wait.pid"), Duration::from_secs(3));
    let waiter_pid: u32 = std::fs::read_to_string(out_dir.join("wait.pid"))
        .expect("wait.pid written")
        .trim()
        .parse()
        .expect("wait.pid parses");
    assert!(waiter_pid > 0);

    let status = env.daemon_status().json("daemon status with waiter");
    assert!(
        status
            .get("live_waiters")
            .and_then(Value::as_array)
            .unwrap()
            .iter()
            .any(|w| w.get("pid").and_then(Value::as_u64) == Some(waiter_pid as u64)),
        "daemon status should list waiter pid {waiter_pid}: {status}"
    );

    let stopped = env.run_with_session(
        receiver,
        [
            "--json",
            "--address",
            receiver_addr,
            "station",
            "stop",
            "--session",
            receiver,
            "--wait-grace-ms",
            "3000",
        ],
        Duration::from_secs(6),
    );
    stopped.assert_success("station stop");
    let stopped_json = stopped.json("station stop");
    assert_eq!(
        stopped_json.get("waiters_after").and_then(Value::as_u64),
        Some(0)
    );

    let (wait_code, wait_timed_out) = wait_status_with_timeout(waiter, Duration::from_secs(3));
    assert_eq!(wait_code, Some(5), "waiter should exit presence-ended");
    assert!(
        !wait_timed_out,
        "station stop should not leave waiter running"
    );

    let body = "message after station stop";
    let sent = env.run_with_session(
        sender,
        [
            "--json",
            "--address",
            sender_addr,
            "send",
            "--session",
            sender,
            "--from",
            sender_addr,
            "--to",
            receiver_addr,
            "--subject",
            "after station stop",
            "--body",
            body,
        ],
        Duration::from_secs(5),
    );
    sent.assert_success("send after station stop");
    let sent_json = sent.json("send after station stop");
    assert_eq!(
        sent_json.get("occupied").and_then(Value::as_bool),
        Some(false),
        "stopped station should be unoccupied: {sent_json}"
    );

    env.attach(next_receiver, receiver_addr);
    let delivered = wait_for_message(&env, next_receiver, receiver_addr, body);
    assert_eq!(delivered.get("body").and_then(Value::as_str), Some(body));
}

#[test]
fn real_process_killed_waiter_leaves_daemon_authored_abnormal_status() {
    let env = ProcessEnv::new("real-abnormal-waiter-status");
    let receiver = "real-abnormal-waiter-status-receiver";
    let address = "addr:real-abnormal-waiter-status";
    env.attach(receiver, address);

    let out_dir = env.root.join("abnormal-wait-out");
    let mut wait_cmd = env.command_with_session(receiver);
    wait_cmd
        .args([
            "--json",
            "--address",
            address,
            "wait",
            "--session",
            receiver,
            "--timeout-ms",
            "30000",
            "--out-dir",
            out_dir.to_str().expect("out dir is utf8"),
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let waiter = wait_cmd.spawn().expect("spawn abnormal waiter");
    wait_until_path_exists(&out_dir.join("wait.pid"), Duration::from_secs(3));
    let waiter_pid: u32 = std::fs::read_to_string(out_dir.join("wait.pid"))
        .expect("wait.pid written")
        .trim()
        .parse()
        .expect("wait.pid parses");
    wait_until_daemon_lists_waiter(&env, waiter_pid, Duration::from_secs(3));
    terminate_pid(waiter_pid);
    let (_wait_code, wait_timed_out) = wait_status_with_timeout(waiter, Duration::from_secs(3));
    assert!(
        !wait_timed_out,
        "killed waiter process should exit promptly"
    );

    std::thread::sleep(Duration::from_secs(6));

    let status = env.run_with_session(
        receiver,
        [
            "--json",
            "--address",
            address,
            "station",
            "status",
            "--session",
            receiver,
            "--all-sessions",
        ],
        Duration::from_secs(5),
    );
    status.assert_success("station status after killed waiter");
    let status_json = status.json("station status after killed waiter");
    let station = &status_json
        .get("stations")
        .and_then(Value::as_array)
        .unwrap()[0];
    assert_eq!(
        station.get("last_waiter_outcome").and_then(Value::as_str),
        Some("abnormal-exit"),
        "daemon should author terminal abnormal-exit status: {status_json}"
    );
    assert_eq!(
        station.get("last_waiter_pid").and_then(Value::as_u64),
        Some(waiter_pid as u64)
    );
    assert!(
        !out_dir.join("exit.code").exists(),
        "killed waiter should not have to write exit.code for daemon status to be useful"
    );
}

#[test]
fn real_process_status_and_address_list_agree_after_attach() {
    let env = ProcessEnv::new("real-status");
    let session = "real-status-session";
    let address = "addr:real-status";
    env.attach(session, address);

    let daemon_status = env.daemon_status().json("daemon status");
    assert!(
        !daemon_status
            .get("stores")
            .and_then(Value::as_array)
            .unwrap()
            .is_empty(),
        "daemon status should include stores: {daemon_status}"
    );
    assert!(
        daemon_status
            .get("members")
            .and_then(Value::as_array)
            .unwrap()
            .iter()
            .any(|m| m.get("address").and_then(Value::as_str) == Some(address)),
        "daemon status should include attached member: {daemon_status}"
    );

    let status = env.run_with_session(
        session,
        ["--json", "--address", address, "status"],
        Duration::from_secs(5),
    );
    status.assert_success("status --address");
    let status_json = status.json("status --address");
    assert_eq!(
        status_json
            .get("occupancy")
            .and_then(|o| o.get("occupied"))
            .and_then(Value::as_bool),
        Some(true),
        "status --address should report occupied: {status_json}"
    );
    assert!(
        !status_json
            .get("daemon_members")
            .and_then(Value::as_array)
            .unwrap()
            .is_empty(),
        "status --address should include daemon members: {status_json}"
    );

    let list = env.run(
        ["--json", "address", "list", "--match", "real-status"],
        Duration::from_secs(5),
    );
    list.assert_success("address list");
    let list_json = list.json("address list");
    let listed = list_json
        .get("addresses")
        .and_then(Value::as_array)
        .unwrap()
        .iter()
        .find(|entry| entry.get("address").and_then(Value::as_str) == Some(address))
        .expect("address listed");
    assert_eq!(
        listed.get("occupied").and_then(Value::as_bool),
        Some(true),
        "address list should agree with status --address: {list_json}"
    );
}

#[test]
fn real_process_status_reports_unattended_with_backlog() {
    let env = ProcessEnv::new("real-health-backlog");
    let receiver = "real-health-backlog-receiver";
    let sender = "real-health-backlog-sender";
    let receiver_addr = "addr:real-health-backlog-receiver";
    let sender_addr = "addr:real-health-backlog-sender";
    env.attach(receiver, receiver_addr);
    env.attach(sender, sender_addr);

    let sent = env.run_with_session(
        sender,
        [
            "--json",
            "--address",
            sender_addr,
            "send",
            "--session",
            sender,
            "--from",
            sender_addr,
            "--to",
            receiver_addr,
            "--subject",
            "health backlog",
            "--body",
            "queued without waiter",
        ],
        Duration::from_secs(5),
    );
    sent.assert_success("send backlog message");

    let status = env.run_with_session(
        receiver,
        ["--json", "--address", receiver_addr, "status"],
        Duration::from_secs(5),
    );
    status.assert_success("status --address backlog");
    let status_json = status.json("status --address backlog");
    assert_eq!(
        status_json.get("station_health").and_then(Value::as_str),
        Some("unattended_with_backlog"),
        "status should flag unattended backlog: {status_json}"
    );
    assert_eq!(
        status_json
            .get("pending_unconsumed_count")
            .and_then(Value::as_i64),
        Some(1)
    );
    assert_eq!(
        status_json
            .get("live_waiters_count")
            .and_then(Value::as_u64),
        Some(0)
    );
}

#[test]
fn real_process_station_status_filters_by_session_and_reports_waiter_state() {
    let env = ProcessEnv::new("real-station-status");
    let session = "real-station-status-session";
    let address = "addr:real-station-status";
    env.attach(session, address);

    let initial = env.run_with_session(
        session,
        ["--json", "station", "status", "--session", session],
        Duration::from_secs(5),
    );
    initial.assert_success("station status initial");
    let initial_json = initial.json("station status initial");
    assert_eq!(initial_json.get("count").and_then(Value::as_u64), Some(1));
    assert_eq!(
        initial_json
            .get("stations")
            .and_then(Value::as_array)
            .unwrap()[0]
            .get("station_health")
            .and_then(Value::as_str),
        Some("unattended")
    );

    let out_dir = env.root.join("station-status-wait");
    let mut wait_cmd = env.command_with_session(session);
    wait_cmd
        .args([
            "--json",
            "--address",
            address,
            "wait",
            "--session",
            session,
            "--timeout-ms",
            "10000",
            "--out-dir",
            out_dir.to_str().expect("out dir is utf8"),
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let waiter = wait_cmd.spawn().expect("spawn station status waiter");
    wait_until_path_exists(&out_dir.join("wait.pid"), Duration::from_secs(3));

    let armed = env.run_with_session(
        session,
        ["--json", "station", "status", "--session", session],
        Duration::from_secs(5),
    );
    armed.assert_success("station status armed");
    let armed_json = armed.json("station status armed");
    let station = &armed_json
        .get("stations")
        .and_then(Value::as_array)
        .unwrap()[0];
    assert_eq!(
        station.get("station_health").and_then(Value::as_str),
        Some("armed")
    );
    assert_eq!(
        station.get("live_waiters_count").and_then(Value::as_u64),
        Some(1)
    );

    let stopped = env.run_with_session(
        session,
        [
            "--json",
            "--address",
            address,
            "station",
            "stop",
            "--session",
            session,
            "--wait-grace-ms",
            "3000",
        ],
        Duration::from_secs(5),
    );
    stopped.assert_success("station stop after status");
    let _ = wait_status_with_timeout(waiter, Duration::from_secs(3));
}

#[test]
fn real_process_station_status_all_sessions_exposes_foreign_station() {
    let env = ProcessEnv::new("real-station-all-sessions");
    let foreign_session = "real-station-all-sessions-foreign";
    let observer_session = "real-station-all-sessions-observer";
    let address = "addr:real-station-all-sessions";
    env.attach(foreign_session, address);

    let scoped = env.run_with_session(
        observer_session,
        [
            "--json",
            "--address",
            address,
            "station",
            "status",
            "--session",
            observer_session,
        ],
        Duration::from_secs(5),
    );
    scoped.assert_success("station status scoped foreign");
    let scoped_json = scoped.json("station status scoped foreign");
    assert_eq!(scoped_json.get("count").and_then(Value::as_u64), Some(0));

    let all = env.run_with_session(
        observer_session,
        [
            "--json",
            "--address",
            address,
            "station",
            "status",
            "--session",
            observer_session,
            "--all-sessions",
        ],
        Duration::from_secs(5),
    );
    all.assert_success("station status all sessions");
    let all_json = all.json("station status all sessions");
    assert_eq!(all_json.get("count").and_then(Value::as_u64), Some(1));
    let station = &all_json.get("stations").and_then(Value::as_array).unwrap()[0];
    assert_eq!(
        station.get("session_id").and_then(Value::as_str),
        Some(foreign_session)
    );
    assert_eq!(
        station.get("foreign_session").and_then(Value::as_bool),
        Some(true)
    );

    let text = env.run_with_session(
        observer_session,
        [
            "--text",
            "--address",
            address,
            "station",
            "status",
            "--session",
            observer_session,
            "--all-sessions",
        ],
        Duration::from_secs(5),
    );
    text.assert_success("station status all sessions text");
    assert!(
        text.stdout.contains("foreign"),
        "text output should mark foreign station: {}",
        text.stdout
    );

    let mut all_cmd = env.command_with_session("unused-session");
    all_cmd
        .env_remove("TELEX_SESSION_ID")
        .env_remove("COPILOT_AGENT_SESSION_ID")
        .args([
            "--json",
            "--address",
            address,
            "station",
            "status",
            "--all-sessions",
        ]);
    let all = run_command_with_capture(all_cmd, &env.root, Duration::from_secs(5));
    all.assert_success("station status all sessions without session env");
    let all_json = all.json("station status all sessions without session env");
    assert_eq!(all_json.get("session_id"), Some(&Value::Null));
    assert_eq!(all_json.get("count").and_then(Value::as_u64), Some(1));
    let station = &all_json.get("stations").and_then(Value::as_array).unwrap()[0];
    assert_eq!(
        station.get("foreign_session").and_then(Value::as_bool),
        Some(true)
    );
}

#[test]
fn real_process_copilot_attach_maps_session_and_loader_pid() {
    let env = ProcessEnv::new("real-copilot-attach");
    let session = "real-copilot-session";
    let address = "addr:real-copilot-attach";
    let mut cmd = env.command_with_session("ignored");
    cmd.env_remove("TELEX_SESSION_ID")
        .env("COPILOT_AGENT_SESSION_ID", session)
        .env("COPILOT_LOADER_PID", std::process::id().to_string())
        .args([
            "--json",
            "--address",
            address,
            "copilot",
            "attach",
            "--description",
            "copilot process test",
        ]);
    let attach = run_command_with_capture(cmd, &env.root, Duration::from_secs(8));
    attach.assert_success("copilot attach");
    let attach_json = attach.json("copilot attach");
    assert_eq!(
        attach_json.get("session_id").and_then(Value::as_str),
        Some(session)
    );

    let status = env.daemon_status();
    status.assert_success("daemon status after copilot attach");
    let status_json = status.json("daemon status after copilot attach");
    let member = status_json
        .get("members")
        .and_then(Value::as_array)
        .unwrap()
        .iter()
        .find(|member| member.get("address").and_then(Value::as_str) == Some(address))
        .expect("copilot-attached member");
    assert_eq!(
        member.get("session_id").and_then(Value::as_str),
        Some(session)
    );
    assert_eq!(
        member.get("watch_pids").and_then(Value::as_array).unwrap()[0]
            .get("pid")
            .and_then(Value::as_u64),
        Some(std::process::id() as u64)
    );
}

#[test]
fn real_process_address_surfaces_report_deaf_and_foreign_state() {
    let env = ProcessEnv::new("real-visibility-surfaces");
    let receiver = "real-visibility-surfaces-receiver";
    let sender = "real-visibility-surfaces-sender";
    let observer = "real-visibility-surfaces-observer";
    let receiver_addr = "addr:real-visibility-surfaces-receiver";
    let sender_addr = "addr:real-visibility-surfaces-sender";
    let mut attach_receiver = env.command_with_session(receiver);
    attach_receiver.env("TELEX_DEAF_WARN_MS", "0").args([
        "--json",
        "--address",
        receiver_addr,
        "attach",
        "--session",
        receiver,
        "--description",
        "process integration test",
    ]);
    let attached_receiver =
        run_command_with_capture(attach_receiver, &env.root, Duration::from_secs(8));
    attached_receiver.assert_success("attach receiver with deaf threshold");
    env.attach(sender, sender_addr);

    let sent = env.run_with_session(
        sender,
        [
            "--json",
            "--address",
            sender_addr,
            "send",
            "--session",
            sender,
            "--from",
            sender_addr,
            "--to",
            receiver_addr,
            "--subject",
            "deaf visibility",
            "--body",
            "queued without waiter",
        ],
        Duration::from_secs(5),
    );
    sent.assert_success("send visibility backlog");

    let mut status_cmd = env.command_with_session(observer);
    status_cmd.env("TELEX_DEAF_WARN_MS", "0").args([
        "--json",
        "--address",
        receiver_addr,
        "status",
    ]);
    let status = run_command_with_capture(status_cmd, &env.root, Duration::from_secs(5));
    status.assert_success("status deaf foreign");
    let status_json = status.json("status deaf foreign");
    assert_eq!(
        status_json.get("station_health").and_then(Value::as_str),
        Some("unattended_with_backlog")
    );
    assert_eq!(
        status_json.get("deaf_warn").and_then(Value::as_bool),
        Some(true)
    );
    assert!(
        !status_json
            .get("foreign_members")
            .and_then(Value::as_array)
            .unwrap()
            .is_empty(),
        "status should expose foreign current-store members: {status_json}"
    );

    let mut show_cmd = env.command_with_session(observer);
    show_cmd.env("TELEX_DEAF_WARN_MS", "0").args([
        "--json",
        "--address",
        receiver_addr,
        "address",
        "show",
    ]);
    let show = run_command_with_capture(show_cmd, &env.root, Duration::from_secs(5));
    show.assert_success("address show deaf foreign");
    let show_json = show.json("address show deaf foreign");
    assert_eq!(
        show_json.get("deaf_warn").and_then(Value::as_bool),
        Some(true)
    );
    assert!(
        !show_json
            .get("foreign_members")
            .and_then(Value::as_array)
            .unwrap()
            .is_empty(),
        "address show should expose foreign members: {show_json}"
    );

    let mut list_cmd = env.command_with_session(observer);
    list_cmd.env("TELEX_DEAF_WARN_MS", "0").args([
        "--json",
        "address",
        "list",
        "--match",
        receiver_addr,
    ]);
    let list = run_command_with_capture(list_cmd, &env.root, Duration::from_secs(5));
    list.assert_success("address list deaf foreign");
    let list_json = list.json("address list deaf foreign");
    let listed = &list_json
        .get("addresses")
        .and_then(Value::as_array)
        .unwrap()[0];
    assert_eq!(listed.get("deaf_warn").and_then(Value::as_bool), Some(true));
    assert!(
        !listed
            .get("foreign_members")
            .and_then(Value::as_array)
            .unwrap()
            .is_empty(),
        "address list should expose foreign members: {list_json}"
    );
}

#[test]
fn real_process_copilot_session_end_is_store_scoped() {
    let env = ProcessEnv::new("real-copilot-session-end-store");
    let session = "real-copilot-store-session";
    let addr_a = "addr:copilot-store-a";
    let addr_b = "addr:copilot-store-b";
    let db_b = env.root.join("store-b.sqlite");
    env.attach(session, addr_a);
    let attach_b = env.run_with_session(
        session,
        [
            "--json",
            "--db",
            db_b.to_str().expect("db_b path"),
            "--address",
            addr_b,
            "attach",
            "--session",
            session,
            "--description",
            "store b",
        ],
        Duration::from_secs(8),
    );
    attach_b.assert_success("attach store b");

    let mut end_cmd = env.command_with_session("ignored");
    end_cmd
        .env_remove("TELEX_SESSION_ID")
        .env("COPILOT_AGENT_SESSION_ID", session)
        .args(["--json", "copilot", "session-end"]);
    let ended = run_command_with_capture(end_cmd, &env.root, Duration::from_secs(8));
    ended.assert_success("copilot session-end");

    let status_a = env.run_with_session(
        session,
        ["--json", "station", "status", "--session", session],
        Duration::from_secs(5),
    );
    status_a.assert_success("station status store a");
    let status_a_json = status_a.json("station status store a");
    assert_eq!(
        status_a_json["stations"][0]
            .get("idle")
            .and_then(Value::as_bool),
        Some(true)
    );

    let status_b = env.run_with_session(
        session,
        [
            "--json",
            "--db",
            db_b.to_str().expect("db_b path"),
            "station",
            "status",
            "--session",
            session,
        ],
        Duration::from_secs(5),
    );
    status_b.assert_success("station status store b");
    let status_b_json = status_b.json("station status store b");
    assert_eq!(
        status_b_json["stations"][0]
            .get("idle")
            .and_then(Value::as_bool),
        Some(false),
        "sessionEnd for store A must not mark store B idle: {status_b_json}"
    );
}

#[test]
fn real_process_status_reports_foreign_members_without_session_env() {
    let env = ProcessEnv::new("real-foreign-no-session");
    let owner = "real-foreign-no-session-owner";
    let address = "addr:real-foreign-no-session";
    env.attach(owner, address);

    let mut status_cmd = env.command_with_session("unused-session");
    status_cmd
        .env_remove("TELEX_SESSION_ID")
        .env_remove("COPILOT_AGENT_SESSION_ID")
        .args(["--json", "--address", address, "status"]);
    let status = run_command_with_capture(status_cmd, &env.root, Duration::from_secs(5));
    status.assert_success("status without session env");
    let status_json = status.json("status without session env");

    assert!(
        !status_json
            .get("foreign_members")
            .and_then(Value::as_array)
            .unwrap()
            .is_empty(),
        "session-less operator status should still expose foreign members: {status_json}"
    );
}

#[test]
fn real_process_copilot_turn_guard_caps_mixed_armed_unarmed_state() {
    let env = ProcessEnv::new("real-copilot-guard-cap");
    let session = "real-copilot-guard-session";
    let armed = "addr:copilot-armed";
    let unarmed = "addr:copilot-unarmed";
    env.attach(session, armed);
    env.attach(session, unarmed);

    let out_dir = env.root.join("copilot-armed-wait");
    let mut wait_cmd = env.command_with_session(session);
    wait_cmd
        .args([
            "--json",
            "--address",
            armed,
            "wait",
            "--session",
            session,
            "--timeout-ms",
            "10000",
            "--out-dir",
            out_dir.to_str().expect("out dir"),
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let waiter = wait_cmd.spawn().expect("spawn armed waiter");
    wait_until_path_exists(&out_dir.join("wait.pid"), Duration::from_secs(3));

    let mut first_cmd = env.command_with_session("ignored");
    first_cmd
        .env_remove("TELEX_SESSION_ID")
        .env("COPILOT_AGENT_SESSION_ID", session)
        .env("TELEX_TURN_GUARD_MAX_NUDGES", "1")
        .args(["--json", "copilot", "turn-guard"]);
    let first = run_command_with_capture(first_cmd, &env.root, Duration::from_secs(5));
    first.assert_success("first copilot turn guard");
    assert_eq!(
        first
            .json("first copilot turn guard")
            .get("decision")
            .and_then(Value::as_str),
        Some("block")
    );

    let mut second_cmd = env.command_with_session("ignored");
    second_cmd
        .env_remove("TELEX_SESSION_ID")
        .env("COPILOT_AGENT_SESSION_ID", session)
        .env("TELEX_TURN_GUARD_MAX_NUDGES", "1")
        .args(["--json", "copilot", "turn-guard"]);
    let second = run_command_with_capture(second_cmd, &env.root, Duration::from_secs(5));
    second.assert_success("second copilot turn guard");
    assert_eq!(
        second
            .json("second copilot turn guard")
            .get("decision")
            .and_then(Value::as_str),
        Some("allow"),
        "same unresolved unarmed station should hit cap even while another station is armed"
    );

    let stopped = env.run_with_session(
        session,
        [
            "--json",
            "--address",
            armed,
            "station",
            "stop",
            "--session",
            session,
        ],
        Duration::from_secs(5),
    );
    stopped.assert_success("stop armed station");
    let _ = wait_status_with_timeout(waiter, Duration::from_secs(3));
}

#[test]
fn real_process_copilot_turn_guard_nudges_delivered_unacked_message() {
    let env = ProcessEnv::new("real-copilot-unacked-guard");
    let session = "real-copilot-unacked-session";
    let address = "addr:copilot-unacked";
    env.attach(session, address);

    let out_dir = env.root.join("copilot-unacked-wait");
    let mut wait_cmd = env.command_with_session(session);
    wait_cmd
        .args([
            "--json",
            "--address",
            address,
            "wait",
            "--session",
            session,
            "--timeout-ms",
            "10000",
            "--out-dir",
            out_dir.to_str().expect("out dir"),
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let waiter = wait_cmd.spawn().expect("spawn unacked waiter");
    wait_until_path_exists(&out_dir.join("wait.pid"), Duration::from_secs(3));

    let send = env.run_with_session(
        session,
        [
            "--json",
            "send",
            "--to",
            address,
            "--body",
            "needs ack",
            "--from",
            address,
        ],
        Duration::from_secs(5),
    );
    send.assert_success("send unacked message");
    let _ = wait_status_with_timeout(waiter, Duration::from_secs(5));
    wait_until_path_exists(&out_dir.join("message.json"), Duration::from_secs(3));

    let mut guard_cmd = env.command_with_session("ignored");
    guard_cmd
        .env_remove("TELEX_SESSION_ID")
        .env("COPILOT_AGENT_SESSION_ID", session)
        .args(["--json", "copilot", "turn-guard"]);
    let guard = run_command_with_capture(guard_cmd, &env.root, Duration::from_secs(5));
    guard.assert_success("copilot turn guard delivered unacked");
    let guard_json = guard.json("copilot turn guard delivered unacked");
    assert_eq!(
        guard_json.get("decision").and_then(Value::as_str),
        Some("block")
    );
    assert!(
        guard_json
            .get("reason")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .contains("delivered/unacked"),
        "guard should mention delivered/unacked work: {guard_json}"
    );
}

#[test]
fn real_process_copilot_turn_guard_daemon_down_fails_open_and_logs() {
    let env = ProcessEnv::new("real-copilot-daemon-down");
    let mut cmd = env.command_with_session("ignored");
    cmd.env_remove("TELEX_SESSION_ID")
        .env("COPILOT_AGENT_SESSION_ID", "daemon-down-session")
        .args(["--json", "copilot", "turn-guard"]);
    let out = run_command_with_capture(cmd, &env.root, Duration::from_secs(5));
    out.assert_success("daemon-down turn guard");
    assert_eq!(
        out.json("daemon-down turn guard")
            .get("decision")
            .and_then(Value::as_str),
        Some("allow")
    );
    let log = env.run_dir.join("copilot").join("hook-events.ndjson");
    let text = std::fs::read_to_string(&log)
        .unwrap_or_else(|e| panic!("reading hook log {}: {e}", log.display()));
    assert!(text.contains("daemon_unavailable"), "hook log was: {text}");
}

#[test]
fn real_process_status_hints_when_address_active_on_other_store() {
    let env = ProcessEnv::new("real-backend-hint");
    let session = "real-backend-hint-session";
    let address = "addr:real-backend-hint";
    env.attach(session, address);

    let wrong_db = env.root.join("wrong-store.sqlite");
    let status = env.run_with_session(
        session,
        [
            "--json",
            "--db",
            wrong_db.to_str().expect("wrong db is utf8"),
            "--address",
            address,
            "status",
        ],
        Duration::from_secs(5),
    );
    status.assert_success("wrong-store status");
    let status_json = status.json("wrong-store status");
    assert_eq!(
        status_json
            .get("occupancy")
            .and_then(|o| o.get("occupied"))
            .and_then(Value::as_bool),
        Some(false),
        "wrong selected store should not report direct occupancy: {status_json}"
    );
    assert!(
        !status_json
            .get("also_active_on")
            .and_then(Value::as_array)
            .unwrap()
            .is_empty(),
        "status should hint active station on another store: {status_json}"
    );
    assert!(
        status_json
            .get("backend_warning")
            .and_then(Value::as_str)
            .is_some(),
        "status should include backend warning: {status_json}"
    );
}

#[cfg(target_os = "linux")]
fn assert_hostile_prebound_endpoint_rejected_before_hello(env: &ProcessEnv) {
    use std::io::Read;
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::net::UnixListener;
    use std::sync::mpsc;

    let local = env.daemon_status();
    local.assert_success("local daemon status");
    let local_json = local.json("local daemon status");
    let endpoint = PathBuf::from(
        local_json
            .get("endpoint")
            .and_then(Value::as_str)
            .expect("local endpoint"),
    );
    let cap_path = PathBuf::from(
        local_json
            .get("cap_path")
            .and_then(Value::as_str)
            .expect("local cap path"),
    );
    let singleton_hash = local_json
        .get("singleton_hash")
        .and_then(Value::as_str)
        .expect("singleton hash");
    let protocol_major = local_json
        .get("protocol_version")
        .and_then(|v| v.get("major"))
        .and_then(Value::as_u64)
        .expect("protocol major");

    let _ = std::fs::remove_file(&endpoint);
    let listener = UnixListener::bind(&endpoint).expect("bind hostile unix listener");
    std::fs::set_permissions(&endpoint, std::fs::Permissions::from_mode(0o600))
        .expect("restrict hostile socket path");

    let fake_cap = serde_json::json!({
        "instance_id": "hostile-instance",
        "admin_cap": "hostile-admin-cap",
        "singleton_hash": singleton_hash,
        "protocol_major": protocol_major,
        "server_pid": std::process::id(),
        "server_start_time": 1,
    });
    std::fs::write(
        &cap_path,
        serde_json::to_vec(&fake_cap).expect("fake cap json"),
    )
    .expect("write fake cap");

    let (tx, rx) = mpsc::channel();
    let acceptor = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept hostile client");
        stream
            .set_read_timeout(Some(Duration::from_millis(500)))
            .expect("set read timeout");
        let mut buf = [0u8; 512];
        let read = match stream.read(&mut buf) {
            Ok(n) => buf[..n].to_vec(),
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                Vec::new()
            }
            Err(e) => panic!("hostile read failed: {e}"),
        };
        tx.send(read).expect("send leaked bytes");
    });

    let out = env.run_with_session(
        "real14-hostile-session",
        [
            "--json",
            "--address",
            "addr:real14-hostile",
            "attach",
            "--session",
            "real14-hostile-session",
        ],
        Duration::from_secs(4),
    );
    out.assert_failure("hostile pre-bound attach");
    assert_text_contains_any(
        &format!("{} {}", out.stdout, out.stderr),
        &[
            "server executable",
            "unauthorized",
            "server authentication failed",
        ],
        "hostile pre-bound rejection",
    );

    let leaked = rx
        .recv_timeout(Duration::from_secs(2))
        .expect("hostile listener observed connection");
    acceptor.join().expect("hostile listener thread");
    assert!(
        leaked.is_empty(),
        "client disclosed Hello/store metadata to hostile endpoint: {:?}",
        String::from_utf8_lossy(&leaked)
    );
}

#[test]
fn real_process_crash_recovery_wait_needsattach_no_loss() {
    let env = ProcessEnv::new("real-crash");
    let receiver = "real-crash-receiver";
    let sender = "real-crash-sender";
    let receiver_addr = "addr:real-crash-receiver";
    let sender_addr = "addr:real-crash-sender";
    let body = "durable body across daemon crash";

    env.attach(receiver, receiver_addr);
    env.attach(sender, sender_addr);
    let sent = env.run_with_session(
        sender,
        [
            "--json",
            "--address",
            sender_addr,
            "send",
            "--session",
            sender,
            "--from",
            sender_addr,
            "--to",
            receiver_addr,
            "--subject",
            "crash recovery",
            "--body",
            body,
        ],
        Duration::from_secs(5),
    );
    sent.assert_success("send before crash");

    let old_pid = env.daemon_pid();
    terminate_pid(old_pid);
    std::thread::sleep(Duration::from_millis(120));

    let no_spawn_wait = env.run_with_session(
        receiver,
        [
            "--json",
            "--address",
            receiver_addr,
            "wait",
            "--session",
            receiver,
            "--timeout-ms",
            "250",
        ],
        Duration::from_secs(3),
    );
    assert_eq!(
        no_spawn_wait.code,
        Some(3),
        "wait should not respawn after crash; attach owns daemon recovery: stdout={} stderr={}",
        no_spawn_wait.stdout,
        no_spawn_wait.stderr
    );

    env.attach(receiver, receiver_addr);
    let delivered = wait_for_message(&env, receiver, receiver_addr, body);
    let id = message_id(&delivered);
    let ack = env.run_with_session(
        receiver,
        [
            "--json",
            "--address",
            receiver_addr,
            "ack",
            "--session",
            receiver,
            "--id",
            &id.to_string(),
        ],
        Duration::from_secs(5),
    );
    ack.assert_success("ack after crash recovery");
    let ack_json = ack.json("ack after crash recovery");
    assert_eq!(
        ack_json
            .get("delivery_outcome")
            .and_then(Value::as_str)
            .unwrap_or_default(),
        "marked"
    );

    let redelivery = env.run_with_session(
        receiver,
        [
            "--json",
            "--address",
            receiver_addr,
            "wait",
            "--session",
            receiver,
            "--timeout-ms",
            "250",
            "--hang-ms",
            "1000",
        ],
        Duration::from_secs(3),
    );
    assert_eq!(
        redelivery.code,
        Some(2),
        "consumed message should not redeliver: stdout={} stderr={}",
        redelivery.stdout,
        redelivery.stderr
    );
}

#[test]
fn real_process_drain_respawn_epoch_advances() {
    let env = ProcessEnv::new("real-epoch");
    let first = env.attach("real-epoch-one", "addr:real-epoch");
    let first_epoch = first
        .get("lease_epoch")
        .and_then(Value::as_i64)
        .expect("first lease epoch");

    env.stop_daemon_best_effort();
    assert!(env.wait_until_not_running(Duration::from_secs(3)));

    let second = env.attach("real-epoch-two", "addr:real-epoch");
    let second_epoch = second
        .get("lease_epoch")
        .and_then(Value::as_i64)
        .expect("second lease epoch");
    assert!(
        second_epoch > first_epoch,
        "lease epoch should advance after drain/respawn: {first_epoch} -> {second_epoch}"
    );
}

#[cfg(unix)]
fn terminate_pid(pid: u32) {
    assert_ne!(pid, std::process::id(), "refusing to kill current process");
    let rc = unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL) };
    if rc != 0 {
        let err = std::io::Error::last_os_error();
        if err.raw_os_error() != Some(libc::ESRCH) {
            panic!("killing daemon pid {pid}: {err}");
        }
    }
}

#[cfg(windows)]
fn terminate_pid(pid: u32) {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{OpenProcess, TerminateProcess, PROCESS_TERMINATE};

    assert_ne!(pid, std::process::id(), "refusing to kill current process");
    let handle = unsafe { OpenProcess(PROCESS_TERMINATE, 0, pid) };
    if handle == 0 {
        panic!(
            "opening daemon pid {pid} for termination: {}",
            std::io::Error::last_os_error()
        );
    }
    let terminated = unsafe { TerminateProcess(handle, 1) };
    if terminated == 0 {
        let err = std::io::Error::last_os_error();
        unsafe {
            CloseHandle(handle);
        }
        panic!("terminating daemon pid {pid}: {err}");
    }
    unsafe {
        CloseHandle(handle);
    }
    std::thread::sleep(Duration::from_millis(150));
}

#[cfg(not(any(unix, windows)))]
fn terminate_pid(_pid: u32) {
    panic!("daemon process termination is not implemented for this platform");
}
