#![cfg(any(feature = "sqlite", feature = "postgres"))]
//! Executes the external, defaults-disabled consumer fixture against a
//! production `InstalledCurrent` daemon.
//!
//! The fixture crate (`tests/fixtures/application-client-consumer`) is built
//! and *run* here -- compile-only coverage is not enough. Its two probes are
//! consumer-shaped: a send-only Watcher and a bidirectional Operator Station.
//! Both are selected by argument, use only the public
//! `telex::application_client` surface, and never touch daemon, install,
//! backend, CLI, or product code.
//!
//! The harness (this file) is allowed to use isolated setup seams; the fixture
//! under test is not.

#[path = "support/telex_isolation.rs"]
mod isolation;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use isolation::{run_with_timeout, CliOutput, Isolation};
#[cfg(feature = "postgres")]
use telex::model::now_ms;
use telex::profiles::{BackendProfile, ConfigFile};

fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("application-client-consumer")
}

fn cargo() -> String {
    std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string())
}

/// Build the fixture with an explicit, defaults-disabled feature profile and
/// return its executable. The fixture crate carries its own `[workspace]`, so
/// this never contends with the parent target directory lock.
fn build_fixture(features: &str) -> PathBuf {
    let dir = fixture_dir();
    let mut command = Command::new(cargo());
    command
        .arg("build")
        .arg("--manifest-path")
        .arg(dir.join("Cargo.toml"))
        .arg("--no-default-features")
        .arg("--features")
        .arg(features)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let output = run_with_timeout(command, Duration::from_secs(900));
    output.assert_success(&format!("building the consumer fixture with [{features}]"));
    let binary = dir.join("target").join("debug").join(format!(
        "telex-application-client-consumer{}",
        std::env::consts::EXE_SUFFIX
    ));
    assert!(
        binary.is_file(),
        "fixture binary not produced at {}",
        binary.display()
    );
    binary
}

fn run_probe(
    iso: &Isolation,
    binary: &Path,
    backend: &str,
    probe: &str,
    run_id: &str,
) -> CliOutput {
    let mut command = iso.command_for(binary);
    command
        .env("TELEX_FIXTURE_TRUSTED_ROOT", iso.trusted_root())
        .env("TELEX_FIXTURE_BACKEND", backend)
        .env("TELEX_FIXTURE_RUN_ID", run_id)
        .arg(probe);
    run_with_timeout(command, Duration::from_secs(180))
}

fn assert_probe_evidence(output: &CliOutput, probe: &str, expected: &[&str]) {
    output.assert_success(&format!("consumer fixture probe '{probe}'"));
    assert!(
        output.stdout.contains("probe-result=ok"),
        "probe '{probe}' did not report success: stdout={} stderr={}",
        output.stdout,
        output.stderr
    );
    for marker in expected {
        assert!(
            output.stdout.contains(marker),
            "probe '{probe}' missing evidence '{marker}': stdout={}",
            output.stdout
        );
    }
}

const WATCHER_EVIDENCE: &[&str] = &[
    "identity=stable-store-fresh-runtime",
    "retry=replayed",
    "recovery=recorded",
    "attendance=send-only",
    "retention=boundary-crossed",
];

const STATION_EVIDENCE: &[&str] = &[
    "lifecycle=attached",
    "lifecycle=cancellation-partitioned",
    "lifecycle=compensable",
    "ingested:",
    "ack=marked",
    "source=store-scoped",
    "compound=reply-then-handle",
    "provenance=",
    "delta=",
    "detach=deliberate",
    "cleanup_deleted=",
];

fn write_profile_config(iso: &Isolation, name: &str, profile: BackendProfile) {
    let mut backends = BTreeMap::new();
    backends.insert(name.to_string(), profile);
    iso.write_config(&ConfigFile {
        default: None,
        backends,
    });
}

#[cfg(feature = "sqlite")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sqlite_consumer_fixture_probes_execute_against_installed_current() {
    let binary = build_fixture("sqlite");
    let iso = Isolation::new("fixture-sqlite");
    let db = iso.root.join("fixture.db");
    let mut profile = telex::profiles::implicit_sqlite(None);
    profile.path = Some(db.to_string_lossy().into_owned());
    write_profile_config(&iso, "fixture_sqlite", profile);

    let watcher = run_probe(&iso, &binary, "fixture_sqlite", "watcher", "sqlite-watcher");
    assert_probe_evidence(&watcher, "watcher", WATCHER_EVIDENCE);

    let station = run_probe(&iso, &binary, "fixture_sqlite", "station", "sqlite-station");
    assert_probe_evidence(&station, "station", STATION_EVIDENCE);

    // The fixture reached the store through the production InstalledCurrent
    // seam, so an installed daemon must exist for this isolated environment.
    assert!(
        iso.cap_path().is_some(),
        "the fixture must have spawned the installed daemon"
    );
}

#[cfg(feature = "postgres")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn postgres_consumer_fixture_probes_execute_against_installed_current() {
    use telex::backend::postgres::{make_tls, sanitize_ident};

    let Some(url) = isolation::postgres_url_or_fail_closed("consumer-fixture-probes") else {
        return;
    };
    let binary = build_fixture("postgres");
    let iso = Isolation::new("fixture-postgres");

    let base = sanitize_ident(
        &std::env::var("TELEX_PG_SCHEMA").unwrap_or_else(|_| "telex_fixture".into()),
    )
    .expect("TELEX_PG_SCHEMA must be a valid identifier");
    let schema = sanitize_ident(&format!("{base}_{}_{}", std::process::id(), now_ms()))
        .expect("derived schema name must be a valid identifier");

    let mut cfg: tokio_postgres::Config = url
        .parse()
        .expect("TELEX_PG_URL must be a libpq URI or key=value DSN");
    if let Ok(password) = std::env::var("TELEX_PG_PASSWORD") {
        if !password.is_empty() {
            cfg.password(password);
        }
    }
    let admin_exec = |sql: String| {
        let cfg = cfg.clone();
        async move {
            let (client, connection) = cfg
                .connect(make_tls().expect("tls"))
                .await
                .expect("admin connect");
            let handle = tokio::spawn(async move {
                let _ = connection.await;
            });
            let result = client.batch_execute(&sql).await;
            drop(client);
            let _ = handle.await;
            result.unwrap_or_else(|e| panic!("admin statement failed: {e}"));
        }
    };
    admin_exec(format!("DROP SCHEMA IF EXISTS {schema} CASCADE")).await;

    let mut profile = telex::profiles::implicit_sqlite(None);
    profile.kind = "postgres".to_string();
    profile.path = None;
    profile.url = Some(url.clone());
    profile.schema = Some(schema.clone());
    profile.auth = Some("password".to_string());
    if std::env::var("TELEX_PG_PASSWORD").is_ok() {
        profile.password_env = Some("TELEX_PG_PASSWORD".to_string());
    }
    write_profile_config(&iso, "fixture_pg", profile);

    let watcher = run_probe(&iso, &binary, "fixture_pg", "watcher", "pg-watcher");
    let station = run_probe(&iso, &binary, "fixture_pg", "station", "pg-station");
    let cap_present = iso.cap_path().is_some();
    admin_exec(format!("DROP SCHEMA IF EXISTS {schema} CASCADE")).await;

    assert_probe_evidence(&watcher, "watcher", WATCHER_EVIDENCE);
    assert_probe_evidence(&station, "station", STATION_EVIDENCE);
    assert!(
        cap_present,
        "the fixture must have spawned the installed daemon"
    );
}
