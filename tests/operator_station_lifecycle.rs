#![cfg(feature = "sqlite")]

use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::ffi::OsStr;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const BACKEND: &str = "operator-validation";
const INGRESS: &str = "validation:operator-ingress";
const HUMAN: &str = "validation:human-station";
const WORKER: &str = "validation:worker";
const HANDOFF_WORKER: &str = "validation:handoff-worker";
const EXTENSION_ID: &str = "urn:telex:operator-station:v1";
const WORKFLOW_SIGNATURE: &str = "telex-copilot-v0.1.2/operator-station-op-v1";
const STALE_AFTER: Duration = Duration::from_secs(24 * 60 * 60);

static NEXT_RUN: AtomicUsize = AtomicUsize::new(1);

#[derive(Debug)]
struct RunOutput {
    status: std::process::ExitStatus,
    stdout: String,
    stderr: String,
}

impl RunOutput {
    fn assert_success(&self, context: &str) {
        assert!(
            self.status.success(),
            "{context} failed: status={} stdout={} stderr={}",
            self.status,
            self.stdout,
            self.stderr
        );
    }

    fn json(&self, context: &str) -> Value {
        serde_json::from_str(&self.stdout).unwrap_or_else(|error| {
            panic!(
                "{context} did not emit JSON: {error}; stdout={} stderr={}",
                self.stdout, self.stderr
            )
        })
    }
}

struct IsolatedTelexPlane {
    repo: PathBuf,
    dedicated_root: PathBuf,
    run_root: PathBuf,
    home: PathBuf,
    db: PathBuf,
    install_root: PathBuf,
    run_dir: PathBuf,
    state_dir: PathBuf,
    bin: PathBuf,
    cleaned: bool,
}

impl IsolatedTelexPlane {
    fn new() -> Self {
        let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .canonicalize()
            .expect("canonical repository root");
        let dedicated_root = repo.join("target").join("operator-station-lifecycle-tests");
        std::fs::create_dir_all(&dedicated_root).expect("create dedicated lifecycle test root");
        let dedicated_root = dedicated_root
            .canonicalize()
            .expect("canonical dedicated lifecycle test root");
        cleanup_stale_runs(&dedicated_root);

        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock after epoch")
            .as_secs();
        let sequence = NEXT_RUN.fetch_add(1, Ordering::SeqCst);
        let run_root =
            dedicated_root.join(format!("run-{timestamp}-{}-{sequence}", std::process::id()));
        let home = run_root.join("home");
        let db = run_root.join("database").join("telex.sqlite");
        let install_root = run_root.join("install");
        let run_dir = run_root.join("run");
        let state_dir = run_root.join("state");

        for path in [
            &run_root,
            &home,
            db.parent().expect("database parent"),
            &install_root,
            &run_dir,
            &state_dir,
        ] {
            std::fs::create_dir_all(path)
                .unwrap_or_else(|error| panic!("create {}: {error}", path.display()));
            restrict_directory(path);
        }

        let bin = worktree_telex_bin();
        let plane = Self {
            repo,
            dedicated_root,
            run_root,
            home,
            db,
            install_root,
            run_dir,
            state_dir,
            bin,
            cleaned: false,
        };
        plane.assert_isolated();
        plane.configure_backend();
        plane
    }

    fn assert_isolated(&self) {
        assert!(self.bin.is_absolute(), "worktree binary must be absolute");
        assert_eq!(
            self.run_root.parent(),
            Some(self.dedicated_root.as_path()),
            "current plane must be a direct child of the dedicated root"
        );
        for path in [
            &self.run_root,
            &self.home,
            &self.db,
            &self.install_root,
            &self.run_dir,
            &self.state_dir,
        ] {
            assert!(path.is_absolute(), "{} must be absolute", path.display());
            assert!(
                path.starts_with(&self.run_root),
                "{} escaped isolated run root {}",
                path.display(),
                self.run_root.display()
            );
        }
    }

    fn configure_backend(&self) {
        let db = self.db.to_string_lossy().into_owned();
        let out = self.run(
            "bootstrap",
            [
                "--json", "backend", "add", BACKEND, "--sqlite", "--path", &db,
            ],
        );
        out.assert_success("configure isolated named backend");
        let configured = out.json("configure isolated named backend");
        assert_eq!(configured["added"], BACKEND);
    }

    fn command(&self, session: &str) -> Command {
        let mut command = Command::new(&self.bin);
        for (key, _) in std::env::vars_os() {
            if key.to_string_lossy().starts_with("TELEX_") {
                command.env_remove(key);
            }
        }
        command
            .current_dir(&self.repo)
            .env("TELEX_HOME", &self.home)
            .env("TELEX_DB", &self.db)
            .env("TELEX_INSTALL_ROOT", &self.install_root)
            .env("TELEX_RUN_DIR", &self.run_dir)
            .env("TELEX_CONFIG", self.home.join("config.toml"))
            .env("TELEX_SESSION_ID", session)
            .env("TELEX_RECONNECT_GRACE_MS", "3000");
        #[cfg(windows)]
        command.env("LOCALAPPDATA", &self.state_dir);
        #[cfg(not(windows))]
        command.env("XDG_STATE_HOME", &self.state_dir);
        command
    }

    fn run<I, S>(&self, session: &str, args: I) -> RunOutput
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let mut command = self.command(session);
        command
            .args(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        run_with_timeout(command, Duration::from_secs(15))
    }

    fn run_backend<I, S>(&self, session: &str, args: I) -> RunOutput
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let mut all = vec![
            "--json".to_string(),
            "--backend".to_string(),
            BACKEND.to_string(),
        ];
        all.extend(
            args.into_iter()
                .map(|arg| arg.as_ref().to_string_lossy().into_owned()),
        );
        self.run(session, all)
    }

    fn attach(&self, session: &str, address: &str) {
        let out = self.run_backend(
            session,
            [
                "--address",
                address,
                "attach",
                "--session",
                session,
                "--description",
                "isolated Operator Station lifecycle validation",
            ],
        );
        out.assert_success(&format!("attach {session} to {address}"));
    }

    fn stop_station(&self, session: &str, address: &str) {
        let out = self.run_backend(
            session,
            [
                "--address",
                address,
                "station",
                "stop",
                "--session",
                session,
            ],
        );
        out.assert_success(&format!("stop station {session} at {address}"));
    }

    #[allow(clippy::too_many_arguments)]
    fn send(
        &self,
        session: &str,
        from: &str,
        to: &str,
        kind: &str,
        attention: &str,
        requires_disposition: bool,
        subject: &str,
        body: &str,
        metadata: Option<&Value>,
    ) -> Value {
        let mut args = vec![
            "--address".to_string(),
            from.to_string(),
            "send".to_string(),
            "--session".to_string(),
            session.to_string(),
            "--from".to_string(),
            from.to_string(),
            "--to".to_string(),
            to.to_string(),
            "--kind".to_string(),
            kind.to_string(),
            "--attention".to_string(),
            attention.to_string(),
            "--subject".to_string(),
            subject.to_string(),
            "--body".to_string(),
            body.to_string(),
        ];
        if requires_disposition {
            args.push("--requires-disposition".to_string());
        }
        if let Some(metadata) = metadata {
            args.push("--metadata".to_string());
            args.push(metadata.to_string());
        }
        let out = self.run_backend(session, args);
        out.assert_success(&format!("send {kind} from {from} to {to}"));
        out.json("send receipt")
    }

    #[allow(clippy::too_many_arguments)]
    fn reply(
        &self,
        session: &str,
        from: &str,
        parent: i64,
        kind: &str,
        requires_disposition: bool,
        subject: &str,
        body: &str,
        metadata: Option<&Value>,
    ) -> Value {
        let mut args = vec![
            "--address".to_string(),
            from.to_string(),
            "reply".to_string(),
            "--session".to_string(),
            session.to_string(),
            "--from".to_string(),
            from.to_string(),
            "--to-message".to_string(),
            parent.to_string(),
            "--kind".to_string(),
            kind.to_string(),
            "--attention".to_string(),
            "next-checkpoint".to_string(),
            "--subject".to_string(),
            subject.to_string(),
            "--body".to_string(),
            body.to_string(),
        ];
        if requires_disposition {
            args.push("--requires-disposition".to_string());
        }
        if let Some(metadata) = metadata {
            args.push("--metadata".to_string());
            args.push(metadata.to_string());
        }
        let out = self.run_backend(session, args);
        out.assert_success(&format!("reply {kind} from {from}"));
        out.json("reply receipt")
    }

    fn disposition(
        &self,
        session: &str,
        recipient: &str,
        state: &str,
        message_id: i64,
        note: &Value,
    ) -> Value {
        let out = self.run_backend(
            session,
            vec![
                "--address".to_string(),
                recipient.to_string(),
                state.to_string(),
                "--session".to_string(),
                session.to_string(),
                "--recipient".to_string(),
                recipient.to_string(),
                "--id".to_string(),
                message_id.to_string(),
                "--note".to_string(),
                note.to_string(),
            ],
        );
        out.assert_success(&format!("{state} message {message_id} for {recipient}"));
        out.json("disposition result")
    }

    fn read_full(&self, session: &str, address: &str, message_id: i64) -> Value {
        let out = self.run_backend(
            session,
            [
                "--address",
                address,
                "read",
                "--id",
                &message_id.to_string(),
                "--full",
            ],
        );
        out.assert_success(&format!("read message {message_id}"));
        out.json("full message read")
    }

    fn export(&self, session: &str) -> Vec<Value> {
        let out = self.run_backend(session, ["export"]);
        out.assert_success("export isolated lifecycle plane");
        out.stdout
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| {
                serde_json::from_str(line)
                    .unwrap_or_else(|error| panic!("parse export line {line:?}: {error}"))
            })
            .collect()
    }

    fn cleanup(&mut self) {
        if self.cleaned {
            return;
        }
        self.stop_daemon().unwrap_or_else(|error| {
            panic!(
                "stop isolated lifecycle daemon for {}: {error}",
                self.run_root.display()
            )
        });
        remove_directory_with_retry(&self.run_root, Duration::from_secs(5)).unwrap_or_else(
            |error| {
                panic!(
                    "remove isolated lifecycle plane {}: {error}",
                    self.run_root.display()
                )
            },
        );
        self.cleaned = true;
    }

    fn stop_daemon(&self) -> Result<(), String> {
        let _ = self.run_backend("cleanup", ["daemon", "stop", "--drain"]);
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            let status = self.run_backend("cleanup", ["daemon", "status"]);
            if !status.status.success() {
                return Ok(());
            }
            let running = serde_json::from_str::<Value>(&status.stdout)
                .ok()
                .and_then(|value| value.get("running").and_then(Value::as_bool));
            if running == Some(false) {
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        Err("daemon remained running after the cleanup deadline".to_string())
    }
}

impl Drop for IsolatedTelexPlane {
    fn drop(&mut self) {
        if !self.cleaned {
            let cleanup = self.stop_daemon().and_then(|()| {
                remove_directory_with_retry(&self.run_root, Duration::from_secs(5))
                    .map_err(|error| error.to_string())
            });
            if let Err(error) = cleanup {
                if std::thread::panicking() {
                    eprintln!(
                        "isolated Operator Station cleanup failed during unwind for {}: {error}",
                        self.run_root.display()
                    );
                } else {
                    panic!(
                        "isolated Operator Station cleanup failed for {}: {error}",
                        self.run_root.display()
                    );
                }
            }
        }
    }
}

#[test]
fn isolated_operator_station_lifecycle_preserves_v1_invariants() {
    let mut plane = IsolatedTelexPlane::new();

    let fixture: Value = serde_json::from_str(include_str!(
        "../copilot/plugin/skills/operator-station/compatibility.json"
    ))
    .expect("compatibility fixture parses");
    let version = plane.run("compatibility", ["--json", "version"]);
    version.assert_success("read worktree binary version");
    assert_eq!(
        version.json("worktree version")["version"]["package_version"],
        fixture["telex_package_version"]
    );
    let runtime_skill = plane.run(
        "compatibility",
        [
            "copilot",
            "skill",
            "--plugin-version",
            fixture["plugin_version"]
                .as_str()
                .expect("fixture plugin version"),
        ],
    );
    runtime_skill.assert_success("load version-matched Copilot workflow");
    assert!(runtime_skill
        .stdout
        .contains("Syntax is owned by the binary"));

    plane.attach("worker-session", WORKER);
    plane.attach("handoff-worker-session", HANDOFF_WORKER);
    plane.attach("operator-session", INGRESS);
    plane.attach("human-session", HUMAN);

    // Normal resolution: operator-authored raw reply, then terminal disposition.
    let normal = plane.send(
        "worker-session",
        WORKER,
        INGRESS,
        "decision-request",
        "next-checkpoint",
        true,
        "Routine validation question",
        "Can the operator resolve this routine request?",
        None,
    );
    let normal_id = message_id(&normal);
    let normal_thread = thread_id(&normal);
    let normal_op = operation_id(
        "resolve",
        &[
            ("storeId", BACKEND),
            ("rawMessageId", &normal_id.to_string()),
            ("ingressAddress", INGRESS),
        ],
    );
    plane.disposition(
        "operator-session",
        INGRESS,
        "defer",
        normal_id,
        &operation_note("resolve", &normal_op, normal_id, "planned"),
    );
    let normal_reply = plane.reply(
        "operator-session",
        INGRESS,
        normal_id,
        "note",
        false,
        "Routine request resolved",
        "Operator resolution based on the available durable evidence.",
        None,
    );
    assert_receipt(
        &normal_reply,
        INGRESS,
        WORKER,
        Some(normal_id),
        normal_thread,
    );
    plane.disposition(
        "operator-session",
        INGRESS,
        "handle",
        normal_id,
        &operation_note("resolve", &normal_op, normal_id, "accepted"),
    );
    assert_disposition_order(
        &plane.read_full("operator-session", INGRESS, normal_id),
        &["deferred", "handled"],
    );

    // Clarification remains in the raw thread and leaves the source deferred.
    let clarification_raw = plane.send(
        "worker-session",
        WORKER,
        INGRESS,
        "decision-request",
        "next-checkpoint",
        true,
        "Clarification needed",
        "Please act, but the required scope is missing.",
        None,
    );
    let clarification_id = message_id(&clarification_raw);
    let clarification_thread = thread_id(&clarification_raw);
    let clarification_op = operation_id(
        "clarification",
        &[
            ("storeId", BACKEND),
            ("rawMessageId", &clarification_id.to_string()),
            ("clarificationOrdinal", "1"),
        ],
    );
    plane.disposition(
        "operator-session",
        INGRESS,
        "defer",
        clarification_id,
        &operation_note(
            "clarification",
            &clarification_op,
            clarification_id,
            "planned",
        ),
    );
    let clarification = plane.reply(
        "operator-session",
        INGRESS,
        clarification_id,
        "note",
        false,
        "Clarify requested scope",
        "Which exact scope should the operator use?",
        None,
    );
    assert_receipt(
        &clarification,
        INGRESS,
        WORKER,
        Some(clarification_id),
        clarification_thread,
    );

    // Persist identity, replace the operator before send, then author exactly once.
    let escalation_raw = plane.send(
        "worker-session",
        WORKER,
        INGRESS,
        "approval-request",
        "next-checkpoint",
        true,
        "Human judgment required",
        "A human must choose the safe option.",
        None,
    );
    let escalation_raw_id = message_id(&escalation_raw);
    let escalation_mediation_id = mediation_id(escalation_raw_id);
    let escalation_op = operation_id(
        "escalation",
        &[
            ("mediationId", &escalation_mediation_id),
            ("rootRawMessageId", &escalation_raw_id.to_string()),
        ],
    );
    plane.disposition(
        "operator-session",
        INGRESS,
        "defer",
        escalation_raw_id,
        &planned_escalation_note(escalation_raw_id, &escalation_mediation_id, &escalation_op),
    );
    plane.stop_station("operator-session", INGRESS);
    plane.attach("replacement-session", INGRESS);
    assert_eq!(
        matching_operation_messages(&plane.export("replacement-session"), &escalation_op).len(),
        0,
        "pre-send recovery must find planned evidence but no authored message"
    );

    let escalation_envelope = escalation_metadata(
        escalation_raw_id,
        thread_id(&escalation_raw),
        WORKER,
        "approval-request",
        "Human judgment required",
        &escalation_mediation_id,
        &escalation_op,
    );
    let escalation = plane.send(
        "replacement-session",
        INGRESS,
        HUMAN,
        "operator-station.escalation",
        "next-checkpoint",
        true,
        "Operator recommendation requires human decision",
        "Human judgment is required. The operator recommends the reversible option. Choose approve or reject.",
        Some(&escalation_envelope),
    );
    let escalation_id = message_id(&escalation);
    let mediated_thread = thread_id(&escalation);
    assert_ne!(
        thread_id(&escalation_raw),
        mediated_thread,
        "raw and mediated threads must stay distinct"
    );
    assert_receipt(&escalation, INGRESS, HUMAN, None, mediated_thread);
    assert_operator_envelope(
        &plane.read_full("replacement-session", INGRESS, escalation_id)["message"],
        "operator-station.escalation",
        "urn:telex:operator-station:v1#escalation",
    );

    // Replacement after durable acceptance reconciles the prior operation instead of resending.
    plane.stop_station("replacement-session", INGRESS);
    plane.attach("replacement-two-session", INGRESS);
    let exports = plane.export("replacement-two-session");
    assert_eq!(
        matching_operation_messages(&exports, &escalation_op).len(),
        1,
        "accepted escalation must reconcile as one duplicate/retry identity"
    );
    plane.disposition(
        "replacement-two-session",
        INGRESS,
        "escalate",
        escalation_raw_id,
        &operation_note(
            "escalation",
            &escalation_op,
            escalation_raw_id,
            "duplicate-reconciled",
        ),
    );

    // Quiet digest freezes its source set before authoring; a late source is excluded.
    let quiet_one = plane.send(
        "worker-session",
        WORKER,
        INGRESS,
        "status",
        "fyi",
        true,
        "Quiet item one",
        "Informational item one.",
        None,
    );
    let quiet_two = plane.send(
        "worker-session",
        WORKER,
        INGRESS,
        "status",
        "fyi",
        true,
        "Quiet item two",
        "Informational item two.",
        None,
    );
    let quiet_ids = [message_id(&quiet_one), message_id(&quiet_two)];
    let window_start = 1_800_000_000_000_i64;
    let window_end = window_start + 3_600_000;
    let frozen_digest_id = digest_id(window_start, window_end, &quiet_ids);
    for source_id in quiet_ids {
        plane.disposition(
            "replacement-two-session",
            INGRESS,
            "defer",
            source_id,
            &json!({
                "recordType": "operator-station-pending-digest",
                "derivationVersion": "operator-station-op-v1",
                "digestId": frozen_digest_id,
                "windowStartMs": window_start,
                "windowEndMs": window_end,
                "sourceMessageIds": quiet_ids,
                "workflowSignature": WORKFLOW_SIGNATURE,
                "phase": "planned"
            }),
        );
    }
    plane.stop_station("replacement-two-session", INGRESS);
    plane.attach("digest-replacement-session", INGRESS);

    let late = plane.send(
        "worker-session",
        WORKER,
        INGRESS,
        "status",
        "fyi",
        true,
        "Late quiet item",
        "This arrived after the pending window was frozen.",
        None,
    );
    let late_id = message_id(&late);
    let digest_metadata = digest_metadata(
        &frozen_digest_id,
        window_start,
        window_end,
        &quiet_ids,
        &[thread_id(&quiet_one), thread_id(&quiet_two)],
    );
    let digest = plane.send(
        "digest-replacement-session",
        INGRESS,
        HUMAN,
        "operator-station.digest",
        "background",
        false,
        "Quiet digest",
        "Two informational items were aggregated; the later arrival is deferred to the next window.",
        Some(&digest_metadata),
    );
    assert_operator_envelope(
        &plane.read_full("digest-replacement-session", INGRESS, message_id(&digest))["message"],
        "operator-station.digest",
        "urn:telex:operator-station:v1#digest",
    );
    let digest_items = digest_metadata["ext"]["operator-station"]["items"]
        .as_array()
        .expect("digest items");
    assert!(digest_items
        .iter()
        .all(|item| item["messageId"].as_i64() != Some(late_id)));
    plane.disposition(
        "digest-replacement-session",
        INGRESS,
        "defer",
        late_id,
        &json!({
            "recordType": "operator-station-pending-digest",
            "derivationVersion": "operator-station-op-v1",
            "digestId": digest_id(window_end, window_end + 3_600_000, &[late_id]),
            "windowStartMs": window_end,
            "windowEndMs": window_end + 3_600_000,
            "sourceMessageIds": [late_id],
            "workflowSignature": WORKFLOW_SIGNATURE,
            "phase": "planned-next-window"
        }),
    );

    // Degraded human-address states leave durable diagnostics rather than silent action.
    for (source_id, health) in [
        (quiet_ids[0], "attended-deaf"),
        (quiet_ids[1], "attended-with-backlog"),
    ] {
        plane.disposition(
            "digest-replacement-session",
            INGRESS,
            "defer",
            source_id,
            &json!({
                "recordType": "operator-station-health-diagnostic",
                "humanAddress": HUMAN,
                "health": health,
                "decision": "blocked",
                "workflowSignature": WORKFLOW_SIGNATURE
            }),
        );
    }

    // A human text response arrives while ingress is unoccupied and is recovered later.
    plane.stop_station("digest-replacement-session", INGRESS);
    let delayed_text = plane.reply(
        "human-session",
        HUMAN,
        escalation_id,
        "operator-station.human-reply",
        true,
        "Human response",
        "Human chose the reversible option.",
        None,
    );
    assert_eq!(delayed_text["receipt"], "queued-unoccupied");
    assert_receipt(
        &delayed_text,
        HUMAN,
        INGRESS,
        Some(escalation_id),
        mediated_thread,
    );
    plane.attach("response-replacement-session", INGRESS);
    let delayed_text_id = message_id(&delayed_text);
    assert!(plane
        .export("response-replacement-session")
        .iter()
        .any(|row| row["message"]["id"].as_i64() == Some(delayed_text_id)));

    // Route the human text response into the raw thread with the opaque v1 envelope.
    let text_route_op = operation_id(
        "route-back",
        &[
            ("mediationId", &escalation_mediation_id),
            ("humanResponseMessageId", &delayed_text_id.to_string()),
        ],
    );
    plane.disposition(
        "response-replacement-session",
        INGRESS,
        "defer",
        delayed_text_id,
        &operation_note("route-back", &text_route_op, escalation_raw_id, "planned"),
    );
    let text_route_metadata = routed_outcome_metadata(
        &escalation_mediation_id,
        &text_route_op,
        delayed_text_id,
        None,
    );
    let routed_text = plane.reply(
        "response-replacement-session",
        INGRESS,
        escalation_raw_id,
        "note",
        false,
        "Human outcome relayed",
        "Relayed human outcome: choose the reversible option.",
        Some(&text_route_metadata),
    );
    assert_receipt(
        &routed_text,
        INGRESS,
        WORKER,
        Some(escalation_raw_id),
        thread_id(&escalation_raw),
    );
    assert_routed_outcome(
        &plane.read_full(
            "response-replacement-session",
            INGRESS,
            message_id(&routed_text),
        )["message"],
        escalation_raw_id,
        thread_id(&escalation_raw),
        &escalation_mediation_id,
        &text_route_op,
        delayed_text_id,
        None,
    );
    plane.disposition(
        "response-replacement-session",
        INGRESS,
        "handle",
        delayed_text_id,
        &operation_note("route-back", &text_route_op, escalation_raw_id, "accepted"),
    );

    let disposition_only = plane.reply(
        "human-session",
        HUMAN,
        escalation_id,
        "operator-station.human-reply",
        true,
        "Human disposition outcome",
        "Human rejected this escalation without a textual reply.",
        None,
    );
    assert_receipt(
        &disposition_only,
        HUMAN,
        INGRESS,
        Some(escalation_id),
        mediated_thread,
    );
    let disposition_only_id = message_id(&disposition_only);
    let disposition_route_op = operation_id(
        "route-back",
        &[
            ("mediationId", &escalation_mediation_id),
            ("humanResponseMessageId", &disposition_only_id.to_string()),
        ],
    );
    plane.disposition(
        "response-replacement-session",
        INGRESS,
        "defer",
        disposition_only_id,
        &operation_note(
            "route-back",
            &disposition_route_op,
            escalation_raw_id,
            "planned",
        ),
    );
    let disposition_route_metadata = routed_outcome_metadata(
        &escalation_mediation_id,
        &disposition_route_op,
        disposition_only_id,
        Some("rejected"),
    );
    let routed_disposition = plane.reply(
        "response-replacement-session",
        INGRESS,
        escalation_raw_id,
        "note",
        false,
        "Human disposition relayed",
        "Relayed machine-readable human outcome: rejected.",
        Some(&disposition_route_metadata),
    );
    assert_receipt(
        &routed_disposition,
        INGRESS,
        WORKER,
        Some(escalation_raw_id),
        thread_id(&escalation_raw),
    );
    assert_routed_outcome(
        &plane.read_full(
            "response-replacement-session",
            INGRESS,
            message_id(&routed_disposition),
        )["message"],
        escalation_raw_id,
        thread_id(&escalation_raw),
        &escalation_mediation_id,
        &disposition_route_op,
        disposition_only_id,
        Some("rejected"),
    );
    plane.disposition(
        "response-replacement-session",
        INGRESS,
        "reject",
        escalation_raw_id,
        &operation_note(
            "route-back",
            &disposition_route_op,
            escalation_raw_id,
            "accepted",
        ),
    );
    plane.disposition(
        "response-replacement-session",
        INGRESS,
        "handle",
        disposition_only_id,
        &operation_note(
            "route-back",
            &disposition_route_op,
            escalation_raw_id,
            "accepted",
        ),
    );

    // Stale retired origin uses the audited exception and never guesses a replacement.
    let stale_response = plane.reply(
        "human-session",
        HUMAN,
        escalation_id,
        "operator-station.human-reply",
        true,
        "Late human response",
        "A later human response arrived after the raw outcome was closed.",
        None,
    );
    assert_receipt(
        &stale_response,
        HUMAN,
        INGRESS,
        Some(escalation_id),
        mediated_thread,
    );
    let stale_response_id = message_id(&stale_response);
    let retire = plane.run_backend("worker-session", ["--address", WORKER, "address", "retire"]);
    retire.assert_success("retire stale source address");
    let stale_route = operation_id(
        "route-back",
        &[
            ("mediationId", &escalation_mediation_id),
            ("humanResponseMessageId", &stale_response_id.to_string()),
        ],
    );
    plane.disposition(
        "response-replacement-session",
        INGRESS,
        "reject",
        escalation_raw_id,
        &json!({
            "recordType": "operator-station-stale-origin",
            "mediationId": escalation_mediation_id,
            "operationId": stale_route,
            "sourceAddress": WORKER,
            "reason": "source address is retired; no reachable raw thread remains",
            "humanVisible": true,
            "phase": "terminal-audited"
        }),
    );
    plane.disposition(
        "response-replacement-session",
        INGRESS,
        "handle",
        stale_response_id,
        &json!({
            "recordType": "operator-station-stale-origin",
            "mediationId": escalation_mediation_id,
            "rawOutcome": "rejected",
            "phase": "reconciled"
        }),
    );

    // Create an unresolved mediation and prove handoff inventory is readable before detach.
    let handoff_source = plane.send(
        "handoff-worker-session",
        HANDOFF_WORKER,
        INGRESS,
        "handoff-source",
        "next-checkpoint",
        true,
        "Transition handoff source",
        "Synthetic unresolved source for deterministic transition validation.",
        None,
    );
    let handoff_raw_id = message_id(&handoff_source);
    let handoff_mediation = mediation_id(handoff_raw_id);
    let handoff_operation = operation_id(
        "escalation",
        &[
            ("mediationId", &handoff_mediation),
            ("rootRawMessageId", &handoff_raw_id.to_string()),
        ],
    );
    let handoff_escalation = plane.send(
        "response-replacement-session",
        INGRESS,
        HUMAN,
        "operator-station.escalation",
        "next-checkpoint",
        true,
        "Unresolved transition mediation",
        "This mediation remains unresolved for handoff validation.",
        Some(&escalation_metadata(
            handoff_raw_id,
            thread_id(&handoff_source),
            HANDOFF_WORKER,
            "handoff-source",
            "Transition handoff source",
            &handoff_mediation,
            &handoff_operation,
        )),
    );
    plane.disposition(
        "response-replacement-session",
        INGRESS,
        "escalate",
        handoff_raw_id,
        &json!({
            "recordType": "operator-station-handoff",
            "derivationVersion": "operator-station-op-v1",
            "mediationId": handoff_mediation,
            "sourceReference": {
                "storeId": BACKEND,
                "messageId": handoff_raw_id,
                "threadId": thread_id(&handoff_source)
            },
            "mediatedRootMessageId": message_id(&handoff_escalation),
            "humanResponseMessageId": Value::Null,
            "inFlightOperations": [{
                "operationId": handoff_operation,
                "state": "accepted",
                "nextAction": "await human response"
            }],
            "workflowSignature": WORKFLOW_SIGNATURE,
            "stationConfirmation": "reconstructable"
        }),
    );
    let handoff_read = plane.read_full("response-replacement-session", INGRESS, handoff_raw_id);
    let handoff_note = handoff_read["dispositions"]
        .as_array()
        .expect("handoff disposition history")
        .last()
        .and_then(|row| row["note"].as_str())
        .and_then(|note| serde_json::from_str::<Value>(note).ok())
        .expect("durable handoff note");
    assert_eq!(handoff_note["stationConfirmation"], "reconstructable");
    assert_eq!(
        handoff_note["mediatedRootMessageId"],
        message_id(&handoff_escalation)
    );

    plane.stop_station("response-replacement-session", INGRESS);
    plane.attach("direct-station-session", INGRESS);
    let station_status = plane.run_backend(
        "direct-station-session",
        [
            "--address",
            INGRESS,
            "station",
            "status",
            "--session",
            "direct-station-session",
            "--all-sessions",
        ],
    );
    station_status.assert_success("verify ordered direct takeover");
    let station_status = station_status.json("station status after takeover");
    let stations = station_status["stations"]
        .as_array()
        .expect("station status rows");
    assert!(stations
        .iter()
        .any(|row| { row["address"] == INGRESS && row["session_id"] == "direct-station-session" }));
    assert!(!stations.iter().any(|row| {
        row["address"] == INGRESS && row["session_id"] == "response-replacement-session"
    }));

    assert!(
        plane
            .export("direct-station-session")
            .iter()
            .flat_map(|row| row["dispositions"].as_array().into_iter().flatten())
            .filter_map(|row| row["note"].as_str())
            .all(|note| !note.contains("operator-station-spike")),
        "isolated plane must contain only production Operator Station namespace evidence"
    );

    let run_root = plane.run_root.clone();
    plane.cleanup();
    assert!(
        !run_root.exists(),
        "green validation must remove its current isolated plane"
    );
}

#[test]
fn stale_run_cleanup_accepts_only_strict_timestamped_children() {
    assert_eq!(
        timestamped_run_name("run-1785346589-1234-7"),
        Some(1_785_346_589)
    );
    for invalid in [
        "run-not-a-time-1234-7",
        "run-1785346589-no-pid-7",
        "run-1785346589-1234",
        "run-1785346589-1234-7-extra",
        "other-1785346589-1234-7",
    ] {
        assert_eq!(
            timestamped_run_name(invalid),
            None,
            "cleanup must refuse non-plane child {invalid:?}"
        );
    }
}

fn worktree_telex_bin() -> PathBuf {
    let candidate = option_env!("CARGO_BIN_EXE_telex")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("target")
                .join("debug")
                .join(format!("telex{}", std::env::consts::EXE_SUFFIX))
        });
    candidate.canonicalize().unwrap_or_else(|error| {
        panic!("canonical worktree binary {}: {error}", candidate.display())
    })
}

fn cleanup_stale_runs(root: &Path) {
    let canonical_root = root
        .canonicalize()
        .expect("canonical dedicated lifecycle root");
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock after epoch")
        .as_secs();
    let entries = std::fs::read_dir(&canonical_root).expect("list dedicated lifecycle root");
    for entry in entries.filter_map(Result::ok) {
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(timestamp) = timestamped_run_name(&name) else {
            continue;
        };
        let path = entry.path();
        if Duration::from_secs(now.saturating_sub(timestamp)) <= STALE_AFTER {
            continue;
        }
        let Ok(canonical) = path.canonicalize() else {
            continue;
        };
        if canonical.parent() == Some(canonical_root.as_path()) {
            std::fs::remove_dir_all(&canonical).unwrap_or_else(|error| {
                panic!("remove stale run {}: {error}", canonical.display())
            });
        }
    }
}

fn timestamped_run_name(name: &str) -> Option<u64> {
    let mut fields = name.strip_prefix("run-")?.split('-');
    let timestamp = fields.next()?.parse().ok()?;
    let _pid: u32 = fields.next()?.parse().ok()?;
    let _sequence: usize = fields.next()?.parse().ok()?;
    fields.next().is_none().then_some(timestamp)
}

fn remove_directory_with_retry(path: &Path, timeout: Duration) -> std::io::Result<()> {
    let deadline = Instant::now() + timeout;
    loop {
        if !path.exists() {
            return Ok(());
        }
        match std::fs::remove_dir_all(path) {
            Ok(()) => return Ok(()),
            Err(error) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(50));
                if !path.exists() {
                    return Ok(());
                }
                let _ = error;
            }
            Err(error) => return Err(error),
        }
    }
}

fn restrict_directory(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
            .unwrap_or_else(|error| panic!("restrict {}: {error}", path.display()));
    }
    #[cfg(windows)]
    {
        let _ = path;
    }
}

fn run_with_timeout(mut command: Command, timeout: Duration) -> RunOutput {
    let mut child = command.spawn().expect("spawn worktree telex command");
    let deadline = Instant::now() + timeout;
    let status = loop {
        if let Some(status) = child.try_wait().expect("poll worktree telex command") {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("worktree telex command timed out after {timeout:?}");
        }
        std::thread::sleep(Duration::from_millis(25));
    };
    let mut stdout = String::new();
    let mut stderr = String::new();
    child
        .stdout
        .take()
        .expect("captured stdout")
        .read_to_string(&mut stdout)
        .expect("read worktree telex stdout");
    child
        .stderr
        .take()
        .expect("captured stderr")
        .read_to_string(&mut stderr)
        .expect("read worktree telex stderr");
    RunOutput {
        status,
        stdout,
        stderr,
    }
}

fn message_id(receipt: &Value) -> i64 {
    receipt["id"].as_i64().expect("receipt id")
}

fn thread_id(receipt: &Value) -> i64 {
    receipt["thread_id"].as_i64().expect("receipt thread_id")
}

fn assert_receipt(
    receipt: &Value,
    expected_from: &str,
    expected_to: &str,
    expected_parent: Option<i64>,
    expected_thread: i64,
) {
    assert!(matches!(
        receipt["receipt"].as_str(),
        Some("delivered" | "queued-unoccupied")
    ));
    assert_eq!(receipt["from"], expected_from);
    assert_eq!(receipt["to"], expected_to);
    assert_eq!(receipt["parent_id"].as_i64(), expected_parent);
    assert_eq!(receipt["thread_id"], expected_thread);
}

fn assert_disposition_order(read: &Value, expected: &[&str]) {
    let actual: Vec<_> = read["dispositions"]
        .as_array()
        .expect("disposition history")
        .iter()
        .filter_map(|row| row["state"].as_str())
        .collect();
    assert_eq!(actual, expected);
}

fn identity(purpose: &str, fields: &[(&str, &str)]) -> String {
    let mut canonical = canonical_field("derivationVersion", "operator-station-op-v1");
    canonical.push_str(&canonical_field("purpose", purpose));
    for (field, value) in fields {
        canonical.push_str(&canonical_field(field, value));
    }
    let digest = Sha256::digest(canonical.as_bytes());
    let hex = digest
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("operator-station-op-v1/{purpose}/{hex}")
}

fn canonical_field(field: &str, value: &str) -> String {
    format!("{field}={}:{}\n", value.len(), value)
}

fn mediation_id(raw_message_id: i64) -> String {
    identity(
        "mediation",
        &[
            ("storeId", BACKEND),
            ("rawMessageId", &raw_message_id.to_string()),
            ("ingressAddress", INGRESS),
            ("humanAddress", HUMAN),
        ],
    )
}

fn operation_id(purpose: &str, fields: &[(&str, &str)]) -> String {
    identity(purpose, fields)
}

fn digest_id(window_start: i64, window_end: i64, source_ids: &[i64]) -> String {
    let mut owned = vec![
        ("storeId".to_string(), BACKEND.to_string()),
        ("windowStartMs".to_string(), window_start.to_string()),
        ("windowEndMs".to_string(), window_end.to_string()),
    ];
    let mut sorted = source_ids.to_vec();
    sorted.sort_unstable();
    owned.extend(
        sorted
            .iter()
            .map(|id| ("sourceMessageId".to_string(), id.to_string())),
    );
    let borrowed: Vec<_> = owned
        .iter()
        .map(|(field, value)| (field.as_str(), value.as_str()))
        .collect();
    identity("digest", &borrowed)
}

fn operation_note(purpose: &str, operation_id: &str, raw_id: i64, phase: &str) -> Value {
    json!({
        "recordType": "operator-station-operation",
        "derivationVersion": "operator-station-op-v1",
        "operationId": operation_id,
        "purpose": purpose,
        "sourceReference": {
            "storeId": BACKEND,
            "messageId": raw_id
        },
        "workflowSignature": WORKFLOW_SIGNATURE,
        "phase": phase
    })
}

fn planned_escalation_note(raw_id: i64, mediation_id: &str, operation_id: &str) -> Value {
    json!({
        "recordType": "operator-station-operation",
        "derivationVersion": "operator-station-op-v1",
        "mediationId": mediation_id,
        "operationId": operation_id,
        "purpose": "escalation",
        "sourceReference": {
            "storeId": BACKEND,
            "messageId": raw_id
        },
        "intended": {
            "from": INGRESS,
            "to": HUMAN,
            "parentId": Value::Null,
            "kind": "operator-station.escalation",
            "dataschema": "urn:telex:operator-station:v1#escalation"
        },
        "workflowSignature": WORKFLOW_SIGNATURE,
        "phase": "planned"
    })
}

fn escalation_metadata(
    raw_id: i64,
    raw_thread_id: i64,
    source_from: &str,
    source_kind: &str,
    source_subject: &str,
    mediation_id: &str,
    operation_id: &str,
) -> Value {
    json!({
        "extensions": {
            "operator-station": EXTENSION_ID
        },
        "dataschema": "urn:telex:operator-station:v1#escalation",
        "ext": {
            "operator-station": {
                "mediationId": mediation_id,
                "operationId": operation_id,
                "ingressAddress": INGRESS,
                "humanAddress": HUMAN,
                "requestedOutcome": "Choose approve or reject.",
                "recommendation": "Operator-authored recommendation: choose the reversible option.",
                "sourceMessages": [{
                    "storeId": BACKEND,
                    "messageId": raw_id,
                    "threadId": raw_thread_id,
                    "from": source_from,
                    "to": INGRESS,
                    "kind": source_kind,
                    "attention": "next-checkpoint",
                    "requiresDisposition": true,
                    "subject": source_subject,
                    "sentAtMs": 1_800_000_000_000_i64
                }]
            }
        }
    })
}

fn digest_metadata(
    digest_id: &str,
    window_start: i64,
    window_end: i64,
    source_ids: &[i64],
    thread_ids: &[i64],
) -> Value {
    let items: Vec<_> = source_ids
        .iter()
        .zip(thread_ids)
        .map(|(message_id, thread_id)| {
            json!({
                "storeId": BACKEND,
                "messageId": message_id,
                "threadId": thread_id,
                "from": WORKER,
                "to": INGRESS,
                "kind": "status",
                "sentAtMs": window_start + 1
            })
        })
        .collect();
    json!({
        "extensions": {
            "operator-station": EXTENSION_ID
        },
        "dataschema": "urn:telex:operator-station:v1#digest",
        "ext": {
            "operator-station": {
                "digestId": digest_id,
                "windowStartMs": window_start,
                "windowEndMs": window_end,
                "items": items
            }
        }
    })
}

fn routed_outcome_metadata(
    mediation_id: &str,
    operation_id: &str,
    human_response_message_id: i64,
    outcome_type: Option<&str>,
) -> Value {
    let mut metadata = json!({
        "extensions": {
            "operator-station": EXTENSION_ID
        },
        "dataschema": "urn:telex:operator-station:v1#routed-outcome",
        "ext": {
            "operator-station": {
                "mediationId": mediation_id,
                "operationId": operation_id,
                "humanOriginated": true,
                "humanAddress": HUMAN,
                "humanResponseMessageId": human_response_message_id
            }
        }
    });
    if let Some(outcome_type) = outcome_type {
        metadata["ext"]["operator-station"]["outcomeType"] =
            Value::String(outcome_type.to_string());
    }
    metadata
}

fn assert_operator_envelope(message: &Value, kind: &str, dataschema: &str) {
    assert_eq!(message["kind"], kind);
    let metadata: Value = serde_json::from_str(
        message["metadata"]
            .as_str()
            .expect("Operator Station message metadata"),
    )
    .expect("Operator Station metadata parses");
    assert_eq!(metadata["extensions"]["operator-station"], EXTENSION_ID);
    assert_eq!(metadata["dataschema"], dataschema);
    assert_eq!(message["from_addr"], INGRESS);
}

#[allow(clippy::too_many_arguments)]
fn assert_routed_outcome(
    message: &Value,
    raw_message_id: i64,
    raw_thread_id: i64,
    mediation_id: &str,
    operation_id: &str,
    human_response_message_id: i64,
    outcome_type: Option<&str>,
) {
    assert_operator_envelope(
        message,
        "note",
        "urn:telex:operator-station:v1#routed-outcome",
    );
    assert_eq!(message["parent_id"], raw_message_id);
    assert_eq!(message["thread_id"], raw_thread_id);
    assert_eq!(message["to_addr"], WORKER);
    let metadata: Value =
        serde_json::from_str(message["metadata"].as_str().expect("routed metadata"))
            .expect("routed metadata parses");
    let extension = &metadata["ext"]["operator-station"];
    assert_eq!(extension["mediationId"], mediation_id);
    assert_eq!(extension["operationId"], operation_id);
    assert_eq!(extension["humanOriginated"], true);
    assert_eq!(extension["humanAddress"], HUMAN);
    assert_eq!(
        extension["humanResponseMessageId"],
        human_response_message_id
    );
    match outcome_type {
        Some(outcome_type) => assert_eq!(extension["outcomeType"], outcome_type),
        None => assert!(extension.get("outcomeType").is_none()),
    }
}

fn matching_operation_messages<'a>(export: &'a [Value], operation_id: &str) -> Vec<&'a Value> {
    export
        .iter()
        .filter(|row| {
            row["message"]["metadata"]
                .as_str()
                .and_then(|metadata| serde_json::from_str::<Value>(metadata).ok())
                .and_then(|metadata| {
                    metadata["ext"]["operator-station"]["operationId"]
                        .as_str()
                        .map(str::to_string)
                })
                .as_deref()
                == Some(operation_id)
        })
        .collect()
}
