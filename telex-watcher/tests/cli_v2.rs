use clap::Parser;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::process::{Child, Command as ProcessCommand, Output, Stdio};
use std::time::{Duration, Instant};
use telex::application_client::*;
use telex_watcher::cli::{Cli, Filters};
use telex_watcher::evidence::*;
use telex_watcher::management::{Command, Response};
use telex_watcher::presentation::Envelope;
use telex_watcher::protocol_v2::Registration;
use telex_watcher::{ManagementAudit, Mutation, Refusal};

const SECRET: &str = "opaque-password=do-not-print-793183";

struct Fixture {
    root: PathBuf,
    user_home: Option<PathBuf>,
}

impl Fixture {
    fn new() -> Self {
        let root = std::env::current_dir()
            .unwrap()
            .join(format!(".cli-v2-fixture-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("home")).unwrap();
        std::fs::create_dir_all(root.join("install")).unwrap();
        Self {
            root: std::fs::canonicalize(root).unwrap(),
            user_home: None,
        }
    }

    fn file(&self, name: &str, bytes: impl AsRef<[u8]>) -> PathBuf {
        let path = self.root.join(name);
        std::fs::write(&path, bytes).unwrap();
        path
    }

    fn process(&self) -> ProcessCommand {
        let binary = PathBuf::from(env!("CARGO_BIN_EXE_telex-watcher"));
        assert!(binary.is_absolute());
        let mut command = ProcessCommand::new(binary);
        command
            .env("TELEX_HOME", self.root.join("home"))
            .env("TELEX_DB", self.root.join("telex.sqlite"))
            .env("TELEX_INSTALL_ROOT", self.root.join("install"))
            .env("TELEX_WATCHER_REGISTRY", self.root.join("registry.sqlite"))
            .env("CLI_FIXTURE_TOKEN", SECRET)
            .current_dir(&self.root);
        if let Some(home) = &self.user_home {
            command.env("HOME", home).env("USERPROFILE", home);
        }
        command
    }

    fn execute(&self, args: &[&str]) -> (Output, Value) {
        let output = self.process().args(args).arg("--json").output().unwrap();
        let value: Value = serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
            panic!(
                "expected JSON ({error}); status={} stdout={} stderr={}",
                output.status,
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr),
            )
        });
        assert_eq!(value["schemaVersion"], 1);
        assert_eq!(value["ok"], output.status.success());
        (output, value)
    }

    fn start_host(&self) -> Host {
        let mut host = Host(
            self.process()
                .args(["run", "--json"])
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap(),
        );
        let started = Instant::now();
        loop {
            let (_, response) = self.execute(&["status"]);
            if response["ok"] == true {
                return host;
            }
            assert!(host.0.try_wait().unwrap().is_none(), "runtime exited early");
            assert!(started.elapsed() < Duration::from_secs(15), "{response}");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn stop_host(&self, host: &mut Host) {
        let (_, shutdown) = self.execute(&["shutdown"]);
        assert_eq!(shutdown["ok"], true);
        let stopped = Instant::now();
        while host.0.try_wait().unwrap().is_none() {
            assert!(stopped.elapsed() < Duration::from_secs(15));
            std::thread::sleep(Duration::from_millis(20));
        }
        let (_, stopped) = self.execute(&["status"]);
        assert_eq!(stopped["error"]["code"], "runtime-not-running");
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let home_removed = self.user_home.as_ref().map(std::fs::remove_dir_all);
        let removed = std::fs::remove_dir_all(&self.root);
        if !std::thread::panicking() {
            if let Some(home_removed) = home_removed {
                home_removed.expect("remove isolated user home");
            }
            removed.expect("remove isolated CLI fixture");
        }
    }
}

struct Host(Child);

impl Drop for Host {
    fn drop(&mut self) {
        let status = self.0.try_wait();
        let killed = if matches!(&status, Ok(Some(_))) {
            Ok(())
        } else {
            self.0.kill()
        };
        let _ = self.0.wait();
        if !std::thread::panicking() {
            let _ = status.expect("query isolated CLI host");
            killed.expect("kill isolated CLI host");
        }
    }
}

#[test]
fn fixture_cleanup_failure_while_unwinding_preserves_the_original_panic() {
    let unwound = std::panic::catch_unwind(|| {
        let fixture = Fixture::new();
        std::fs::remove_dir_all(&fixture.root).unwrap();
        panic!("original test failure");
    })
    .unwrap_err();
    assert_eq!(
        unwound.downcast_ref::<&str>(),
        Some(&"original test failure")
    );
}

#[test]
fn fixture_cleanup_failure_without_unwinding_still_fails() {
    let fixture = Fixture::new();
    std::fs::remove_dir_all(&fixture.root).unwrap();
    assert!(std::panic::catch_unwind(|| drop(fixture)).is_err());
}

#[test]
fn isolated_home_cleanup_failure_preserves_unwinding_but_fails_normally() {
    for panicking in [true, false] {
        let unwound = std::panic::catch_unwind(|| {
            let mut fixture = Fixture::new();
            fixture.user_home = Some(fixture.root.join("missing-home"));
            if panicking {
                panic!("original home test failure");
            }
            drop(fixture);
        })
        .unwrap_err();
        if panicking {
            assert_eq!(
                unwound.downcast_ref::<&str>(),
                Some(&"original home test failure")
            );
        }
    }
}

fn parse(arguments: &[&str]) -> Cli {
    Cli::try_parse_from(
        ["telex-watcher", "--registry", "fixture.sqlite"]
            .into_iter()
            .chain(arguments.iter().copied()),
    )
    .unwrap()
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn binary_owned_stall_detector_reads_live_management_and_readonly_store() {
    use std::io::Write;
    use telex::backend::{sqlite::SqliteBackend, Backend};
    let mut fixture = Fixture::new();
    // Unix management sockets use HOME and need a short, non-world-writable ancestry.
    let home = dirs::home_dir().unwrap().join(format!(
        ".{}",
        &uuid::Uuid::new_v4().simple().to_string()[..8]
    ));
    std::fs::create_dir(&home).unwrap();
    fixture.user_home = Some(home.clone());
    let telex = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join(if cfg!(windows) { "telex.exe" } else { "telex" });
    assert!(
        telex.is_absolute() && telex.is_file(),
        "run this test with cargo test --workspace"
    );
    let store = fixture.root.join("source.sqlite");
    let backend = SqliteBackend::open(store.to_str().unwrap()).unwrap();
    backend.init_schema().await.unwrap();
    drop(backend);
    let config = fixture.file("profiles.toml", format!(
        "[backends.fixture]\nkind = \"sqlite\"\npath = {}\n[backends.alias]\nkind = \"sqlite\"\npath = {}\n",
        serde_json::to_string(&store).unwrap(), serde_json::to_string(&store).unwrap()
    ));
    let group = fixture.root.join("group");
    std::fs::create_dir(&group).unwrap();
    fixture.file(
        "group/member.json",
        serde_json::to_vec(&json!({
            "schemaVersion":1,"sessionId":"member","address":"member:a","state":"active"
        }))
        .unwrap(),
    );
    let log_dir = home.join(".copilot").join("session-state").join("member");
    std::fs::create_dir_all(&log_dir).unwrap();
    std::fs::write(
        log_dir.join("events.jsonl"),
        include_bytes!("../../tests/fixtures/copilot-session-state/idle.jsonl"),
    )
    .unwrap();
    let mut host = fixture.start_host();
    // The root is active at member:a; its own monitoring watch must not suppress detection.
    let own_watch = fixture.file(
        "own-watch.json",
        serde_json::to_vec(&json!({
            "schemaVersion":2,"id":"group-stall","command":[&telex,"copilot","stall-detect"],
            "intervalSeconds":86400,"timeoutSeconds":1,"backendProfile":"fixture",
            "sender":"service:fixture","target":"member:a"
        }))
        .unwrap(),
    );
    let (_, own_registered) = fixture.execute(&["register", own_watch.to_str().unwrap()]);
    assert_eq!(own_registered["ok"], true);
    let parameters = json!({
        "groupDirectory":group,"backendProfile":"fixture","watcherExecutable":env!("CARGO_BIN_EXE_telex-watcher"),
        "watcherRegistry":fixture.root.join("registry.sqlite"),"graceSeconds":60
    });
    let invoke = |seconds: i64, state: Value| {
        let now = chrono::DateTime::parse_from_rfc3339("2026-10-26T12:00:00Z").unwrap()
            + chrono::Duration::seconds(seconds);
        let request = json!({
            "schemaVersion":2,"attempt":{"id":"fixture","now":now.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)},
            "watch":{"id":"group-stall","activatedAt":"2026-10-26T11:00:00Z","lastSuccessAt":null,
                "parameters":parameters}, "state":state
        });
        let mut child = ProcessCommand::new(&telex)
            .args(["copilot", "stall-detect"])
            .env("HOME", &home)
            .env("USERPROFILE", &home)
            .env("LOCALAPPDATA", fixture.root.join("local"))
            .env("TELEX_HOME", fixture.root.join("home"))
            .env("TELEX_DB", &store)
            .env("TELEX_CONFIG", &config)
            .env("TELEX_INSTALL_ROOT", fixture.root.join("install"))
            .env_remove("TELEX_BACKEND")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(&serde_json::to_vec(&request).unwrap())
            .unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let result: Value = serde_json::from_slice(&output.stdout).unwrap();
        telex::watcher_protocol::parse_result(&output.stdout).unwrap();
        (result, output)
    };
    let before_store = std::fs::read(&store).unwrap();
    let before_log = std::fs::read(log_dir.join("events.jsonl")).unwrap();
    let mut state = Value::Null;
    for seconds in [0, 30, 60] {
        let (result, output) = invoke(seconds, state);
        assert!(
            output.stderr.is_empty(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(result["outcome"], "idle");
        state = result["nextState"].clone();
    }
    let (event, _) = invoke(61, state);
    assert_eq!(event["event"]["kind"], "copilot.group.stalled");
    assert_eq!(invoke(62, event["nextState"].clone()).0["outcome"], "idle");
    assert_eq!(std::fs::read(&store).unwrap(), before_store);
    assert_eq!(
        std::fs::read(log_dir.join("events.jsonl")).unwrap(),
        before_log
    );
    std::fs::write(
        log_dir.join("events.jsonl"),
        concat!(
            "{\"type\":\"session.error\",\"id\":\"error-1\",",
            "\"timestamp\":\"2026-10-25T20:00:00Z\",\"data\":{}}\n"
        ),
    )
    .unwrap();
    let (advisory, _) = invoke(90, event["nextState"].clone());
    assert_eq!(advisory["event"]["kind"], "copilot.group.attention-needed");
    assert_eq!(advisory["event"]["metadata"]["degraded"], true);
    assert_eq!(
        invoke(100, advisory["nextState"].clone()).0["outcome"],
        "degraded"
    );
    std::fs::write(log_dir.join("events.jsonl"), before_log).unwrap();
    // The same physical store under a different profile still arms a member's wake source.
    let registration = fixture.file(
        "wake.json",
        serde_json::to_vec(&json!({
            "schemaVersion":2,"id":"member-wake","command":[&telex,"copilot","stall-detect"],
            "intervalSeconds":86400,"timeoutSeconds":1,"backendProfile":"alias",
            "sender":"service:fixture","target":"member:a"
        }))
        .unwrap(),
    );
    let (_, registered) = fixture.execute(&["register", registration.to_str().unwrap()]);
    assert_eq!(registered["ok"], true);
    let (result, _) = invoke(120, Value::Null);
    assert_eq!(result["outcome"], "idle");
    assert_eq!(result["nextState"], Value::Null);
    fixture.stop_host(&mut host);
    let (result, output) = invoke(180, Value::Null);
    assert_eq!(result["outcome"], "degraded");
    assert!(!output.stderr.is_empty());
    assert!(result.get("nextState").is_none());
}

#[test]
fn parser_covers_every_operation_without_requiring_install_root() {
    let commands = [
        ("run", vec!["run"]),
        ("register", vec!["register", "registration.json"]),
        ("list", vec!["list"]),
        ("show", vec!["show", "watch"]),
        ("pause", vec!["pause", "watch"]),
        ("resume", vec!["resume", "watch"]),
        ("update", vec!["update", "watch", "registration.json"]),
        ("reconcile-now", vec!["reconcile-now", "watch"]),
        (
            "grant-reconciliation",
            vec![
                "grant-reconciliation",
                "watch",
                "--attempts",
                "2",
                "--seconds",
                "60",
            ],
        ),
        (
            "grant-send-retry",
            vec![
                "grant-send-retry",
                "watch",
                "--attempts",
                "2",
                "--seconds",
                "60",
            ],
        ),
        ("close-not-accepted", vec!["close-not-accepted", "watch"]),
        ("remove", vec!["remove", "watch"]),
        ("status", vec!["status"]),
        ("health", vec!["health"]),
        ("status", vec!["status", "watch"]),
        ("health", vec!["health", "watch"]),
        ("attempts", vec!["attempts", "watch", "--limit", "10"]),
        ("events", vec!["events", "watch", "--limit", "10"]),
        ("pending", vec!["pending", "watch"]),
        (
            "receipt-evidence",
            vec!["receipt-evidence", "operation", "--limit", "10"],
        ),
        (
            "removed-evidence",
            vec!["removed-evidence", "watch", "--limit", "10"],
        ),
        ("shutdown", vec!["shutdown"]),
        ("shutdown", vec!["stop"]),
    ];
    for (name, mut arguments) in commands {
        arguments.push("--json");
        let cli = parse(&arguments);
        assert_eq!(cli.command_name(), name);
        assert!(cli.json);
        // TELEX_INSTALL_ROOT may be inherited by the test runner, but never required.
        if !matches!(name, "run" | "register" | "update") {
            assert!(cli.management_command().is_ok());
        }
    }
    assert!(parse(&[]).is_runtime());
    assert!(matches!(
        parse(&["status"]).management_command().unwrap(),
        Command::Health
    ));
    assert!(matches!(
        parse(&["status", "watch"]).management_command().unwrap(),
        Command::WatchHealth { id } if id == "watch"
    ));
}

#[test]
fn parser_rejects_unbounded_grants_and_queries() {
    for operation in ["grant-reconciliation", "grant-send-retry"] {
        for (attempts, seconds) in [
            ("0", "1"),
            ("101", "1"),
            ("-1", "1"),
            ("1", "0"),
            ("1", "86401"),
            ("1", "-1"),
        ] {
            assert!(Cli::try_parse_from([
                "watcher",
                "--registry",
                "r",
                operation,
                "watch",
                "--attempts",
                attempts,
                "--seconds",
                seconds,
            ])
            .is_err());
        }
    }
    for limit in ["0", "1001", "-1"] {
        assert!(Cli::try_parse_from([
            "watcher",
            "--registry",
            "r",
            "events",
            "watch",
            "--limit",
            limit,
        ])
        .is_err());
    }
}

#[test]
fn registration_and_update_forward_original_bytes_and_capture_current_directory() {
    let fixture = Fixture::new();
    let bytes = br#"{"doNotParseOrCanonicalizeHere":2.5}"#;
    let path = fixture.file("registration.json", bytes);
    for operation in ["register", "update"] {
        let mut arguments = vec![operation];
        if operation == "update" {
            arguments.push("watch");
        }
        arguments.push(path.to_str().unwrap());
        match parse(&arguments).management_command().unwrap() {
            Command::Register {
                json,
                working_directory,
            }
            | Command::Update {
                json,
                working_directory,
                ..
            } => {
                assert_eq!(json, bytes);
                assert_eq!(working_directory, std::env::current_dir().unwrap());
                assert!(working_directory.is_absolute());
            }
            other => panic!("unexpected {other:?}"),
        }
    }
}

#[test]
fn explicit_null_replaces_state_while_omission_preserves_it_and_strict_checks_apply() {
    let fixture = Fixture::new();
    let registration = fixture.file("registration.json", "{}");
    let state = fixture.file("state.json", "null");
    let base = ["update", "watch", registration.to_str().unwrap()];
    assert!(matches!(
        parse(&base).management_command().unwrap(),
        Command::Update { state: None, .. }
    ));
    let args = [
        "update",
        "watch",
        registration.to_str().unwrap(),
        "--state-file",
        state.to_str().unwrap(),
    ];
    assert!(matches!(
        parse(&args).management_command().unwrap(),
        Command::Update {
            state: Some(Value::Null),
            ..
        }
    ));
    for invalid in [
        br#"{"x":1,"x":2}"#.as_slice(),
        b"9007199254740992",
        br#""\ud800""#,
        b"NaN",
        br#"{"x":null} trailing"#,
    ] {
        std::fs::write(&state, invalid).unwrap();
        assert_eq!(
            parse(&args).management_command().unwrap_err().code,
            "invalid-state"
        );
    }
    std::fs::write(
        &state,
        format!(
            "\"{}\"",
            "a".repeat(telex_watcher::protocol_v2::MAX_STATE_BYTES)
        ),
    )
    .unwrap();
    assert_eq!(
        parse(&args).management_command().unwrap_err().code,
        "input-too-large"
    );
    std::fs::write(&state, "9007199254740992.0").unwrap();
    assert!(parse(&args).management_command().is_ok());
}

fn registration() -> Registration {
    Registration {
        schema_version: 2,
        id: "watch".into(),
        command: vec!["detector".into(), SECRET.into()],
        interval_seconds: 60,
        timeout_seconds: 30,
        backend_profile: "fixture".into(),
        sender: "service:fixture".into(),
        target: "agent:fixture".into(),
        attention: "next-checkpoint".into(),
        requires_disposition: true,
        working_directory: std::env::current_dir().unwrap(),
        environment_allowlist: vec!["API_TOKEN".into()],
        parameters: json!({"anything":SECRET}),
        initial_state: json!({"anything":SECRET}),
    }
}

fn watch() -> Watch {
    Watch {
        schema_version: 2,
        id: "watch".into(),
        registration: Some(registration()),
        registration_revision: 5,
        lifecycle: Lifecycle::Active,
        eligibility: Eligibility::Eligible,
        health: Health::Ready,
        blocked_reason: None,
        activated_at: "2026-01-01T00:00:00.000Z".into(),
        last_success_at: None,
        last_event_at: None,
        last_attempt_id: Some("attempt".into()),
        running_attempt_id: None,
        running_cancelled: false,
        state: json!({"anything":SECRET}),
        logical_store_id: Some("store".into()),
        observed_logical_store_id: Some("store".into()),
        last_allocated_event_sequence: 2,
        last_committed_event_sequence: 1,
        pending_operation_id: Some("operation".into()),
        next_due_ms: 0,
        next_attempt_ms: 0,
        suppressed_due_count: 3,
        coalesced_due: true,
        consecutive_failures: 0,
        latest_attempt_duration_ms: Some(2),
        diagnostic_category: None,
        recurrence: Recurrence::default(),
    }
}

fn receipt() -> SendResult {
    SendResult {
        logical_store_id: LogicalStoreId("store".into()),
        operation_id: OperationId("operation".into()),
        message_id: 4,
        thread_id: 4,
        sender: "service:fixture".into(),
        recipient: "agent:fixture".into(),
        payload_identity: PayloadIdentity {
            algorithm: "sha256".into(),
            digest: "digest".into(),
            comparable: true,
        },
        axes: ReceiptAxes {
            durable_acceptance: EvidenceState::Accepted,
            occupied_at_acceptance: Some(false),
            push_acceptance: EvidenceState::Pending,
            recipient_consumption: EvidenceState::NotAttempted,
            workflow_disposition: EvidenceState::Disposition(SECRET.into()),
        },
        replayed: false,
    }
}

fn operation() -> Operation {
    let recovery = RecoveryHandle {
        logical_store_id: LogicalStoreId("store".into()),
        responsibility: ApplicationResponsibility(SECRET.into()),
        operation_id: OperationId("operation".into()),
        payload_identity: receipt().payload_identity,
        retention_generation: Some(1),
    };
    Operation {
        schema_version: 2,
        watch_id: "watch".into(),
        event_sequence: 2,
        event_id: "event".into(),
        operation_id: "operation".into(),
        attempt_id: "attempt".into(),
        attempt_now: "2026-01-01T00:00:00.000Z".into(),
        runtime_id: "runtime".into(),
        registration_revision: 5,
        registration: registration(),
        prior_state: json!(SECRET),
        proposed_state: json!(SECRET),
        terminal: false,
        envelope: SendRequest {
            operation_id: OperationId("operation".into()),
            sender: "service:fixture".into(),
            to: "agent:fixture".into(),
            cc: vec![],
            kind: SECRET.into(),
            attention: "next-checkpoint".into(),
            requires_disposition: true,
            subject: Some(SECRET.into()),
            body: SECRET.into(),
            metadata: Some(SECRET.into()),
            retry_budget: 0,
        },
        recovery,
        prior_state_hash: "prior-hash".into(),
        proposed_state_hash: "proposed-hash".into(),
        envelope_hash: "envelope-hash".into(),
        recurrence_hash: "recurrence-hash".into(),
        hash_algorithm: "sha256".into(),
        hash_preimage_version: 1,
        status: OperationStatus::QueryRequired,
        query_only: true,
        reconciliation_budget: Budget {
            remaining: 2,
            used: 1,
            deadline_ms: 1000,
        },
        send_budget: Budget {
            remaining: 0,
            used: 1,
            deadline_ms: 1000,
        },
        next_recovery_ms: 100,
        last_evidence_at: None,
        last_evidence: Some(SendEvidence::Send(Err(
            ApplicationClientError::Indeterminate {
                detail: SECRET.into(),
                recovery: Box::new(RecoveryHandle {
                    responsibility: ApplicationResponsibility(SECRET.into()),
                    logical_store_id: LogicalStoreId("store".into()),
                    operation_id: OperationId("operation".into()),
                    payload_identity: receipt().payload_identity,
                    retention_generation: Some(1),
                }),
            },
        ))),
        receipt: Some(receipt()),
    }
}

fn assert_safe(response: Response) -> Value {
    let output = Envelope::from_response("fixture", &response, None, 1000);
    let value = serde_json::to_value(&output).unwrap();
    assert!(!value.to_string().contains(SECRET));
    assert!(!output.human().contains(SECRET));
    assert_eq!(value["schemaVersion"], 1);
    value
}

#[test]
fn safe_projection_omits_all_opaque_policy_state_attempt_and_operation_fields() {
    let value = assert_safe(Response::Watch(Some(watch())));
    let policy = &value["result"]["value"]["registration"];
    assert_eq!(policy["executable"], "detector");
    assert_eq!(policy["argvLength"], 2);
    assert_eq!(policy["environmentAllowlist"], json!(["API_TOKEN"]));
    assert!(policy.get("parameters").is_none());
    assert!(policy.get("initialState").is_none());
    assert!(policy.get("command").is_none());
    assert!(value["result"]["value"].get("state").is_none());
    assert!(value["result"]["value"]["stateHash"].is_string());
    assert_safe(Response::Watches(vec![watch()]));
    assert_safe(Response::Mutation(Mutation {
        watch: Some(watch()),
        refusal: Some(Refusal::PendingOperation),
        diagnostic: Some(SECRET.into()),
        recovery: operation().last_evidence,
        audit: None,
    }));
    let value = assert_safe(Response::Attempts(vec![Attempt {
        schema_version: 2,
        id: "attempt".into(),
        watch_id: "watch".into(),
        runtime_id: "runtime".into(),
        registration_revision: 5,
        started_at: "2026-01-01T00:00:00.000Z".into(),
        started_at_ms: 0,
        finished_at_ms: Some(2),
        command: vec!["detector".into(), SECRET.into()],
        working_directory: std::env::current_dir().unwrap(),
        prior_state_hash: "prior-hash".into(),
        outcome: AttemptOutcome::Failed,
        diagnostic_category: Some(DiagnosticCategory::ProcessFailure),
        diagnostic: Some(SECRET.into()),
        containment: Some(json!({"anything":SECRET})),
        state_committed: false,
    }]));
    assert_eq!(
        value["result"]["value"][0]["diagnosticCategory"],
        "process-failure"
    );
    assert_safe(Response::Pending(Some(Box::new(operation()))));
    let value = assert_safe(Response::Events(vec![operation()]));
    assert_eq!(value["result"]["value"][0]["sendBudget"]["remaining"], 0);
    assert_eq!(
        value["result"]["value"][0]["receipt"]["axes"]["durableAcceptance"],
        "accepted"
    );
    assert!(value["result"]["value"][0].get("envelope").is_none());
    assert!(value["result"]["value"][0].get("recovery").is_none());
}

#[test]
fn every_client_error_is_projected_without_credential_bearing_text() {
    let errors = vec![
        ApplicationClientError::MembershipLost {
            address: "service:fixture".into(),
            reason: MembershipLossReason::Unknown {
                raw_reason: Some(SECRET.into()),
            },
            detail: SECRET.into(),
        },
        ApplicationClientError::Collision(CollisionEvidence {
            address: "service:fixture".into(),
            owner_instance_id: Some("other".into()),
            lease_epoch: Some(1),
            guidance: SECRET.into(),
        }),
        ApplicationClientError::AmbiguousSender(vec![SECRET.into()]),
        ApplicationClientError::UnsupportedCapability(SECRET.into()),
        ApplicationClientError::Partial(SECRET.into()),
        ApplicationClientError::InvalidRequest(SECRET.into()),
        ApplicationClientError::Protocol {
            code: SECRET.into(),
        },
        ApplicationClientError::RejectedBeforeAcceptance {
            code: SECRET.into(),
            retryability: RejectionRetryability::Permanent,
            detail: SECRET.into(),
        },
        ApplicationClientError::TransportUncertain(SECRET.into()),
        ApplicationClientError::Unavailable(SECRET.into()),
    ];
    let evidence: Vec<_> = errors
        .into_iter()
        .map(|error| ReceiptEvidence {
            schema_version: 2,
            operation_id: "operation".into(),
            observed_at: "2026-01-01T00:00:00.000Z".into(),
            evidence: SendEvidence::Reconciliation(Err(error)),
        })
        .collect();
    let value = assert_safe(Response::ReceiptEvidence(evidence));
    assert_eq!(
        value["result"]["value"][7]["evidence"]["error"]["retryability"],
        "permanent"
    );
}

#[test]
fn reconciliation_projects_every_outcome_without_serializing_recovery_handles() {
    let recovery = operation().recovery;
    let outcomes = vec![
        RecordedOperationOutcome::Accepted(receipt()),
        RecordedOperationOutcome::Duplicate(receipt()),
        RecordedOperationOutcome::Rejected(ApplicationClientError::RejectedBeforeAcceptance {
            code: SECRET.into(),
            retryability: RejectionRetryability::Permanent,
            detail: SECRET.into(),
        }),
        RecordedOperationOutcome::Partial {
            error: Some(ApplicationClientError::Partial(SECRET.into())),
            recovery: Some(recovery.clone()),
        },
        RecordedOperationOutcome::Indeterminate {
            error: Some(ApplicationClientError::TransportUncertain(SECRET.into())),
            recovery: recovery.clone(),
        },
        RecordedOperationOutcome::Pending {
            recovery: recovery.clone(),
        },
    ];
    let mut evidence = vec![
        SendEvidence::Send(Ok(Box::new(receipt()))),
        SendEvidence::ObservationCancelled,
        SendEvidence::Reconciliation(Ok(OperationReconciliation::NotRecorded(
            NotRecordedEvidence {
                logical_store_id: LogicalStoreId("store".into()),
                responsibility: ApplicationResponsibility(SECRET.into()),
                operation_id: OperationId("operation".into()),
                payload_identity: recovery.payload_identity.clone(),
                retention_generation: 1,
            },
        ))),
        SendEvidence::Reconciliation(Ok(OperationReconciliation::RetentionBoundaryCrossed {
            staged_generation: Some(1),
            current_generation: 2,
        })),
    ];
    evidence.extend(outcomes.into_iter().map(|outcome| {
        SendEvidence::Reconciliation(Ok(OperationReconciliation::Recorded(Box::new(
            RecordedOperation {
                logical_store_id: LogicalStoreId("store".into()),
                responsibility: ApplicationResponsibility(SECRET.into()),
                operation_id: OperationId("operation".into()),
                sender: "service:fixture".into(),
                target: OperationTarget::Send {
                    to: SECRET.into(),
                    cc: vec![SECRET.into()],
                },
                payload_identity: recovery.payload_identity.clone(),
                retry_budget: 0,
                outcome,
            },
        ))))
    }));
    let response = Response::ReceiptEvidence(
        evidence
            .into_iter()
            .map(|evidence| ReceiptEvidence {
                schema_version: 2,
                operation_id: "operation".into(),
                observed_at: "2026-01-01T00:00:00.000Z".into(),
                evidence,
            })
            .collect(),
    );
    let value = assert_safe(response);
    assert_eq!(
        value["result"]["value"][2]["evidence"]["result"]["status"],
        "not-recorded"
    );
    assert_eq!(
        value["result"]["value"][3]["evidence"]["result"]["status"],
        "retention-boundary-crossed"
    );
    for item in &value["result"]["value"].as_array().unwrap()[4..] {
        assert_eq!(item["evidence"]["result"]["operationId"], "operation");
        assert_eq!(item["evidence"]["result"]["logicalStoreId"], "store");
        assert!(item["evidence"]["result"].get("responsibility").is_none());
        assert!(item["evidence"]["result"]["outcome"]
            .get("recovery")
            .is_none());
    }
}

#[test]
fn filters_are_explicit_unions_and_fixed_id_show_discovers_removed_and_unsupported() {
    let mut active = watch();
    active.pending_operation_id = None;
    let mut paused = active.clone();
    paused.lifecycle = Lifecycle::Paused;
    let mut terminal = active.clone();
    terminal.lifecycle = Lifecycle::Terminal;
    let mut removed = active.clone();
    removed.lifecycle = Lifecycle::Removed;
    let mut unresolved = removed.clone();
    unresolved.pending_operation_id = Some("operation".into());
    let mut unsupported = paused.clone();
    unsupported.schema_version = 1;
    unsupported.registration = None;
    unsupported.blocked_reason = Some(BlockedReason::UnsupportedRegistrationVersion);
    let records = vec![active, paused, terminal, removed, unresolved, unsupported];
    for (arguments, expected) in [
        (vec!["list"], 3),
        (vec!["list", "--active"], 1),
        (vec!["list", "--paused", "--terminal"], 2),
        (vec!["list", "--removed"], 2),
        (vec!["list", "--unresolved-tombstones"], 1),
        (vec!["list", "--unsupported-v1"], 1),
        (vec!["list", "--include-removed"], 5),
        (vec!["list", "--include-unsupported"], 4),
        (vec!["list", "--all"], 6),
        (
            vec!["list", "--unresolved-tombstones", "--unsupported-v1"],
            2,
        ),
    ] {
        let cli = parse(&arguments);
        let result = Envelope::from_response(
            "list",
            &Response::Watches(records.clone()),
            cli.filters(),
            1000,
        );
        assert_eq!(
            result.result.unwrap().value.as_array().unwrap().len(),
            expected,
            "{arguments:?}"
        );
    }
    let cli = parse(&["show", "watch"]);
    assert!(cli.filters().is_none());
    let result = Envelope::from_response(
        "show",
        &Response::Watch(Some(records[3].clone())),
        cli.filters(),
        1000,
    );
    assert!(result.ok);
    assert!(!Filters::default().matches(&records[5]));
}

#[test]
fn unsupported_removal_audit_has_exact_non_applicable_shape_and_no_hash_keys() {
    let mut item = watch();
    item.schema_version = 1;
    item.registration = None;
    item.lifecycle = Lifecycle::Removed;
    let value = assert_safe(Response::Mutation(Mutation {
        watch: Some(item),
        refusal: None,
        diagnostic: None,
        recovery: None,
        audit: Some(Box::new(ManagementAudit {
            action: "remove".into(),
            prior_revision: Some(5),
            resulting_revision: Some(6),
            state_changed: Some(false),
            state_hash_status: Some("not-applicable".into()),
            state_hash_reason: Some(BlockedReason::UnsupportedRegistrationVersion),
            old_state_hash: None,
            new_state_hash: None,
            refusal: None,
        })),
    }));
    assert_eq!(
        value["result"]["value"]["audit"],
        json!({
            "action":"remove",
            "priorRevision":5,
            "resultingRevision":6,
            "stateChanged":false,
            "stateHashStatus":"not-applicable",
            "stateHashReason":"unsupported-registration-version",
        })
    );
    assert!(value["result"]["value"]["watch"].get("stateHash").is_none());
}

#[test]
fn runtime_and_watch_health_preserve_public_axes_but_not_raw_client_diagnostics() {
    let corpus: Value = serde_json::from_str(include_str!("fixtures/evidence-v2.json")).unwrap();
    let watch: WatchHealth = serde_json::from_value(corpus["watch"].clone()).unwrap();
    let projected = assert_safe(Response::WatchHealth(watch));
    assert_eq!(projected["result"]["value"], corpus["watch"]);
    let mut health: RuntimeHealth = serde_json::from_value(corpus["runtime"].clone()).unwrap();
    let heartbeat = chrono::DateTime::parse_from_rfc3339(&health.heartbeat_at)
        .unwrap()
        .timestamp_millis();
    health.senders.push(SenderHealth {
        backend_profile: "fixture".into(),
        required_senders: vec!["service:fixture".into()],
        logical_store_id: Some("store".into()),
        ready: false,
        membership: vec![ApplicationHealth {
            logical_store_id: LogicalStoreId("store".into()),
            responsibility: ApplicationResponsibility(SECRET.into()),
            runtime_id: RuntimeId("runtime".into()),
            address: "service:fixture".into(),
            capability: ApplicationCapability::SendOnly,
            registered: true,
            lease_epoch: Some(1),
            owner_instance_id: Some("owner".into()),
            pending_unconsumed: 7,
            inbound_actionable: 4,
            acknowledgment_pending: true,
            outstanding_ack_count: 1,
            liveness: vec![ProcessLivenessEvidence {
                pid: 1,
                start_time: None,
                alive: true,
                role: SECRET.into(),
            }],
            sender_ready: false,
            receive_ready: false,
            attended_but_deaf: true,
            recovering: true,
            last_recovery_failure: Some(SECRET.into()),
            degraded: true,
            stopped_or_unattended: false,
            principal: PrincipalProvenance {
                principal: Some(SECRET.into()),
                verification: PrincipalVerification::Unverified,
                evidence: Some(SECRET.into()),
            },
            lifecycle: vec![ApplicationLifecycleEvidence::MembershipLoss {
                reason: MembershipLossReason::Unknown {
                    raw_reason: Some(SECRET.into()),
                },
                detail: SECRET.into(),
            }],
            evidence: vec![SECRET.into()],
        }],
        lifecycle: Some(MultiAddressOutcome {
            ready: false,
            results: [(
                "service:fixture".into(),
                AddressLifecycleResult::Failed(ApplicationClientError::Unavailable(SECRET.into())),
            )]
            .into_iter()
            .collect(),
            compensation: vec![],
            validation_error: Some(ApplicationClientError::Partial(SECRET.into())),
            cancellation: None,
        }),
        compensation: vec![],
        error: Some(ApplicationClientError::TransportUncertain(SECRET.into())),
    });
    let response = Response::Health(health);
    let fresh = Envelope::from_response("health", &response, None, heartbeat + 1_000);
    let fresh = serde_json::to_value(fresh).unwrap();
    assert_eq!(fresh["result"]["value"]["heartbeatAgeMs"], 1_000);
    assert_eq!(fresh["result"]["value"]["heartbeatStale"], false);
    assert_eq!(
        fresh["result"]["value"]["senders"][0]["membership"][0]["inboundActionable"],
        4
    );
    assert_eq!(
        fresh["result"]["value"]["senders"][0]["error"]["code"],
        "transport-uncertain"
    );
    let stale = Envelope::from_response("health", &response, None, heartbeat + 6_000);
    assert_eq!(
        serde_json::to_value(stale).unwrap()["result"]["value"]["heartbeatStale"],
        true
    );
    assert_safe(response);
}

#[test]
fn refusal_envelope_and_human_output_preserve_typed_failure_reason() {
    let result = Envelope::from_response(
        "pause",
        &Response::Mutation(Mutation {
            watch: Some(watch()),
            refusal: Some(Refusal::PendingOperation),
            diagnostic: Some(SECRET.into()),
            recovery: None,
            audit: None,
        }),
        None,
        1000,
    );
    assert!(!result.ok);
    assert!(result.human().contains("pending-operation"));
    assert!(result.human().contains("reconciled or removed"));
    assert!(!result.human().contains(SECRET));
    let json = serde_json::to_value(result).unwrap();
    assert_eq!(json["error"]["code"], "pending-operation");
    assert_eq!(json["result"]["type"], "mutation");
    assert_eq!(json["result"]["value"]["refusal"], "pending-operation");
}

#[test]
fn executable_not_running_and_parser_errors_are_stable_json_and_do_not_open_registry() {
    let fixture = Fixture::new();
    for args in [
        vec!["list"],
        vec!["pause", "watch"],
        vec!["status"],
        vec!["shutdown"],
    ] {
        let (_, value) = fixture.execute(&args);
        assert_eq!(value["error"]["code"], "runtime-not-running");
    }
    assert!(!fixture.root.join("registry.sqlite").exists());
    let (output, value) = fixture.execute(&[
        "grant-send-retry",
        "watch",
        "--attempts",
        "0",
        "--seconds",
        "1",
    ]);
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(value["error"]["code"], "invalid-arguments");
    assert!(!value.to_string().contains(SECRET));
    let output = fixture
        .process()
        .env_remove("TELEX_INSTALL_ROOT")
        .args(["list", "--json"])
        .output()
        .unwrap();
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["error"]["code"], "runtime-not-running");
    let output = fixture
        .process()
        .env_remove("TELEX_INSTALL_ROOT")
        .args(["run", "--json"])
        .output()
        .unwrap();
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["error"]["code"], "install-root-required");
    let output = fixture
        .process()
        .env_remove("TELEX_WATCHER_REGISTRY")
        .args(["list", "--json"])
        .output()
        .unwrap();
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["error"]["code"], "invalid-arguments");
    assert!(!fixture.root.join("registry.sqlite").exists());
}

#[test]
fn executable_runtime_supports_complete_ordinary_lifecycle_and_safe_defaults() {
    let fixture = Fixture::new();
    let mut host = fixture.start_host();
    // Missing isolated profile prevents any daemon connection or detector execution.
    let registration = fixture.file(
        "registration.json",
        serde_json::to_vec(&json!({
            "schemaVersion":2,
            "id":"ordinary",
            "command":["not-executed-cli-fixture",SECRET],
            "backendProfile":"nonexistent-isolated-cli-fixture",
            "sender":"service:cli-fixture",
            "target":"agent:cli-fixture",
            "intervalSeconds":86400,
            "timeoutSeconds":1,
            "parameters":{"anything":SECRET},
            "initialState":{"anything":SECRET},
            "environmentAllowlist":["CLI_FIXTURE_TOKEN"]
        }))
        .unwrap(),
    );
    let registration = registration.to_str().unwrap();
    let (_, registered) = fixture.execute(&["register", registration]);
    assert_eq!(registered["ok"], true, "{registered}");
    assert_eq!(
        registered["result"]["value"]["watch"]["registration"]["workingDirectory"],
        fixture.root.to_str().unwrap()
    );
    assert!(!registered.to_string().contains(SECRET));
    let first_revision = registered["result"]["value"]["watch"]["registrationRevision"]
        .as_u64()
        .unwrap();
    let (_, refused) = fixture.execute(&["register", registration]);
    assert_eq!(refused["error"]["code"], "id-already-allocated");
    assert!(!refused.to_string().contains(SECRET));
    let (_, listed) = fixture.execute(&["list", "--active"]);
    assert_eq!(listed["result"]["value"].as_array().unwrap().len(), 1);
    let (_, paused) = fixture.execute(&["pause", "ordinary"]);
    assert_eq!(paused["result"]["value"]["watch"]["lifecycle"], "paused");
    let null = fixture.file("state.json", "null");
    let (_, updated) = fixture.execute(&[
        "update",
        "ordinary",
        registration,
        "--state-file",
        null.to_str().unwrap(),
    ]);
    assert_eq!(updated["ok"], true, "{updated}");
    assert_eq!(
        updated["result"]["value"]["watch"]["stateHash"],
        telex_watcher::protocol_v2::hash_value(&Value::Null).unwrap()
    );
    assert!(
        updated["result"]["value"]["watch"]["registrationRevision"]
            .as_u64()
            .unwrap()
            > first_revision
    );
    let (_, preserved) = fixture.execute(&["update", "ordinary", registration]);
    assert_eq!(
        preserved["result"]["value"]["watch"]["stateHash"],
        updated["result"]["value"]["watch"]["stateHash"]
    );
    let (_, resumed) = fixture.execute(&["resume", "ordinary"]);
    assert_eq!(resumed["result"]["value"]["watch"]["lifecycle"], "active");
    for args in [
        vec!["show", "ordinary"],
        vec!["status", "ordinary"],
        vec!["health", "ordinary"],
        vec!["attempts", "ordinary"],
        vec!["events", "ordinary"],
        vec!["pending", "ordinary"],
        vec!["receipt-evidence", "unknown-operation"],
    ] {
        let (_, value) = fixture.execute(&args);
        assert_eq!(value["ok"], true, "{args:?}: {value}");
        assert!(!value.to_string().contains(SECRET));
    }
    let (_, no_pending) = fixture.execute(&["reconcile-now", "ordinary"]);
    assert_eq!(no_pending["error"]["code"], "no-pending-operation");
    let (_, removed) = fixture.execute(&["remove", "ordinary"]);
    assert_eq!(removed["result"]["value"]["watch"]["lifecycle"], "removed");
    let (_, listed) = fixture.execute(&["list"]);
    assert!(listed["result"]["value"].as_array().unwrap().is_empty());
    let (_, listed) = fixture.execute(&["list", "--removed"]);
    assert_eq!(listed["result"]["value"].as_array().unwrap().len(), 1);
    let (_, shown) = fixture.execute(&["show", "ordinary"]);
    assert_eq!(shown["result"]["value"]["lifecycle"], "removed");
    let (_, refused) = fixture.execute(&["register", registration]);
    assert_eq!(refused["error"]["code"], "id-already-allocated");
    let (_, evidence) = fixture.execute(&["removed-evidence", "ordinary"]);
    assert_eq!(evidence["ok"], true);
    fixture.stop_host(&mut host);
}

#[test]
fn executable_accepted_nested_state_is_projected_before_response_decoding() {
    let fixture = Fixture::new();
    let mut host = fixture.start_host();
    let mut state = Value::Null;
    for _ in 0..122 {
        state = json!([state]);
    }
    let bytes = serde_json::to_vec(&json!({
        "schemaVersion":2,
        "id":"nested",
        "command":["not-executed-cli-fixture"],
        "backendProfile":"nonexistent-isolated-cli-fixture",
        "sender":"service:cli-fixture",
        "target":"agent:cli-fixture",
        "intervalSeconds":86400,
        "timeoutSeconds":1,
        "initialState":state
    }))
    .unwrap();
    telex_watcher::protocol_v2::parse_registration(&bytes, &fixture.root).unwrap();
    let registration = fixture.file("nested.json", bytes);
    let (_, registered) = fixture.execute(&["register", registration.to_str().unwrap()]);
    assert_eq!(registered["ok"], true, "{registered}");
    assert_eq!(
        registered["result"]["value"]["watch"]["stateHash"],
        telex_watcher::protocol_v2::hash_value(&state).unwrap()
    );
    for args in [vec!["show", "nested"], vec!["list"]] {
        let (_, response) = fixture.execute(&args);
        assert_eq!(response["ok"], true, "{response}");
    }
    let state_file = fixture.file("nested-state.json", serde_json::to_vec(&state).unwrap());
    let (_, updated) = fixture.execute(&[
        "update",
        "nested",
        registration.to_str().unwrap(),
        "--state-file",
        state_file.to_str().unwrap(),
    ]);
    assert_eq!(updated["ok"], true, "{updated}");
    let (_, removed) = fixture.execute(&["remove", "nested"]);
    assert_eq!(removed["ok"], true, "{removed}");
    assert_eq!(removed["result"]["value"]["watch"]["lifecycle"], "removed");
    let (_, listed) = fixture.execute(&["list", "--removed"]);
    assert_eq!(listed["result"]["value"].as_array().unwrap().len(), 1);
    fixture.stop_host(&mut host);
}

#[test]
fn executable_large_raw_registrations_do_not_break_safe_or_zero_match_lists() {
    let fixture = Fixture::new();
    let mut host = fixture.start_host();
    let argument = format!("{SECRET}{}", "x".repeat(1024 * 1024 - SECRET.len()));
    for index in 0..16 {
        let bytes = serde_json::to_vec(&json!({
            "schemaVersion":2,
            "id":format!("large-{index}"),
            "command":["not-executed-cli-fixture",argument],
            "backendProfile":"nonexistent-isolated-cli-fixture",
            "sender":"service:cli-fixture",
            "target":"agent:cli-fixture",
            "intervalSeconds":86400,
            "timeoutSeconds":1
        }))
        .unwrap();
        telex_watcher::protocol_v2::parse_registration(&bytes, &fixture.root).unwrap();
        let registration = fixture.file("large.json", bytes);
        let (_, registered) = fixture.execute(&["register", registration.to_str().unwrap()]);
        assert_eq!(registered["ok"], true, "{registered}");
        assert!(!registered.to_string().contains(SECRET));
    }
    for args in [vec!["list"], vec!["list", "--active"]] {
        let (_, listed) = fixture.execute(&args);
        assert_eq!(listed["ok"], true, "{listed}");
        let watches = listed["result"]["value"].as_array().unwrap();
        assert_eq!(watches.len(), 16);
        assert!(watches
            .iter()
            .all(|watch| watch["registration"]["argvLength"] == 2));
        assert!(!listed.to_string().contains(SECRET));
    }
    let (_, listed) = fixture.execute(&["list", "--paused"]);
    assert_eq!(listed["ok"], true, "{listed}");
    assert_eq!(listed["result"]["value"], json!([]));
    fixture.stop_host(&mut host);
}
