#![cfg(feature = "postgres")]

use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};
use telex::profiles::{pg_connect_config, BackendProfile};

#[path = "fixtures/credential_command_helpers.rs"]
mod credential_helpers;

fn profile(command: String) -> BackendProfile {
    BackendProfile {
        kind: "postgres".into(),
        path: None,
        url: Some("host=127.0.0.1 port=1 user=test dbname=test".into()),
        auth: Some("password".into()),
        password_env: None,
        password_command: Some(command),
        schema: None,
        entra_cred: None,
        entra_scope: None,
    }
}

fn delayed_command(root: &Path, source: &str) -> String {
    let ready = root.join(format!("{source}-ready"));
    let release = root.join("release");
    #[cfg(windows)]
    {
        credential_helpers::encoded_powershell(&format!(
            "[IO.File]::WriteAllText('{}', 'ready'); \
             while (-not (Test-Path -LiteralPath '{}')) {{ Start-Sleep -Milliseconds 10 }}; \
             [Console]::Out.Write('non-secret-test-placeholder')",
            ready.to_string_lossy().replace('\'', "''"),
            release.to_string_lossy().replace('\'', "''"),
        ))
    }
    #[cfg(unix)]
    {
        format!(
            "printf ready > '{}'; while test ! -f '{}'; do sleep 0.01; done; \
             printf non-secret-test-placeholder",
            ready.to_string_lossy().replace('\'', "'\\''"),
            release.to_string_lossy().replace('\'', "'\\''"),
        )
    }
}

#[test]
#[ignore = "owned subprocess entrypoint invoked by lifecycle tests"]
fn credential_host_fixture() {
    let root = PathBuf::from(std::env::var_os("TELEX_CREDENTIAL_FIXTURE_ROOT").unwrap());
    let mode = std::env::var("TELEX_CREDENTIAL_FIXTURE_MODE").unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let p = profile(delayed_command(&root, "first"));
    let second = profile(delayed_command(&root, "second"));
    runtime.block_on(async {
        let two = mode == "host-exit-two";
        let mut acquisition = Box::pin(async {
            if two {
                let _ = tokio::join!(pg_connect_config(&p), pg_connect_config(&second));
            } else {
                let _ = pg_connect_config(&p).await;
            }
        });
        let ready_deadline = Instant::now() + Duration::from_secs(4);
        loop {
            tokio::select! {
                _ = &mut acquisition => panic!("fixture command completed before cancellation"),
                _ = tokio::time::sleep(Duration::from_millis(5)) => {
                    if root.join("first-ready").exists()
                        && (!two || root.join("second-ready").exists()) {
                        break;
                    }
                    assert!(Instant::now() < ready_deadline, "credential helper readiness expired");
                }
            }
        }
        if two {
            assert_eq!(telex::profiles::credential_command_obligations().len(), 2);
            fs::write(root.join("two-admitted"), "2").unwrap();
        }
        assert!(
            tokio::time::timeout(Duration::from_millis(100), &mut acquisition)
                .await
                .is_err()
        );
        drop(acquisition);
    });
    fs::write(root.join("logical-completion"), "timeout").unwrap();
    drop(runtime);
    if mode == "runtime-drop" {
        let deadline = Instant::now() + Duration::from_millis(3_500);
        while !telex::profiles::credential_command_obligations().is_empty() {
            assert!(
                Instant::now() < deadline,
                "native owner did not survive runtime drop"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        fs::write(root.join("native-receipt"), "joined").unwrap();
    }
    telex::profiles::drain_password_commands_before_exit();
    fs::write(root.join("host-drained"), "complete").unwrap();
    std::process::exit(2);
}

struct OwnedHost {
    child: Child,
    root: PathBuf,
}

impl OwnedHost {
    fn spawn(mode: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "telex-credential-host-{mode}-{}-{}",
            std::process::id(),
            telex::model::now_ms(),
        ));
        fs::create_dir(&root).unwrap();
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "credential_host_fixture",
                "--ignored",
                "--nocapture",
            ])
            .env("TELEX_CREDENTIAL_FIXTURE_ROOT", &root)
            .env("TELEX_CREDENTIAL_FIXTURE_MODE", mode)
            .env("TELEX_HOME", root.join("home"))
            .env("TELEX_DB", root.join("unused.db"))
            .env("TELEX_CONFIG", root.join("config.toml"))
            .env("TELEX_INSTALL_ROOT", root.join("install"))
            .stdin(Stdio::null())
            .stdout(File::create(root.join("stdout")).unwrap())
            .stderr(File::create(root.join("stderr")).unwrap());
        Self {
            child: command.spawn().unwrap(),
            root,
        }
    }

    fn wait_for(&mut self, name: &str) -> Instant {
        let deadline = Instant::now() + Duration::from_secs(8);
        loop {
            if self.root.join(name).exists() {
                return Instant::now();
            }
            assert!(
                self.child.try_wait().unwrap().is_none(),
                "host exited before {name}: {}",
                fs::read_to_string(self.root.join("stderr")).unwrap()
            );
            assert!(Instant::now() < deadline, "host did not reach {name}");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn verify_exit(&mut self, logical_completion: Instant) {
        let deadline = logical_completion + Duration::from_millis(3_500);
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                assert_eq!(
                    status.code(),
                    Some(2),
                    "{}",
                    fs::read_to_string(self.root.join("stderr")).unwrap()
                );
                assert!(self.root.join("host-drained").exists());
                assert!(Instant::now() <= deadline);
                return;
            }
            assert!(
                Instant::now() < deadline,
                "normal host drain exceeded one cleanup budget"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

impl Drop for OwnedHost {
    fn drop(&mut self) {
        if self.child.try_wait().unwrap().is_none() {
            fs::write(self.root.join("release"), "fixture rescue").unwrap();
            let deadline = Instant::now() + Duration::from_secs(4);
            while self.child.try_wait().unwrap().is_none() && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(10));
            }
        }
        if self.child.try_wait().unwrap().is_none() {
            // This is the Child owned by this fixture, never a recovered numeric PID.
            self.child.kill().unwrap();
            self.child.wait().unwrap();
        }
        if !std::thread::panicking() {
            fs::remove_dir_all(&self.root).unwrap();
        }
    }
}

#[test]
fn native_credential_owner_finishes_after_tokio_runtime_drop() {
    let mut host = OwnedHost::spawn("runtime-drop");
    let logical_completion = host.wait_for("logical-completion");
    host.verify_exit(logical_completion);
    assert!(host.root.join("native-receipt").exists());
}

#[test]
fn owning_host_drains_before_actual_process_exit() {
    let mut host = OwnedHost::spawn("host-exit");
    let logical_completion = host.wait_for("logical-completion");
    host.verify_exit(logical_completion);
}

#[test]
fn owning_host_drains_two_sources_under_one_absolute_budget() {
    let mut host = OwnedHost::spawn("host-exit-two");
    let logical_completion = host.wait_for("logical-completion");
    host.verify_exit(logical_completion);
    assert!(host.root.join("two-admitted").exists());
}

#[test]
fn actual_cli_ownerless_exit_preserves_version_outcome() {
    let bin = Path::new(env!("CARGO_BIN_EXE_telex"));
    assert!(bin.is_absolute());
    let root = std::env::temp_dir().join(format!(
        "telex-credential-ownerless-{}-{}",
        std::process::id(),
        telex::model::now_ms(),
    ));
    fs::create_dir(&root).unwrap();
    let output = Command::new(bin)
        .args(["--json", "version"])
        .env("TELEX_HOME", root.join("home"))
        .env("TELEX_DB", root.join("unused.db"))
        .env("TELEX_CONFIG", root.join("config.toml"))
        .env("TELEX_INSTALL_ROOT", root.join("install"))
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(serde_json::from_slice::<serde_json::Value>(&output.stdout).is_ok());
    fs::remove_dir_all(root).unwrap();
}
