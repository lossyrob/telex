//! Independent #245 evidence, retaining the #234/#239 outage battery unchanged.
//!
//! Idle and mid-attempt deaths use the ordinary CLI. The subprocess-only host
//! uses the existing cfg(test) send barrier: the staged death is *after durable
//! send intent, before polling send*, not an alleged after-acceptance crash.
//! All transport acceptance, exact reconciliation and recipient multiplicity
//! assertions use the public Application Client, on both live backend routes.

use crate::evidence::*;
use crate::isolation::{self, Isolation};
use crate::management::Command;
use crate::registry_v2::{now_ms, Registry};
use crate::{protocol_v2, transport, Runtime, RuntimeConfig};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::{Child, Stdio};
use std::time::Duration;
use telex::application_client::*;
use tokio::time::{sleep, timeout, Instant};

const PROFILE: &str = "hardening-test";
const TARGET: &str = "agent:hardening-recipient";
const DEADLINE: Duration = Duration::from_secs(100);

struct RuntimeProcess {
    child: Child,
    log: PathBuf,
}

impl RuntimeProcess {
    fn spawn(iso: &Isolation, registry: &Path, mode: Option<&str>) -> Self {
        let label = registry.file_stem().unwrap().to_string_lossy();
        let log = iso.root.join(format!("{label}-host.log"));
        let binary = if mode.is_some() {
            std::env::current_exe().unwrap()
        } else {
            isolation::branch_binary()
                .with_file_name(format!("telex-watcher{}", std::env::consts::EXE_SUFFIX))
        };
        assert!(binary.is_absolute() && binary.is_file(), "{binary:?}");
        let mut command = iso.command_for(&binary);
        if let Some(mode) = mode {
            command
                .args([
                    "--exact",
                    subprocess_test_name(),
                    "--ignored",
                    "--nocapture",
                ])
                .env("TELEX_HARDENING_HOST_REGISTRY", registry)
                .env("TELEX_HARDENING_HOST_MODE", mode)
                .env("TELEX_HARDENING_HOST_ROOT", &iso.root);
        } else {
            command
                .arg("--registry")
                .arg(registry)
                .args(["--json", "run"]);
        }
        let output = std::fs::File::create(&log).unwrap();
        command
            .stdout(Stdio::from(output.try_clone().unwrap()))
            .stderr(Stdio::from(output));
        Self {
            child: command.spawn().unwrap(),
            log,
        }
    }

    fn assert_alive(&mut self) {
        assert!(
            self.child.try_wait().unwrap().is_none(),
            "runtime exited: {}",
            std::fs::read_to_string(&self.log).unwrap_or_default()
        );
    }

    fn kill(&mut self) {
        self.assert_alive();
        self.child.kill().unwrap();
        assert!(!self.child.wait().unwrap().success());
    }

    async fn stop(&mut self, registry: &Path) {
        request(registry, Command::Shutdown).await;
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                assert!(
                    status.success(),
                    "runtime shutdown failed: {}",
                    std::fs::read_to_string(&self.log).unwrap_or_default()
                );
                return;
            }
            assert!(Instant::now() < deadline, "runtime shutdown deadline");
            sleep(Duration::from_millis(20)).await;
        }
    }
}

impl Drop for RuntimeProcess {
    fn drop(&mut self) {
        // The owned Child handle, never a PID/name search, is the kill authority.
        if !matches!(self.child.try_wait(), Ok(Some(_))) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

fn subprocess_test_name() -> &'static str {
    #[cfg(feature = "sqlite")]
    {
        "runtime_hardening_tests::sqlite_watcher_hardening_process_host"
    }
    #[cfg(not(feature = "sqlite"))]
    {
        "runtime_hardening_tests::postgres_watcher_hardening_process_host"
    }
}

async fn subprocess_host() {
    let registry = PathBuf::from(std::env::var_os("TELEX_HARDENING_HOST_REGISTRY").unwrap());
    let root = PathBuf::from(std::env::var_os("TELEX_HARDENING_HOST_ROOT").unwrap());
    let mode = std::env::var("TELEX_HARDENING_HOST_MODE").unwrap();
    let install = PathBuf::from(std::env::var_os("TELEX_INSTALL_ROOT").unwrap());
    let mut config = RuntimeConfig::installed(registry.clone(), install);
    assert_eq!(config.concurrency, 4);
    if mode == "capacity" {
        config.retention_warning_rows = 200;
    }
    let (mut runtime, management) = Runtime::open(config).unwrap();
    let ownership = runtime.retain_registry_ownership();
    if mode == "staged" {
        let (reached, reached_rx) = tokio::sync::oneshot::channel();
        let (mut resume, resume_rx) = tokio::sync::oneshot::channel();
        runtime.send_barrier = Some(crate::runtime_resilience_tests::SendBarrier {
            event_sequence: 1,
            reached,
            resume: resume_rx,
        });
        tokio::spawn(async move {
            reached_rx.await.unwrap();
            std::fs::write(root.join("staged-send-intent"), b"before-send-poll").unwrap();
            resume.closed().await;
        });
    }
    let mut server = tokio::spawn(
        transport::Server::bind(&registry, management)
            .unwrap()
            .run(),
    );
    runtime.run().await.unwrap();
    if timeout(Duration::from_secs(6), &mut server).await.is_err() {
        server.abort();
        let _ = server.await;
    }
    drop(ownership);
}

#[cfg(feature = "sqlite")]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "subprocess fixture; invoked only by the hardening battery"]
async fn sqlite_watcher_hardening_process_host() {
    subprocess_host().await;
}

#[cfg(all(feature = "postgres", not(feature = "sqlite")))]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "subprocess fixture; invoked only by the hardening battery"]
async fn postgres_watcher_hardening_process_host() {
    subprocess_host().await;
}

async fn request(registry: &Path, command: Command) -> Value {
    let response = timeout(
        Duration::from_secs(45),
        transport::execute(registry, command, None),
    )
    .await
    .expect("management deadline")
    .expect("running owner must serve management");
    assert!(response.ok, "{response:?}");
    response.result.unwrap().value
}

async fn wait_for(
    registry: &Path,
    command: impl Fn() -> Command,
    predicate: impl Fn(&Value) -> bool,
) -> Value {
    let deadline = Instant::now() + DEADLINE;
    loop {
        let response = request(registry, command()).await;
        if predicate(&response) {
            return response;
        }
        assert!(
            Instant::now() < deadline,
            "condition not reached: {response}"
        );
        sleep(Duration::from_millis(30)).await;
    }
}

async fn ready(process: &mut RuntimeProcess, registry: &Path) -> Value {
    let deadline = Instant::now() + DEADLINE;
    loop {
        process.assert_alive();
        if let Ok(Ok(response)) = timeout(
            Duration::from_secs(3),
            transport::execute(registry, Command::Health, None),
        )
        .await
        {
            if let Some(result) = response.result {
                if result.value["productionReady"] == true {
                    assert_eq!(result.value["pid"], process.child.id());
                    assert_eq!(result.value["restartReconciled"], true);
                    return result.value;
                }
            }
        }
        assert!(
            Instant::now() < deadline,
            "runtime not ready: {}",
            std::fs::read_to_string(&process.log).unwrap_or_default()
        );
        sleep(Duration::from_millis(30)).await;
    }
}

fn registration(iso: &Isolation, id: &str, group: &str, mode: &str) -> Value {
    let detector = isolation::branch_binary().with_file_name(format!(
        "watcher_hardening_detector{}",
        std::env::consts::EXE_SUFFIX
    ));
    assert!(
        detector.is_absolute() && detector.is_file(),
        "build branch binaries first: cargo build -p telex -p telex-watcher --all-features --bins"
    );
    json!({
        "schemaVersion":2, "id":id, "command":[detector, iso.root.join(group), mode],
        "intervalSeconds":1, "timeoutSeconds":if mode == "hang" { 2 } else { 60 },
        "backendProfile":PROFILE, "sender":format!("service:hardening-{group}"), "target":TARGET,
        "initialState":0
    })
}

fn seed(iso: &Isolation, name: &str, registrations: Vec<Value>) -> PathBuf {
    let registry_path = iso.root.join(format!("{name}.sqlite"));
    let mut registry = Registry::open(&registry_path, 100_000, 256 * 1024 * 1024).unwrap();
    for registration in registrations {
        std::fs::create_dir_all(Path::new(registration["command"][1].as_str().unwrap())).unwrap();
        let parsed =
            protocol_v2::parse_registration(&serde_json::to_vec(&registration).unwrap(), &iso.root)
                .unwrap();
        assert!(registry
            .register(parsed, now_ms())
            .unwrap()
            .refusal
            .is_none());
    }
    registry_path
}

fn started(root: &Path) -> Vec<Value> {
    std::fs::read_dir(root)
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name().to_string_lossy().starts_with("started-"))
        .map(|entry| serde_json::from_slice(&std::fs::read(entry.path()).unwrap()).unwrap())
        .collect()
}

async fn wait_for_file(process: &mut RuntimeProcess, file: &Path) {
    let deadline = Instant::now() + DEADLINE;
    while !file.is_file() {
        process.assert_alive();
        assert!(Instant::now() < deadline, "barrier not reached: {file:?}");
        sleep(Duration::from_millis(20)).await;
    }
}

async fn recipient_history(receiver: &ApplicationClient) -> Vec<HistoryItem> {
    receiver
        .history(Some(TARGET.into()), false, None, None, None, 1000)
        .await
        .unwrap()
}

fn report_retention(
    case: &str,
    before: &Value,
    after: &Value,
    watches: (u64, u64),
    events: (u64, u64),
) {
    let rows = (
        before["retention"]["rows"].as_u64().unwrap(),
        after["retention"]["rows"].as_u64().unwrap(),
    );
    let bytes = (
        before["retention"]["bytes"].as_u64().unwrap(),
        after["retention"]["bytes"].as_u64().unwrap(),
    );
    let row_delta = rows.1.checked_sub(rows.0).unwrap();
    let byte_delta = bytes.1.checked_sub(bytes.0).unwrap();
    println!(
        "{case}: logical retained rows {} -> {} (delta {row_delta}); bytes {} -> {} \
         (delta {byte_delta}); registrations {} -> {}; committed events {} -> {}",
        rows.0, rows.1, bytes.0, bytes.1, watches.0, watches.1, events.0, events.1,
    );
    if watches.1 > watches.0 {
        println!(
            "{case}: measured logical bytes/new watch = {byte_delta}/{} = {:.2} \
             (includes this workload's completed detector history)",
            watches.1 - watches.0,
            byte_delta as f64 / (watches.1 - watches.0) as f64,
        );
    }
    if events.1 > events.0 {
        println!(
            "{case}: measured logical bytes/committed event = {byte_delta}/{} = {:.2} \
             (includes this crash/replacement's runtime and receipt evidence)",
            events.1 - events.0,
            byte_delta as f64 / (events.1 - events.0) as f64,
        );
    }
}

async fn idle_crash(iso: &Isolation) {
    let path = seed(iso, "idle", vec![]);
    let mut first = RuntimeProcess::spawn(iso, &path, None);
    let first_health = ready(&mut first, &path).await;
    assert_eq!(
        request(
            &path,
            Command::List {
                include_removed: true,
                include_unsupported: true
            }
        )
        .await,
        json!([])
    );
    first.kill();
    let mut replacement = RuntimeProcess::spawn(iso, &path, None);
    let second_health = ready(&mut replacement, &path).await;
    assert_ne!(first_health["runtimeId"], second_health["runtimeId"]);
    replacement.stop(&path).await;
    let registry = Registry::open(&path, 100_000, 256 * 1024 * 1024).unwrap();
    assert_eq!(
        registry
            .runtime(first_health["runtimeId"].as_str().unwrap())
            .unwrap()
            .unwrap()
            .status,
        RuntimeStatus::Interrupted
    );
    assert!(registry.list().unwrap().is_empty());
}

async fn mid_attempt_crash(iso: &Isolation, receiver: &ApplicationClient) {
    let path = seed(
        iso,
        "mid",
        vec![registration(iso, "mid", "mid", "terminal")],
    );
    let mut first = RuntimeProcess::spawn(iso, &path, None);
    let first_health = ready(&mut first, &path).await;
    let watch = wait_for(
        &path,
        || Command::Show { id: "mid".into() },
        |watch| watch["runningAttemptId"].is_string(),
    )
    .await;
    let attempt_id = watch["runningAttemptId"].as_str().unwrap().to_owned();
    let capture = iso.root.join("mid").join(format!("started-{attempt_id}"));
    wait_for_file(&mut first, &capture).await;
    let captured: Value = serde_json::from_slice(&std::fs::read(capture).unwrap()).unwrap();
    assert_eq!(captured["request"]["state"], 0);
    first.kill();
    let pid = captured["pid"].as_u64().unwrap() as u32;
    let start_time = captured["startTime"].as_u64().expect("detector identity");
    let deadline = Instant::now() + Duration::from_secs(10);
    while telex::session_watch::process_alive_with_start_time(pid, Some(start_time)) {
        assert!(
            Instant::now() < deadline,
            "killed owner left its detector alive"
        );
        sleep(Duration::from_millis(20)).await;
    }

    let mut replacement = RuntimeProcess::spawn(iso, &path, None);
    let health = ready(&mut replacement, &path).await;
    assert_ne!(first_health["runtimeId"], health["runtimeId"]);
    let watch = wait_for(
        &path,
        || Command::Show { id: "mid".into() },
        |watch| watch["runningAttemptId"].is_string() && watch["runningAttemptId"] != attempt_id,
    )
    .await;
    assert_eq!(
        watch["stateHash"],
        protocol_v2::hash_value(&json!(0)).unwrap()
    );
    assert_eq!(watch["lastCommittedEventSequence"], 0);
    let attempts = request(
        &path,
        Command::Attempts {
            id: "mid".into(),
            limit: 100,
        },
    )
    .await;
    let interrupted = attempts
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["id"] == attempt_id)
        .unwrap();
    assert_eq!(interrupted["outcome"], "interrupted");
    assert_eq!(interrupted["diagnosticCategory"], "runtime-interrupted");
    assert_eq!(interrupted["stateCommitted"], false);
    assert_eq!(interrupted["containmentEvidencePresent"], true);
    assert!(interrupted["finishedAtMs"].is_i64());
    assert_eq!(watch["latestFailure"]["attemptId"], attempt_id);
    std::fs::write(iso.root.join("mid").join("release-all"), b"terminal").unwrap();
    wait_for(
        &path,
        || Command::Show { id: "mid".into() },
        |watch| watch["lifecycle"] == "terminal",
    )
    .await;
    replacement.stop(&path).await;
    let registry = Registry::open(&path, 100_000, 256 * 1024 * 1024).unwrap();
    assert_eq!(registry.get("mid").unwrap().unwrap().state, json!(1));
    assert!(registry.events("mid", 100).unwrap().is_empty());
    let records = started(&iso.root.join("mid"));
    assert_eq!(records.len(), 2);
    assert!(records.iter().all(|record| record["request"]["state"] == 0));
    assert!(recipient_history(receiver).await.is_empty());
}

async fn staged_send_crash(iso: &Isolation, receiver: &ApplicationClient) {
    let path = seed(
        iso,
        "staged",
        vec![registration(iso, "staged", "staged", "event")],
    );
    let mut first = RuntimeProcess::spawn(iso, &path, Some("staged"));
    let before = ready(&mut first, &path).await;
    wait_for(
        &path,
        || Command::Show {
            id: "staged".into(),
        },
        |watch| watch["runningAttemptId"].is_string(),
    )
    .await;
    assert_eq!(
        request(
            &path,
            Command::Events {
                id: "staged".into(),
                limit: 100,
            }
        )
        .await,
        json!([])
    );
    std::fs::write(iso.root.join("staged").join("release-all"), b"event").unwrap();
    wait_for_file(&mut first, &iso.root.join("staged-send-intent")).await;
    first.kill();
    let frozen = {
        let registry = Registry::open(&path, 100_000, 256 * 1024 * 1024).unwrap();
        let watch = registry.get("staged").unwrap().unwrap();
        assert_eq!(watch.state, json!(0));
        assert_eq!(watch.last_committed_event_sequence, 0);
        assert_eq!(watch.last_allocated_event_sequence, 1);
        let operation = registry
            .operation(watch.pending_operation_id.as_ref().unwrap())
            .unwrap()
            .unwrap();
        assert_eq!(operation.status, OperationStatus::QueryRequired);
        assert_eq!(operation.send_budget.used, 1);
        assert_eq!(operation.reconciliation_budget.used, 0);
        assert_eq!(
            registry.attempts("staged", 100).unwrap()[0].outcome,
            AttemptOutcome::Pending
        );
        assert_eq!(
            registry
                .runtime(&operation.runtime_id)
                .unwrap()
                .unwrap()
                .pid,
            first.child.id()
        );
        operation
    };
    let reconciler = ApplicationClient::connect_with_daemon(
        ApplicationClientConfig {
            responsibility: frozen.recovery.responsibility.clone(),
            backend: Some(PROFILE.into()),
            db_override: None,
        },
        ApplicationDaemonBootstrap::InstalledCurrent {
            trusted_root: iso.trusted_root(),
        },
    )
    .await
    .unwrap();
    assert!(matches!(
        reconciler
            .reconcile_operation(&frozen.recovery)
            .await
            .unwrap(),
        OperationReconciliation::NotRecorded(_)
    ));
    assert!(recipient_history(receiver).await.is_empty());

    let mut replacement = RuntimeProcess::spawn(iso, &path, None);
    ready(&mut replacement, &path).await;
    wait_for(
        &path,
        || Command::Show {
            id: "staged".into(),
        },
        |watch| watch["lifecycle"] == "terminal" && watch["lastCommittedEventSequence"] == 1,
    )
    .await;
    let evidence = request(
        &path,
        Command::ReceiptEvidence {
            operation_id: frozen.operation_id.clone(),
            limit: 100,
        },
    )
    .await;
    assert!(
        evidence.as_array().unwrap().iter().any(|row| {
            row["evidence"]["source"] == "reconciliation"
                && row["evidence"]["result"]["status"] == "not-recorded"
        }),
        "{evidence}"
    );
    let after = request(&path, Command::Health).await;
    report_retention("staged-recovery", &before, &after, (1, 1), (0, 1));
    // Kill again after the receipt commit; another owner must not regenerate it.
    replacement.kill();
    let mut last = RuntimeProcess::spawn(iso, &path, None);
    ready(&mut last, &path).await;
    last.stop(&path).await;
    let recovered = {
        let registry = Registry::open(&path, 100_000, 256 * 1024 * 1024).unwrap();
        let watch = registry.get("staged").unwrap().unwrap();
        assert_eq!(watch.state, frozen.proposed_state);
        assert_eq!(watch.last_allocated_event_sequence, 1);
        assert_eq!(watch.last_committed_event_sequence, 1);
        assert!(watch.pending_operation_id.is_none());
        let operations = registry.events("staged", 100).unwrap();
        assert_eq!(operations.len(), 1);
        assert_eq!(registry.attempts("staged", 100).unwrap().len(), 1);
        operations.into_iter().next().unwrap()
    };
    assert_eq!(recovered.status, OperationStatus::Accepted);
    assert_eq!(recovered.operation_id, frozen.operation_id);
    assert_eq!(recovered.event_id, frozen.event_id);
    assert_eq!(recovered.attempt_id, frozen.attempt_id);
    assert_eq!(recovered.runtime_id, frozen.runtime_id);
    assert_eq!(
        recovered.registration_revision,
        frozen.registration_revision
    );
    assert_eq!(recovered.envelope, frozen.envelope);
    assert_eq!(recovered.recovery, frozen.recovery);
    assert_eq!(recovered.envelope_hash, frozen.envelope_hash);
    assert_eq!(recovered.prior_state_hash, frozen.prior_state_hash);
    assert_eq!(recovered.proposed_state_hash, frozen.proposed_state_hash);
    crate::staging::verify(&recovered).unwrap();
    let receipt = recovered.receipt.as_ref().unwrap();
    assert_eq!(receipt.axes.durable_acceptance, EvidenceState::Accepted);
    let OperationReconciliation::Recorded(record) = reconciler
        .reconcile_operation(&frozen.recovery)
        .await
        .unwrap()
    else {
        panic!("accepted exact operation missing from public reconciliation");
    };
    let recorded_receipt = match record.outcome {
        RecordedOperationOutcome::Accepted(receipt)
        | RecordedOperationOutcome::Duplicate(receipt) => receipt,
        other => panic!("exact operation did not remain accepted: {other:?}"),
    };
    assert_eq!(recorded_receipt.message_id, receipt.message_id);
    assert_eq!(
        recorded_receipt.payload_identity,
        frozen.recovery.payload_identity
    );
    let history = recipient_history(receiver).await;
    assert_eq!(history.len(), 1, "duplicate or missing durable send");
    assert_eq!(history[0].message.id, receipt.message_id);
    let metadata: EventMetadata =
        serde_json::from_str(history[0].message.metadata.as_deref().unwrap()).unwrap();
    assert_eq!(metadata.watcher.operation_id, frozen.operation_id);
    assert_eq!(metadata.watcher.event_id, frozen.event_id);
    assert_eq!(metadata.watcher.event_sequence, 1);
    assert_eq!(started(&iso.root.join("staged")).len(), 1);
}

async fn bad_detectors(iso: &Isolation, receiver: &ApplicationClient) {
    let cases = [
        ("malformed", DiagnosticCategory::MalformedOutput),
        ("nonzero", DiagnosticCategory::ProcessFailure),
        ("hang", DiagnosticCategory::Timeout),
        ("stdout-cap", DiagnosticCategory::OutputLimit),
        ("stderr-cap", DiagnosticCategory::OutputLimit),
    ];
    let registrations = cases
        .iter()
        .map(|(id, _)| registration(iso, id, "bad", id))
        .collect();
    let path = seed(iso, "bad", registrations);
    let before = recipient_history(receiver).await.len();
    let mut process = RuntimeProcess::spawn(iso, &path, None);
    ready(&mut process, &path).await;
    for (id, category) in cases {
        let category = serde_json::to_value(category).unwrap();
        let attempts = wait_for(
            &path,
            || Command::Attempts {
                id: id.into(),
                limit: 100,
            },
            |rows| {
                rows.as_array()
                    .unwrap()
                    .iter()
                    .any(|row| row["outcome"] == "failed" && row["diagnosticCategory"] == category)
            },
        )
        .await;
        let failed = attempts
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["diagnosticCategory"] == category)
            .unwrap();
        assert_eq!(failed["stateCommitted"], false);
        assert_eq!(failed["containmentEvidencePresent"], true);
        let watch = request(&path, Command::Show { id: id.into() }).await;
        assert_eq!(
            watch["stateHash"],
            protocol_v2::hash_value(&json!(0)).unwrap()
        );
        assert_eq!(watch["lastAllocatedEventSequence"], 0);
        assert_eq!(watch["lastCommittedEventSequence"], 0);
        assert!(watch["pendingOperationId"].is_null());
        assert_eq!(watch["latestFailure"]["diagnosticCategory"], category);
        assert_eq!(
            request(
                &path,
                Command::Events {
                    id: id.into(),
                    limit: 100
                }
            )
            .await,
            json!([])
        );
        request(&path, Command::Pause { id: id.into() }).await;
    }
    process.stop(&path).await;
    let registry = Registry::open(&path, 100_000, 256 * 1024 * 1024).unwrap();
    for (id, _) in cases {
        assert_eq!(registry.get(id).unwrap().unwrap().state, json!(0));
        assert!(registry.events(id, 100).unwrap().is_empty());
    }
    let records = started(&iso.root.join("bad"));
    let ran: BTreeSet<_> = records
        .iter()
        .map(|record| record["request"]["watch"]["id"].as_str().unwrap())
        .collect();
    assert_eq!(
        ran,
        cases.iter().map(|(id, _)| *id).collect(),
        "every bad-detector case must actually launch"
    );
    assert_eq!(
        recipient_history(receiver).await.len(),
        before,
        "bad detector sent an event"
    );
}

async fn capacity(iso: &Isolation, receiver: &ApplicationClient) {
    use clap::Parser;
    assert_eq!(
        crate::cli::Cli::try_parse_from(["watcher", "run"])
            .unwrap()
            .concurrency,
        4
    );
    assert_eq!(
        crate::cli::Cli::try_parse_from(["watcher", "--concurrency", "16", "run"])
            .unwrap()
            .concurrency,
        16
    );
    assert!(crate::cli::Cli::try_parse_from(["watcher", "--concurrency", "17", "run"]).is_err());
    let registrations: Vec<_> = (0..100)
        .map(|index| {
            registration(
                iso,
                &format!("capacity-{index:03}"),
                "capacity",
                if index == 0 { "coalescing" } else { "terminal" },
            )
        })
        .collect();
    let path = seed(iso, "capacity", vec![]);
    std::fs::create_dir_all(iso.root.join("capacity")).unwrap();
    let before = recipient_history(receiver).await.len();
    let mut process = RuntimeProcess::spawn(iso, &path, Some("capacity"));
    let before_health = ready(&mut process, &path).await;
    for registration in registrations {
        request(
            &path,
            Command::Register {
                json: serde_json::to_vec(&registration).unwrap(),
                working_directory: iso.root.clone(),
            },
        )
        .await;
    }
    let list_command = || Command::List {
        include_removed: false,
        include_unsupported: false,
    };
    let first = wait_for(&path, list_command, |rows| {
        rows.as_array()
            .unwrap()
            .iter()
            .filter(|row| row["runningAttemptId"].is_string())
            .count()
            == 4
    })
    .await;
    assert_eq!(
        first.as_array().unwrap().len(),
        100,
        "operating target is not a registry cap"
    );
    let initial: BTreeSet<_> = first
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|row| row["runningAttemptId"].as_str().map(str::to_owned))
        .collect();
    for attempt in &initial {
        wait_for_file(
            &mut process,
            &iso.root.join("capacity").join(format!("started-{attempt}")),
        )
        .await;
    }
    let coalesced = wait_for(&path, list_command, |rows| {
        rows.as_array()
            .unwrap()
            .iter()
            .filter(|row| row["runningAttemptId"].is_string())
            .all(|row| {
                row["coalescedDue"] == true && row["suppressedDueCount"].as_u64().unwrap() >= 2
            })
    })
    .await;
    let held: BTreeSet<_> = coalesced
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|row| row["runningAttemptId"].as_str().map(str::to_owned))
        .collect();
    assert_eq!(
        held, initial,
        "single-flight attempt IDs changed while held"
    );
    assert_eq!(
        started(&iso.root.join("capacity")).len(),
        4,
        "global concurrency exceeded four"
    );
    for watch in coalesced
        .as_array()
        .unwrap()
        .iter()
        .filter(|row| row["runningAttemptId"].is_string())
    {
        let attempts = request(
            &path,
            Command::Attempts {
                id: watch["id"].as_str().unwrap().into(),
                limit: 100,
            },
        )
        .await;
        assert_eq!(
            attempts.as_array().unwrap().len(),
            1,
            "overlap created a second attempt"
        );
    }

    std::fs::write(iso.root.join("capacity").join("release-all"), b"finish-all").unwrap();
    let followup = wait_for(
        &path,
        || Command::Show {
            id: "capacity-000".into(),
        },
        |watch| {
            watch["stateHash"] == protocol_v2::hash_value(&json!(1)).unwrap()
                && watch["runningAttemptId"].is_string()
        },
    )
    .await;
    let followup_id = followup["runningAttemptId"].as_str().unwrap();
    assert!(!initial.contains(followup_id));
    wait_for_file(
        &mut process,
        &iso.root
            .join("capacity")
            .join(format!("started-{followup_id}")),
    )
    .await;
    let attempts = request(
        &path,
        Command::Attempts {
            id: "capacity-000".into(),
            limit: 100,
        },
    )
    .await;
    assert_eq!(
        attempts.as_array().unwrap().len(),
        2,
        "multiple missed ticks must produce one single-flight follow-up"
    );
    std::fs::write(
        iso.root.join("capacity").join("release-followup"),
        b"finish-followup",
    )
    .unwrap();
    let deadline = Instant::now() + DEADLINE;
    loop {
        let watches = request(&path, list_command()).await;
        let watches = watches.as_array().unwrap();
        assert_eq!(watches.len(), 100);
        assert!(
            watches
                .iter()
                .filter(|row| row["runningAttemptId"].is_string())
                .count()
                <= 4
        );
        if watches.iter().all(|row| row["lifecycle"] == "terminal") {
            break;
        }
        assert!(Instant::now() < deadline, "an eligible watch was starved");
        sleep(Duration::from_millis(30)).await;
    }
    let health = request(&path, Command::Health).await;
    report_retention("capacity-100", &before_health, &health, (0, 100), (0, 0));
    let retained = &health["retention"];
    assert!(retained["rows"].as_u64().unwrap() >= 200);
    assert!(retained["rows"].as_u64().unwrap() < 5_000);
    assert!(retained["bytes"].as_u64().unwrap() > 0);
    assert!(retained["bytes"].as_u64().unwrap() < 16 * 1024 * 1024);
    assert_eq!(retained["warningRows"], 200);
    assert_eq!(retained["warningBytes"], 256 * 1024 * 1024);
    assert_eq!(retained["warning"], true);
    assert!(health["diagnostics"]
        .as_array()
        .unwrap()
        .contains(&json!("retention-warning")));
    process.stop(&path).await;
    let registry = Registry::open(&path, 100_000, 256 * 1024 * 1024).unwrap();
    let records = started(&iso.root.join("capacity"));
    assert_eq!(records.len(), 101, "more than one coalesced follow-up ran");
    let watches: BTreeSet<_> = records
        .iter()
        .map(|record| record["request"]["watch"]["id"].as_str().unwrap())
        .collect();
    assert_eq!(
        watches.len(),
        100,
        "not every registered watch actually executed"
    );
    for watch in registry.list().unwrap() {
        let expected = if watch.id == "capacity-000" { 2 } else { 1 };
        assert_eq!(watch.state, json!(expected));
        assert_eq!(registry.attempts(&watch.id, 100).unwrap().len(), expected);
        assert!(registry.events(&watch.id, 100).unwrap().is_empty());
    }
    assert_eq!(recipient_history(receiver).await.len(), before);
}

async fn battery(iso: &Isolation) {
    let receiver = ApplicationClient::connect_with_daemon(
        ApplicationClientConfig {
            responsibility: ApplicationResponsibility(format!(
                "hardening-receiver-{}",
                uuid::Uuid::new_v4()
            )),
            backend: Some(PROFILE.into()),
            db_override: None,
        },
        ApplicationDaemonBootstrap::InstalledCurrent {
            trusted_root: iso.trusted_root(),
        },
    )
    .await
    .unwrap();
    let attached = receiver
        .attach(&[AddressSpec {
            address: TARGET.into(),
            capability: ApplicationCapability::Bidirectional,
            description: None,
            scope: None,
            tags: None,
        }])
        .await;
    assert!(attached.ready, "{attached:?}");
    idle_crash(iso).await;
    mid_attempt_crash(iso, &receiver).await;
    staged_send_crash(iso, &receiver).await;
    bad_detectors(iso, &receiver).await;
    capacity(iso, &receiver).await;
    receiver.detach(TARGET).await.unwrap();
}

struct Fixture {
    iso: Isolation,
    #[cfg(feature = "postgres")]
    schema: Option<PostgresSchema>,
}

impl Fixture {
    #[cfg(feature = "sqlite")]
    fn sqlite() -> Self {
        let iso = Isolation::new("hardening-sqlite");
        iso.write_config(&telex::profiles::ConfigFile {
            default: None,
            backends: BTreeMap::from([(
                PROFILE.into(),
                telex::profiles::implicit_sqlite(Some(iso.db_path.to_str().unwrap())),
            )]),
        });
        Self {
            iso,
            #[cfg(feature = "postgres")]
            schema: None,
        }
    }

    #[cfg(feature = "postgres")]
    fn postgres(url: String) -> Self {
        let iso = Isolation::new("hardening-postgres");
        let schema = format!("hardening_{}", uuid::Uuid::new_v4().simple());
        let mut profile = telex::profiles::implicit_sqlite(None);
        profile.kind = "postgres".into();
        profile.path = None;
        profile.url = Some(url.clone());
        profile.schema = Some(schema.clone());
        profile.auth = Some("password".into());
        if std::env::var("TELEX_PG_PASSWORD").is_ok() {
            profile.password_env = Some("TELEX_PG_PASSWORD".into());
        }
        iso.write_config(&telex::profiles::ConfigFile {
            default: None,
            backends: BTreeMap::from([(PROFILE.into(), profile)]),
        });
        Self {
            iso,
            schema: Some(PostgresSchema { url, schema }),
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        // Isolation verifies the daemon PID/start identity before confirming stop.
        // Preserve the root if that proof is unavailable; never widen cleanup.
        if self.iso.stop_daemon_best_effort() {
            let _ = std::fs::remove_dir_all(&self.iso.root);
        }
        #[cfg(feature = "postgres")]
        drop(self.schema.take());
    }
}

#[cfg(feature = "postgres")]
struct PostgresSchema {
    url: String,
    schema: String,
}

#[cfg(feature = "postgres")]
impl Drop for PostgresSchema {
    fn drop(&mut self) {
        let url = self.url.clone();
        let schema = self.schema.clone();
        // A separate thread avoids nesting or dropping a Tokio runtime inside
        // the test runtime. Teardown must also be safe while unwinding.
        let thread = std::thread::Builder::new().spawn(move || {
            let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            else {
                return;
            };
            runtime.block_on(async move {
                let _ = timeout(Duration::from_secs(10), async move {
                    let Ok(mut config) = url.parse::<tokio_postgres::Config>() else {
                        return;
                    };
                    if let Ok(password) = std::env::var("TELEX_PG_PASSWORD") {
                        config.password(password);
                    }
                    let Ok(tls) = telex::backend::postgres::make_tls() else {
                        return;
                    };
                    let Ok((client, connection)) = config.connect(tls).await else {
                        return;
                    };
                    let connection = tokio::spawn(connection);
                    let _ = client
                        .batch_execute(&format!("DROP SCHEMA IF EXISTS {schema} CASCADE"))
                        .await;
                    drop(client);
                    connection.abort();
                    let _ = connection.await;
                })
                .await;
            });
        });
        if let Ok(thread) = thread {
            let _ = thread.join();
        }
    }
}

#[cfg(feature = "sqlite")]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn sqlite_watcher_runtime_hardening() {
    let _lock = isolation::ENV_LOCK.lock().await;
    let fixture = Fixture::sqlite();
    let _restore = fixture.iso.apply_env();
    battery(&fixture.iso).await;
}

#[cfg(feature = "postgres")]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn postgres_watcher_runtime_hardening() {
    let _lock = isolation::ENV_LOCK.lock().await;
    let Some(url) = isolation::postgres_url_or_fail_closed("watcher-hardening") else {
        return;
    };
    let fixture = Fixture::postgres(url);
    let _restore = fixture.iso.apply_env();
    battery(&fixture.iso).await;
}
