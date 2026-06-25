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
