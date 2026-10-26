#![cfg(all(feature = "sqlite", any(windows, target_os = "linux")))]

#[path = "../../tests/support/telex_isolation.rs"]
mod isolation;

use isolation::{run_with_timeout, CliOutput, Isolation};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

struct Fixture {
    isolation: Isolation,
    watcher: PathBuf,
    registry: PathBuf,
    user_home: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let isolation = Isolation::new("watcher-installation");
        let mut profile = telex::profiles::implicit_sqlite(None);
        profile.path = Some(isolation.db_path.to_string_lossy().into_owned());
        isolation.write_config(&telex::profiles::ConfigFile {
            default: None,
            backends: BTreeMap::from([("fixture".into(), profile)]),
        });
        let watcher = isolation
            .root
            .join(format!("telex-watcher{}", std::env::consts::EXE_SUFFIX));
        std::fs::copy(env!("CARGO_BIN_EXE_telex-watcher"), &watcher).unwrap();
        #[cfg(windows)]
        let user_home = isolation.root.join("user");
        #[cfg(unix)]
        let user_home = PathBuf::from(
            std::env::var_os("TELEX_TEST_HOME_ROOT")
                .expect("set the private short TELEX_TEST_HOME_ROOT described in CONTRIBUTING.md"),
        )
        .join(&uuid::Uuid::new_v4().simple().to_string()[..8]);
        isolation::create_owner_private_dir(&user_home);
        let registry = isolation.root.join("registry.sqlite");
        Self {
            isolation,
            watcher,
            registry,
            user_home,
        }
    }

    fn process(&self, binary: &Path) -> Command {
        let mut command = self.isolation.command_for(binary);
        command
            .env("HOME", &self.user_home)
            .env("USERPROFILE", &self.user_home)
            .env("LOCALAPPDATA", &self.isolation.state_dir)
            .env_remove("TELEX_WATCHER_REGISTRY");
        command
    }

    fn parameters(&self) -> Value {
        json!({
            "Action": "create",
            "TelexExecutable": self.isolation.current_binary(),
            "WatcherExecutable": self.watcher,
            "WatcherRegistry": self.registry,
            "InstallRoot": self.isolation.install_root,
            "BackendProfile": "fixture",
            "Sender": "service:watcher/bootstrap",
            "EnvironmentNames": ["TELEX_HOME", "TELEX_CONFIG", "LOCALAPPDATA"]
        })
    }

    fn tool(&self, parameters: Value) -> CliOutput {
        self.tool_with_home(parameters, true)
    }

    fn tool_with_home(&self, parameters: Value, explicit_telex_home: bool) -> CliOutput {
        let mut command = self.process(Path::new("pwsh"));
        if !explicit_telex_home {
            command.env_remove("TELEX_HOME");
        }
        command
            .env(
                "TELEX_TEST_INSTALLATION_TOOL",
                Path::new(env!("CARGO_MANIFEST_DIR")).join("installation.ps1"),
            )
            .env("TELEX_TEST_INSTALLATION_ARGS", parameters.to_string())
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "$ErrorActionPreference='Stop'; \
                 $p=$env:TELEX_TEST_INSTALLATION_ARGS | ConvertFrom-Json -AsHashtable; \
                 & $env:TELEX_TEST_INSTALLATION_TOOL @p",
            ]);
        let output = run_with_timeout(command, Duration::from_secs(30));
        assert!(!output.timed_out, "bootstrap command timed out: {output:?}");
        output
    }

    fn record_path(&self) -> PathBuf {
        self.isolation
            .home
            .join("watcher")
            .join("installation.json")
    }

    fn management(&self, args: &[&str]) -> Value {
        let mut command = self.process(&self.watcher);
        command
            .arg("--registry")
            .arg(&self.registry)
            .arg("--json")
            .args(args);
        let output = run_with_timeout(command, Duration::from_secs(45));
        serde_json::from_str(&output.stdout).unwrap_or_else(|e| panic!("{e}: {output:?}"))
    }
}

#[test]
fn unset_telex_home_uses_only_the_isolated_user_home_default() {
    let fixture = Fixture::new();
    let mut probe = fixture.process(Path::new("pwsh"));
    probe.args(["-NoProfile", "-NonInteractive", "-Command", "$HOME"]);
    let home = run_with_timeout(probe, Duration::from_secs(15));
    home.assert_success("resolve PowerShell home without any state access");
    assert_eq!(
        PathBuf::from(home.stdout.trim()),
        fixture.user_home,
        "refuse bootstrap if shell home isolation did not take effect"
    );
    let config_root = fixture.user_home.join(".telex");
    isolation::create_owner_private_dir(&config_root);
    let output = fixture.tool_with_home(fixture.parameters(), false);
    output.assert_success("create default under user home when TELEX_HOME is absent");
    let record: Value = serde_json::from_str(&output.stdout).unwrap();
    assert_eq!(record["configRoot"], json!(config_root));
    assert!(config_root
        .join("watcher")
        .join("installation.json")
        .exists());
    fixture
        .tool_with_home(json!({"Action":"verify"}), false)
        .assert_success("verify the same home default without searching elsewhere");
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if self.isolation.cap_path().is_some() {
            self.isolation.stop_daemon();
        }
        std::fs::remove_dir_all(&self.user_home).expect("remove private fixture home");
        self.isolation.cleanup();
    }
}

struct Host(Child);

impl Drop for Host {
    fn drop(&mut self) {
        if self.0.try_wait().unwrap().is_none() {
            self.0.kill().expect("kill only fixture runtime");
        }
        self.0.wait().expect("reap fixture runtime");
    }
}

#[test]
fn record_creation_discovery_verification_and_refusals() {
    let fixture = Fixture::new();
    let parameters = fixture.parameters();
    let created = fixture.tool(parameters.clone());
    created.assert_success("create default installation record");
    let record: Value = serde_json::from_str(&created.stdout).unwrap();
    assert_eq!(record["schemaVersion"], 1);
    assert_eq!(record["configRoot"], json!(fixture.isolation.home));
    assert_eq!(record["watcherRegistry"], json!(fixture.registry));
    assert!(!fixture.registry.exists(), "record preparation is offline");
    assert!(!fixture.isolation.daemon_running());
    let original = std::fs::read(fixture.record_path()).unwrap();
    fixture
        .tool(parameters.clone())
        .assert_failure("existing record is never overwritten");
    assert_eq!(std::fs::read(fixture.record_path()).unwrap(), original);
    fixture
        .tool(json!({"Action": "verify"}))
        .assert_success("discover and verify default record");
    let mut missing = parameters.clone();
    missing["RecordPath"] = json!(fixture
        .isolation
        .root
        .join("missing")
        .join("installation.json"));
    fixture
        .tool(json!({"Action": "verify", "RecordPath": missing["RecordPath"]}))
        .assert_failure("an explicit missing record never falls back to the default");
    for (key, value) in [
        ("schemaVersion", json!(2)),
        ("schemaVersion", json!(true)),
        ("watcherExecutable", json!("relative-watcher")),
        ("watcherSha256", json!("changed")),
        ("pollIntervalSeconds", json!(0)),
        (
            "environmentNames",
            json!(["TELEX_HOME", "ABSENT_BOOTSTRAP_CREDENTIAL"]),
        ),
    ] {
        let mut changed = record.clone();
        changed[key] = value;
        std::fs::write(fixture.record_path(), changed.to_string()).unwrap();
        fixture
            .tool(json!({"Action": "verify"}))
            .assert_failure(&format!("reject changed {key}"));
    }
    let duplicate = format!(
        "{},\"sender\":\"duplicate\"}}",
        record.to_string().trim_end_matches('}')
    );
    std::fs::write(fixture.record_path(), duplicate).unwrap();
    fixture
        .tool(json!({"Action": "verify"}))
        .assert_failure("duplicate fields are invalid, not last-value-wins configuration");
    std::fs::write(fixture.record_path(), &original).unwrap();
    fixture
        .isolation
        .install_tag("v0.3.0-bootstrap-candidate", true);
    fixture
        .tool(json!({"Action": "verify"}))
        .assert_failure("a selector change does not silently adopt the new installation");
    telex::install::switch_to(&fixture.isolation.layout(), &fixture.isolation.tag).unwrap();
    fixture
        .tool(json!({"Action": "verify"}))
        .assert_success("rolling the isolated selector back restores the original record");
    assert_eq!(std::fs::read(fixture.record_path()).unwrap(), original);
    let mut explicit = parameters.clone();
    explicit["RecordPath"] = json!(fixture
        .isolation
        .root
        .join("second")
        .join("installation.json"));
    fixture
        .tool(explicit.clone())
        .assert_success("create explicit second record");
    let explicit_path = explicit["RecordPath"].clone();
    fixture
        .tool(json!({"Action": "verify", "RecordPath": explicit_path}))
        .assert_success("verify only explicit record when supplied");
    std::fs::write(&fixture.registry, b"existing registry must stay").unwrap();
    explicit["RecordPath"] = json!(fixture
        .isolation
        .root
        .join("third")
        .join("installation.json"));
    fixture
        .tool(explicit)
        .assert_failure("initial setup refuses any existing registry");
    assert_eq!(
        std::fs::read(&fixture.registry).unwrap(),
        b"existing registry must stay"
    );
    std::fs::write(&fixture.watcher, b"changed executable").unwrap();
    fixture
        .tool(json!({"Action": "verify"}))
        .assert_failure("changed retained executable is refused");
}

#[test]
fn unsafe_record_authority_and_missing_auth_features_are_refused() {
    let fixture = Fixture::new();
    fixture
        .tool(fixture.parameters())
        .assert_success("create authority fixture");
    let mut command = fixture.process(Path::new("pwsh"));
    command
        .env("TELEX_TEST_RECORD", fixture.record_path())
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "$ErrorActionPreference='Stop'; $p=$env:TELEX_TEST_RECORD; \
             if ($IsWindows) { \
               $f=[IO.FileInfo]::new($p); \
               $a=[IO.FileSystemAclExtensions]::GetAccessControl($f,[Security.AccessControl.AccessControlSections]::Access); \
               $s=[Security.Principal.SecurityIdentifier]::new('S-1-1-0'); \
               $a.AddAccessRule([Security.AccessControl.FileSystemAccessRule]::new($s,'Write','Allow')); \
               [IO.FileSystemAclExtensions]::SetAccessControl($f,$a) \
             } else { [IO.File]::SetUnixFileMode($p,[IO.UnixFileMode]438) }",
        ]);
    run_with_timeout(command, Duration::from_secs(15))
        .assert_success("make only disposable record unsafe");
    fixture
        .tool(json!({"Action": "verify"}))
        .assert_failure("record writable by another user is refused");
    let mut profile = telex::profiles::implicit_sqlite(None);
    profile.kind = "postgres".into();
    profile.auth = Some("entra".into());
    fixture
        .isolation
        .write_config(&telex::profiles::ConfigFile {
            default: None,
            backends: BTreeMap::from([("fixture".into(), profile)]),
        });
    let mut parameters = fixture.parameters();
    parameters["RecordPath"] = json!(fixture
        .isolation
        .root
        .join("auth")
        .join("installation.json"));
    if !cfg!(feature = "entra") {
        let output = fixture.tool(parameters);
        output.assert_failure("missing Watcher Entra support is refused before launch");
        assert!(output
            .stderr
            .contains("lacks the configured backend/authentication feature"));
    }
}

#[test]
fn record_launch_registration_shutdown_and_retained_registry() {
    let fixture = Fixture::new();
    fixture
        .tool(fixture.parameters())
        .assert_success("prepare launch record");
    let mut command = fixture.process(&fixture.watcher);
    command
        .arg("--registry")
        .arg(&fixture.registry)
        .arg("--install-root")
        .arg(&fixture.isolation.install_root)
        .args(["--json", "run"])
        .stdout(Stdio::null())
        .stderr(Stdio::inherit());
    let mut host = Host(command.spawn().unwrap());
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let status = fixture.management(&["status"]);
        if status["ok"] == true && status["result"]["value"]["productionReady"] == true {
            break;
        }
        assert!(
            host.0.try_wait().unwrap().is_none(),
            "fixture runtime exited"
        );
        assert!(Instant::now() < deadline, "{status}");
        std::thread::sleep(Duration::from_millis(25));
    }
    let config = fixture.isolation.root.join("detector.json");
    std::fs::write(
        &config,
        r#"{"stdout":"{\"schemaVersion\":2,\"outcome\":\"idle\",\"nextState\":null}"}"#,
    )
    .unwrap();
    let registration = fixture.isolation.root.join("registration.txt");
    std::fs::write(
        &registration,
        json!({
            "schemaVersion": 2,
            "id": "bootstrap-proof",
            "command": [env!("CARGO_BIN_EXE_fake_detector"), config],
            "intervalSeconds": 30,
            "timeoutSeconds": 30,
            "backendProfile": "fixture",
            "sender": "service:watcher/bootstrap",
            "target": "project:bootstrap/root",
            "environmentAllowlist": ["TELEX_HOME", "TELEX_CONFIG", "LOCALAPPDATA"]
        })
        .to_string(),
    )
    .unwrap();
    assert_eq!(
        fixture.management(&["register", registration.to_str().unwrap()])["ok"],
        true
    );
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let status = fixture.management(&["status"]);
        let watch = fixture.management(&["show", "bootstrap-proof"]);
        if status["result"]["value"]["senders"]
            .as_array()
            .unwrap()
            .iter()
            .any(|sender| sender["ready"] == true)
            && watch["result"]["value"]["lastSuccessAt"].is_string()
        {
            break;
        }
        assert!(Instant::now() < deadline, "{status}\n{watch}");
        std::thread::sleep(Duration::from_millis(25));
    }
    fixture
        .tool(json!({"Action":"verify"}))
        .assert_success("verify running installation without mutating it");
    assert_eq!(fixture.management(&["shutdown"])["ok"], true);
    let deadline = Instant::now() + Duration::from_secs(15);
    while host.0.try_wait().unwrap().is_none() {
        assert!(
            Instant::now() < deadline,
            "shutdown must exit the exact owner"
        );
        std::thread::sleep(Duration::from_millis(25));
    }
    assert_eq!(
        fixture.management(&["status"])["error"]["code"],
        "runtime-not-running"
    );
    assert!(fixture.registry.exists());
    assert!(fixture.record_path().exists());
}
