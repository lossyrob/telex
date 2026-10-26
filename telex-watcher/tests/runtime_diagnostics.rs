use serde_json::{json, Value};
use std::path::PathBuf;
use std::process::{Command, Output};
#[cfg(windows)]
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
            .env("SHLVL", "1")
            .env("LANG", "C")
            .env("CI", "true")
            .env("DIAGNOSTIC_STAGE_LABEL", "registry open")
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
    let expected_code = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&registry)
        .unwrap_err()
        .raw_os_error()
        .unwrap();
    let (output, envelope) = fixture.failed_run();
    assert_runtime_failure(&output, &envelope);
    let diagnostic = String::from_utf8(output.stderr).unwrap();
    assert!(
        diagnostic.contains(&format!("os error {expected_code})")),
        "{diagnostic}"
    );
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
    let connection = rusqlite::Connection::open(&registry).unwrap();
    let secret = "opaque-environment-value";
    let dsn = "postgres://reader:private-password@localhost/private";
    let private_text = "unclassified_private_database_content";
    connection
        .execute_batch(
            "CREATE TABLE watcher_v2_meta (
                singleton INTEGER PRIMARY KEY, version INTEGER, revision INTEGER,
                responsibility TEXT);
             PRAGMA writable_schema=ON;",
        )
        .unwrap();
    // Fail while reading the schema, before startup needs a successful write.
    connection
        .execute(
            "UPDATE sqlite_master SET sql=?1 WHERE name='watcher_v2_meta'",
            [format!(
                "CREATE TABLE watcher_v2_meta {private_text} {secret} {dsn} token=hidden"
            )],
        )
        .unwrap();
    drop(connection);
    let connection = rusqlite::Connection::open(&registry).unwrap();
    let error = connection
        .query_row(
            "SELECT version FROM watcher_v2_meta WHERE singleton=1",
            [],
            |row| row.get::<_, u32>(0),
        )
        .unwrap_err();
    assert_eq!(
        error.sqlite_error().unwrap().extended_code,
        rusqlite::ffi::SQLITE_CORRUPT
    );
    assert!(error.to_string().contains(private_text), "{error}");
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
    assert!(diagnostic.contains("Error code 11:"), "{diagnostic}");
    for value in [secret, dsn, "hidden", private_text] {
        assert!(!diagnostic.contains(value), "{diagnostic}");
        assert!(!String::from_utf8_lossy(&output.stdout).contains(value));
    }
}

#[test]
fn registry_ownership_and_input_codes_do_not_gain_runtime_failure_diagnostics() {
    let fixture = Fixture::new();
    #[cfg(windows)]
    let (ownership, _management) = Runtime::open(RuntimeConfig::installed(
        fixture.root.join("registry.sqlite"),
        fixture.root.join("install"),
    ))
    .unwrap();
    #[cfg(unix)]
    let ownership = {
        // Match the Unix owner's flock without requiring SQLite startup.
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(fixture.root.join("registry.sqlite"))
            .unwrap();
        fs2::FileExt::try_lock_exclusive(&file).unwrap();
        file
    };
    let (output, envelope) = fixture.failed_run();
    assert_eq!(envelope["error"]["code"], "registry-already-owned");
    assert!(output.stderr.is_empty());
    drop(ownership);
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
