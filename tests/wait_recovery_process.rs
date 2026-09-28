#![cfg(all(feature = "sqlite", feature = "postgres"))]

use clap::Parser;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};
use telex::backend::postgres::make_tls;
use telex::daemon_ipc::{Request, Response, CAP_WAIT_BACKEND_RECOVERY};
use telex::profiles::{BackendProfile, ConfigFile};

pub use telex::{cli, daemon_ipc, identity, model, session_watch};

#[allow(dead_code)]
mod historical_wait {
    include!("fixtures/wait-recovery/historical_wait.rs");
}

mod historical_protocol {
    use serde::{Deserialize, Serialize};
    use telex::daemon_ipc::*;
    include!("fixtures/wait-recovery/historical_hello.rs");
    include!("fixtures/wait-recovery/historical_outcome.rs");
}

// Module adapter: the unchanged historical frontend uses the production verified
// connection with its pinned Hello, and records the actual wire response.
mod daemon {
    use super::*;
    pub use telex::daemon::{DaemonError, Result};

    pub struct DaemonClient(telex::daemon::DaemonClient);

    pub async fn connect_existing(store_key: &str) -> Result<DaemonClient> {
        let hello = historical_protocol::client_hello(store_key);
        let client = telex::daemon::test_support::connect_with_hello(&hello).await?;
        trace(json!({"hello": hello, "ack": client.ack}));
        Ok(DaemonClient(client))
    }

    impl DaemonClient {
        pub async fn request(&mut self, request: &Request) -> Result<Response> {
            let response = telex::daemon::test_support::request_wire(&mut self.0, request).await?;
            trace(json!({"request": request, "response": response}));
            serde_json::from_value(response)
                .map_err(|error| DaemonError::Protocol(error.to_string()))
        }
    }
}

fn trace(value: Value) {
    let path = std::env::var_os("TELEX_FIXTURE_TRACE").expect("fixture trace path");
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .unwrap();
    writeln!(file, "{value}").unwrap();
}

fn historical_result(result: anyhow::Result<i32>) -> i32 {
    include!("fixtures/wait-recovery/historical_result.rs")
}

#[test]
#[ignore = "subprocess role, invoked only by the isolated process proof"]
fn fixture_process() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    std::process::exit(runtime.block_on(async { historical_result(fixture_role().await) }));
}

async fn fixture_role() -> anyhow::Result<i32> {
    let role = std::env::var("TELEX_FIXTURE_ROLE")?;
    if role == "daemon" {
        telex::daemon::serve().await?;
        return Ok(0);
    }
    let args: Vec<String> = serde_json::from_str(&std::env::var("TELEX_FIXTURE_ARGS")?)?;
    let parsed = cli::Cli::try_parse_from(args)?;
    let ctx = cli::Ctx {
        cfg: telex::config::Config::resolve(parsed.backend, parsed.db, parsed.address.clone())?,
        fmt: telex::output::Format::resolve(parsed.json, parsed.text),
        address: parsed.address,
    };
    if role == "legacy-status" || role == "capable-status" {
        let store = ctx.store_key()?;
        let hello = if role == "legacy-status" {
            historical_protocol::client_hello(&store)
        } else {
            daemon_ipc::client_hello(&store)
        };
        let mut client = telex::daemon::test_support::connect_with_hello(&hello).await?;
        trace(json!({"hello": hello, "ack": client.ack}));
        let cap = telex::daemon::read_cap_file(&client.paths.cap_path)?;
        let response = telex::daemon::test_support::request_wire(
            &mut client,
            &Request::Status {
                store_key: Some(store),
                detail: true,
                proof: Some(cap.admin_cap),
            },
        )
        .await?;
        trace(json!({"response": response}));
        return Ok(0);
    }
    match parsed.command {
        cli::Command::Wait(args) => historical_wait::run(&ctx, args).await,
        cli::Command::Attach(args) => telex::commands::attach::run(&ctx, args).await,
        cli::Command::Send(args) => telex::commands::send::run(&ctx, args).await,
        cli::Command::Daemon(args) => telex::commands::daemon::run(&ctx, args).await,
        _ => anyhow::bail!("unsupported fixture command"),
    }
}

#[test]
fn historical_fixture_sources_match_pinned_extractions() {
    use sha2::{Digest, Sha256};
    let manifest: Value =
        serde_json::from_str(include_str!("fixtures/wait-recovery/provenance.json")).unwrap();
    assert_eq!(
        manifest["commit"],
        "7ed886b07620e8ab8adba68249ab84f96ea26013"
    );
    for (name, bytes) in [
        (
            "historical_wait.rs",
            include_bytes!("fixtures/wait-recovery/historical_wait.rs").as_slice(),
        ),
        (
            "historical_hello.rs",
            include_bytes!("fixtures/wait-recovery/historical_hello.rs").as_slice(),
        ),
        (
            "historical_outcome.rs",
            include_bytes!("fixtures/wait-recovery/historical_outcome.rs").as_slice(),
        ),
        (
            "historical_result.rs",
            include_bytes!("fixtures/wait-recovery/historical_result.rs").as_slice(),
        ),
    ] {
        // Git checkouts can translate LF to CRLF; the source text must otherwise be identical.
        let text = std::str::from_utf8(bytes).unwrap().replace("\r\n", "\n");
        let actual: String = Sha256::digest(text.as_bytes())
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        assert_eq!(actual, manifest["extractions"][name]["sha256"], "{name}");
    }
    let hello = historical_protocol::client_hello("test");
    assert_eq!(
        hello.protocol_version,
        daemon_ipc::ProtocolVersion { major: 1, minor: 5 }
    );
    assert!(!hello
        .capabilities
        .iter()
        .any(|cap| cap == CAP_WAIT_BACKEND_RECOVERY));
    assert!(!hello
        .required_capabilities
        .iter()
        .any(|cap| cap == CAP_WAIT_BACKEND_RECOVERY));
}

struct ProcessOutput {
    code: i32,
    pid: u32,
    stdout: String,
    stderr: String,
    trace: Vec<Value>,
}

struct Fixture {
    root: PathBuf,
    schema: String,
    legacy: bool,
    env: BTreeMap<String, String>,
    server: Option<Child>,
    sequence: usize,
}

impl Fixture {
    fn new(legacy: bool, url: &str, password: &str) -> Self {
        let suffix = format!(
            "{}_{}_{}",
            std::process::id(),
            model::now_ms(),
            usize::from(legacy)
        );
        let root = std::env::temp_dir().join(format!("tw{suffix}"));
        fs::create_dir(&root).unwrap();
        let schema = format!("telex_wait_process_{suffix}");
        let profile = BackendProfile {
            kind: "postgres".into(),
            path: None,
            url: Some(url.into()),
            auth: Some("password".into()),
            password_env: Some("TELEX_PG_PASSWORD".into()),
            password_command: None,
            schema: Some(schema.clone()),
            entra_cred: None,
            entra_scope: None,
        };
        let config = ConfigFile {
            default: Some("test".into()),
            backends: BTreeMap::from([("test".into(), profile)]),
        };
        fs::write(root.join("config.toml"), toml::to_string(&config).unwrap()).unwrap();
        let env = BTreeMap::from([
            (
                "TELEX_HOME".into(),
                root.join("home").to_string_lossy().into_owned(),
            ),
            (
                "TELEX_RUN_DIR".into(),
                root.join("run").to_string_lossy().into_owned(),
            ),
            (
                "TELEX_DB".into(),
                root.join("unused.db").to_string_lossy().into_owned(),
            ),
            (
                "TELEX_INSTALL_ROOT".into(),
                root.join("install").to_string_lossy().into_owned(),
            ),
            (
                "TELEX_CONFIG".into(),
                root.join("config.toml").to_string_lossy().into_owned(),
            ),
            ("TELEX_SESSION_ID".into(), "wait-process".into()),
            ("TELEX_PG_PASSWORD".into(), password.into()),
        ]);
        Self {
            root,
            schema,
            legacy,
            env,
            server: None,
            sequence: 0,
        }
    }

    fn command(&self, role: &str, args: &[String]) -> Command {
        let mut command = if self.legacy {
            let mut command = Command::new(std::env::current_exe().unwrap());
            command
                .args(["--exact", "fixture_process", "--ignored", "--nocapture"])
                .env("TELEX_FIXTURE_ROLE", role)
                .env("TELEX_FIXTURE_ARGS", serde_json::to_string(args).unwrap());
            command
        } else {
            let bin = Path::new(env!("CARGO_BIN_EXE_telex"));
            assert!(bin.is_absolute());
            let mut command = Command::new(bin);
            command.args(&args[1..]);
            command
        };
        command
            .envs(&self.env)
            .env_remove("TELEX_BACKEND")
            .env_remove("TELEX_ADDRESS")
            .env_remove("TELEX_SESSION_PID")
            .env_remove("COPILOT_AGENT_SESSION_ID")
            .env_remove("COPILOT_LOADER_PID")
            .stdin(Stdio::null());
        command
    }

    fn args(extra: &[&str]) -> Vec<String> {
        [
            "telex",
            "--backend",
            "test",
            "--address",
            "addr:pull",
            "--json",
        ]
        .into_iter()
        .chain(extra.iter().copied())
        .map(str::to_string)
        .collect()
    }

    fn start(&mut self) {
        let mut command = self.command("daemon", &Self::args(&["daemon", "serve"]));
        command
            .stdout(File::create(self.root.join("daemon.stdout")).unwrap())
            .stderr(File::create(self.root.join("daemon.stderr")).unwrap());
        self.server = Some(command.spawn().unwrap());
        let deadline = Instant::now() + Duration::from_secs(8);
        loop {
            assert!(
                self.server.as_mut().unwrap().try_wait().unwrap().is_none(),
                "fixture daemon exited: {}",
                fs::read_to_string(self.root.join("daemon.stderr")).unwrap()
            );
            if fs::read_dir(self.root.join("run"))
                .ok()
                .is_some_and(|entries| {
                    entries
                        .filter_map(Result::ok)
                        .any(|entry| entry.path().extension().is_some_and(|ext| ext == "cap"))
                })
            {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "daemon readiness expired: {:?}",
                self.root
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        let parent = std::process::id().to_string();
        self.run(
            "command",
            &[
                "attach",
                "--session",
                "wait-process",
                "--watch-pid",
                &parent,
            ],
            0,
        );
    }

    fn run(&mut self, role: &str, args: &[&str], expected: i32) -> ProcessOutput {
        self.sequence += 1;
        let stem = self.root.join(format!("command-{}", self.sequence));
        let trace_path = stem.with_extension("jsonl");
        let mut command = self.command(role, &Self::args(args));
        command
            .env("TELEX_FIXTURE_TRACE", &trace_path)
            .stdout(File::create(stem.with_extension("stdout")).unwrap())
            .stderr(File::create(stem.with_extension("stderr")).unwrap());
        let mut child = command.spawn().unwrap();
        let pid = child.id();
        let deadline = Instant::now() + Duration::from_secs(15);
        let status = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break status;
            }
            if Instant::now() >= deadline {
                child.kill().unwrap();
                child.wait().unwrap();
                panic!("child {pid} timed out; evidence: {:?}", self.root);
            }
            std::thread::sleep(Duration::from_millis(10));
        };
        let stdout = fs::read_to_string(stem.with_extension("stdout")).unwrap();
        let stderr = fs::read_to_string(stem.with_extension("stderr")).unwrap();
        assert_eq!(
            status.code(),
            Some(expected),
            "stdout={stdout}\nstderr={stderr}\nroot={:?}",
            self.root
        );
        assert!(!stderr.contains("panicked at"), "{stderr}");
        let trace = if trace_path.exists() {
            fs::read_to_string(trace_path)
                .unwrap()
                .lines()
                .map(|line| serde_json::from_str(line).unwrap())
                .collect()
        } else {
            Vec::new()
        };
        ProcessOutput {
            code: expected,
            pid,
            stdout,
            stderr,
            trace,
        }
    }

    fn stop(&mut self) {
        self.run("command", &["daemon", "stop", "--drain"], 0);
        let child = self.server.as_mut().unwrap();
        let deadline = Instant::now() + Duration::from_secs(8);
        loop {
            if let Some(status) = child.try_wait().unwrap() {
                assert!(status.success(), "daemon failed to drain");
                break;
            }
            assert!(Instant::now() < deadline, "daemon did not stop");
            std::thread::sleep(Duration::from_millis(10));
        }
        self.server = None;
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if let Some(mut child) = self.server.take() {
            if child.try_wait().ok().flatten().is_none() {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
        if std::thread::panicking() {
            eprintln!("retained failed fixture evidence: {}", self.root.display());
        }
    }
}

#[derive(Deserialize)]
struct HistoricalMember {
    #[serde(default)]
    last_waiter_outcome: Option<historical_protocol::WaiterOutcome>,
    last_waiter_exit_code: Option<i32>,
    last_waiter_detail: Option<String>,
    last_waiter_exit_at_ms: Option<i64>,
    last_waiter_pid: Option<u32>,
}

#[derive(Deserialize)]
struct HistoricalStatus {
    members: Vec<HistoricalMember>,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum HistoricalResponse {
    StatusReport { status: HistoricalStatus },
}

fn wire_response(output: &ProcessOutput) -> &Value {
    &output
        .trace
        .iter()
        .rev()
        .find(|entry| entry.get("response").is_some())
        .expect("actual wire response")["response"]
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn postgres_wait_recovery_historical_and_current_process_outcomes() {
    let url = match std::env::var("TELEX_PG_URL") {
        Ok(url) if !url.is_empty() => url,
        _ => {
            let required = std::env::var("TELEX_PG_REQUIRE").unwrap_or_default();
            assert!(
                required != "1" && !required.eq_ignore_ascii_case("true"),
                "TELEX_PG_REQUIRE set but TELEX_PG_URL missing"
            );
            eprintln!("TELEX_PG_URL unset; skipping process proof");
            return;
        }
    };
    let mut cfg: tokio_postgres::Config = url.parse().unwrap();
    if let Ok(password) = std::env::var("TELEX_PG_PASSWORD") {
        cfg.password(password);
    }
    let password = std::str::from_utf8(cfg.get_password().expect("test password")).unwrap();
    let (control, connection) = cfg.connect(make_tls().unwrap()).await.unwrap();
    tokio::spawn(async move { connection.await.expect("process proof control") });

    for legacy in [true, false] {
        let mut fixture = Fixture::new(legacy, &url, password);
        fixture.start();
        let control_dir = fixture.root.join("timeout");
        let control_path = control_dir.to_str().unwrap();
        fixture.run(
            "command",
            &["wait", "--timeout-ms", "100", "--out-dir", control_path],
            2,
        );
        assert_eq!(
            fs::read_to_string(control_dir.join("exit.code"))
                .unwrap()
                .trim(),
            "2"
        );
        fixture.run(
            "command",
            &["send", "--to", "addr:pull", "--body", "preserved message"],
            0,
        );
        control
            .batch_execute(&format!(
                "SET search_path TO {}, public;
             CREATE FUNCTION fail_wait_fetch() RETURNS trigger AS $$
             BEGIN RAISE EXCEPTION 'isolated wait recovery fault' USING ERRCODE='57P01'; END
             $$ LANGUAGE plpgsql;
             CREATE TRIGGER fail_wait_fetch BEFORE INSERT ON deliveries
             FOR EACH ROW EXECUTE FUNCTION fail_wait_fetch();
             DELETE FROM deliveries WHERE recipient='addr:pull';",
                fixture.schema,
            ))
            .await
            .unwrap();
        let out_dir = fixture.root.join("exhaustion");
        fs::create_dir(&out_dir).unwrap();
        fs::write(out_dir.join("message.json"), "stale message").unwrap();
        fs::write(out_dir.join("delivery.json"), "stale delivery").unwrap();
        let outcome = fixture.run(
            "command",
            &[
                "wait",
                "--timeout-ms",
                "8000",
                "--out-dir",
                out_dir.to_str().unwrap(),
            ],
            if legacy { 1 } else { 7 },
        );
        let artifacts: Value =
            serde_json::from_slice(&fs::read(out_dir.join("status.json")).unwrap()).unwrap();
        assert_eq!(
            artifacts["outcome"],
            if legacy {
                "error"
            } else {
                "backend-unavailable"
            }
        );
        assert_eq!(artifacts["exit_code"], outcome.code);
        assert_eq!(
            fs::read_to_string(out_dir.join("exit.code"))
                .unwrap()
                .trim(),
            outcome.code.to_string()
        );
        assert_eq!(
            fs::read_to_string(out_dir.join("wait.pid")).unwrap().trim(),
            outcome.pid.to_string()
        );
        assert!(!out_dir.join("message.json").exists());
        assert!(!out_dir.join("delivery.json").exists());
        assert!(outcome.stderr.contains(if legacy {
            "BackendUnavailable"
        } else {
            "backend-unavailable"
        }));

        if legacy {
            let hello = &outcome.trace[0]["hello"];
            assert_eq!(hello["protocol_version"], json!({"major": 1, "minor": 5}));
            for key in ["capabilities", "required_capabilities"] {
                assert!(!hello[key]
                    .as_array()
                    .unwrap()
                    .contains(&json!(CAP_WAIT_BACKEND_RECOVERY)));
            }
            assert_eq!(outcome.trace[0]["ack"]["accepted"], true);
            assert!(outcome.trace[0]["ack"]["capabilities"]
                .as_array()
                .unwrap()
                .contains(&json!(CAP_WAIT_BACKEND_RECOVERY)));
            let error = wire_response(&outcome);
            assert_eq!(error["type"], "error");
            assert_eq!(error["code"], "BackendUnavailable");
            let detail = error["message"].as_str().unwrap();
            assert!(detail.contains("backend recovery grace expired"));
            assert_eq!(artifacts["detail"], format!("BackendUnavailable: {detail}"));
            let legacy_status = fixture.run("legacy-status", &["daemon", "status"], 0);
            let wire = wire_response(&legacy_status);
            assert!(wire["status"]["members"][0]
                .get("last_waiter_outcome")
                .is_none());
            let HistoricalResponse::StatusReport { status } =
                serde_json::from_value::<HistoricalResponse>(wire.clone()).unwrap();
            let member = &status.members[0];
            assert_eq!(member.last_waiter_outcome, None);
            assert_eq!(member.last_waiter_exit_code, Some(7));
            assert_eq!(member.last_waiter_detail.as_deref(), Some(detail));
            assert!(member.last_waiter_exit_at_ms.unwrap() > 0);
            assert_eq!(member.last_waiter_pid, Some(outcome.pid));
            let capable_status = fixture.run("capable-status", &["daemon", "status"], 0);
            let capable = &wire_response(&capable_status)["status"]["members"][0];
            assert_eq!(capable["last_waiter_outcome"], "backend-unavailable");
            for field in [
                "last_waiter_exit_code",
                "last_waiter_detail",
                "last_waiter_exit_at_ms",
                "last_waiter_pid",
            ] {
                assert_eq!(
                    capable[field], wire["status"]["members"][0][field],
                    "{field}"
                );
            }
            let without_dir = fixture.run("command", &["wait", "--timeout-ms", "8000"], 1);
            assert_eq!(wire_response(&without_dir)["code"], "BackendUnavailable");
            assert!(without_dir
                .stderr
                .contains("BackendUnavailable: backend recovery grace expired"));
        } else {
            let output = fixture.run("command", &["daemon", "status"], 0);
            let status: Value = serde_json::from_str(&output.stdout).unwrap();
            let member = &status["members"][0];
            assert_eq!(member["last_waiter_outcome"], "backend-unavailable");
            assert_eq!(member["last_waiter_exit_code"], 7);
            assert_eq!(member["last_waiter_detail"], artifacts["detail"]);
            assert_eq!(member["last_waiter_pid"], outcome.pid);
        }
        control
            .batch_execute(&format!(
                "DROP TRIGGER fail_wait_fetch ON {}.deliveries",
                fixture.schema
            ))
            .await
            .unwrap();
        fixture.stop();
        control
            .batch_execute(&format!("DROP SCHEMA {} CASCADE", fixture.schema))
            .await
            .unwrap();
        fs::remove_dir_all(&fixture.root).unwrap();
        assert!(!fixture.root.exists());
    }
}
