//! End-to-end behavioral tests that drive the real `telex-watcher` binary against a fake detector
//! (a real child process) and a fake `telex` executable. These exercise the receipt-gated state
//! machine, duplicate/collision safety, terminal ordering, restart recovery, bounded failure
//! handling, process-group termination, and the management/diagnostic CLI.

use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const WATCHER: &str = env!("CARGO_BIN_EXE_telex-watcher");
const FAKE_DETECTOR: &str = env!("CARGO_BIN_EXE_fake_detector");
const FAKE_TELEX: &str = env!("CARGO_BIN_EXE_fake_telex");

struct Captured {
    code: i32,
    stdout: String,
    stderr: String,
}

impl Captured {
    fn json(&self) -> Value {
        serde_json::from_str(&self.stdout)
            .unwrap_or_else(|error| panic!("stdout was not JSON ({error}): {}", self.stdout))
    }
}

struct Harness {
    root: PathBuf,
    registry: PathBuf,
    state: PathBuf,
}

impl Drop for Harness {
    fn drop(&mut self) {
        // Remove the scratch tree even if a test panics, so runs leave no artifacts behind.
        let _ = fs::remove_dir_all(&self.root);
    }
}

impl Harness {
    fn new(name: &str) -> Self {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "telex-watcher-behavior-{name}-{}-{unique}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let state = root.join("telex-state");
        fs::create_dir_all(&state).unwrap();
        Self {
            registry: root.join("watcher.sqlite"),
            state,
            root,
        }
    }

    /// Write a detector config file. The same path is used as the watch `scriptPath` so it is the
    /// hashed script the runtime tracks.
    fn write_config(&self, config: Value) -> PathBuf {
        let path = self.root.join("detector-config.json");
        fs::write(&path, config.to_string()).unwrap();
        path
    }

    fn write_spec(&self, spec: Value) -> PathBuf {
        let path = self.root.join("watch-spec.json");
        fs::write(&path, spec.to_string()).unwrap();
        path
    }

    fn watcher(&self, args: &[&str]) -> Command {
        let mut command = Command::new(WATCHER);
        command
            .arg("--registry")
            .arg(&self.registry)
            .arg("--telex")
            .arg(FAKE_TELEX)
            .args(args)
            .current_dir(&self.root)
            .env("FAKE_TELEX_STATE", &self.state);
        command
    }

    fn run(&self, args: &[&str]) -> Captured {
        capture(self.watcher(args))
    }

    fn run_env(&self, args: &[&str], envs: &[(&str, &str)]) -> Captured {
        let mut command = self.watcher(args);
        for (key, value) in envs {
            command.env(key, value);
        }
        capture(command)
    }
}

fn capture(mut command: Command) -> Captured {
    let output = command.output().expect("run telex-watcher");
    Captured {
        code: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

/// Build a follow-path watch spec whose detector is the fake detector reading `config`.
fn spec_for(config_path: &Path, id: &str, sender: &str) -> Value {
    let script = config_path.to_string_lossy().into_owned();
    json!({
        "id": id,
        "command": [FAKE_DETECTOR, script],
        "scriptPath": script,
        "workingDirectory": config_path.parent().unwrap().to_string_lossy(),
        "scriptMode": "follow-path",
        "sender": sender,
        "target": "project:telex",
        "intervalSeconds": 60,
        "timeoutSeconds": 1,
        "attention": "background",
        "state": {"cursor": 1},
    })
}

fn event_stdout(event_id: &str, body: &str, cursor: i64) -> String {
    json!({
        "schemaVersion": 1,
        "outcome": "event",
        "nextState": {"cursor": cursor},
        "event": {
            "id": event_id,
            "kind": "watch.test",
            "subject": "subject",
            "body": body,
            "metadata": {"provider": "fake"},
        },
    })
    .to_string()
}

fn add_watch(harness: &Harness, config_path: &Path, id: &str, sender: &str) -> Value {
    let spec = harness.write_spec(spec_for(config_path, id, sender));
    let added = harness.run(&["add", "--file", &spec.to_string_lossy()]);
    assert_eq!(added.code, 0, "add failed: {}", added.stderr);
    added.json()
}

fn run_once(harness: &Harness, id: &str, envs: &[(&str, &str)]) -> Captured {
    harness.run_env(&["run", "--once", "--watch", id], envs)
}

fn events(harness: &Harness, id: &str) -> Vec<Value> {
    let out = harness.run(&["events", id]);
    assert_eq!(out.code, 0, "events failed: {}", out.stderr);
    out.json().as_array().cloned().unwrap_or_default()
}

fn latest_attempt(harness: &Harness, id: &str) -> Value {
    let out = harness.run(&["attempts", id, "--limit", "1"]);
    assert_eq!(out.code, 0, "attempts failed: {}", out.stderr);
    out.json()
        .as_array()
        .and_then(|rows| rows.first().cloned())
        .expect("at least one attempt")
}

fn show(harness: &Harness, id: &str) -> Value {
    let out = harness.run(&["show", id]);
    assert_eq!(out.code, 0, "show failed: {}", out.stderr);
    out.json()
}

// --------------------------------------------------------------------------------------------
// Management and diagnostic CLI.
// --------------------------------------------------------------------------------------------

#[test]
fn management_and_diagnostic_cli_lifecycle() {
    let harness = Harness::new("cli-lifecycle");
    let config = harness.write_config(json!({"stdout": event_stdout("provider:1", "body", 2)}));

    // add + show + list
    let added = add_watch(&harness, &config, "cli-watch", "service:cli");
    assert_eq!(added["id"], "cli-watch");
    assert_eq!(added["status"], "active");

    let listed = harness.run(&["list"]);
    assert_eq!(listed.code, 0);
    assert_eq!(listed.json().as_array().unwrap().len(), 1);

    assert_eq!(show(&harness, "cli-watch")["id"], "cli-watch");

    // pause + resume
    assert_eq!(
        harness.run(&["pause", "cli-watch"]).json()["status"],
        "paused"
    );
    assert_eq!(
        harness.run(&["resume", "cli-watch"]).json()["status"],
        "active"
    );

    // update mutable settings (interval), immutable sender/target unchanged
    let mut updated_spec = spec_for(&config, "cli-watch", "service:cli");
    updated_spec["intervalSeconds"] = json!(120);
    let update_file = harness.root.join("update.json");
    fs::write(&update_file, updated_spec.to_string()).unwrap();
    let updated = harness.run(&[
        "update",
        "cli-watch",
        "--file",
        &update_file.to_string_lossy(),
    ]);
    assert_eq!(updated.code, 0, "update failed: {}", updated.stderr);
    assert_eq!(updated.json()["intervalSeconds"], 120);

    // attempts + events for a known watch return arrays (empty before any run)
    assert!(harness.run(&["attempts", "cli-watch"]).json().is_array());
    assert!(harness.run(&["events", "cli-watch"]).json().is_array());

    // unknown watch is a clean error for diagnostics
    let missing = harness.run(&["show", "does-not-exist"]);
    assert_ne!(missing.code, 0);
    assert!(missing.stderr.contains("does not exist"));

    // remove retains provenance but drops it from the active set
    assert_eq!(
        harness.run(&["remove", "cli-watch"]).json()["status"],
        "removed"
    );
    // update of a removed watch is rejected
    let reupdate = harness.run(&[
        "update",
        "cli-watch",
        "--file",
        &update_file.to_string_lossy(),
    ]);
    assert_ne!(reupdate.code, 0);
}

// --------------------------------------------------------------------------------------------
// Receipt-gated commit.
// --------------------------------------------------------------------------------------------

#[test]
fn delivered_event_commits_state_and_ledger() {
    let harness = Harness::new("delivered");
    let config = harness.write_config(json!({"stdout": event_stdout("provider:1", "body", 2)}));
    add_watch(&harness, &config, "watch", "service:watcher");

    let run = run_once(&harness, "watch", &[]);
    assert_eq!(run.code, 0, "run failed: {}", run.stderr);
    assert_eq!(run.json()["runs"], 1);

    assert_eq!(show(&harness, "watch")["state"], json!({"cursor": 2}));
    let ledger = events(&harness, "watch");
    assert_eq!(ledger.len(), 1);
    assert_eq!(ledger[0]["eventId"], "provider:1");
    assert_eq!(ledger[0]["messageId"], 4242);
    let attempt = latest_attempt(&harness, "watch");
    assert_eq!(attempt["outcome"], "event-sent");
    assert_eq!(attempt["stateCommitted"], true);
}

#[test]
fn queued_unoccupied_receipt_commits_state_and_ledger() {
    let harness = Harness::new("queued");
    let config = harness.write_config(json!({"stdout": event_stdout("provider:q", "body", 2)}));
    add_watch(&harness, &config, "watch", "service:watcher");

    let run = run_once(
        &harness,
        "watch",
        &[("FAKE_TELEX_RECEIPT", "queued-unoccupied")],
    );
    assert_eq!(run.code, 0, "run failed: {}", run.stderr);

    assert_eq!(show(&harness, "watch")["state"], json!({"cursor": 2}));
    let ledger = events(&harness, "watch");
    assert_eq!(ledger.len(), 1);
    assert_eq!(ledger[0]["receiptJson"]["receipt"], "queued-unoccupied");
}

#[test]
fn unknown_receipt_leaves_state_unchanged() {
    let harness = Harness::new("unknown-receipt");
    let config = harness.write_config(json!({"stdout": event_stdout("provider:u", "body", 2)}));
    add_watch(&harness, &config, "watch", "service:watcher");

    let run = run_once(
        &harness,
        "watch",
        &[("FAKE_TELEX_RECEIPT", "made-up-receipt")],
    );
    assert_eq!(run.code, 0, "run failed: {}", run.stderr);

    // Fail-closed: no ledger row, state unchanged, attempt records the unknown receipt.
    assert_eq!(show(&harness, "watch")["state"], json!({"cursor": 1}));
    assert!(events(&harness, "watch").is_empty());
    assert_eq!(
        latest_attempt(&harness, "watch")["outcome"],
        "unknown-receipt"
    );
}

#[test]
fn needs_attach_reconciles_once_then_delivers() {
    let harness = Harness::new("needs-attach");
    let config = harness.write_config(json!({"stdout": event_stdout("provider:na", "body", 2)}));
    add_watch(&harness, &config, "watch", "service:watcher");
    // Force the daemon to report a one-shot NeedsAttach so the coordinator reconciles and retries.
    fs::write(harness.state.join("needs-attach-once"), b"1").unwrap();

    let run = run_once(&harness, "watch", &[]);
    assert_eq!(run.code, 0, "run failed: {}", run.stderr);
    assert_eq!(show(&harness, "watch")["state"], json!({"cursor": 2}));
    assert_eq!(events(&harness, "watch").len(), 1);
}

// --------------------------------------------------------------------------------------------
// Idle / terminal outcomes.
// --------------------------------------------------------------------------------------------

#[test]
fn idle_outcome_commits_state_without_ledger() {
    let harness = Harness::new("idle");
    let config = harness.write_config(json!({
        "stdout": json!({"schemaVersion": 1, "outcome": "idle", "nextState": {"cursor": 7}}).to_string()
    }));
    add_watch(&harness, &config, "watch", "service:watcher");

    let run = run_once(&harness, "watch", &[]);
    assert_eq!(run.code, 0, "run failed: {}", run.stderr);
    assert_eq!(show(&harness, "watch")["state"], json!({"cursor": 7}));
    assert!(events(&harness, "watch").is_empty());
    assert_eq!(latest_attempt(&harness, "watch")["outcome"], "success");
}

#[test]
fn detector_environment_is_cleared_to_baseline_plus_allowlist() {
    let harness = Harness::new("env-baseline");
    // The detector reports, via committed state, which names were visible in its (cleared +
    // baseline + allowlist) environment.
    let config = harness.write_config(json!({
        "reportEnv": ["LANG", "WATCHER_NONBASELINE_UNIQUE", "MY_SECRET", "PATH"],
    }));
    let mut spec = spec_for(&config, "watch", "service:watcher");
    spec["environmentAllowlist"] = json!(["MY_SECRET"]);
    let spec_file = harness.write_spec(spec);
    let added = harness.run(&["add", "--file", &spec_file.to_string_lossy()]);
    assert_eq!(added.code, 0, "add failed: {}", added.stderr);

    let run = run_once(
        &harness,
        "watch",
        &[
            ("LANG", "en_US.UTF-8"),
            ("WATCHER_NONBASELINE_UNIQUE", "leaky"),
            ("MY_SECRET", "topsecret"),
        ],
    );
    assert_eq!(run.code, 0, "run failed: {}", run.stderr);

    let seen = &show(&harness, "watch")["state"]["seenEnv"];
    assert_eq!(
        seen["PATH"],
        json!(true),
        "PATH baseline must reach detector"
    );
    assert_eq!(
        seen["LANG"],
        json!(true),
        "locale baseline must reach detector"
    );
    assert_eq!(
        seen["MY_SECRET"],
        json!(true),
        "allowlisted credential must reach detector"
    );
    assert_eq!(
        seen["WATCHER_NONBASELINE_UNIQUE"],
        json!(false),
        "non-baseline, non-allowlisted vars must be cleared"
    );
}

#[test]
fn terminal_event_marks_watch_terminal() {
    let harness = Harness::new("terminal");
    let config = harness.write_config(json!({
        "stdout": json!({
            "schemaVersion": 1,
            "outcome": "terminal",
            "nextState": {"cursor": 3},
            "event": {"id": "provider:final", "kind": "watch.done", "subject": "s", "body": "b", "metadata": {}},
        }).to_string()
    }));
    add_watch(&harness, &config, "watch", "service:watcher");

    let run = run_once(&harness, "watch", &[]);
    assert_eq!(run.code, 0, "run failed: {}", run.stderr);
    assert_eq!(show(&harness, "watch")["status"], "terminal");
    assert_eq!(events(&harness, "watch").len(), 1);
}

// --------------------------------------------------------------------------------------------
// Duplicate / collision safety.
// --------------------------------------------------------------------------------------------

#[test]
fn stale_duplicate_never_sends_or_advances() {
    let harness = Harness::new("stale-duplicate");
    let config = harness.write_config(json!({"stdout": event_stdout("provider:dup", "body", 2)}));
    add_watch(&harness, &config, "watch", "service:watcher");

    assert_eq!(run_once(&harness, "watch", &[]).code, 0);
    assert_eq!(events(&harness, "watch").len(), 1);

    // Second run emits the identical event ID + envelope: a visible no-op, no new ledger row.
    assert_eq!(run_once(&harness, "watch", &[]).code, 0);
    assert_eq!(events(&harness, "watch").len(), 1);
    assert_eq!(show(&harness, "watch")["state"], json!({"cursor": 2}));
    assert_eq!(
        latest_attempt(&harness, "watch")["outcome"],
        "stale-duplicate"
    );
}

#[test]
fn duplicate_event_conflict_never_sends_or_advances() {
    let harness = Harness::new("conflict");
    let config = harness.write_config(json!({"stdout": event_stdout("provider:x", "body-a", 2)}));
    add_watch(&harness, &config, "watch", "service:watcher");
    assert_eq!(run_once(&harness, "watch", &[]).code, 0);

    // Reuse the same event ID with different evidence.
    fs::write(
        &config,
        json!({"stdout": event_stdout("provider:x", "body-b", 9)}).to_string(),
    )
    .unwrap();
    assert_eq!(run_once(&harness, "watch", &[]).code, 0);

    assert_eq!(events(&harness, "watch").len(), 1);
    assert_eq!(show(&harness, "watch")["state"], json!({"cursor": 2}));
    assert_eq!(show(&harness, "watch")["status"], "active");
    assert_eq!(
        latest_attempt(&harness, "watch")["outcome"],
        "duplicate-event-conflict"
    );
}

// --------------------------------------------------------------------------------------------
// Restart recovery.
// --------------------------------------------------------------------------------------------

#[test]
fn committed_state_survives_restart_and_new_events_continue() {
    let harness = Harness::new("restart");
    let config = harness.write_config(json!({"stdout": event_stdout("provider:1", "body", 2)}));
    add_watch(&harness, &config, "watch", "service:watcher");
    assert_eq!(run_once(&harness, "watch", &[]).code, 0);

    // A fresh watcher process (new registry open) still sees the committed opaque state.
    assert_eq!(show(&harness, "watch")["state"], json!({"cursor": 2}));

    // A later distinct event continues from the recovered cursor and appends a second ledger row.
    fs::write(
        &config,
        json!({"stdout": event_stdout("provider:2", "body", 3)}).to_string(),
    )
    .unwrap();
    assert_eq!(run_once(&harness, "watch", &[]).code, 0);
    assert_eq!(show(&harness, "watch")["state"], json!({"cursor": 3}));
    assert_eq!(events(&harness, "watch").len(), 2);
}

// --------------------------------------------------------------------------------------------
// Bounded failure handling.
// --------------------------------------------------------------------------------------------

#[test]
fn malformed_output_is_a_visible_failure() {
    let harness = Harness::new("malformed");
    let config = harness.write_config(json!({"stdout": "this is not json"}));
    add_watch(&harness, &config, "watch", "service:watcher");

    assert_eq!(run_once(&harness, "watch", &[]).code, 0);
    assert_eq!(show(&harness, "watch")["state"], json!({"cursor": 1}));
    assert_eq!(
        latest_attempt(&harness, "watch")["outcome"],
        "malformed-output"
    );
}

#[test]
fn oversize_output_is_a_visible_failure() {
    let harness = Harness::new("oversize");
    let config = harness.write_config(json!({"stdoutBytes": 300 * 1024}));
    add_watch(&harness, &config, "watch", "service:watcher");

    assert_eq!(run_once(&harness, "watch", &[]).code, 0);
    assert_eq!(show(&harness, "watch")["state"], json!({"cursor": 1}));
    // read_capped bails during execution.
    assert_eq!(
        latest_attempt(&harness, "watch")["outcome"],
        "execution-failed"
    );
}

#[test]
fn nonzero_exit_is_a_visible_failure() {
    let harness = Harness::new("nonzero");
    let config =
        harness.write_config(json!({"stdout": event_stdout("provider:1", "b", 2), "exitCode": 3}));
    add_watch(&harness, &config, "watch", "service:watcher");

    assert_eq!(run_once(&harness, "watch", &[]).code, 0);
    assert_eq!(show(&harness, "watch")["state"], json!({"cursor": 1}));
    assert_eq!(
        latest_attempt(&harness, "watch")["outcome"],
        "execution-failed"
    );
}

#[test]
fn degraded_output_applies_backoff_without_state_change() {
    let harness = Harness::new("degraded");
    let config = harness.write_config(json!({
        "stdout": json!({"schemaVersion": 1, "outcome": "degraded"}).to_string()
    }));
    add_watch(&harness, &config, "watch", "service:watcher");

    assert_eq!(run_once(&harness, "watch", &[]).code, 0);
    let after = show(&harness, "watch");
    assert_eq!(after["state"], json!({"cursor": 1}));
    assert_eq!(after["failureCount"], 1);
    assert_eq!(latest_attempt(&harness, "watch")["outcome"], "degraded");
}

#[test]
fn pinned_digest_mismatch_at_runtime_is_a_visible_failure() {
    let harness = Harness::new("pinned-mismatch");
    let config = harness.write_config(json!({"stdout": event_stdout("provider:1", "b", 2)}));
    // Compute the current digest so `add` accepts a pinned watch.
    let digest = sha256_hex(&fs::read(&config).unwrap());
    let mut spec = spec_for(&config, "watch", "service:watcher");
    spec["scriptMode"] = json!("pinned");
    spec["scriptDigest"] = json!(digest);
    let spec_file = harness.write_spec(spec);
    assert_eq!(
        harness
            .run(&["add", "--file", &spec_file.to_string_lossy()])
            .code,
        0
    );

    // Change the pinned script before running: the runtime must refuse to execute it.
    fs::write(
        &config,
        json!({"stdout": event_stdout("provider:1", "b", 2), "sleepMs": 0}).to_string(),
    )
    .unwrap();
    assert_eq!(run_once(&harness, "watch", &[]).code, 0);
    assert_eq!(
        latest_attempt(&harness, "watch")["outcome"],
        "pinned-digest-mismatch"
    );
    assert_eq!(show(&harness, "watch")["state"], json!({"cursor": 1}));
}

#[test]
fn follow_path_drift_during_execution_is_a_visible_failure() {
    let harness = Harness::new("drift");
    // The detector mutates its own script mid-attempt, so the post-execution digest differs.
    let config = harness.write_config(json!({
        "stdout": event_stdout("provider:1", "b", 2),
        "mutateSelf": true,
    }));
    add_watch(&harness, &config, "watch", "service:watcher");

    assert_eq!(run_once(&harness, "watch", &[]).code, 0);
    assert_eq!(latest_attempt(&harness, "watch")["outcome"], "script-drift");
    assert_eq!(show(&harness, "watch")["state"], json!({"cursor": 1}));
}

// --------------------------------------------------------------------------------------------
// Timeout and full process-group termination.
// --------------------------------------------------------------------------------------------

#[test]
fn timeout_terminates_the_whole_process_group() {
    let harness = Harness::new("timeout");
    let markers = harness.root.join("child-markers");
    fs::create_dir_all(&markers).unwrap();
    let config = harness.write_config(json!({
        "sleepMs": 60_000,
        "spawnChildDir": markers.to_string_lossy(),
        "childSleepMs": 60_000,
    }));
    add_watch(&harness, &config, "watch", "service:watcher");

    let start = std::time::Instant::now();
    let run = run_once(&harness, "watch", &[]);
    let elapsed = start.elapsed();
    assert_eq!(run.code, 0, "run failed: {}", run.stderr);
    // The 1s timeout plus teardown must be far below the 60s detector sleep.
    assert!(
        elapsed < std::time::Duration::from_secs(20),
        "shutdown hung for {elapsed:?}"
    );

    // Give the OS a moment to reap the terminated group.
    std::thread::sleep(std::time::Duration::from_millis(500));
    assert!(
        markers.join("child-started").exists(),
        "descendant should have started"
    );
    assert!(
        !markers.join("child-finished").exists(),
        "descendant must be killed with the process group before finishing"
    );
    assert_eq!(
        latest_attempt(&harness, "watch")["outcome"],
        "execution-failed"
    );
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(bytes);
    let mut text = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(text, "{byte:02x}");
    }
    text
}
