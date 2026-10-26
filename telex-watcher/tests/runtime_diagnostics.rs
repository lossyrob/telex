use serde_json::{json, Value};
use std::path::PathBuf;
use std::process::{Command, Output};
use telex_watcher::{Runtime, RuntimeConfig};

struct Fixture {
    root: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let root = std::env::current_dir()
            .unwrap()
            .join(format!(".runtime-diagnostics-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        Self {
            root: std::fs::canonicalize(root).unwrap(),
        }
    }

    fn command(&self) -> Command {
        let binary = PathBuf::from(env!("CARGO_BIN_EXE_telex-watcher"));
        assert!(binary.is_absolute());
        let mut command = Command::new(binary);
        command
            .env("TELEX_HOME", self.root.join("home"))
            .env("TELEX_DB", self.root.join("telex.sqlite"))
            .env("TELEX_INSTALL_ROOT", self.root.join("install"))
            .env("TELEX_WATCHER_REGISTRY", self.root.join("registry.sqlite"))
            .env("LOCALAPPDATA", self.root.join("local"))
            .env("HOME", self.root.join("user-home"))
            .env("USERPROFILE", self.root.join("user-home"))
            .current_dir(&self.root);
        command
    }

    fn failed_run(&self) -> (Output, Value) {
        let output = self.command().args(["run", "--json"]).output().unwrap();
        assert_eq!(output.status.code(), Some(1));
        let envelope: Value = serde_json::from_slice(&output.stdout).unwrap();
        (output, envelope)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let removed = std::fs::remove_dir_all(&self.root);
        if !std::thread::panicking() {
            removed.expect("remove isolated diagnostics fixture");
        }
    }
}

fn assert_runtime_failure(output: &Output, envelope: &Value) {
    assert_eq!(
        envelope,
        &json!({
            "schemaVersion":1,
            "command":"run",
            "ok":false,
            "error":{
                "code":"runtime-failed",
                "message":"runtime startup or execution failed; inspect stderr for the stage and cause"
            }
        })
    );
    assert_eq!(
        output.stdout.iter().filter(|byte| **byte == b'\n').count(),
        1
    );
    let diagnostic = std::str::from_utf8(&output.stderr).unwrap();
    assert!(diagnostic.starts_with("runtime-failed\n"));
    assert!(diagnostic.contains("registry open"), "{diagnostic}");
    assert!(diagnostic.ends_with('\n'));
    assert!(output.stderr.len() <= 4096);
}

#[test]
fn startup_io_failure_reports_stage_and_cause_without_changing_envelope() {
    let fixture = Fixture::new();
    let registry = fixture.root.join("registry.sqlite");
    std::fs::create_dir(&registry).unwrap();
    let (output, envelope) = fixture.failed_run();
    assert_runtime_failure(&output, &envelope);
    let diagnostic = String::from_utf8(output.stderr).unwrap();
    assert!(diagnostic.contains("os error"), "{diagnostic}");
    // The implicit host command and human presentation retain the same diagnostic.
    let output = fixture.command().output().unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8(output.stdout)
        .unwrap()
        .contains("runtime-failed"));
    assert!(String::from_utf8(output.stderr)
        .unwrap()
        .contains("registry open"));
}

#[test]
fn sqlite_startup_failure_omits_environment_values_dsns_and_database_error_text() {
    let fixture = Fixture::new();
    let registry = fixture.root.join("registry.sqlite");
    let connection = rusqlite::Connection::open(registry).unwrap();
    let secret = "opaque-environment-value";
    let dsn = "postgres://reader:private-password@localhost/private";
    let private_text = "unclassified private database content";
    connection
        .execute_batch(&format!(
            "CREATE TABLE watcher_v2_meta (
                singleton INTEGER PRIMARY KEY, version INTEGER, revision INTEGER,
                responsibility TEXT);
             CREATE TRIGGER fail_startup BEFORE INSERT ON watcher_v2_meta BEGIN
                SELECT RAISE(ABORT, '{secret} {dsn} token=hidden {private_text}');
             END;"
        ))
        .unwrap();
    drop(connection);
    let output = fixture
        .command()
        .env("DIAGNOSTIC_FIXTURE_VALUE", secret)
        .args(["run", "--json"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let envelope = serde_json::from_slice(&output.stdout).unwrap();
    assert_runtime_failure(&output, &envelope);
    let diagnostic = String::from_utf8(output.stderr).unwrap();
    assert!(diagnostic.contains("SQLite"), "{diagnostic}");
    for value in [secret, dsn, "hidden", private_text] {
        assert!(!diagnostic.contains(value), "{diagnostic}");
        assert!(!String::from_utf8_lossy(&output.stdout).contains(value));
    }
}

#[test]
fn registry_ownership_and_input_codes_do_not_gain_runtime_failure_diagnostics() {
    let fixture = Fixture::new();
    let (runtime, _management) = Runtime::open(RuntimeConfig::installed(
        fixture.root.join("registry.sqlite"),
        fixture.root.join("install"),
    ))
    .unwrap();
    let (output, envelope) = fixture.failed_run();
    assert_eq!(envelope["error"]["code"], "registry-already-owned");
    assert!(output.stderr.is_empty());
    drop(runtime);
    let output = fixture
        .command()
        .env_remove("TELEX_INSTALL_ROOT")
        .args(["run", "--json"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let envelope: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(envelope["error"]["code"], "install-root-required");
    assert!(output.stderr.is_empty());
}
