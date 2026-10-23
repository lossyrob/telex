#![cfg(any(windows, target_os = "linux"))]

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};
use telex_watcher::process::{
    execute, probe_previous, ContainmentIdentity, ContainmentStatus, ProcessFailureKind,
    ProcessSpec, MAX_STDIN_BYTES,
};
use tokio::sync::oneshot;

static ENVIRONMENT: Mutex<()> = Mutex::new(());

struct Isolated {
    root: PathBuf,
    previous: Vec<(String, Option<std::ffi::OsString>)>,
    _lock: MutexGuard<'static, ()>,
}

impl Isolated {
    fn new() -> Self {
        let lock = ENVIRONMENT
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let root = std::env::current_dir()
            .unwrap()
            .join(".watcher-process-tests")
            .join(uuid::Uuid::new_v4().to_string());
        std::fs::create_dir_all(root.join("home")).unwrap();
        std::fs::create_dir_all(root.join("install")).unwrap();
        let mut previous = Vec::new();
        for (name, value) in [
            ("TELEX_HOME", root.join("home").into_os_string()),
            ("TELEX_DB", root.join("registry.sqlite3").into_os_string()),
            ("TELEX_INSTALL_ROOT", root.join("install").into_os_string()),
            ("WATCHER_TEST_ALLOWED", "explicitly inherited".into()),
            ("WATCHER_TEST_AMBIENT", "must not leak".into()),
            ("WATCHER_TEST_TOKEN", "test-only-secret-0123456789".into()),
        ] {
            previous.push((name.to_string(), std::env::var_os(name)));
            std::env::set_var(name, value);
        }
        Self {
            root,
            previous,
            _lock: lock,
        }
    }

    fn spec(&self, mode: &str) -> ProcessSpec {
        ProcessSpec {
            command: vec![fixture().to_string_lossy().into_owned(), mode.into()],
            working_directory: self.root.clone(),
            environment_allowlist: vec![
                "TELEX_HOME".into(),
                "TELEX_DB".into(),
                "TELEX_INSTALL_ROOT".into(),
                "WATCHER_TEST_ALLOWED".into(),
                "WATCHER_TEST_TOKEN".into(),
            ],
            timeout: Duration::from_secs(10),
            containment_directory: self.root.join("containment"),
        }
    }
}

impl Drop for Isolated {
    fn drop(&mut self) {
        for (name, value) in &self.previous {
            match value {
                Some(value) => std::env::set_var(name, value),
                None => std::env::remove_var(name),
            }
        }
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_watcher_process_fixture"))
}

#[tokio::test]
async fn direct_argv_canonical_cwd_environment_and_redaction() {
    let isolated = Isolated::new();
    let mut spec = isolated.spec("emit");
    spec.working_directory.push(".");
    let arguments = [
        "literal ; & | $(no shell)",
        "",
        "spaces \"quotes\" backslash\\",
        "Unicode-\u{2603}",
    ];
    spec.command
        .extend(arguments.iter().map(|arg| (*arg).to_owned()));
    let (_keepalive, cancel) = oneshot::channel();
    let output = execute(&spec, b"{\"request\":true}\n".to_vec(), cancel, |_| async {
        Ok(())
    })
    .await
    .unwrap();
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["arguments"], serde_json::json!(arguments));
    assert_eq!(value["input"], "{\"request\":true}\n");
    assert_eq!(
        value["cwd"],
        serde_json::json!(std::fs::canonicalize(&isolated.root).unwrap())
    );
    assert_eq!(value["allowed"], "explicitly inherited");
    assert!(value["ambient"].is_null());
    assert_eq!(
        Path::new(value["home"].as_str().unwrap()),
        isolated.root.join("home")
    );
    assert!(output.stderr.contains("[REDACTED]"));
    assert!(!output.stderr.contains("test-only-secret"));
    assert_eq!(
        probe_previous(&output.containment).await,
        ContainmentStatus::Ended
    );
}

#[tokio::test]
async fn launch_gate_requires_durable_callback_success() {
    let isolated = Isolated::new();
    let marker = isolated.root.join("executed");
    let mut spec = isolated.spec("marker");
    spec.command.push(marker.to_string_lossy().into_owned());
    let (_keepalive, cancel) = oneshot::channel();
    let error = execute(&spec, Vec::new(), cancel, |identity| {
        let marker = marker.clone();
        async move {
            assert!(!marker.exists(), "detector ran before durable recording");
            assert_eq!(probe_previous(&identity).await, ContainmentStatus::Unproven);
            Err("simulated durable write failure".into())
        }
    })
    .await
    .unwrap_err();
    assert_eq!(error.kind, ProcessFailureKind::ContainmentRecording);
    assert!(!marker.exists());
    assert_eq!(
        probe_previous(error.containment.as_ref().unwrap()).await,
        ContainmentStatus::Ended
    );
}

#[tokio::test]
async fn output_input_and_blocked_stdin_are_bounded() {
    let isolated = Isolated::new();
    for (mode, expected) in [
        ("flood-out", ProcessFailureKind::StdoutLimit),
        ("flood-err", ProcessFailureKind::StderrLimit),
        ("stdin-block", ProcessFailureKind::Timeout),
    ] {
        let mut spec = isolated.spec(mode);
        spec.timeout = Duration::from_millis(if mode == "stdin-block" { 300 } else { 5000 });
        let (_keepalive, cancel) = oneshot::channel();
        let started = Instant::now();
        let error = execute(&spec, vec![b'x'; MAX_STDIN_BYTES], cancel, |_| async {
            Ok(())
        })
        .await
        .unwrap_err();
        assert_eq!(error.kind, expected, "{error}");
        assert!(started.elapsed() < Duration::from_secs(8));
        assert_eq!(
            probe_previous(error.containment.as_ref().unwrap()).await,
            ContainmentStatus::Ended
        );
    }
    let (_keepalive, cancel) = oneshot::channel();
    let error = execute(
        &isolated.spec("emit"),
        vec![0; MAX_STDIN_BYTES + 1],
        cancel,
        |_| async { panic!("overlarge request reached launch") },
    )
    .await
    .unwrap_err();
    assert_eq!(error.kind, ProcessFailureKind::InputLimit);
}

#[tokio::test]
async fn nonzero_exit_and_missing_executable_are_typed() {
    let isolated = Isolated::new();
    let (_keepalive, cancel) = oneshot::channel();
    let error = execute(&isolated.spec("fail"), Vec::new(), cancel, |_| async {
        Ok(())
    })
    .await
    .unwrap_err();
    assert_eq!(error.kind, ProcessFailureKind::NonzeroExit);
    assert!(!error.message.contains("test-only-secret"));
    let mut spec = isolated.spec("emit");
    spec.command[0] = isolated
        .root
        .join("missing-binary")
        .to_string_lossy()
        .into_owned();
    let (_keepalive, cancel) = oneshot::channel();
    let error = execute(&spec, Vec::new(), cancel, |_| async { Ok(()) })
        .await
        .unwrap_err();
    assert_eq!(error.kind, ProcessFailureKind::Spawn);
}

#[tokio::test]
async fn invalid_cwd_and_cancel_before_launch_never_reach_the_gate() {
    let isolated = Isolated::new();
    let mut spec = isolated.spec("emit");
    spec.working_directory = isolated.root.join("missing-directory");
    let (_keepalive, cancel) = oneshot::channel();
    let error = execute(&spec, Vec::new(), cancel, |_| async {
        panic!("invalid working directory reached launch gate")
    })
    .await
    .unwrap_err();
    assert_eq!(error.kind, ProcessFailureKind::WorkingDirectory);
    let (sender, cancel) = oneshot::channel();
    drop(sender);
    let error = execute(&isolated.spec("emit"), Vec::new(), cancel, |_| async {
        panic!("cancelled attempt reached launch gate")
    })
    .await
    .unwrap_err();
    assert_eq!(error.kind, ProcessFailureKind::Cancelled);
    assert!(error.containment.is_none());
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn missing_guardian_is_not_proof_without_its_cleanup_receipt() {
    let isolated = Isolated::new();
    let mut spec = isolated.spec("marker");
    let marker = isolated.root.join("must-not-execute");
    spec.command.push(marker.to_string_lossy().into_owned());
    let (_keepalive, cancel) = oneshot::channel();
    let error = execute(&spec, Vec::new(), cancel, |identity| async move {
        let ContainmentIdentity::LinuxGuard { pid, .. } = identity else {
            panic!("wrong platform identity");
        };
        // This is our own newly created, unreaped helper, still behind the
        // launch gate. No detector or unrelated process can be affected.
        assert_eq!(unsafe { libc::kill(pid as i32, libc::SIGKILL) }, 0);
        Err("simulate guardian loss before detector execution".into())
    })
    .await
    .unwrap_err();
    assert_eq!(error.kind, ProcessFailureKind::ContainmentUnproven);
    assert!(!marker.exists());
    assert_eq!(
        probe_previous(error.containment.as_ref().unwrap()).await,
        ContainmentStatus::Unproven
    );
}

#[tokio::test]
async fn successful_leader_exit_kills_descendants_holding_output_pipes() {
    let isolated = Isolated::new();
    let mut spec = isolated.spec("tree-return");
    spec.command
        .push(isolated.root.to_string_lossy().into_owned());
    let (_keepalive, cancel) = oneshot::channel();
    let output = execute(&spec, b"{}\n".to_vec(), cancel, |_| async { Ok(()) })
        .await
        .unwrap();
    assert!(String::from_utf8_lossy(&output.stdout).contains("complete"));
    assert_tree_ended(&isolated.root);
}

#[tokio::test]
async fn timeout_and_shutdown_kill_the_complete_tree() {
    let isolated = Isolated::new();
    for shutdown in [false, true] {
        let root = isolated
            .root
            .join(if shutdown { "shutdown" } else { "timeout" });
        std::fs::create_dir_all(&root).unwrap();
        let mut spec = isolated.spec("tree");
        spec.command.push(root.to_string_lossy().into_owned());
        spec.timeout = Duration::from_secs(2);
        let (cancel, receiver) = oneshot::channel();
        let cancellation = async {
            if shutdown {
                wait_file_async(&root.join("leaf.pid")).await;
                cancel.send(()).unwrap();
            } else {
                tokio::time::sleep(Duration::from_secs(3)).await;
                drop(cancel);
            }
        };
        let (result, _) = tokio::join!(
            execute(&spec, b"{}\n".to_vec(), receiver, |_| async { Ok(()) }),
            cancellation
        );
        assert_eq!(
            result.unwrap_err().kind,
            if shutdown {
                ProcessFailureKind::Cancelled
            } else {
                ProcessFailureKind::Timeout
            }
        );
        assert_tree_ended(&root);
    }
}

#[test]
fn abrupt_runtime_death_kills_child_and_detached_descendants_before_restart_probe() {
    let isolated = Isolated::new();
    let mut runtime = spawn_runtime(&isolated.root, false);
    wait_file(&isolated.root.join("leaf.pid"));
    let identity = read_identity(&isolated.root);
    runtime.kill().unwrap();
    runtime.wait().unwrap();
    // This assertion deliberately precedes recovery: cleanup must not depend
    // on a new Watcher starting or calling probe_previous.
    assert_tree_ended(&isolated.root);
    let runtime = tokio::runtime::Runtime::new().unwrap();
    assert_eq!(
        runtime.block_on(probe_previous(&identity)),
        ContainmentStatus::Ended
    );
}

#[test]
fn abrupt_death_while_durable_gate_is_pending_never_executes_detector() {
    let isolated = Isolated::new();
    let mut runtime = spawn_runtime(&isolated.root, true);
    wait_file(&isolated.root.join("identity.json"));
    let identity = read_identity(&isolated.root);
    runtime.kill().unwrap();
    runtime.wait().unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    assert_eq!(
        runtime.block_on(probe_previous(&identity)),
        ContainmentStatus::Ended
    );
    assert!(!isolated.root.join("root.pid").exists());
}

struct RuntimeChild(std::process::Child);

impl std::ops::Deref for RuntimeChild {
    type Target = std::process::Child;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl std::ops::DerefMut for RuntimeChild {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl Drop for RuntimeChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn spawn_runtime(root: &Path, hold_gate: bool) -> RuntimeChild {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", "fixture_runtime", "--ignored", "--nocapture"])
        .env("WATCHER_FIXTURE_ROOT", root)
        .env("WATCHER_FIXTURE_BIN", fixture())
        .env(
            "WATCHER_FIXTURE_HOLD_GATE",
            if hold_gate { "1" } else { "0" },
        )
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit());
    RuntimeChild(command.spawn().unwrap())
}

#[test]
#[ignore = "invoked only as an isolated subprocess by destructive containment tests"]
fn fixture_runtime() {
    let root = PathBuf::from(std::env::var_os("WATCHER_FIXTURE_ROOT").unwrap());
    let binary = PathBuf::from(std::env::var_os("WATCHER_FIXTURE_BIN").unwrap());
    assert!(root.is_absolute() && binary.is_absolute());
    let spec = ProcessSpec {
        command: vec![
            binary.to_string_lossy().into_owned(),
            "tree".into(),
            root.to_string_lossy().into_owned(),
        ],
        working_directory: root.clone(),
        environment_allowlist: vec![
            "TELEX_HOME".into(),
            "TELEX_DB".into(),
            "TELEX_INSTALL_ROOT".into(),
        ],
        timeout: Duration::from_secs(60),
        containment_directory: root.join("containment"),
    };
    let (_keepalive, cancel) = oneshot::channel();
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let result = execute(&spec, b"{}\n".to_vec(), cancel, |identity| async move {
            use std::io::Write;
            let mut file = std::fs::File::create(root.join("identity.json")).unwrap();
            file.write_all(&serde_json::to_vec(&identity).unwrap())
                .unwrap();
            file.sync_all().unwrap();
            if std::env::var("WATCHER_FIXTURE_HOLD_GATE").as_deref() == Ok("1") {
                std::future::pending::<()>().await;
            }
            Ok(())
        })
        .await;
        panic!("fixture runtime unexpectedly completed: {result:?}");
    });
}

fn read_identity(root: &Path) -> ContainmentIdentity {
    serde_json::from_slice(&std::fs::read(root.join("identity.json")).unwrap()).unwrap()
}

async fn wait_file_async(path: &Path) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !path.exists() {
        assert!(
            Instant::now() < deadline,
            "fixture did not create {}",
            path.display()
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

fn wait_file(path: &Path) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !path.exists() || std::fs::metadata(path).unwrap().len() == 0 {
        assert!(
            Instant::now() < deadline,
            "fixture did not create {}",
            path.display()
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn assert_tree_ended(root: &Path) {
    for name in ["root", "child", "leaf"] {
        let path = root.join(format!("{name}.pid"));
        wait_file(&path);
        let pid: u32 = std::fs::read_to_string(path).unwrap().parse().unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        while alive(pid) {
            assert!(
                Instant::now() < deadline,
                "{name} process {pid} survived containment"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

#[cfg(windows)]
fn alive(pid: u32) -> bool {
    use windows_sys::Win32::Foundation::*;
    use windows_sys::Win32::System::Threading::*;
    unsafe {
        let handle = OpenProcess(PROCESS_SYNCHRONIZE, 0, pid);
        if handle == 0 {
            assert_eq!(
                GetLastError(),
                ERROR_INVALID_PARAMETER,
                "cannot prove PID ended"
            );
            return false;
        }
        let status = WaitForSingleObject(handle, 0);
        CloseHandle(handle);
        assert!(status == WAIT_OBJECT_0 || status == WAIT_TIMEOUT);
        status == WAIT_TIMEOUT
    }
}

#[cfg(target_os = "linux")]
fn alive(pid: u32) -> bool {
    match std::fs::read_to_string(format!("/proc/{pid}/stat")) {
        Ok(_) => true,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(error) => panic!("cannot prove PID ended: {error}"),
    }
}
