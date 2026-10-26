#![cfg(all(windows, feature = "sqlite"))]

#[path = "../../tests/support/telex_isolation.rs"]
mod isolation;

use isolation::Isolation;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::{Duration, Instant};
use telex::backend::{sqlite::SqliteBackend, Backend};
use telex::model::NewMessage;

struct Host(Child);

impl Drop for Host {
    fn drop(&mut self) {
        if !matches!(self.0.try_wait(), Ok(Some(_))) {
            let _ = self.0.kill();
        }
        let _ = self.0.wait();
    }
}

struct Gate {
    isolation: Isolation,
    registry: PathBuf,
    group: PathBuf,
    log: PathBuf,
}

impl Gate {
    fn command(&self, binary: &Path) -> Command {
        let mut command = self.isolation.command_for(binary);
        command
            .env("HOME", &self.isolation.root)
            .env("USERPROFILE", &self.isolation.root);
        command
    }

    fn management(&self, arguments: &[&str]) -> Value {
        let mut command = self.command(Path::new(env!("CARGO_BIN_EXE_telex-watcher")));
        command
            .arg("--registry")
            .arg(&self.registry)
            .arg("--json")
            .args(arguments);
        let output = isolation::run_with_timeout(command, Duration::from_secs(10));
        let response: Value = serde_json::from_str(&output.stdout).unwrap_or_else(|error| {
            panic!(
                "management JSON: {error}; code={:?}; stderr={}",
                output.code, output.stderr
            )
        });
        assert_eq!(response["ok"], output.code == Some(0), "{response}");
        response
    }

    fn successful(&self, arguments: &[&str]) -> Value {
        let response = self.management(arguments);
        assert_eq!(response["ok"], true, "{response}");
        response["result"]["value"].clone()
    }

    fn wait_for(&self, arguments: &[&str], predicate: impl Fn(&Value) -> bool) -> Value {
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            let response = self.management(arguments);
            if response["ok"] == true && predicate(&response["result"]["value"]) {
                return response["result"]["value"].clone();
            }
            assert!(Instant::now() < deadline, "{arguments:?}: {response}");
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    fn register(&self) {
        let registration = json!({
            "schemaVersion":2,
            "id":"scheduled-stall",
            "command":[isolation::branch_binary(),"copilot","stall-detect"],
            "intervalSeconds":1,
            "timeoutSeconds":30,
            "backendProfile":"fixture",
            "sender":"service:gate-watcher",
            "target":"project:gate/root",
            "environmentAllowlist":["TELEX_HOME","TELEX_CONFIG","LOCALAPPDATA"],
            "parameters":{
                "groupDirectory":self.group,
                "backendProfile":"fixture",
                "watcherExecutable":env!("CARGO_BIN_EXE_telex-watcher"),
                "watcherRegistry":self.registry,
                "graceSeconds":2,
                "pollIntervalSeconds":1
            }
        });
        let path = self.isolation.root.join("registration.txt");
        std::fs::write(&path, serde_json::to_vec_pretty(&registration).unwrap()).unwrap();
        self.successful(&["register", path.to_str().unwrap()]);
    }

    fn set_log(&self, bytes: &[u8]) {
        let temporary = self.log.with_extension("tmp");
        std::fs::write(&temporary, bytes).unwrap();
        std::fs::rename(temporary, &self.log).unwrap();
    }

    fn assert_suppressed(&self) {
        let after = telex::model::now_ms() + 3_000;
        self.wait_for(&["attempts", "scheduled-stall"], |attempts| {
            attempts.as_array().unwrap().iter().any(|attempt| {
                attempt["startedAtMs"]
                    .as_i64()
                    .is_some_and(|at| at >= after)
                    && attempt["outcome"] == "idle"
                    && attempt["stateCommitted"] == true
            })
        });
        assert!(self
            .successful(&["events", "scheduled-stall"])
            .as_array()
            .unwrap()
            .is_empty());
    }

    fn invoke_at(&self, seconds: i64, state: Value) -> Value {
        let now = chrono::DateTime::parse_from_rfc3339("2026-10-26T12:00:00Z").unwrap()
            + chrono::Duration::seconds(seconds);
        let request = json!({
            "schemaVersion":2,
            "attempt":{"id":format!("gap-{seconds}"),"now":now.to_rfc3339_opts(chrono::SecondsFormat::Secs,true)},
            "watch":{
                "id":"manual-gap","activatedAt":"2026-10-26T11:00:00Z","lastSuccessAt":null,
                "parameters":{
                    "groupDirectory":self.group,"backendProfile":"fixture",
                    "watcherExecutable":env!("CARGO_BIN_EXE_telex-watcher"),
                    "watcherRegistry":self.registry,"graceSeconds":60,"pollIntervalSeconds":30
                }
            },
            "state":state
        });
        let input = self.isolation.root.join("request.txt");
        std::fs::write(&input, serde_json::to_vec(&request).unwrap()).unwrap();
        let mut command = self.command(&isolation::branch_binary());
        command
            .args(["copilot", "stall-detect"])
            .stdin(std::fs::File::open(input).unwrap());
        let output = isolation::run_with_timeout(command, Duration::from_secs(30));
        output.assert_success("isolated detector observation");
        telex::watcher_protocol::parse_result(output.stdout.as_bytes()).unwrap();
        serde_json::from_str(&output.stdout).unwrap()
    }
}

#[tokio::test]
async fn scheduled_stall_reaches_own_host_and_delivers_once_per_episode_with_long_history() {
    let isolation = Isolation::new("stall-gate");
    let group = isolation.root.join("group");
    std::fs::create_dir(&group).unwrap();
    std::fs::write(
        &isolation.config_path,
        format!(
            "[backends.fixture]\nkind = \"sqlite\"\npath = {}\n",
            serde_json::to_string(&isolation.db_path).unwrap()
        ),
    )
    .unwrap();
    let backend = SqliteBackend::open(isolation.db_path.to_str().unwrap()).unwrap();
    backend.init_schema().await.unwrap();
    for _ in 0..1100 {
        let message = backend
            .insert_message(&NewMessage {
                to_addr: "project:gate/member".into(),
                body: "synthetic gate history".into(),
                ..Default::default()
            })
            .await
            .unwrap();
        backend
            .mark_delivered(message.id, "project:gate/member", None)
            .await
            .unwrap();
    }
    drop(backend);
    std::fs::write(
        group.join("member.json"),
        serde_json::to_vec(&json!({
            "schemaVersion":1,"sessionId":"member",
            "address":"project:gate/member","state":"active"
        }))
        .unwrap(),
    )
    .unwrap();
    let log_directory = isolation
        .root
        .join(".copilot")
        .join("session-state")
        .join("member");
    std::fs::create_dir_all(&log_directory).unwrap();
    let gate = Gate {
        registry: isolation.root.join("watcher.sqlite"),
        group,
        log: log_directory.join("events.jsonl"),
        isolation,
    };
    gate.set_log(include_bytes!(
        "../../tests/fixtures/copilot-session-state/busy.jsonl"
    ));
    let mut host = Host(
        gate.command(Path::new(env!("CARGO_BIN_EXE_telex-watcher")))
            .arg("--registry")
            .arg(&gate.registry)
            .arg("run")
            .spawn()
            .unwrap(),
    );
    gate.wait_for(&["status"], |_| true);
    gate.register();
    gate.assert_suppressed();
    gate.set_log(include_bytes!(
        "../../tests/fixtures/copilot-session-state/waiting.jsonl"
    ));
    gate.assert_suppressed();
    let wake_registration = gate.isolation.root.join("wake.txt");
    std::fs::write(
        &wake_registration,
        serde_json::to_vec(&json!({
            "schemaVersion":2,"id":"member-wake",
            "command":[isolation::branch_binary(),"copilot","stall-detect"],
            "intervalSeconds":86400,"timeoutSeconds":30,"backendProfile":"fixture",
            "sender":"service:gate-watcher","target":"project:gate/member",
            "environmentAllowlist":["TELEX_HOME","TELEX_CONFIG","LOCALAPPDATA"],
            "parameters":{
                "groupDirectory":gate.isolation.root.join("absent-group"),
                "backendProfile":"fixture",
                "watcherExecutable":env!("CARGO_BIN_EXE_telex-watcher"),
                "watcherRegistry":gate.registry,"graceSeconds":2,"pollIntervalSeconds":86400
            }
        }))
        .unwrap(),
    )
    .unwrap();
    gate.successful(&["register", wake_registration.to_str().unwrap()]);
    gate.set_log(include_bytes!(
        "../../tests/fixtures/copilot-session-state/idle.jsonl"
    ));
    gate.assert_suppressed();
    let removal_started_at = telex::model::now_ms();
    gate.successful(&["remove", "member-wake"]);
    let first = gate.wait_for(&["show", "scheduled-stall"], |watch| {
        watch["lastCommittedEventSequence"] == 1
    });
    assert_eq!(first["consecutiveFailures"], 0);
    let first_events = gate.successful(&["events", "scheduled-stall"]);
    assert_eq!(first_events.as_array().unwrap().len(), 1);
    assert_eq!(first_events[0]["status"], "accepted");
    assert_eq!(
        first_events[0]["receipt"]["axes"]["durableAcceptance"],
        "accepted"
    );
    let emitted_attempt = first["lastAttemptId"].clone();
    gate.wait_for(&["attempts", "scheduled-stall"], |attempts| {
        attempts.as_array().unwrap().iter().any(|attempt| {
            attempt["id"] != emitted_attempt
                && attempt["startedAtMs"].as_i64().is_some_and(|time| {
                    time > first_events[0]["attemptNow"]
                        .as_str()
                        .and_then(|now| chrono::DateTime::parse_from_rfc3339(now).ok())
                        .unwrap()
                        .timestamp_millis()
                })
                && attempt["outcome"] == "idle"
                && attempt["stateCommitted"] == true
        })
    });
    assert_eq!(
        gate.successful(&["events", "scheduled-stall"])
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let backend = SqliteBackend::open_readonly(gate.isolation.db_path.to_str().unwrap()).unwrap();
    let inbox = backend.inbox("project:gate/root", true, 10).await.unwrap();
    assert_eq!(inbox.len(), 1);
    assert_eq!(inbox[0].message.kind, "copilot.group.stalled");
    let first_snapshot: Value = serde_json::from_str(&inbox[0].message.body).unwrap();
    assert!(
        first_snapshot["firstSeenAt"].as_i64().unwrap() >= removal_started_at,
        "episode began while the member wake was still armed: {first_snapshot}"
    );
    drop(backend);

    gate.set_log(b"{\"type\":\"session.error\",\"id\":\"error-1\",\"timestamp\":\"2026-10-25T20:00:00Z\",\"data\":{}}\n");
    gate.wait_for(&["show", "scheduled-stall"], |watch| {
        watch["lastCommittedEventSequence"] == 2
    });
    gate.wait_for(&["health", "scheduled-stall"], |health| {
        health["diagnosticCategory"] == "detector-degraded"
    });
    assert_eq!(
        gate.successful(&["events", "scheduled-stall"])
            .as_array()
            .unwrap()
            .len(),
        2
    );
    let backend = SqliteBackend::open_readonly(gate.isolation.db_path.to_str().unwrap()).unwrap();
    let inbox = backend.inbox("project:gate/root", true, 10).await.unwrap();
    assert_eq!(inbox.len(), 2);
    for kind in ["copilot.group.stalled", "copilot.group.attention-needed"] {
        let rows: Vec<_> = inbox
            .iter()
            .filter(|row| row.message.kind == kind)
            .collect();
        assert_eq!(rows.len(), 1);
        let snapshot: Value = serde_json::from_str(&rows[0].message.body).unwrap();
        assert_eq!(snapshot["members"][0]["backgroundWork"], "unobserved");
        assert_eq!(
            snapshot["degraded"],
            kind == "copilot.group.attention-needed"
        );
    }
    drop(backend);
    gate.successful(&["pause", "scheduled-stall"]);
    gate.set_log(include_bytes!(
        "../../tests/fixtures/copilot-session-state/idle.jsonl"
    ));
    let mut state = gate.invoke_at(0, Value::Null)["nextState"].clone();
    let member_path = gate.group.join("member.json");
    let member = std::fs::read(&member_path).unwrap();
    for (failed_at, successful_at, expected) in [(30, 70, "idle"), (100, 140, "event")] {
        std::fs::write(&member_path, b"{").unwrap();
        let failed = gate.invoke_at(failed_at, state.clone());
        assert_eq!(failed, json!({"schemaVersion":2,"outcome":"degraded"}));
        std::fs::write(&member_path, &member).unwrap();
        let successful = gate.invoke_at(successful_at, state);
        assert_eq!(successful["outcome"], expected);
        state = successful["nextState"].clone();
        assert_eq!(state["gapRestarted"], true);
        if expected == "event" {
            assert_eq!(successful["event"]["kind"], "copilot.group.stalled");
        }
    }
    assert_eq!(gate.invoke_at(210, state)["outcome"], "idle");
    gate.successful(&["shutdown"]);
    let deadline = Instant::now() + Duration::from_secs(15);
    while host.0.try_wait().unwrap().is_none() {
        assert!(Instant::now() < deadline, "isolated host did not stop");
        std::thread::sleep(Duration::from_millis(20));
    }
    gate.isolation.stop_daemon();
}
