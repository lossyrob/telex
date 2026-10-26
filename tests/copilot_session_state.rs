use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

struct LocalHome(PathBuf);

impl LocalHome {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "telex-classifier-{}-{}-{}",
            std::process::id(),
            telex::model::now_ms(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&root).unwrap();
        Self(root)
    }

    fn log(&self) -> PathBuf {
        self.0
            .join(".copilot")
            .join("session-state")
            .join("fixture")
            .join("events.jsonl")
    }

    fn write(&self, bytes: &[u8]) {
        let log = self.log();
        std::fs::create_dir_all(log.parent().unwrap()).unwrap();
        std::fs::write(log, bytes).unwrap();
    }

    fn run(&self, format: &str, session: &str) -> Output {
        let binary = Path::new(env!("CARGO_BIN_EXE_telex"));
        assert!(binary.is_absolute());
        Command::new(binary)
            .args([format, "copilot", "session-state", "--session", session])
            .env("HOME", &self.0)
            .env("USERPROFILE", &self.0)
            .env("LOCALAPPDATA", self.0.join("local"))
            .env("TELEX_HOME", self.0.join("telex"))
            .env("TELEX_DB", self.0.join("db.sqlite"))
            .env("TELEX_INSTALL_ROOT", self.0.join("install"))
            .env("TELEX_BACKEND", "must-not-connect")
            .output()
            .unwrap()
    }

    fn json(&self) -> Value {
        let output = self.run("--json", "fixture");
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(output.stderr.is_empty());
        serde_json::from_slice(&output.stdout).unwrap()
    }
}

impl Drop for LocalHome {
    fn drop(&mut self) {
        let removed = std::fs::remove_dir_all(&self.0);
        if !std::thread::panicking() {
            removed.expect("remove isolated Copilot home");
        }
    }
}

#[test]
fn local_home_cleanup_failure_while_unwinding_preserves_the_original_panic() {
    let unwound = std::panic::catch_unwind(|| {
        let home = LocalHome::new();
        std::fs::remove_dir_all(&home.0).unwrap();
        panic!("original test failure");
    })
    .unwrap_err();
    assert_eq!(
        unwound.downcast_ref::<&str>(),
        Some(&"original test failure")
    );
}

#[test]
fn local_home_cleanup_failure_without_unwinding_still_fails() {
    let home = LocalHome::new();
    std::fs::remove_dir_all(&home.0).unwrap();
    assert!(std::panic::catch_unwind(|| drop(home)).is_err());
}

#[test]
fn session_state_cli_reads_all_fixture_states_without_backend_or_writes() {
    let home = LocalHome::new();
    for (fixture, expected) in [
        (
            include_bytes!("fixtures/copilot-session-state/busy.jsonl").as_slice(),
            "busy",
        ),
        (
            include_bytes!("fixtures/copilot-session-state/idle.jsonl").as_slice(),
            "idle",
        ),
        (
            include_bytes!("fixtures/copilot-session-state/waiting.jsonl").as_slice(),
            "waiting-on-operator",
        ),
        (
            include_bytes!("fixtures/copilot-session-state/not-loaded.jsonl").as_slice(),
            "not-loaded",
        ),
        (
            include_bytes!("fixtures/copilot-session-state/unknown.jsonl").as_slice(),
            "unknown",
        ),
        (
            include_bytes!("fixtures/copilot-session-state/malformed.jsonl").as_slice(),
            "unknown",
        ),
    ] {
        home.write(fixture);
        let before = std::fs::read(home.log()).unwrap();
        let result = home.json();
        assert_eq!(result["session_id"], "fixture");
        assert_eq!(result["state"], expected);
        assert!(result["reason"].is_string());
        assert_eq!(std::fs::read(home.log()).unwrap(), before);
        assert!(!home.0.join("telex").exists());
        assert!(!home.0.join("db.sqlite").exists());
        assert!(!home.0.join("install").exists());
        assert!(!home.0.join("local").exists());
    }
    home.write(include_bytes!("fixtures/copilot-session-state/idle.jsonl"));
    let result = home.json();
    assert_eq!(result["activity"]["id"], "stop-end-1");
    assert_eq!(result["activity"]["timestamp"], "2026-10-25T20:00:02Z");
    let text = home.run("--text", "fixture");
    assert!(text.status.success());
    assert_eq!(
        String::from_utf8(text.stdout).unwrap().trim(),
        "fixture: idle (agent-stop-allowed)"
    );
}

#[test]
fn session_state_cli_missing_unreadable_and_empty_logs_are_explicit_unknowns() {
    let home = LocalHome::new();
    assert_eq!(home.json()["reason"], "log-missing");
    home.write(b"");
    assert_eq!(home.json()["state"], "unknown");
    std::fs::remove_file(home.log()).unwrap();
    std::fs::create_dir(home.log()).unwrap();
    assert_eq!(home.json()["state"], "unknown");
}

#[cfg(windows)]
#[test]
fn session_state_cli_read_denied_is_unknown() {
    use std::os::windows::fs::OpenOptionsExt;
    let home = LocalHome::new();
    home.write(include_bytes!("fixtures/copilot-session-state/idle.jsonl"));
    let _exclusive = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(home.log())
        .unwrap();
    let result = home.json();
    assert_eq!(result["state"], "unknown");
    assert_eq!(result["reason"], "log-unreadable");
}

#[test]
fn session_state_cli_rejects_path_inputs_before_reading() {
    let home = LocalHome::new();
    for session in [
        "../fixture",
        r"..\fixture",
        r"C:\fixture",
        "remote:fixture",
        ".",
    ] {
        let output = home.run("--json", session);
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        assert!(String::from_utf8_lossy(&output.stderr).contains("session id must"));
    }
}

#[test]
fn session_state_cli_ignores_a_large_prefix_and_returns_unknown_without_a_tail_signal() {
    let home = LocalHome::new();
    let mut bytes = vec![b'x'; telex::commands::copilot::session_state::TAIL_BYTES * 3];
    bytes.push(b'\n');
    bytes.extend_from_slice(include_bytes!("fixtures/copilot-session-state/idle.jsonl"));
    home.write(&bytes);
    assert_eq!(home.json()["state"], "idle");
    let mut long_record = serde_json::to_vec(&serde_json::json!({
        "type":"session.usage_checkpoint", "id":"large-accounting",
        "timestamp":"2026-10-25T20:01:00Z", "data":{"padding":"x".repeat(300_000)}
    }))
    .unwrap();
    long_record.push(b'\n');
    bytes.extend_from_slice(&long_record);
    home.write(&bytes);
    assert_eq!(home.json()["state"], "unknown");
    home.write(b"{\"type\":");
    assert_eq!(home.json()["reason"], "incomplete-tail");
}
