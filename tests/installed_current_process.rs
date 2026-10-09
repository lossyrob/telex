#![cfg(feature = "sqlite")]
//! Real-process proofs for the `InstalledCurrent` Application Client bootstrap.
//!
//! Every test builds a unique, owner-private install root from the *branch*
//! binary (absolute `CARGO_BIN_EXE_telex`), points
//! `ApplicationDaemonBootstrap::InstalledCurrent` at it, and observes real
//! daemon processes. Installed/user state is never targeted.
//!
//! These tests mutate process-global `TELEX_*` selection (the daemon rendezvous
//! is derived from it), so each holds the shared environment lock for its whole
//! duration and restores the previous environment afterwards.
//!
//! Coverage:
//! - current-version spawn + matching prestarted reuse
//! - concurrent first use
//! - crash / restart / reattach
//! - upgrade and rollback `current` selector behavior
//! - selector movement at resolution and spawn boundaries
//! - hard-killed selector client does not wedge the selector
//! - stale prestarted image
//! - PID-reuse evidence in the runtime capability record
//! - symlink / reparse-point authority escape
//! - unsafe (foreign-writable) authority
//! - incompatible manifest, build, protocol, and capability metadata
//! - platform file-identity mismatch (pinned exact target)
//! - foreign / hostile pre-bound endpoint
//! - spawned-child readiness refusal without a valid selection

#[path = "support/telex_isolation.rs"]
mod isolation;

use std::collections::BTreeMap;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::Duration;

use telex::application_client::{
    AddressSpec, ApplicationCapability, ApplicationClient, ApplicationClientConfig,
    ApplicationClientError, ApplicationDaemonBootstrap, ApplicationResponsibility,
    DaemonBootstrapFailure, RecoveryPolicy,
};
use telex::install::{self, VersionManifest};
use telex::model::now_ms;
use telex::profiles::ConfigFile;

use isolation::{Isolation, ENV_LOCK};

/// Hidden child-side selection handoff. Reproduced here as a *test-only*
/// constant so a spawned daemon can be driven into readiness refusal; the
/// production constant stays crate-private.
const BOOTSTRAP_TOKEN_ENV: &str = "TELEX_DAEMON_SELECTION_TOKEN";
static PROCESS_PROFILE_LOCK: Mutex<()> = Mutex::new(());

fn config(responsibility: &str, db: &Path) -> ApplicationClientConfig {
    if let Ok(url) = std::env::var("TELEX_PROCESS_PG_URL") {
        if !url.trim().is_empty() {
            let _guard = PROCESS_PROFILE_LOCK.lock().unwrap();
            let profile_name = "installed_current_process_pg";
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            db.hash(&mut hasher);
            let schema = format!("telex_ic_{:016x}", hasher.finish());
            let mut profile = telex::profiles::implicit_sqlite(None);
            profile.kind = "postgres".to_string();
            profile.path = None;
            profile.url = Some(url);
            profile.schema = Some(schema);
            profile.auth = Some("password".to_string());
            if std::env::var("TELEX_PG_PASSWORD").is_ok() {
                profile.password_env = Some("TELEX_PG_PASSWORD".to_string());
            }
            let config_path =
                PathBuf::from(std::env::var_os("TELEX_CONFIG").expect("isolated TELEX_CONFIG"));
            std::fs::write(
                config_path,
                toml::to_string_pretty(&ConfigFile {
                    default: None,
                    backends: BTreeMap::from([(profile_name.to_string(), profile)]),
                })
                .expect("serialize process Postgres profile"),
            )
            .expect("write process Postgres profile");
            return ApplicationClientConfig {
                responsibility: ApplicationResponsibility(responsibility.to_string()),
                backend: Some(profile_name.to_string()),
                db_override: None,
            };
        }
    }
    let require_postgres = std::env::var("TELEX_PG_REQUIRE")
        .is_ok_and(|value| value == "1" || value.eq_ignore_ascii_case("true"));
    assert!(
        !require_postgres,
        "TELEX_PG_REQUIRE is set but TELEX_PROCESS_PG_URL is unset/empty; \
         refusing to run InstalledCurrent process proofs on SQLite"
    );
    ApplicationClientConfig {
        responsibility: ApplicationResponsibility(responsibility.to_string()),
        backend: None,
        db_override: Some(db.to_string_lossy().into_owned()),
    }
}

fn installed_current(iso: &Isolation) -> ApplicationDaemonBootstrap {
    ApplicationDaemonBootstrap::InstalledCurrent {
        trusted_root: iso.trusted_root(),
    }
}

fn spec(address: &str, capability: ApplicationCapability) -> AddressSpec {
    AddressSpec {
        address: address.to_string(),
        capability,
        description: Some("installed-current process proof".to_string()),
        scope: None,
        tags: None,
    }
}

async fn connect(iso: &Isolation, responsibility: &str, db: &Path) -> ApplicationClient {
    ApplicationClient::connect_with_daemon(config(responsibility, db), installed_current(iso))
        .await
        .unwrap_or_else(|e| panic!("connect {responsibility}: {e}"))
}

fn bootstrap_failure(error: &ApplicationClientError) -> DaemonBootstrapFailure {
    match error {
        ApplicationClientError::DaemonBootstrap(failure) => *failure,
        other => panic!("expected a typed daemon bootstrap failure, got {other:?}"),
    }
}

fn attach_failure(
    outcome: &telex::application_client::MultiAddressOutcome,
) -> DaemonBootstrapFailure {
    let error = outcome
        .results
        .values()
        .find_map(|result| match result {
            telex::application_client::AddressLifecycleResult::Failed(error) => Some(error),
            _ => None,
        })
        .unwrap_or_else(|| panic!("expected a failed address, got {outcome:?}"));
    bootstrap_failure(error)
}

fn write_manifest(iso: &Isolation, tag: &str, mutate: impl FnOnce(&mut VersionManifest)) {
    let layout = iso.layout();
    let mut manifest = install::read_manifest(&layout, tag).expect("read manifest");
    mutate(&mut manifest);
    let path = layout.versions_dir.join(tag).join("manifest.json");
    std::fs::write(
        &path,
        serde_json::to_string_pretty(&manifest).expect("serialize manifest"),
    )
    .expect("write manifest");
}

fn remove_manifest_admission_field(iso: &Isolation, tag: &str) -> PathBuf {
    let path = iso.layout().versions_dir.join(tag).join("manifest.json");
    let mut value: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).expect("read manifest JSON"))
            .expect("parse manifest JSON");
    value
        .as_object_mut()
        .expect("manifest object")
        .remove("application_bootstrap_admission_version")
        .expect("manifest admission field");
    std::fs::write(
        &path,
        serde_json::to_vec_pretty(&value).expect("serialize v0.2.1-shaped manifest"),
    )
    .expect("write v0.2.1-shaped manifest");
    path
}

#[cfg(unix)]
fn platform_file_identity(path: &Path) -> (u64, u64) {
    use std::os::unix::fs::MetadataExt;
    let metadata = std::fs::metadata(path).expect("manifest metadata");
    (metadata.dev(), metadata.ino())
}

#[cfg(windows)]
fn platform_file_identity(path: &Path) -> (u64, u64) {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Foundation::{CloseHandle, GENERIC_READ, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION, FILE_SHARE_READ,
        OPEN_EXISTING,
    };
    let wide: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let handle = unsafe {
        CreateFileW(
            wide.as_ptr(),
            GENERIC_READ,
            FILE_SHARE_READ,
            std::ptr::null(),
            OPEN_EXISTING,
            0,
            0,
        )
    };
    assert_ne!(handle, INVALID_HANDLE_VALUE, "open manifest identity");
    let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
    assert_ne!(
        unsafe { GetFileInformationByHandle(handle, &mut info) },
        0,
        "read manifest identity"
    );
    unsafe {
        CloseHandle(handle);
    }
    (
        u64::from(info.dwVolumeSerialNumber),
        (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow),
    )
}

fn build_public_fixture() -> PathBuf {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("application-client-consumer");
    let mut command = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string()));
    command
        .arg("build")
        .arg("--manifest-path")
        .arg(fixture.join("Cargo.toml"))
        .arg("--no-default-features")
        .arg("--features")
        .arg(if process_postgres_enabled() {
            "postgres"
        } else {
            "sqlite"
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    isolation::run_with_timeout(command, Duration::from_secs(900))
        .assert_success("build public selector-client fixture");
    fixture.join("target").join("debug").join(format!(
        "telex-application-client-consumer{}",
        std::env::consts::EXE_SUFFIX
    ))
}

fn process_postgres_enabled() -> bool {
    std::env::var("TELEX_PROCESS_PG_URL").is_ok_and(|url| !url.trim().is_empty())
}

fn write_fixture_profile(iso: &Isolation, profile_name: &str) {
    let db = iso.root.join("selector-client.db");
    let mut profile = telex::profiles::implicit_sqlite(None);
    if let Ok(url) = std::env::var("TELEX_PROCESS_PG_URL") {
        if !url.trim().is_empty() {
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            db.hash(&mut hasher);
            profile.kind = "postgres".to_string();
            profile.path = None;
            profile.url = Some(url);
            profile.schema = Some(format!("telex_ic_fixture_{:016x}", hasher.finish()));
            profile.auth = Some("password".to_string());
            if std::env::var("TELEX_PG_PASSWORD").is_ok() {
                profile.password_env = Some("TELEX_PG_PASSWORD".to_string());
            }
        } else {
            profile.path = Some(db.to_string_lossy().into_owned());
        }
    } else {
        profile.path = Some(db.to_string_lossy().into_owned());
    }
    iso.write_config(&ConfigFile {
        default: None,
        backends: BTreeMap::from([(profile_name.to_string(), profile)]),
    });
}

// ----------------------------------------------------------------------------------------
// Spawn, reuse, concurrency, crash recovery
// ----------------------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn installed_current_spawns_selected_version_and_reuses_matching_prestarted_daemon() {
    let _env = ENV_LOCK.lock().await;
    let iso = Isolation::new("ic-spawn-reuse");
    let _restore = iso.apply_env();
    let db = iso.root.join("spawn.db");

    assert!(iso.cap_path().is_none(), "the environment starts cold");
    let first = connect(&iso, "proof", &db).await;
    let outcome = first
        .attach(&[spec("ic:spawn:a", ApplicationCapability::Bidirectional)])
        .await;
    assert!(outcome.ready, "cold-start attach must spawn: {outcome:?}");
    let pid = iso
        .daemon_pid()
        .expect("the spawned daemon records its pid");

    // A second client must *reuse* the matching prestarted daemon rather than
    // spawning a second one.
    let second = connect(&iso, "proof", &db).await;
    let outcome = second
        .attach(&[spec("ic:spawn:b", ApplicationCapability::SendOnly)])
        .await;
    assert!(outcome.ready, "reuse attach must succeed: {outcome:?}");
    assert_eq!(
        iso.daemon_pid().expect("cap still present"),
        pid,
        "a matching prestarted daemon must be reused, not replaced"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn installed_current_concurrent_first_use_converges_on_one_daemon() {
    let _env = ENV_LOCK.lock().await;
    let iso = Isolation::new("ic-concurrent");
    let _restore = iso.apply_env();
    let db = iso.root.join("concurrent.db");
    let initialized = connect(&iso, "proof-initializer", &db).await;
    drop(initialized);

    let mut tasks = Vec::new();
    for index in 0..4 {
        let root = iso.trusted_root();
        let db = db.clone();
        tasks.push(tokio::spawn(async move {
            let client_config = config(&format!("proof-{index}"), &db);
            let client = ApplicationClient::connect_with_daemon(
                client_config,
                ApplicationDaemonBootstrap::InstalledCurrent { trusted_root: root },
            )
            .await
            .expect("concurrent connect");
            client
                .attach(&[spec(
                    &format!("ic:concurrent:{index}"),
                    ApplicationCapability::Bidirectional,
                )])
                .await
                .ready
        }));
    }
    for task in tasks {
        assert!(
            task.await.expect("concurrent task"),
            "every concurrent first use must attach"
        );
    }

    let caps: Vec<_> = std::fs::read_dir(&iso.run_dir)
        .expect("read run dir")
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name().to_string_lossy().ends_with(".cap"))
        .collect();
    assert_eq!(
        caps.len(),
        1,
        "concurrent first use must converge on exactly one daemon singleton"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn installed_current_crash_restart_and_reattach_recovers_membership() {
    let _env = ENV_LOCK.lock().await;
    let iso = Isolation::new("ic-crash");
    let _restore = iso.apply_env();
    let db = iso.root.join("crash.db");

    let client = connect(&iso, "proof", &db).await;
    let station = spec("ic:crash:station", ApplicationCapability::Bidirectional);
    assert!(client.attach(std::slice::from_ref(&station)).await.ready);
    let pid = iso.daemon_pid().expect("daemon pid");

    // Hard-kill the daemon: no drain, no graceful release.
    terminate_pid(pid);
    wait_until_gone(&iso, pid, Duration::from_secs(15));

    // The next spawning call restarts the daemon, and bounded repair recovers
    // the membership.
    let outcome = client
        .reconcile_many(
            std::slice::from_ref(&station),
            RecoveryPolicy::BoundedRepair { retries: 4 },
        )
        .await;
    assert!(
        outcome.ready,
        "reattach after a crash must recover: {outcome:?}"
    );
    let restarted = iso.daemon_pid().expect("restarted daemon pid");
    assert_ne!(restarted, pid, "a crashed daemon must be replaced");
}

// ----------------------------------------------------------------------------------------
// Selector movement, upgrade/rollback, stale image
// ----------------------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn installed_current_upgrade_and_rollback_move_the_selector() {
    let _env = ENV_LOCK.lock().await;
    let iso = Isolation::new("ic-upgrade");
    let mut restore = iso.apply_env();
    restore.set("TELEX_TEST_RECOVERABLE_INTENTS", "1");
    let db = iso.root.join("upgrade.db");
    let first_tag = iso.tag.clone();

    let client = connect(&iso, "proof", &db).await;
    assert!(
        client
            .attach(&[spec("ic:upgrade:a", ApplicationCapability::SendOnly)])
            .await
            .ready
    );
    let first_pid = iso.daemon_pid().expect("first selected daemon pid");

    // Exercise the production upgrade path: it takes exclusive selector
    // admission, drains the predecessor, validates the candidate, and publishes
    // `current` atomically.
    let second_tag = format!("{first_tag}-next");
    let source = iso.current_binary();
    let upgraded = iso.run_cli(
        [
            "--json",
            "upgrade",
            "--from",
            &source.to_string_lossy(),
            "--version",
            &second_tag,
            "--force",
        ],
        Duration::from_secs(90),
    );
    upgraded.assert_success("coordinated installed-current upgrade");
    assert_eq!(
        install::version_info(Some(iso.install_root.clone()))
            .expect("version info after upgrade")
            .install
            .current_tag
            .as_deref(),
        Some(second_tag.as_str())
    );
    let upgraded_client = connect(&iso, "proof", &db).await;
    let outcome = upgraded_client
        .attach(&[spec("ic:upgrade:c", ApplicationCapability::SendOnly)])
        .await;
    assert!(
        outcome.ready,
        "the upgraded selection must serve: {outcome:?}"
    );
    assert_ne!(
        iso.daemon_pid().expect("successor daemon pid"),
        first_pid,
        "upgrade must drain and replace the selected daemon"
    );

    // Exercise the production rollback path under the same exclusive admission.
    let successor_pid = iso.daemon_pid().expect("successor daemon pid");
    let rolled_back = iso.run_cli(
        ["--json", "rollback", "--version", &first_tag],
        Duration::from_secs(90),
    );
    rolled_back.assert_success("coordinated installed-current rollback");
    assert_eq!(
        install::version_info(Some(iso.install_root.clone()))
            .expect("version info after rollback")
            .install
            .current_tag
            .as_deref(),
        Some(first_tag.as_str())
    );
    let after_rollback = connect(&iso, "proof", &db).await;
    assert!(
        after_rollback
            .attach(&[spec("ic:upgrade:e", ApplicationCapability::SendOnly)])
            .await
            .ready,
        "the rolled-back selection must serve"
    );
    assert_ne!(
        iso.daemon_pid().expect("rolled-back daemon pid"),
        successor_pid,
        "rollback must drain and replace the selected daemon"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn installed_current_selector_movement_fails_closed_at_resolution_and_spawn() {
    let _env = ENV_LOCK.lock().await;
    let iso = Isolation::new("ic-movement");
    let _restore = iso.apply_env();
    let db = iso.root.join("movement.db");
    let layout = iso.layout();
    let tag = iso.tag.clone();

    let client = connect(&iso, "proof", &db).await;
    assert!(
        client
            .attach(&[spec("ic:move:a", ApplicationCapability::SendOnly)])
            .await
            .ready
    );
    iso.stop_daemon();

    // The selector disappears between calls: resolution fails closed.
    std::fs::remove_file(&layout.current_path).expect("remove current selector");
    let outcome = client
        .attach(&[spec("ic:move:b", ApplicationCapability::SendOnly)])
        .await;
    assert_eq!(
        attach_failure(&outcome),
        DaemonBootstrapFailure::MissingCurrent
    );

    // The selector is replaced with a path-escaping value: refused as invalid,
    // never followed out of the trusted root.
    std::fs::write(&layout.current_path, "../escape").expect("write escaping selector");
    let outcome = client
        .attach(&[spec("ic:move:c", ApplicationCapability::SendOnly)])
        .await;
    assert_eq!(
        attach_failure(&outcome),
        DaemonBootstrapFailure::InvalidManifest
    );

    // The selector points at a version that was never installed: the authority
    // chain below the trusted root cannot be validated, so it fails closed.
    std::fs::write(&layout.current_path, "v-not-installed").expect("write unknown selector");
    let outcome = client
        .attach(&[spec("ic:move:d", ApplicationCapability::SendOnly)])
        .await;
    assert!(
        matches!(
            attach_failure(&outcome),
            DaemonBootstrapFailure::InvalidTrustedRoot
                | DaemonBootstrapFailure::UnsafeInstallAuthority
                | DaemonBootstrapFailure::InvalidManifest
                | DaemonBootstrapFailure::MissingExecutable
        ),
        "an uninstalled selector must fail closed: {outcome:?}"
    );

    // Restoring the selector restores service; movement is recoverable, not
    // permanently poisoning.
    std::fs::write(&layout.current_path, &tag).expect("restore selector");
    assert!(
        client
            .attach(&[spec("ic:move:e", ApplicationCapability::SendOnly)])
            .await
            .ready,
        "a restored selector must serve again"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn installed_current_frozen_root_identity_rejects_path_replacement() {
    let _env = ENV_LOCK.lock().await;
    let iso = Isolation::new("ic-root-identity");
    let _restore = iso.apply_env();
    let db = iso.root.join("root-identity.db");
    let client = connect(&iso, "proof", &db).await;

    let displaced = iso.root.join("displaced-install");
    std::fs::rename(&iso.install_root, &displaced).expect("move the frozen install root");
    isolation::create_owner_private_dir(&iso.install_root);
    iso.install_tag(&iso.tag, true);

    let outcome = client
        .attach(&[spec("ic:root-identity:a", ApplicationCapability::SendOnly)])
        .await;
    assert_eq!(
        attach_failure(&outcome),
        DaemonBootstrapFailure::SelectionUnstable,
        "a replacement at the same canonical root path must not inherit trust"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn installed_current_hard_killed_daemon_does_not_wedge_the_selector() {
    let _env = ENV_LOCK.lock().await;
    let iso = Isolation::new("ic-wedge");
    let _restore = iso.apply_env();
    let db = iso.root.join("wedge.db");

    let client = connect(&iso, "proof", &db).await;
    assert!(
        client
            .attach(&[spec("ic:wedge:a", ApplicationCapability::SendOnly)])
            .await
            .ready
    );
    let pid = iso.daemon_pid().expect("daemon pid");
    terminate_pid(pid);
    wait_until_gone(&iso, pid, Duration::from_secs(15));

    // Upgrade takes exclusive selector admission. A hard-killed child must not
    // leave its readiness admission wedged after process death.
    let next_tag = format!("{}-after-kill", iso.tag);
    let output = iso.run_cli(
        [
            "--json",
            "upgrade",
            "--from",
            &iso.current_binary().to_string_lossy(),
            "--version",
            &next_tag,
            "--skip-drain",
            "--force",
        ],
        Duration::from_secs(90),
    );
    output.assert_success("upgrade after a hard-killed selector client");
    assert_eq!(
        install::read_manifest(&iso.layout(), &next_tag)
            .expect("post-upgrade manifest")
            .tag,
        next_tag
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn installed_current_killed_selector_client_releases_shared_admission() {
    let _env = ENV_LOCK.lock().await;
    let iso = Isolation::new("ic-client-death");
    let _restore = iso.apply_env();
    let fixture = build_public_fixture();
    let profile = "selector_client";
    write_fixture_profile(&iso, profile);
    let delayed_marker = iso.root.join("hello-delayed");
    let admission_marker = iso.root.join("parent-admission-held");

    let mut command = iso.command_for(&fixture);
    command
        .env("TELEX_FIXTURE_TRUSTED_ROOT", iso.trusted_root())
        .env("TELEX_FIXTURE_BACKEND", profile)
        .env("TELEX_FIXTURE_RUN_ID", "selector-client-death")
        .env("TELEX_TEST_HELLO_ACK_DELAY_MS", "30000")
        .env("TELEX_TEST_HELLO_ACK_DELAY_MARKER", &delayed_marker)
        .env("TELEX_TEST_PARENT_ADMISSION_MARKER", &admission_marker)
        .arg("watcher")
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut client = command.spawn().expect("spawn public selector client");

    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    while (!delayed_marker.is_file() || !admission_marker.is_file())
        && std::time::Instant::now() < deadline
    {
        assert!(
            client.try_wait().expect("poll selector client").is_none(),
            "selector client exited before reaching the delayed HelloAck"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(
        delayed_marker.is_file(),
        "spawned daemon must enter the delayed HelloAck while the parent holds admission"
    );
    assert!(
        admission_marker.is_file(),
        "selector client must report its live parent admission"
    );
    assert!(
        client
            .try_wait()
            .expect("poll delayed selector client")
            .is_none(),
        "selector client must still be waiting for the delayed HelloAck"
    );

    let mut rollback = iso.command();
    rollback
        .args(["--json", "rollback", "--version", &iso.tag, "--skip-drain"])
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut exclusive = rollback.spawn().expect("start exclusive rollback waiter");
    std::thread::sleep(Duration::from_millis(500));
    assert!(
        exclusive
            .try_wait()
            .expect("poll exclusive rollback waiter")
            .is_none(),
        "exclusive admission must remain blocked while the selector client holds shared admission"
    );

    client.kill().expect("kill selector client");
    let _ = client.wait();

    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    let status = loop {
        if let Some(status) = exclusive.try_wait().expect("poll released rollback waiter") {
            break status;
        }
        if std::time::Instant::now() >= deadline {
            exclusive.kill().expect("kill wedged rollback waiter");
            panic!("exclusive admission did not recover after selector client death");
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    assert!(
        status.success(),
        "exclusive admission must complete after selector client death: {status}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn installed_current_gc_obeys_exclusive_selector_admission() {
    let _env = ENV_LOCK.lock().await;
    let iso = Isolation::new("ic-gc-admission");
    let _restore = iso.apply_env();
    let fixture = build_public_fixture();
    let profile = "gc_admission";
    write_fixture_profile(&iso, profile);
    let stale_tag = format!("{}-stale", iso.tag);
    iso.install_tag(&stale_tag, false);
    let delayed_marker = iso.root.join("gc-hello-delayed");
    let admission_marker = iso.root.join("gc-parent-admission");

    let mut command = iso.command_for(&fixture);
    command
        .env("TELEX_FIXTURE_TRUSTED_ROOT", iso.trusted_root())
        .env("TELEX_FIXTURE_BACKEND", profile)
        .env("TELEX_FIXTURE_RUN_ID", "gc-admission")
        .env("TELEX_TEST_HELLO_ACK_DELAY_MS", "30000")
        .env("TELEX_TEST_HELLO_ACK_DELAY_MARKER", &delayed_marker)
        .env("TELEX_TEST_PARENT_ADMISSION_MARKER", &admission_marker)
        .arg("watcher")
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut client = command.spawn().expect("spawn selector client");
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    while (!delayed_marker.is_file() || !admission_marker.is_file())
        && std::time::Instant::now() < deadline
    {
        assert!(
            client.try_wait().expect("poll selector client").is_none(),
            "selector client exited before holding admission"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(delayed_marker.is_file() && admission_marker.is_file());

    let mut gc = iso.command();
    gc.args(["--json", "gc"])
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut exclusive_gc = gc.spawn().expect("start gc exclusive waiter");
    std::thread::sleep(Duration::from_millis(300));
    assert!(
        exclusive_gc.try_wait().expect("poll blocked gc").is_none(),
        "gc must not snapshot/delete versions while shared selection admission is live"
    );

    client.kill().expect("kill selector client");
    let _ = client.wait();
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    let status = loop {
        if let Some(status) = exclusive_gc.try_wait().expect("poll gc") {
            break status;
        }
        if std::time::Instant::now() >= deadline {
            exclusive_gc.kill().expect("kill wedged gc");
            panic!("gc did not recover after shared admission released");
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    assert!(status.success(), "gc must complete after admission release");
    assert!(
        !iso.layout().versions_dir.join(stale_tag).exists(),
        "gc must remove only the stale version after taking its protected selector snapshot"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn installed_current_child_lease_survives_parent_death_until_readiness() {
    let _env = ENV_LOCK.lock().await;
    let iso = Isolation::new("ic-child-lease");
    let _restore = iso.apply_env();
    let fixture = build_public_fixture();
    let profile = "child_lease";
    write_fixture_profile(&iso, profile);
    let prior_tag = format!("{}-prior", iso.tag);
    iso.install_tag(&prior_tag, false);
    let child_marker = iso.root.join("child-admission-held");

    let mut command = iso.command_for(&fixture);
    command
        .env("TELEX_FIXTURE_TRUSTED_ROOT", iso.trusted_root())
        .env("TELEX_FIXTURE_BACKEND", profile)
        .env("TELEX_FIXTURE_RUN_ID", "child-lease-parent-death")
        .env("TELEX_TEST_CHILD_ADMISSION_HELD_DELAY_MS", "2000")
        .env("TELEX_TEST_CHILD_ADMISSION_HELD_MARKER", &child_marker)
        .arg("watcher")
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut parent = command.spawn().expect("spawn public selector client");

    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    while !child_marker.is_file() && std::time::Instant::now() < deadline {
        assert!(
            parent.try_wait().expect("poll selector parent").is_none(),
            "selector parent exited before the child acquired admission"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(
        child_marker.is_file(),
        "spawned child must report independently held selector admission"
    );

    parent.kill().expect("kill selector parent");
    let _ = parent.wait();

    let mut rollback = iso.command();
    rollback
        .args(["--json", "rollback", "--version", &prior_tag])
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut exclusive = rollback
        .spawn()
        .expect("start rollback while child admission is held");
    std::thread::sleep(Duration::from_millis(300));
    assert!(
        exclusive
            .try_wait()
            .expect("poll rollback blocked by child")
            .is_none(),
        "parent death must not release the child's readiness admission"
    );

    let deadline = std::time::Instant::now() + Duration::from_secs(90);
    let status = loop {
        if let Some(status) = exclusive.try_wait().expect("poll rollback completion") {
            break status;
        }
        if std::time::Instant::now() >= deadline {
            exclusive.kill().expect("kill wedged rollback");
            panic!("rollback did not proceed after child readiness publication");
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    assert!(
        status.success(),
        "exclusive rollback must complete after the child publishes readiness: {status}"
    );
    assert_eq!(
        std::fs::read_to_string(iso.layout().current_path)
            .expect("read current selector")
            .trim(),
        prior_tag
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn installed_current_existing_peer_handshake_is_bounded() {
    let _env = ENV_LOCK.lock().await;
    let iso = Isolation::new("ic-existing-timeout");
    let _restore = iso.apply_env();
    let fixture = build_public_fixture();
    let profile = "existing_timeout";
    write_fixture_profile(&iso, profile);
    let delayed_marker = iso.root.join("existing-hello-delayed");

    let mut starter_command = iso.command_for(&fixture);
    starter_command
        .env("TELEX_FIXTURE_TRUSTED_ROOT", iso.trusted_root())
        .env("TELEX_FIXTURE_BACKEND", profile)
        .env("TELEX_FIXTURE_RUN_ID", "existing-timeout-starter")
        .env("TELEX_TEST_HELLO_ACK_DELAY_MS", "30000")
        .env("TELEX_TEST_HELLO_ACK_DELAY_MARKER", &delayed_marker)
        .arg("watcher")
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut starter = starter_command
        .spawn()
        .expect("start token-backed installed daemon");
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    while (iso.cap_path().is_none() || !delayed_marker.is_file())
        && std::time::Instant::now() < deadline
    {
        assert!(
            starter.try_wait().expect("poll starter client").is_none(),
            "starter client exited before the daemon published admitted capability"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(
        iso.cap_path().is_some() && delayed_marker.is_file(),
        "token-backed daemon must publish capability and delay HelloAck"
    );
    starter.kill().expect("stop starter client");
    let _ = starter.wait();
    std::fs::remove_file(&delayed_marker).expect("reset delayed-Hello marker");

    let mut probe = iso.command_for(&fixture);
    probe
        .env("TELEX_FIXTURE_TRUSTED_ROOT", iso.trusted_root())
        .env("TELEX_FIXTURE_BACKEND", profile)
        .env("TELEX_FIXTURE_RUN_ID", "existing-timeout")
        .arg("health");
    let started = std::time::Instant::now();
    let output = isolation::run_with_timeout(probe, Duration::from_secs(10));
    let elapsed = started.elapsed();
    output.assert_failure("existing-daemon delayed HelloAck");
    assert!(
        !output.timed_out,
        "the fixture process must return its bounded handshake failure"
    );
    assert!(
        elapsed < Duration::from_secs(5),
        "existing-peer handshake held admission too long: {elapsed:?}"
    );
    assert!(
        delayed_marker.is_file(),
        "the matching daemon must have accepted the connection and delayed HelloAck"
    );
    assert!(
        output.stderr.contains("timeout"),
        "the public health operation must report the bounded handshake timeout: {}",
        output.stderr
    );

    iso.stop_daemon();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn installed_current_prestarted_peer_without_admission_evidence_is_refused_before_hello() {
    let _env = ENV_LOCK.lock().await;
    let iso = Isolation::new("ic-missing-cap-evidence");
    let _restore = iso.apply_env();
    let db = iso.root.join("missing-cap-evidence.db");
    let hello_marker = iso.root.join("unexpected-hello");

    let mut daemon_command = iso.command_for(&iso.current_binary());
    daemon_command
        .env("TELEX_TEST_HELLO_ACK_DELAY_MS", "1")
        .env("TELEX_TEST_HELLO_ACK_DELAY_MARKER", &hello_marker)
        .args(["daemon", "serve"])
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut daemon = daemon_command
        .spawn()
        .expect("start matching path without bootstrap token");
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    while iso.cap_path().is_none() && std::time::Instant::now() < deadline {
        assert!(
            daemon.try_wait().expect("poll manual daemon").is_none(),
            "manual daemon exited before publishing capability"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(iso.cap_path().is_some(), "manual daemon must publish cap");

    let client = connect(&iso, "proof", &db).await;
    let outcome = client
        .attach(&[spec(
            "ic:missing-cap-evidence:a",
            ApplicationCapability::SendOnly,
        )])
        .await;
    assert_eq!(
        attach_failure(&outcome),
        DaemonBootstrapFailure::ForeignDaemon
    );
    assert!(
        !hello_marker.exists(),
        "missing token-backed cap evidence must fail before any Hello reaches the daemon"
    );

    daemon.kill().expect("stop manual daemon");
    let _ = daemon.wait();
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn installed_current_stale_prestarted_image_is_refused() {
    let _env = ENV_LOCK.lock().await;
    let iso = Isolation::new("ic-stale");
    let _restore = iso.apply_env();
    let db = iso.root.join("stale.db");
    let client = connect(&iso, "proof", &db).await;
    assert!(
        client
            .attach(&[spec("ic:stale:a", ApplicationCapability::SendOnly)])
            .await
            .ready
    );

    // Replace the selected target in place. The running daemon keeps the old
    // image, so its platform file identity no longer matches the selection.
    let target = iso.current_binary();
    let source = isolation::branch_binary();
    std::fs::remove_file(&target).expect("unlink the running image");
    std::fs::copy(&source, &target).expect("install a fresh copy at the same path");

    let outcome = client
        .attach(&[spec("ic:stale:b", ApplicationCapability::SendOnly)])
        .await;
    assert_eq!(
        attach_failure(&outcome),
        DaemonBootstrapFailure::ForeignDaemon,
        "a prestarted daemon running a stale image must be refused"
    );
}

#[cfg(windows)]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn installed_current_stale_prestarted_image_is_refused() {
    let _env = ENV_LOCK.lock().await;
    let iso = Isolation::new("ic-stale");
    let _restore = iso.apply_env();
    let db = iso.root.join("stale.db");
    let client = connect(&iso, "proof", &db).await;
    assert!(
        client
            .attach(&[spec("ic:stale:a", ApplicationCapability::SendOnly)])
            .await
            .ready
    );

    // Windows can keep the mapped image alive after its directory entry is
    // renamed. Replace the original pathname with another copy of the same
    // build: canonical path and HelloAck still match, so only the
    // token-backed startup file identity can reject the stale process before
    // Hello.
    let target = iso.current_binary();
    let displaced = target.with_extension("running-image");
    std::fs::rename(&target, &displaced).expect("rename the running image");
    std::fs::copy(isolation::branch_binary(), &target)
        .expect("replace the original executable pathname");

    let outcome = client
        .attach(&[spec("ic:stale:b", ApplicationCapability::SendOnly)])
        .await;
    assert_eq!(
        attach_failure(&outcome),
        DaemonBootstrapFailure::ForeignDaemon,
        "a replacement at the running image pathname must be refused before Hello"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn installed_current_capability_record_carries_pid_reuse_evidence() {
    let _env = ENV_LOCK.lock().await;
    let iso = Isolation::new("ic-pidreuse");
    let _restore = iso.apply_env();
    let db = iso.root.join("pidreuse.db");

    let client = connect(&iso, "proof", &db).await;
    assert!(
        client
            .attach(&[spec("ic:pid:a", ApplicationCapability::SendOnly)])
            .await
            .ready
    );
    let cap = iso.cap_json().expect("capability record");
    let pid = cap.get("server_pid").and_then(|v| v.as_u64());
    let start_time = cap.get("server_start_time").and_then(|v| v.as_u64());
    assert!(
        pid.is_some() && start_time.is_some(),
        "the capability record must carry both pid and start time so a reused \
         pid can never be mistaken for the same daemon: {cap}"
    );
    assert_eq!(
        cap.get("bootstrap_admission_version")
            .and_then(|value| value.as_u64()),
        Some(u64::from(
            telex::install::APPLICATION_BOOTSTRAP_ADMISSION_VERSION
        )),
        "InstalledCurrent child capability must record successful admission"
    );
    assert!(
        cap.get("startup_executable_file_identity").is_some(),
        "InstalledCurrent child capability must bind startup file identity"
    );

    let cap_path = iso
        .cap_path()
        .unwrap_or_else(|| iso.run_dir.join("daemon-tampered.cap"));
    let mut identity_tampered = cap.clone();
    let original_low = identity_tampered["startup_executable_file_identity"]["low"]
        .as_u64()
        .expect("startup identity low");
    identity_tampered["startup_executable_file_identity"]["low"] =
        serde_json::json!(original_low.wrapping_add(1));
    std::fs::write(
        &cap_path,
        serde_json::to_string(&identity_tampered).expect("serialize identity-tampered cap"),
    )
    .expect("write identity-tampered cap");
    let outcome = client
        .attach(&[spec(
            "ic:pid:identity-tamper",
            ApplicationCapability::SendOnly,
        )])
        .await;
    assert_eq!(
        attach_failure(&outcome),
        DaemonBootstrapFailure::ForeignDaemon,
        "cap startup identity mismatch must fail before Hello"
    );
    std::fs::write(
        &cap_path,
        serde_json::to_string(&cap).expect("serialize original cap"),
    )
    .expect("restore original cap");

    // Tamper the record so it names *this* live process with an impossible
    // start time -- a reused pid. The client must not accept it as the daemon;
    // it must fail closed or replace it, never bind to the impostor.
    iso.stop_daemon();
    let mut tampered = cap;
    tampered["server_pid"] = serde_json::json!(std::process::id());
    tampered["server_start_time"] = serde_json::json!(1u64);
    std::fs::write(
        &cap_path,
        serde_json::to_string(&tampered).expect("serialize tampered cap"),
    )
    .expect("write tampered cap");

    let outcome = client
        .attach(&[spec("ic:pid:b", ApplicationCapability::SendOnly)])
        .await;
    assert!(
        outcome.ready,
        "a stale/reused-pid capability record must never be trusted; the client \
         must fall back to spawning the selected daemon: {outcome:?}"
    );
    let fresh = iso.cap_json().expect("fresh capability record");
    assert_ne!(
        fresh.get("server_pid").and_then(|v| v.as_u64()),
        Some(u64::from(std::process::id())),
        "the fresh capability record must name the real daemon"
    );
    assert_ne!(
        fresh.get("server_start_time").and_then(|v| v.as_u64()),
        Some(1),
        "the tampered start time must not survive"
    );
}

// ----------------------------------------------------------------------------------------
// Authority: reparse/symlink escape, unsafe writability, manifest compatibility
// ----------------------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn installed_current_reparse_or_symlink_authority_is_refused() {
    let _env = ENV_LOCK.lock().await;
    let iso = Isolation::new("ic-reparse");
    let _restore = iso.apply_env();
    let db = iso.root.join("reparse.db");

    // A trusted root reached through a reparse point / symlink is refused at
    // configuration time: the authority chain must be canonical.
    let alias = iso.root.join("install-alias");
    make_dir_link(&iso.install_root, &alias);
    let error = match ApplicationClient::connect_with_daemon(
        config("proof", &db),
        ApplicationDaemonBootstrap::InstalledCurrent {
            trusted_root: alias.clone(),
        },
    )
    .await
    {
        Err(error) => error,
        Ok(_) => panic!("a reparse-point trusted root must be refused"),
    };
    assert_eq!(
        bootstrap_failure(&error),
        DaemonBootstrapFailure::InvalidTrustedRoot
    );

    // A relative root is refused before any filesystem access.
    let error = match ApplicationClient::connect_with_daemon(
        config("proof", &db),
        ApplicationDaemonBootstrap::InstalledCurrent {
            trusted_root: PathBuf::from("relative/install"),
        },
    )
    .await
    {
        Err(error) => error,
        Ok(_) => panic!("a relative trusted root must be refused"),
    };
    assert_eq!(
        bootstrap_failure(&error),
        DaemonBootstrapFailure::InvalidTrustedRoot
    );

    // A versioned target that is a symlink escapes containment and is refused.
    #[cfg(unix)]
    {
        let target = iso.current_binary();
        let outside = iso.root.join("outside-telex");
        std::fs::copy(isolation::branch_binary(), &outside).expect("stage an outside image");
        std::fs::remove_file(&target).expect("remove the versioned target");
        std::os::unix::fs::symlink(&outside, &target).expect("link the versioned target out");
        let client = ApplicationClient::connect_with_daemon(
            config("proof", &db),
            ApplicationDaemonBootstrap::InstalledCurrent {
                trusted_root: iso.trusted_root(),
            },
        )
        .await
        .expect("configuration still succeeds; resolution fails at use");
        let outcome = client
            .attach(&[spec("ic:reparse:a", ApplicationCapability::SendOnly)])
            .await;
        assert_eq!(
            attach_failure(&outcome),
            DaemonBootstrapFailure::UnsafeInstallAuthority
        );
    }
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn installed_current_foreign_writable_authority_is_refused() {
    use std::os::unix::fs::PermissionsExt;

    let _env = ENV_LOCK.lock().await;
    let iso = Isolation::new("ic-writable");
    let _restore = iso.apply_env();
    let db = iso.root.join("writable.db");

    let client = ApplicationClient::connect_with_daemon(
        config("proof", &db),
        ApplicationDaemonBootstrap::InstalledCurrent {
            trusted_root: iso.trusted_root(),
        },
    )
    .await
    .expect("configuration succeeds while the authority chain is safe");
    std::fs::set_permissions(&iso.root, std::fs::Permissions::from_mode(0o777))
        .expect("make a frozen-policy ancestor world-writable");
    let outcome = client
        .attach(&[spec("ic:writable:a", ApplicationCapability::SendOnly)])
        .await;
    assert_eq!(
        attach_failure(&outcome),
        DaemonBootstrapFailure::UnsafeInstallAuthority
    );
}

#[cfg(windows)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn installed_current_foreign_writable_authority_is_refused() {
    let _env = ENV_LOCK.lock().await;
    let iso = Isolation::new("ic-writable");
    let _restore = iso.apply_env();
    let db = iso.root.join("writable.db");

    let client = ApplicationClient::connect_with_daemon(
        config("proof", &db),
        ApplicationDaemonBootstrap::InstalledCurrent {
            trusted_root: iso.trusted_root(),
        },
    )
    .await
    .expect("configuration succeeds while the authority chain is safe");

    // Grant the world-scoped `Everyone` SID write access on the version
    // directory's parent after the policy is frozen, so per-connection
    // resolution must revalidate the complete authority chain.
    let mut command = std::process::Command::new("icacls");
    command
        .arg(&iso.root)
        .arg("/grant")
        .arg("*S-1-1-0:(OI)(CI)(F)")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let granted = isolation::run_with_timeout(command, Duration::from_secs(30));
    granted.assert_success("granting Everyone write on the version directory");

    let outcome = client
        .attach(&[spec("ic:writable:a", ApplicationCapability::SendOnly)])
        .await;
    assert_eq!(
        attach_failure(&outcome),
        DaemonBootstrapFailure::UnsafeInstallAuthority
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn installed_current_incompatible_manifest_metadata_is_refused() {
    let _env = ENV_LOCK.lock().await;
    let iso = Isolation::new("ic-manifest");
    let _restore = iso.apply_env();
    let db = iso.root.join("manifest.db");
    let tag = iso.tag.clone();
    let client = connect(&iso, "proof", &db).await;

    // Protocol skew is a compatibility failure.
    write_manifest(&iso, &tag, |manifest| manifest.protocol_major += 1);
    let outcome = client
        .attach(&[spec("ic:manifest:a", ApplicationCapability::SendOnly)])
        .await;
    assert_eq!(
        attach_failure(&outcome),
        DaemonBootstrapFailure::IncompatibleManifest
    );

    // A schema range this build cannot serve is a compatibility failure.
    write_manifest(&iso, &tag, |manifest| {
        manifest.protocol_major -= 1;
        manifest.schema_min = install::SUPPORTED_SCHEMA_MAX + 5;
        manifest.schema_max = install::SUPPORTED_SCHEMA_MAX + 9;
    });
    let outcome = client
        .attach(&[spec("ic:manifest:b", ApplicationCapability::SendOnly)])
        .await;
    assert_eq!(
        attach_failure(&outcome),
        DaemonBootstrapFailure::IncompatibleManifest
    );

    // A missing required capability is a compatibility failure.
    write_manifest(&iso, &tag, |manifest| {
        manifest.schema_min = install::SUPPORTED_SCHEMA_MIN;
        manifest.schema_max = install::SUPPORTED_SCHEMA_MAX;
        manifest.required_capabilities.clear();
    });
    let outcome = client
        .attach(&[spec("ic:manifest:c", ApplicationCapability::SendOnly)])
        .await;
    assert_eq!(
        attach_failure(&outcome),
        DaemonBootstrapFailure::IncompatibleManifest
    );

    // Missing build identity forfeits the HelloAck build binding: an
    // incomplete manifest, not a compatibility skew.
    write_manifest(&iso, &tag, |manifest| {
        manifest.required_capabilities = telex::daemon_ipc::REQUIRED_CAPABILITIES
            .iter()
            .map(|capability| (*capability).to_string())
            .collect();
        manifest.build_id = install::UNKNOWN_BUILD_ID.to_string();
    });
    let outcome = client
        .attach(&[spec("ic:manifest:d", ApplicationCapability::SendOnly)])
        .await;
    assert_eq!(
        attach_failure(&outcome),
        DaemonBootstrapFailure::InvalidManifest
    );

    // A manifest that binds a different tag is foreign, not skewed.
    write_manifest(&iso, &tag, |manifest| {
        manifest.build_id = install::BUILD_ID.to_string();
        manifest.tag = format!("{}-other", manifest.tag);
    });
    let outcome = client
        .attach(&[spec("ic:manifest:e", ApplicationCapability::SendOnly)])
        .await;
    assert_eq!(
        attach_failure(&outcome),
        DaemonBootstrapFailure::InvalidManifest
    );

    // Restoring a strict, matching manifest restores service.
    write_manifest(&iso, &tag, |manifest| manifest.tag = tag.clone());
    assert!(
        client
            .attach(&[spec("ic:manifest:f", ApplicationCapability::SendOnly)])
            .await
            .ready,
        "a strict, matching manifest must serve"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn installed_current_child_rejects_mismatched_build_before_readiness() {
    let _env = ENV_LOCK.lock().await;
    let iso = Isolation::new("ic-child-build-mismatch");
    let _restore = iso.apply_env();
    let db = iso.root.join("child-build-mismatch.db");
    let tag = iso.tag.clone();

    write_manifest(&iso, &tag, |manifest| {
        manifest.build_id = "different-nonempty-build".to_string();
    });
    let client = connect(&iso, "proof", &db).await;
    let outcome = client
        .attach(&[spec(
            "ic:child-build-mismatch:a",
            ApplicationCapability::SendOnly,
        )])
        .await;
    assert!(
        !outcome.ready,
        "a child whose compiled build does not match the manifest must fail"
    );
    assert_eq!(
        attach_failure(&outcome),
        DaemonBootstrapFailure::IncompatibleManifest,
        "child admission rejection must preserve its typed bootstrap failure"
    );
    assert!(
        iso.cap_path().is_none(),
        "a mismatched child must not publish capability or readiness"
    );
    assert!(
        !iso.daemon_running(),
        "a mismatched child must not remain serving"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn installed_current_accepts_v021_writer_missing_admission_after_exact_probe() {
    let _env = ENV_LOCK.lock().await;
    let iso = Isolation::new("ic-v021-manifest");
    let mut restore = iso.apply_env();
    let db = iso.root.join("v021-manifest.db");
    let tag = iso.tag.clone();
    let manifest_path = remove_manifest_admission_field(&iso, &tag);
    let bytes_before = std::fs::read(&manifest_path).expect("read legacy-shaped manifest");
    let metadata_before = std::fs::metadata(&manifest_path).expect("legacy manifest metadata");
    let modified_before = metadata_before.modified().expect("legacy manifest mtime");
    let identity_before = platform_file_identity(&manifest_path);
    let probe_counter = iso.root.join("admission-probes");
    restore.set("TELEX_TEST_ADMISSION_PROBE_COUNTER", &probe_counter);

    let first = connect(&iso, "proof", &db).await;
    assert!(
        first
            .attach(&[spec("ic:v021-manifest:a", ApplicationCapability::SendOnly,)])
            .await
            .ready,
        "the exact 0.3 binary probe must complete an absent v0.2.1-written field in memory"
    );
    let first_pid = iso.daemon_pid().expect("first admitted daemon pid");
    let second = connect(&iso, "proof", &db).await;
    assert!(
        second
            .attach(&[spec("ic:v021-manifest:b", ApplicationCapability::SendOnly,)])
            .await
            .ready
    );
    assert_eq!(
        iso.daemon_pid(),
        Some(first_pid),
        "matching prestarted reuse must retain complete runtime admission evidence"
    );

    assert_eq!(
        std::fs::read_to_string(&probe_counter)
            .expect("probe counter")
            .lines()
            .count(),
        1,
        "one process must singleflight/cache the exact successful probe"
    );
    assert_eq!(
        std::fs::read(&manifest_path).expect("manifest after compatibility completion"),
        bytes_before,
        "compatibility completion must never rewrite the immutable manifest"
    );
    assert_eq!(
        std::fs::metadata(&manifest_path)
            .expect("manifest metadata after completion")
            .modified()
            .expect("manifest mtime after completion"),
        modified_before,
        "compatibility completion must preserve manifest mtime"
    );
    assert_eq!(
        platform_file_identity(&manifest_path),
        identity_before,
        "compatibility completion must preserve manifest file identity"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn installed_current_missing_admission_probe_is_cross_process_serialized() {
    let _env = ENV_LOCK.lock().await;
    let iso = Isolation::new("ic-v021-cross-process");
    let _restore = iso.apply_env();
    let tag = iso.tag.clone();
    let manifest_path = remove_manifest_admission_field(&iso, &tag);
    let manifest_before = std::fs::read(&manifest_path).expect("legacy-shaped manifest");
    let fixture = build_public_fixture();
    let profile = "v021_cross_process";
    write_fixture_profile(&iso, profile);
    let (_, backend_profile) =
        telex::profiles::resolve(Some(profile), None).expect("resolve fixture profile");
    drop(
        telex::profiles::build(&backend_profile, None)
            .await
            .expect("initialize fixture store before concurrent processes"),
    );

    let mut first = iso.command_for(&fixture);
    first
        .env("TELEX_FIXTURE_TRUSTED_ROOT", iso.trusted_root())
        .env("TELEX_FIXTURE_BACKEND", profile)
        .env("TELEX_FIXTURE_RUN_ID", "v021-cross-process-a")
        .arg("attach");
    let mut second = iso.command_for(&fixture);
    second
        .env("TELEX_FIXTURE_TRUSTED_ROOT", iso.trusted_root())
        .env("TELEX_FIXTURE_BACKEND", profile)
        .env("TELEX_FIXTURE_RUN_ID", "v021-cross-process-b")
        .arg("attach");
    let first_run =
        std::thread::spawn(move || isolation::run_with_timeout(first, Duration::from_secs(180)));
    let second_run =
        std::thread::spawn(move || isolation::run_with_timeout(second, Duration::from_secs(180)));
    first_run
        .join()
        .expect("join first compatibility process")
        .assert_success("first compatibility process");
    second_run
        .join()
        .expect("join second compatibility process")
        .assert_success("second compatibility process");
    assert_eq!(
        std::fs::read(&manifest_path).expect("manifest after concurrent probes"),
        manifest_before,
        "cross-process compatibility probes must leave the manifest byte-identical"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn installed_current_rejects_legacy_daemon_without_admission_support() {
    let _env = ENV_LOCK.lock().await;
    let iso = Isolation::new("ic-legacy-admission");
    let _restore = iso.apply_env();
    let db = iso.root.join("legacy-admission.db");
    let tag = iso.tag.clone();

    write_manifest(&iso, &tag, |manifest| {
        manifest.application_bootstrap_admission_version = 0;
    });
    let client = connect(&iso, "proof", &db).await;
    let outcome = client
        .attach(&[spec(
            "ic:legacy-admission:a",
            ApplicationCapability::SendOnly,
        )])
        .await;
    assert_eq!(
        attach_failure(&outcome),
        DaemonBootstrapFailure::IncompatibleManifest
    );
    assert!(
        iso.cap_path().is_none(),
        "a legacy daemon must be refused before capability publication"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn exact_executable_file_identity_change_is_refused() {
    let _env = ENV_LOCK.lock().await;
    let iso = Isolation::new("ic-identity");
    let _restore = iso.apply_env();
    let db = iso.root.join("identity.db");

    // The pinned development/test seam captures the platform file identity of
    // its target when the policy is frozen.
    let pinned_dir = iso.root.join("pinned");
    isolation::create_owner_private_dir(&pinned_dir);
    let pinned = pinned_dir.join(install::exe_name());
    std::fs::copy(isolation::branch_binary(), &pinned).expect("stage the pinned target");

    let client = ApplicationClient::connect_with_daemon(
        config("proof", &db),
        ApplicationDaemonBootstrap::ExactExecutable {
            executable: pinned.clone(),
        },
    )
    .await
    .expect("pinned exact-executable configuration");
    assert!(
        client
            .attach(&[spec("ic:identity:a", ApplicationCapability::SendOnly)])
            .await
            .ready,
        "the pinned target must serve"
    );
    iso.stop_daemon();

    // Replace the target so its platform file identity changes.
    isolation::remove_file_when_free(&pinned, Duration::from_secs(20));
    std::fs::copy(isolation::branch_binary(), &pinned).expect("re-stage the pinned target");

    let outcome = client
        .attach(&[spec("ic:identity:b", ApplicationCapability::SendOnly)])
        .await;
    assert!(
        matches!(
            attach_failure(&outcome),
            DaemonBootstrapFailure::ExecutableIdentityMismatch
                | DaemonBootstrapFailure::ForeignDaemon
        ),
        "a replaced pinned target must fail closed: {outcome:?}"
    );
}

#[cfg(windows)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn exact_executable_fresh_policy_refuses_stale_running_image() {
    let _env = ENV_LOCK.lock().await;
    let iso = Isolation::new("exact-stale-running");
    let _restore = iso.apply_env();
    let db = iso.root.join("exact-stale-running.db");
    let pinned_dir = iso.root.join("pinned-stale-running");
    isolation::create_owner_private_dir(&pinned_dir);
    let pinned = pinned_dir.join(install::exe_name());
    std::fs::copy(isolation::branch_binary(), &pinned).expect("stage pinned target");

    let first = ApplicationClient::connect_with_daemon(
        config("proof", &db),
        ApplicationDaemonBootstrap::ExactExecutable {
            executable: pinned.clone(),
        },
    )
    .await
    .expect("first exact policy");
    assert!(
        first
            .attach(&[spec("ic:exact-stale:a", ApplicationCapability::SendOnly,)])
            .await
            .ready
    );

    let displaced = pinned.with_extension("running-image");
    std::fs::rename(&pinned, &displaced).expect("rename running exact image");
    std::fs::copy(isolation::branch_binary(), &pinned).expect("replace exact executable pathname");
    let fresh = ApplicationClient::connect_with_daemon(
        config("proof", &db),
        ApplicationDaemonBootstrap::ExactExecutable { executable: pinned },
    )
    .await
    .expect("fresh exact policy over replacement");
    let outcome = fresh
        .attach(&[spec("ic:exact-stale:b", ApplicationCapability::SendOnly)])
        .await;
    assert_eq!(
        attach_failure(&outcome),
        DaemonBootstrapFailure::ForeignDaemon,
        "fresh exact policy must reject the stale mapped peer before Hello"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn exact_executable_authority_change_is_refused_on_reconnect() {
    let _env = ENV_LOCK.lock().await;
    let iso = Isolation::new("exact-authority");
    let _restore = iso.apply_env();
    let db = iso.root.join("exact-authority.db");
    let pinned_dir = iso.root.join("pinned-authority");
    isolation::create_owner_private_dir(&pinned_dir);
    let pinned = pinned_dir.join(install::exe_name());
    std::fs::copy(isolation::branch_binary(), &pinned).expect("stage pinned target");
    let client = ApplicationClient::connect_with_daemon(
        config("proof", &db),
        ApplicationDaemonBootstrap::ExactExecutable { executable: pinned },
    )
    .await
    .expect("freeze safe exact executable");

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&pinned_dir, std::fs::Permissions::from_mode(0o777))
            .expect("make pinned parent foreign-writable");
    }
    #[cfg(windows)]
    {
        let mut command = std::process::Command::new("icacls");
        command
            .arg(&pinned_dir)
            .arg("/grant")
            .arg("*S-1-1-0:(OI)(CI)(W)");
        isolation::run_with_timeout(command, Duration::from_secs(30))
            .assert_success("make pinned parent foreign-writable");
    }

    let outcome = client
        .attach(&[spec(
            "ic:exact-authority:a",
            ApplicationCapability::SendOnly,
        )])
        .await;
    assert_eq!(
        attach_failure(&outcome),
        DaemonBootstrapFailure::UnsafeInstallAuthority
    );
}

// ----------------------------------------------------------------------------------------
// Foreign binder and child readiness refusal
// ----------------------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn installed_current_foreign_binder_is_refused_before_hello() {
    let _env = ENV_LOCK.lock().await;
    let iso = Isolation::new("ic-foreign");
    let _restore = iso.apply_env();
    let db = iso.root.join("foreign.db");

    // Learn this environment's rendezvous, then bind it from *this* process --
    // a same-user peer that is not the selected daemon image.
    let paths = telex::daemon::DaemonPaths::current().expect("daemon paths");
    let _binder = ForeignBinder::bind(&paths);
    write_foreign_cap(&paths);

    let client = connect(&iso, "proof", &db).await;
    let outcome = client
        .attach(&[spec("ic:foreign:a", ApplicationCapability::SendOnly)])
        .await;
    let failure = attach_failure(&outcome);
    // Remove the planted record before teardown so the harness never mistakes
    // this process for the daemon it should stop.
    let _ = std::fs::remove_file(&paths.cap_path);
    assert_eq!(
        failure,
        DaemonBootstrapFailure::ForeignDaemon,
        "a same-user peer that is not the selected image must be refused \
         before any Hello is exchanged"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn installed_current_child_refuses_readiness_without_a_valid_selection() {
    let _env = ENV_LOCK.lock().await;
    let iso = Isolation::new("ic-readiness");
    let _restore = iso.apply_env();

    let mut command = iso.command();
    command
        .env(BOOTSTRAP_TOKEN_ENV, "not-a-valid-selection-token")
        .args(["daemon", "serve"]);
    let output = isolation::run_with_timeout(command, Duration::from_secs(30));
    output.assert_failure("a child with an invalid selection token must refuse readiness");
    assert!(
        iso.cap_path().is_none(),
        "a refused child must not publish a capability record"
    );
    assert!(
        !iso.daemon_running(),
        "a refused child must not serve requests"
    );
}

// ----------------------------------------------------------------------------------------
// Platform helpers (isolated setup / fault induction only)
// ----------------------------------------------------------------------------------------

fn wait_until_gone(iso: &Isolation, pid: u32, timeout: Duration) {
    let deadline = std::time::Instant::now() + timeout;
    while std::time::Instant::now() < deadline {
        if !isolation::process_alive(pid) {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let _ = iso;
    panic!("daemon {pid} did not exit within the deadline");
}

fn terminate_pid(pid: u32) {
    isolation::terminate_process(pid);
}

#[cfg(unix)]
fn make_dir_link(target: &std::path::Path, link: &std::path::Path) {
    std::os::unix::fs::symlink(target, link).expect("create a symlinked install alias");
}

#[cfg(windows)]
fn make_dir_link(target: &std::path::Path, link: &std::path::Path) {
    // A directory junction is a reparse point and needs no elevation.
    let mut command = std::process::Command::new("cmd");
    command
        .args(["/C", "mklink", "/J"])
        .arg(link)
        .arg(target)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let output = isolation::run_with_timeout(command, Duration::from_secs(30));
    output.assert_success("creating a directory junction install alias");
}

fn write_foreign_cap(paths: &telex::daemon::DaemonPaths) {
    let start_time = telex::session_watch::capture_process_start_time(std::process::id());
    let cap = serde_json::json!({
        "instance_id": format!("foreign-{}", now_ms()),
        "admin_cap": "foreign-cap",
        "singleton_hash": paths.singleton_hash,
        "protocol_major": telex::daemon_ipc::PROTOCOL_MAJOR,
        "server_pid": std::process::id(),
        "server_start_time": start_time,
    });
    if let Some(parent) = paths.cap_path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    std::fs::write(
        &paths.cap_path,
        serde_json::to_string(&cap).expect("serialize foreign cap"),
    )
    .expect("write foreign cap");
}

#[cfg(unix)]
struct ForeignBinder {
    _listener: std::os::unix::net::UnixListener,
    path: PathBuf,
}

#[cfg(unix)]
impl ForeignBinder {
    fn bind(paths: &telex::daemon::DaemonPaths) -> Self {
        let path = PathBuf::from(paths.endpoint.display());
        let _ = std::fs::remove_file(&path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        let listener =
            std::os::unix::net::UnixListener::bind(&path).expect("bind a foreign endpoint");
        let accepting = listener.try_clone().expect("clone the foreign listener");
        std::thread::spawn(move || {
            // Accept and briefly hold connections so the client reaches peer
            // authentication rather than a connection error.
            for stream in accepting.incoming() {
                match stream {
                    Ok(stream) => {
                        std::thread::sleep(Duration::from_millis(250));
                        drop(stream);
                    }
                    Err(_) => break,
                }
            }
        });
        Self {
            _listener: listener,
            path,
        }
    }
}

#[cfg(unix)]
impl Drop for ForeignBinder {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

#[cfg(windows)]
struct ForeignBinder {
    handle: isize,
}

#[cfg(windows)]
impl ForeignBinder {
    fn bind(paths: &telex::daemon::DaemonPaths) -> Self {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Storage::FileSystem::{FILE_FLAG_OVERLAPPED, PIPE_ACCESS_DUPLEX};
        use windows_sys::Win32::System::Pipes::{
            CreateNamedPipeW, PIPE_READMODE_BYTE, PIPE_TYPE_BYTE, PIPE_WAIT,
        };

        let name = paths.endpoint.display();
        let wide: Vec<u16> = std::ffi::OsStr::new(&name)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        let handle = unsafe {
            CreateNamedPipeW(
                wide.as_ptr(),
                PIPE_ACCESS_DUPLEX | FILE_FLAG_OVERLAPPED,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
                8,
                4096,
                4096,
                0,
                std::ptr::null(),
            )
        };
        assert_ne!(
            handle,
            -1isize,
            "binding a foreign named pipe endpoint: {}",
            std::io::Error::last_os_error()
        );
        Self { handle }
    }
}

#[cfg(windows)]
impl Drop for ForeignBinder {
    fn drop(&mut self) {
        use windows_sys::Win32::Foundation::CloseHandle;
        unsafe {
            CloseHandle(self.handle);
        }
    }
}
