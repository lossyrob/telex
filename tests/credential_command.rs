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

/// Issue #167 phase and drain evidence for the delayed inherited writer on Windows.
///
/// The writer stamps its process creation time, script start, the instant
/// immediately before its stdout write and write completion into atomic
/// marker files, each created once by rename and never removed before the
/// read. The test reads them only after the operation has completed or has
/// been dropped (cancellation requested), so no file or process query
/// precedes cancellation. A marker that read as NotFound later had not been
/// published by the outcome, given no removal; that does not show the writer
/// never reached that point, because marker I/O can fail while the command
/// continues. A present marker is placed relative to the outcome only by
/// the UTC FILETIME wall clock, which is not monotonic. Writer stamps come
/// from .NET `DateTime.UtcNow`, whose resolution is not verified here, and
/// marker I/O inside the writer, including the pre-write marker just before
/// the write, is an observer effect.
///
/// For a timed-out operation the native owner's drain progress is read after
/// the drop and before the reaping obligation query. It is a post-cancel
/// snapshot of independent loads, not a coherent deadline-state
/// reconstruction: the owner may complete further polls after the outcome,
/// overwriting the last pre-outcome one. Its stamps are the owner's own
/// monotonic `Instant`s reported relative to the outcome, so such later
/// events stay visible as later rather than read as deadline state.
#[cfg(windows)]
mod delayed_writer_phases {
    use std::fmt::Write as _;
    use std::path::PathBuf;
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
    use telex::profiles::CredentialCommandDrain;

    /// 100ns intervals between 1601-01-01 and 1970-01-01.
    const UNIX_EPOCH_TICKS: u64 = 116_444_736_000_000_000;

    fn filetime_now() -> u64 {
        let since = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
        UNIX_EPOCH_TICKS + since.as_secs() * 10_000_000 + u64::from(since.subsec_nanos() / 100)
    }

    fn span(from: Option<u64>, to: Option<u64>) -> String {
        match (from, to) {
            (Some(from), Some(to)) => format!("{:.1}", (to as i64 - from as i64) as f64 / 10_000.0),
            _ => "unobserved".into(),
        }
    }

    fn ms(duration: Duration) -> String {
        format!("{:.1}", duration.as_secs_f64() * 1000.0)
    }

    /// Signed milliseconds from `at` to the outcome. Only a strictly earlier
    /// stamp is reported as before it; an equal or later stamp is reported as
    /// non-positive (`-`), never as a deadline-time event.
    fn before_outcome(outcome: Instant, at: Option<Instant>) -> String {
        match at {
            None => "never".into(),
            Some(at) if at < outcome => ms(outcome - at),
            Some(at) => format!("-{}", ms(at - outcome)),
        }
    }

    /// One marker as seen at the read after the outcome.
    enum Marker {
        Absent,
        Unreadable,
        Present(Vec<Option<u64>>),
    }

    impl Marker {
        fn stamp(&self, index: usize) -> Option<u64> {
            match self {
                Marker::Present(fields) => fields.get(index).copied().flatten(),
                _ => None,
            }
        }

        /// A successful NotFound at the later read means the marker had not
        /// been published by the outcome, given no removal; it does not show
        /// the writer never reached that point, since marker I/O can fail
        /// while the command continues. Presence is placed relative to the
        /// outcome by wall clock only.
        fn at_outcome(&self, index: usize, outcome: u64) -> &'static str {
            match self {
                Marker::Absent => "not_published_at_read",
                Marker::Unreadable => "unreadable",
                Marker::Present(_) => match self.stamp(index) {
                    Some(stamp) if stamp <= outcome => "present_wallclock_before_outcome",
                    Some(_) => "present_wallclock_after_outcome",
                    None => "present_unparsed",
                },
            }
        }
    }

    pub struct Markers {
        root: PathBuf,
    }

    impl Markers {
        pub fn new() -> Self {
            let root = std::env::current_dir()
                .unwrap()
                .join("target")
                .join("credential-command-fixtures")
                .join(format!(
                    "telex-credential-delayed-writer-{}-{}",
                    std::process::id(),
                    telex::model::now_ms(),
                ));
            assert!(!root.to_string_lossy().contains('\''));
            std::fs::create_dir_all(&root).unwrap();
            Self { root }
        }

        /// Wrap the delayed writer with atomic started, pre_write and written
        /// markers. The stdout contract is unchanged; marker failures only
        /// reach stderr.
        pub fn script(&self, before_write: &str, write: &str) -> String {
            let root = self.root.display();
            format!(
                "$m='{root}'; $s=[DateTime]::UtcNow.ToFileTimeUtc(); \
                 $c=[Diagnostics.Process]::GetCurrentProcess().StartTime.ToFileTimeUtc(); \
                 [IO.File]::WriteAllText(\"$m\\started.tmp\", \"$c $s\"); \
                 [IO.File]::Move(\"$m\\started.tmp\", \"$m\\started\"); \
                 {before_write}; \
                 [IO.File]::WriteAllText(\"$m\\pre_write.tmp\", [string][DateTime]::UtcNow.ToFileTimeUtc()); \
                 [IO.File]::Move(\"$m\\pre_write.tmp\", \"$m\\pre_write\"); \
                 {write}; \
                 [IO.File]::WriteAllText(\"$m\\written.tmp\", [string][DateTime]::UtcNow.ToFileTimeUtc()); \
                 [IO.File]::Move(\"$m\\written.tmp\", \"$m\\written\")"
            )
        }

        fn read(&self, name: &str) -> Marker {
            match std::fs::read_to_string(self.root.join(name)) {
                Ok(text) => Marker::Present(
                    text.split_whitespace()
                        .map(|field| field.parse().ok())
                        .collect(),
                ),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Marker::Absent,
                Err(_) => Marker::Unreadable,
            }
        }
    }

    impl Drop for Markers {
        fn drop(&mut self) {
            if !std::thread::panicking() {
                let _ = std::fs::remove_dir_all(&self.root);
            }
        }
    }

    pub struct Clock {
        invoked: u64,
        started: Instant,
    }

    /// Both clocks sampled at the outcome, before any observer I/O.
    pub struct Finished {
        filetime: u64,
        monotonic_total: Duration,
        at: Instant,
    }

    impl Finished {
        pub fn since(&self) -> Duration {
            self.at.elapsed()
        }
    }

    /// Observations after a timed-out operation was dropped, in capture
    /// order: cancel request, non-reaping drain read, reaping obligation
    /// query. Durations are measured from the outcome.
    pub struct Cancellation {
        pub dropped: Duration,
        pub drain: Vec<CredentialCommandDrain>,
        pub drain_read: Duration,
        pub obligations_read: Duration,
    }

    impl Clock {
        pub fn start() -> Self {
            Self {
                invoked: filetime_now(),
                started: Instant::now(),
            }
        }

        pub fn finish(&self) -> Finished {
            Finished {
                filetime: filetime_now(),
                monotonic_total: self.started.elapsed(),
                at: Instant::now(),
            }
        }

        /// `capture_elapsed_ms` is observer cost from the outcome through the
        /// marker reads, not writer or operation duration.
        pub fn report(
            &self,
            markers: &Markers,
            finished: &Finished,
            cancellation: Option<Cancellation>,
        ) -> String {
            let started = markers.read("started");
            let pre_write = markers.read("pre_write");
            let written = markers.read("written");
            let (created, script) = (started.stamp(0), started.stamp(1));
            let (pre, done) = (pre_write.stamp(0), written.stamp(0));
            let invoked = Some(self.invoked);
            let outcome = Some(finished.filetime);
            let mut line = format!(
                "[issue-167 delayed-writer phases] outcome={} monotonic_total_ms={} \
                 filetime_total_ms={} invoked_to_writer_created_ms={} \
                 writer_created_to_script_ms={} script_to_pre_write_ms={} \
                 pre_write_to_written_ms={} written_to_outcome_ms={} \
                 pre_write_at_outcome={} written_at_outcome={} writer_exit=unobserved",
                if cancellation.is_some() {
                    "elapsed"
                } else {
                    "ok"
                },
                ms(finished.monotonic_total),
                span(invoked, outcome),
                span(invoked, created),
                span(created, script),
                span(script, pre),
                span(pre, done),
                span(done, outcome),
                pre_write.at_outcome(0, finished.filetime),
                written.at_outcome(0, finished.filetime),
            );
            if let Some(cancellation) = cancellation {
                write!(
                    line,
                    " cancel_requested_ms={} drain_read_ms={} post_cancel_obligations_ms={} \
                     fixture_stdout_bytes_before_trim={} drain_entries={}",
                    ms(cancellation.dropped),
                    ms(cancellation.drain_read),
                    ms(cancellation.obligations_read),
                    super::DELAYED_WRITER_OUTPUT.len(),
                    cancellation.drain.len(),
                )
                .unwrap();
                for entry in &cancellation.drain {
                    write!(
                        line,
                        " drain[{}]=owner_pid:{},polls_completed:{},\
                         last_poll_started_before_outcome_ms:{},\
                         last_poll_completed_before_outcome_ms:{},stdout_byte_count:{},\
                         first_stdout_before_outcome_ms:{},\
                         shell_exit_observed_by_owner_before_outcome_ms:{}",
                        entry.invocation_id,
                        entry.owner_pid,
                        entry.polls,
                        before_outcome(finished.at, entry.last_poll_started),
                        before_outcome(finished.at, entry.last_poll_completed),
                        entry.stdout_bytes,
                        before_outcome(finished.at, entry.first_stdout),
                        before_outcome(finished.at, entry.shell_exit_observed),
                    )
                    .unwrap();
                }
                line.push_str(
                    " drain=post_cancel_snapshot_independent_loads_not_deadline_state \
                     obligations=post_cancel_reaping_racy markers=post_cancel_racy",
                );
            }
            write!(
                line,
                " capture_elapsed_ms={} limits=filetime_wall_clock_not_monotonic,\
                 writer_utcnow_resolution_unverified,writer_marker_io_observer_effect,\
                pre_write_marker_io_precedes_write,marker_io_failure_hides_progress,\
                 script_to_pre_write_includes_configured_150ms_sleep,\
                marker_not_found_means_unpublished_not_unreached,\
                later_poll_overwrites_last_pre_outcome_poll,\
                shell_exit_is_owner_observation_not_os_exit,\
                byte_count_is_not_eof_or_child_exit,\
                capture_elapsed_is_observer_cost_after_outcome",
                ms(finished.since()),
            )
            .unwrap();
            line
        }
    }
}

/// Write past libtest output capture so passing hosted runs retain measured phases.
#[cfg(windows)]
fn emit_uncaptured(line: &str) {
    use std::io::Write;
    let _ = writeln!(std::io::stderr(), "{line}");
}

/// Exact delayed-writer stdout (22 bytes before trim, 18 after).
#[cfg(windows)]
const DELAYED_WRITER_OUTPUT: &str = "  delayed-non-secret  ";

#[tokio::test]
async fn credential_command_shell_exit_preserves_delayed_inherited_writer() {
    let _guard = TEST_LOCK.lock().await;
    #[cfg(windows)]
    let markers = delayed_writer_phases::Markers::new();
    #[cfg(windows)]
    let command = format!(
        "start /b {}",
        encoded_powershell(&markers.script(
            "Start-Sleep -Milliseconds 150",
            &format!("[Console]::Out.Write('{DELAYED_WRITER_OUTPUT}')"),
        ))
    );
    #[cfg(unix)]
    let command = "sh -c 'sleep 0.15; printf \"  delayed-non-secret  \"' &".to_string();
    let profile = profile(&command);
    // Boxed so a timed-out operation is dropped first. Dropping only requests
    // cancellation in memory. Drain progress is then read without reaping;
    // the owner's own stamps place each poll relative to the outcome. The
    // later obligation query reaps: it may join a finished owner, publish and
    // remove its entry, so an empty list does not mean nothing was
    // outstanding at the timeout. Observer file I/O follows.
    let mut operation = Box::pin(pg_connect_config(&profile));
    #[cfg(windows)]
    let clock = delayed_writer_phases::Clock::start();
    let outcome = tokio::time::timeout(Duration::from_secs(5), &mut operation).await;
    #[cfg(windows)]
    let finished = clock.finish();
    let Ok(result) = outcome else {
        drop(operation);
        #[cfg(windows)]
        let dropped = finished.since();
        #[cfg(windows)]
        let drain = telex::profiles::credential_command_drain_observations();
        #[cfg(windows)]
        let drain_read = finished.since();
        let obligations = telex::profiles::credential_command_obligations();
        #[cfg(windows)]
        let report = clock.report(
            &markers,
            &finished,
            Some(delayed_writer_phases::Cancellation {
                dropped,
                drain,
                drain_read,
                obligations_read: finished.since(),
            }),
        );
        #[cfg(unix)]
        let report = String::new();
        panic!(
            "delayed inherited writer was not collected within the 5s budget; \
             post_cancel_obligations={obligations:?} {report}"
        );
    };
    #[cfg(windows)]
    emit_uncaptured(&clock.report(&markers, &finished, None));
    let (config, _) = result.unwrap();
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
