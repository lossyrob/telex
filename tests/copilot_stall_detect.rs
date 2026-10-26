use serde_json::{json, Value};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "telex-stall-cli-{}-{}-{}",
            std::process::id(),
            telex::model::now_ms(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(path.join("group")).unwrap();
        Self(path)
    }

    fn request(&self) -> Value {
        json!({
            "schemaVersion":2,
            "attempt":{"id":"attempt","now":"2026-10-26T12:00:00Z"},
            "watch":{"id":"group-stall","activatedAt":"2026-10-26T11:00:00Z",
                "lastSuccessAt":null,
                "parameters":{
                    "groupDirectory":self.0.join("group"),
                    "backendProfile":"fixture",
                    "watcherExecutable":self.0.join("absent-watcher"),
                    "watcherRegistry":self.0.join("registry.sqlite"),
                    "graceSeconds":60
                }},
            "state":null
        })
    }

    fn run(&self, bytes: &[u8]) -> Output {
        let binary = Path::new(env!("CARGO_BIN_EXE_telex"));
        assert!(binary.is_absolute());
        let mut child = Command::new(binary)
            .args(["copilot", "stall-detect"])
            .env("HOME", &self.0)
            .env("USERPROFILE", &self.0)
            .env("LOCALAPPDATA", self.0.join("local"))
            .env("TELEX_HOME", self.0.join("home"))
            .env("TELEX_DB", self.0.join("db.sqlite"))
            .env("TELEX_CONFIG", self.0.join("profiles.toml"))
            .env("TELEX_INSTALL_ROOT", self.0.join("install"))
            .env_remove("TELEX_BACKEND")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(bytes).unwrap();
        child.wait_with_output().unwrap()
    }

    fn json(&self, request: &Value) -> (Output, Value) {
        let output = self.run(&serde_json::to_vec(request).unwrap());
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let result: Value = serde_json::from_slice(&output.stdout).unwrap();
        telex::watcher_protocol::parse_result(&output.stdout).unwrap();
        (output, result)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let removed = std::fs::remove_dir_all(&self.0);
        if !std::thread::panicking() {
            removed.expect("remove isolated stall detector CLI fixture");
        }
    }
}

#[test]
fn fixture_cleanup_is_attempted_during_unwind_and_normal_return() {
    for panicking in [true, false] {
        let fixture = Fixture::new();
        let path = fixture.0.clone();
        let result = std::panic::catch_unwind(|| {
            let _fixture = fixture;
            if panicking {
                panic!("original test failure");
            }
        });
        assert_eq!(result.is_err(), panicking);
        assert!(!path.exists());
    }
}

#[test]
fn fixture_cleanup_failure_while_unwinding_preserves_the_original_panic() {
    let unwound = std::panic::catch_unwind(|| {
        let fixture = Fixture::new();
        std::fs::remove_dir_all(&fixture.0).unwrap();
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
    std::fs::remove_dir_all(&fixture.0).unwrap();
    assert!(std::panic::catch_unwind(|| drop(fixture)).is_err());
}

#[test]
fn empty_and_inactive_groups_produce_v2_without_backend_daemon_or_install_writes() {
    let fixture = Fixture::new();
    let (output, result) = fixture.json(&fixture.request());
    assert!(output.stderr.is_empty());
    assert_eq!(
        result,
        json!({"schemaVersion":2,"outcome":"idle","nextState":null})
    );
    for state in ["parked", "operator-held", "done"] {
        std::fs::write(
            fixture.0.join("group").join("member.json"),
            serde_json::to_vec(&json!({
                "schemaVersion":1,"sessionId":"member","address":"member:a","state":state,
                "reason":"planned wait"
            }))
            .unwrap(),
        )
        .unwrap();
        assert_eq!(fixture.json(&fixture.request()).1["outcome"], "idle");
    }
    for name in ["home", "db.sqlite", "install", "local", "registry.sqlite"] {
        assert!(
            !fixture.0.join(name).exists(),
            "{name} was unexpectedly created"
        );
    }
}

#[test]
fn unreadable_observations_are_degraded_with_diagnostics_and_no_state_advance() {
    let fixture = Fixture::new();
    std::fs::write(
        fixture.0.join("group").join("member.json"),
        serde_json::to_vec(&json!({
            "schemaVersion":1,"sessionId":"member","address":"member:a","state":"active"
        }))
        .unwrap(),
    )
    .unwrap();
    let (output, result) = fixture.json(&fixture.request());
    assert!(!output.stderr.is_empty());
    assert_eq!(result, json!({"schemaVersion":2,"outcome":"degraded"}));
    assert!(!fixture.0.join("db.sqlite").exists());
    std::fs::write(fixture.0.join("group").join("member.json"), b"{").unwrap();
    let (output, result) = fixture.json(&fixture.request());
    assert!(String::from_utf8_lossy(&output.stderr).contains("malformed-output"));
    assert_eq!(result["outcome"], "degraded");
}

#[test]
fn malformed_v1_duplicate_and_unknown_request_fields_fail_without_success_output() {
    let fixture = Fixture::new();
    let mut v1 = fixture.request();
    v1["schemaVersion"] = json!(1);
    let mut extra = fixture.request();
    extra["unknown"] = json!(true);
    let valid = serde_json::to_vec(&fixture.request()).unwrap();
    let duplicate = format!(
        "{{\"schemaVersion\":2,{}",
        std::str::from_utf8(&valid[1..]).unwrap()
    );
    for bytes in [
        b"{".to_vec(),
        serde_json::to_vec(&v1).unwrap(),
        serde_json::to_vec(&extra).unwrap(),
        duplicate.into_bytes(),
    ] {
        let output = fixture.run(&bytes);
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        assert!(!output.stderr.is_empty());
    }
}
