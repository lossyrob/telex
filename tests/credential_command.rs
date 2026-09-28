#![cfg(feature = "postgres")]

use std::path::PathBuf;
use std::time::{Duration, Instant};
use telex::profiles::{pg_connect_config, BackendProfile};
use telex::session_watch::capture_process_start_time;
#[cfg(unix)]
use telex::session_watch::process_alive_with_start_time;

static TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
const FIXTURE_ROOT: &str = "TELEX_CREDENTIAL_FIXTURE_ROOT";
const HOST_FIXTURE: &str = "credential_native_host_fixture";
const LEAF_FIXTURE: &str = "credential_native_leaf_fixture";

#[path = "fixtures/credential_command_helpers.rs"]
mod credential_helpers;
#[cfg(windows)]
use credential_helpers::encoded_powershell;

struct DelayedCommand {
    root: PathBuf,
    command: String,
    processes: Vec<(u32, Option<u64>)>,
    #[cfg(windows)]
    handles: Vec<std::os::windows::io::OwnedHandle>,
}

impl DelayedCommand {
    fn new(label: &str) -> Self {
        assert!(label
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-'));
        let relative = PathBuf::from("target")
            .join("credential-command-fixtures")
            .join(format!(
                "telex-credential-{label}-{}-{}",
                std::process::id(),
                telex::model::now_ms(),
            ));
        let root = std::env::current_dir().unwrap().join(&relative);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("keep-running"), "").unwrap();
        let executable_name = format!("helper{}", std::env::consts::EXE_SUFFIX);
        std::fs::copy(
            std::env::current_exe().unwrap(),
            root.join(&executable_name),
        )
        .unwrap();
        let executable = relative.join(executable_name);
        #[cfg(windows)]
        let command = format!(
            "set {FIXTURE_ROOT}={}&& .\\{} --exact {HOST_FIXTURE} --ignored --nocapture --test-threads=1",
            relative.display(), executable.display(),
        );
        #[cfg(unix)]
        let command = format!(
            "{FIXTURE_ROOT}={} ./{} --exact {HOST_FIXTURE} --ignored --nocapture --test-threads=1",
            relative.display(),
            executable.display(),
        );
        Self {
            root,
            command,
            processes: Vec::new(),
            #[cfg(windows)]
            handles: Vec::new(),
        }
    }

    fn capture(&mut self) -> bool {
        let Ok(text) = std::fs::read_to_string(self.root.join("pids")) else {
            return false;
        };
        let pids: Vec<u32> = text
            .split_whitespace()
            .map(|pid| pid.parse().unwrap())
            .collect();
        assert_eq!(
            pids.len(),
            2,
            "native host must publish one complete host/leaf identity record"
        );
        assert!(pids.iter().all(|pid| *pid != std::process::id()));
        #[cfg(windows)]
        {
            use std::os::windows::io::FromRawHandle;
            use windows_sys::Win32::System::Threading::{
                OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
                PROCESS_TERMINATE,
            };
            for pid in &pids {
                let handle = unsafe {
                    OpenProcess(
                        PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE | PROCESS_TERMINATE,
                        0,
                        *pid,
                    )
                };
                assert_ne!(handle, 0, "retain owned fixture process handle");
                self.handles.push(unsafe {
                    std::os::windows::io::OwnedHandle::from_raw_handle(handle as *mut _)
                });
            }
        }
        self.processes = pids
            .into_iter()
            .map(|pid| (pid, capture_process_start_time(pid)))
            .collect();
        true
    }

    fn liveness(&self) -> Vec<bool> {
        #[cfg(windows)]
        {
            use std::os::windows::io::AsRawHandle;
            use windows_sys::Win32::Foundation::{WAIT_OBJECT_0, WAIT_TIMEOUT};
            use windows_sys::Win32::System::Threading::WaitForSingleObject;
            self.handles
                .iter()
                .map(|handle| {
                    match unsafe { WaitForSingleObject(handle.as_raw_handle() as isize, 0) } {
                        WAIT_OBJECT_0 => false,
                        WAIT_TIMEOUT => true,
                        other => panic!("owned fixture process wait failed: {other}"),
                    }
                })
                .collect()
        }
        #[cfg(unix)]
        {
            self.processes
                .iter()
                .map(|(pid, start)| process_alive_with_start_time(*pid, *start))
                .collect()
        }
    }
}

impl Drop for DelayedCommand {
    fn drop(&mut self) {
        // The guardian can release the native leaf even if startup failed before PID handoff.
        let _ = std::fs::remove_file(self.root.join("keep-running"));
        #[cfg(windows)]
        {
            use std::os::windows::io::AsRawHandle;
            use windows_sys::Win32::Foundation::WAIT_TIMEOUT;
            use windows_sys::Win32::System::Threading::{TerminateProcess, WaitForSingleObject};
            for handle in &self.handles {
                let handle = handle.as_raw_handle() as isize;
                if unsafe { WaitForSingleObject(handle, 0) } == WAIT_TIMEOUT {
                    unsafe { TerminateProcess(handle, 1) };
                    let waited = unsafe { WaitForSingleObject(handle, 5_000) };
                    if waited != 0 {
                        eprintln!("owned credential fixture fallback cleanup failed");
                    }
                }
            }
            self.handles.clear();
        }
        #[cfg(unix)]
        {
            // The independent fixture guardian releases its own helper's rescue condition.
            // It never sends a signal to a product-owned or already-reaped numeric identity.
            let _ = std::fs::remove_file(self.root.join("keep-running"));
            let deadline = Instant::now() + Duration::from_secs(5);
            while self.liveness().iter().any(|alive| *alive) && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(10));
            }
            if self.liveness().iter().any(|alive| *alive) {
                eprintln!("owned credential fixture rescue did not observe completion");
            }
        }
        if !std::thread::panicking() {
            std::fs::remove_dir_all(&self.root).unwrap();
        }
    }
}

#[test]
#[ignore = "native credential fixture host, invoked by production command execution"]
fn credential_native_host_fixture() {
    let root = PathBuf::from(std::env::var_os(FIXTURE_ROOT).expect("fixture root"));
    std::fs::write(root.join("host-started"), "started").unwrap();
    if root.join("fail-before-ready").exists() {
        std::process::exit(23);
    }
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            LEAF_FIXTURE,
            "--ignored",
            "--nocapture",
            "--test-threads=1",
        ])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("spawn owned fixture leaf");
    let ready_by = Instant::now() + Duration::from_secs(4);
    while !root.join("leaf-ready").exists() {
        if child.try_wait().unwrap().is_some() || Instant::now() >= ready_by {
            if child.try_wait().unwrap().is_none() {
                child.kill().unwrap();
            }
            child.wait().unwrap();
            panic!("native fixture leaf did not establish readiness");
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    let temporary = root.join("pids.tmp");
    std::fs::write(
        &temporary,
        format!("{}\n{}\n", std::process::id(), child.id()),
    )
    .unwrap();
    std::fs::rename(temporary, root.join("pids")).unwrap();
    let status = child.wait().unwrap();
    std::process::exit(if status.success() { 0 } else { 24 });
}

#[test]
#[ignore = "native credential leaf, owned and awaited by the fixture host"]
fn credential_native_leaf_fixture() {
    let root = PathBuf::from(std::env::var_os(FIXTURE_ROOT).expect("fixture root"));
    std::fs::write(root.join("leaf-ready"), "ready").unwrap();
    let safety_deadline = Instant::now() + Duration::from_secs(30);
    while root.join("keep-running").exists() && Instant::now() < safety_deadline {
        std::thread::sleep(Duration::from_millis(5));
    }
    std::process::exit(0);
}

fn profile(command: &str) -> BackendProfile {
    BackendProfile {
        kind: "postgres".into(),
        path: None,
        url: Some("host=127.0.0.1 port=1 user=test dbname=test".into()),
        auth: Some("password".into()),
        password_env: None,
        password_command: Some(command.into()),
        schema: None,
        entra_cred: None,
        entra_scope: None,
    }
}

async fn cancellation_proof(label: &str, budget: Duration) {
    let mut fixture = DelayedCommand::new(label);
    let profile = profile(&fixture.command);
    let started = Instant::now();
    let mut acquiring = Box::pin(tokio::time::timeout(budget, pg_connect_config(&profile)));
    loop {
        tokio::select! {
            result = &mut acquiring => panic!("command ended before helper readiness: {result:?}"),
            _ = tokio::time::sleep(Duration::from_millis(10)) => {
                if fixture.capture() { break; }
            }
        }
    }
    assert!(fixture.liveness().iter().all(|alive| *alive));
    assert!(
        (&mut acquiring).await.is_err(),
        "delayed command must be cancelled"
    );
    drop(acquiring);
    let elapsed = started.elapsed();
    let at_timeout = fixture.liveness();
    let cleanup_by = Instant::now() + Duration::from_millis(3_500);
    let after_cleanup = loop {
        let alive = fixture.liveness();
        let receipts_complete = telex::profiles::credential_command_obligations().is_empty();
        if (alive.iter().all(|alive| !alive) && receipts_complete) || Instant::now() >= cleanup_by {
            break alive;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    };
    // Capture observations before the fixture's failure cleanup can change them.
    eprintln!(
        "{label}: elapsed={elapsed:?}, at_timeout={at_timeout:?}, after_cleanup={after_cleanup:?}"
    );
    assert!(
        elapsed < budget + Duration::from_millis(500),
        "caller deadline was extended"
    );
    assert!(
        after_cleanup.iter().all(|alive| !alive),
        "owned command work survived cancellation"
    );
    assert!(
        telex::profiles::credential_command_obligations().is_empty(),
        "owned scope needs the production receipt and native join, not just helper death",
    );
}

#[tokio::test]
async fn credential_command_finite_deadline_cleans_owned_helper() {
    let _guard = TEST_LOCK.lock().await;
    cancellation_proof("finite", Duration::from_millis(1_500)).await;
}

#[tokio::test]
async fn credential_command_recovery_grace_cleans_owned_helper() {
    let _guard = TEST_LOCK.lock().await;
    cancellation_proof(
        "grace",
        Duration::from_millis(telex::daemon_ipc::DEFAULT_WAIT_RECONNECT_GRACE_MS),
    )
    .await;
}

#[tokio::test]
async fn credential_command_keeps_output_and_failure_semantics() {
    let _guard = TEST_LOCK.lock().await;
    let command = if cfg!(windows) {
        "echo non-secret-test-placeholder"
    } else {
        "printf ' non-secret-test-placeholder\\n'"
    };
    let (config, _) = pg_connect_config(&profile(command)).await.unwrap();
    assert_eq!(
        config.get_password(),
        Some(b"non-secret-test-placeholder".as_slice())
    );
    let command = if cfg!(windows) {
        "echo test-failure 1>&2 & exit /b 9"
    } else {
        "echo test-failure >&2; exit 9"
    };
    let error = pg_connect_config(&profile(command)).await.unwrap_err();
    assert!(error.to_string().contains("exited unsuccessfully"));
    assert!(!error.to_string().contains("test-failure"));
    assert!(telex::profiles::credential_command_obligations().is_empty());
}

#[tokio::test]
async fn credential_environment_precedence_does_not_start_a_command() {
    let _guard = TEST_LOCK.lock().await;
    let fixture = DelayedCommand::new("env-precedence");
    let key = format!("TELEX_TEST_CREDENTIAL_{}", std::process::id());
    std::env::set_var(&key, "non-secret-environment-value");
    let mut p = profile(&fixture.command);
    p.password_env = Some(key.clone());
    let result = pg_connect_config(&p).await;
    std::env::remove_var(key);
    let (config, _) = result.unwrap();
    assert_eq!(
        config.get_password(),
        Some(b"non-secret-environment-value".as_slice())
    );
    assert!(!fixture.root.join("pids").exists());
    assert!(telex::profiles::credential_command_obligations().is_empty());
}

#[tokio::test]
async fn credential_invalid_utf8_is_an_error_without_exposing_output() {
    let _guard = TEST_LOCK.lock().await;
    #[cfg(windows)]
    let command = encoded_powershell(
        "$output=[Console]::OpenStandardOutput(); $output.WriteByte(255); $output.Flush()",
    );
    #[cfg(unix)]
    let command = "printf '\\377'".to_string();
    let error = pg_connect_config(&profile(&command)).await.unwrap_err();
    assert_eq!(error.to_string(), "credential command output is not UTF-8");
    assert!(telex::profiles::credential_command_obligations().is_empty());
}

struct CredentialCall(
    tokio::task::JoinHandle<anyhow::Result<(tokio_postgres::Config, Option<String>)>>,
);

impl CredentialCall {
    fn start(command: &str) -> Self {
        let p = profile(command);
        Self(tokio::spawn(async move { pg_connect_config(&p).await }))
    }

    async fn cancel(mut self) {
        self.0.abort();
        assert!((&mut self.0).await.unwrap_err().is_cancelled());
    }
}

impl Drop for CredentialCall {
    fn drop(&mut self) {
        self.0.abort();
    }
}

async fn wait_for_helper(
    fixture: &mut DelayedCommand,
    call: &mut CredentialCall,
) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(4);
    loop {
        if fixture.capture() {
            return Ok(());
        }
        tokio::select! {
            result = &mut call.0 => {
                return Err(match result {
                    Ok(Ok(_)) => "credential command completed before fixture readiness".into(),
                    Ok(Err(error)) => format!("credential command failed before fixture readiness: {error:#}"),
                    Err(error) => format!("credential task ended before fixture readiness: {error}"),
                });
            }
            _ = tokio::time::sleep(Duration::from_millis(10)) => {
                if Instant::now() >= deadline {
                    return Err(format!(
                        "owned helper failed readiness: host_started={}, leaf_ready={}, record_present={}, obligations={:?}",
                        fixture.root.join("host-started").exists(),
                        fixture.root.join("leaf-ready").exists(),
                        fixture.root.join("pids").exists(),
                        telex::profiles::credential_command_obligations(),
                    ));
                }
            }
        }
    }
}

async fn wait_for_receipts() {
    let deadline = Instant::now() + Duration::from_millis(3_500);
    while !telex::profiles::credential_command_obligations().is_empty() {
        assert!(
            Instant::now() < deadline,
            "missing native scope/join receipt"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

async fn wait_for_fixture_process_completion(fixture: &DelayedCommand) {
    // Independent fixture ownership outlives the Windows job-terminal receipt.
    // This wait is cleanup evidence, not part of the product's scope predicate.
    let deadline = Instant::now() + Duration::from_secs(5);
    while fixture.liveness().iter().any(|alive| *alive) {
        assert!(
            Instant::now() < deadline,
            "fixture process rundown did not finish"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

#[tokio::test]
async fn credential_command_capacity_and_same_source_queue_launch_nothing() {
    let _guard = TEST_LOCK.lock().await;
    let mut first = DelayedCommand::new("capacity-first");
    let mut second = DelayedCommand::new("capacity-second");
    let third = DelayedCommand::new("capacity-queued");
    let mut first_call = CredentialCall::start(&first.command);
    let mut second_call = CredentialCall::start(&second.command);
    wait_for_helper(&mut first, &mut first_call).await.unwrap();
    wait_for_helper(&mut second, &mut second_call)
        .await
        .unwrap();
    let occupied = telex::profiles::credential_command_obligations();
    assert_eq!(occupied.len(), 2);
    let duplicate_profile = profile(&first.command);
    let third_profile = profile(&third.command);
    assert!(tokio::time::timeout(
        Duration::from_millis(30),
        pg_connect_config(&duplicate_profile)
    )
    .await
    .is_err());
    assert!(
        tokio::time::timeout(Duration::from_millis(30), pg_connect_config(&third_profile))
            .await
            .is_err()
    );
    assert_eq!(telex::profiles::credential_command_obligations(), occupied);
    assert!(
        !third.root.join("pids").exists(),
        "queued command launched a helper"
    );
    assert!(
        !third.root.join("host-started").exists(),
        "queued command executed its host"
    );
    assert!(
        !third.root.join("leaf-ready").exists(),
        "queued command created its leaf"
    );
    first_call.cancel().await;
    second_call.cancel().await;
    wait_for_receipts().await;
    eprintln!(
        "job-terminal receipt: independent fixture handle liveness first={:?}, second={:?}",
        first.liveness(),
        second.liveness(),
    );
    wait_for_fixture_process_completion(&first).await;
    wait_for_fixture_process_completion(&second).await;
}

#[tokio::test]
async fn credential_command_shell_exit_preserves_delayed_inherited_writer() {
    let _guard = TEST_LOCK.lock().await;
    #[cfg(windows)]
    let command = format!(
        "start /b {}",
        encoded_powershell(
            "Start-Sleep -Milliseconds 150; [Console]::Out.Write('  delayed-non-secret  ')",
        )
    );
    #[cfg(unix)]
    let command = "sh -c 'sleep 0.15; printf \"  delayed-non-secret  \"' &".to_string();
    let (config, _) = tokio::time::timeout(
        Duration::from_secs(5),
        pg_connect_config(&profile(&command)),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(
        config.get_password(),
        Some(b"delayed-non-secret".as_slice())
    );
    assert!(telex::profiles::credential_command_obligations().is_empty());
}

#[tokio::test]
async fn credential_command_collects_full_utf8_and_both_pipe_buffers() {
    let _guard = TEST_LOCK.lock().await;
    #[cfg(windows)]
    let command = encoded_powershell(
        "[Console]::OutputEncoding=[Text.UTF8Encoding]::new($false); \
         [Console]::Error.Write(('e' * 131072)); \
         [Console]::Out.Write('  ' + ('x' * 131072) + [char]955 + '  ')",
    );
    #[cfg(unix)]
    let command = "i=0; while test $i -lt 4096; do printf eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee >&2; i=$((i+1)); done; printf '  '; i=0; while test $i -lt 4096; do printf xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx; i=$((i+1)); done; printf '\\316\\273  '".to_string();
    let (config, _) = tokio::time::timeout(
        Duration::from_secs(10),
        pg_connect_config(&profile(&command)),
    )
    .await
    .unwrap()
    .unwrap();
    let output = std::str::from_utf8(config.get_password().unwrap()).unwrap();
    assert_eq!(output, format!("{}\u{03bb}", "x".repeat(131072)));
    assert!(telex::profiles::credential_command_obligations().is_empty());
}

#[tokio::test]
async fn credential_command_repeated_cancellation_releases_all_owned_records() {
    let _guard = TEST_LOCK.lock().await;
    for attempt in 0..3 {
        let mut fixture = DelayedCommand::new(&format!("repeat-{attempt}"));
        let mut call = CredentialCall::start(&fixture.command);
        wait_for_helper(&mut fixture, &mut call).await.unwrap();
        assert_eq!(telex::profiles::credential_command_obligations().len(), 1);
        call.cancel().await;
        wait_for_receipts().await;
        eprintln!(
            "job-terminal receipt iteration {attempt}: independent fixture handle liveness={:?}",
            fixture.liveness(),
        );
        wait_for_fixture_process_completion(&fixture).await;
    }
}

struct OutsideProcess(std::process::Child);

impl Drop for OutsideProcess {
    fn drop(&mut self) {
        if self.0.try_wait().unwrap().is_none() {
            self.0.kill().unwrap();
        }
        self.0.wait().unwrap();
    }
}

#[tokio::test]
async fn credential_cancellation_does_not_terminate_a_preexisting_outside_process() {
    let _guard = TEST_LOCK.lock().await;
    #[cfg(windows)]
    let mut command = {
        let mut command = std::process::Command::new("powershell.exe");
        command.args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "Start-Sleep -Seconds 30",
        ]);
        command
    };
    #[cfg(unix)]
    let mut command = {
        let mut command = std::process::Command::new("sleep");
        command.arg("30");
        command
    };
    let mut outside = OutsideProcess(
        command
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap(),
    );
    let mut fixture = DelayedCommand::new("outside-process");
    let mut call = CredentialCall::start(&fixture.command);
    wait_for_helper(&mut fixture, &mut call).await.unwrap();
    assert!(outside.0.try_wait().unwrap().is_none());
    call.cancel().await;
    wait_for_receipts().await;
    wait_for_fixture_process_completion(&fixture).await;
    assert!(
        outside.0.try_wait().unwrap().is_none(),
        "credential cleanup affected a process outside its invocation"
    );
}

#[tokio::test]
async fn credential_helper_early_exit_is_reported_without_a_readiness_timeout() {
    let _guard = TEST_LOCK.lock().await;
    let mut fixture = DelayedCommand::new("early-exit");
    std::fs::write(fixture.root.join("fail-before-ready"), "").unwrap();
    let mut call = CredentialCall::start(&fixture.command);
    let started = Instant::now();
    let error = wait_for_helper(&mut fixture, &mut call).await.unwrap_err();
    assert!(
        error.contains("credential command failed before fixture readiness"),
        "{error}"
    );
    assert!(
        error.contains("exited unsuccessfully") && error.contains("23"),
        "{error}"
    );
    assert!(started.elapsed() < Duration::from_secs(4));
    assert!(fixture.root.join("host-started").exists());
    assert!(!fixture.root.join("pids").exists());
    assert!(!fixture.root.join("leaf-ready").exists());
    assert!(telex::profiles::credential_command_obligations().is_empty());
}
