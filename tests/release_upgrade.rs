//! End-to-end proof of the in-binary `telex upgrade` release path, hermetic and offline.
//!
//! A local `TcpListener` HTTP server serves a captured-shape GitHub release JSON plus a real
//! archive (the test's own built `telex` binary) and its SHA-256 sidecar. The test drives the
//! real `telex` binary as a subprocess with the discovery/download bases pointed at the local
//! server, and asserts the full download -> verify -> extract -> install -> switch flow, plus
//! the fail-closed cases (checksum mismatch, missing sidecar, missing platform asset).
//!
//! Runs only when the crate is built with the `self-update` feature (the default).

#![cfg(feature = "self-update")]

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use std::thread;

use serde_json::Value;
use sha2::{Digest, Sha256};
use telex::install;
use telex::release::{asset_name, current_target, ArchiveKind};

const REPO: &str = "test/telex";
// The fixture serves the test's own telex binary, so the release tag must match that binary's
// self-reported version (the release path asserts tag == probed package_version, SF-8).
const TAG: &str = concat!("v", env!("CARGO_PKG_VERSION"));

fn telex_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_telex"))
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut s = String::with_capacity(64);
    for b in digest {
        use std::fmt::Write;
        let _ = write!(s, "{b:02x}");
    }
    s
}

/// Package the current telex binary into the platform archive format.
fn pack(kind: ArchiveKind, binary: &[u8]) -> Vec<u8> {
    let exe = install::exe_name();
    match kind {
        ArchiveKind::Zip => {
            let mut buf = Vec::new();
            {
                let mut w = zip::ZipWriter::new(std::io::Cursor::new(&mut buf));
                let opts: zip::write::FileOptions<()> = zip::write::FileOptions::default()
                    .compression_method(zip::CompressionMethod::Deflated);
                w.start_file(exe, opts).unwrap();
                w.write_all(binary).unwrap();
                w.start_file("LICENSE", opts).unwrap();
                w.write_all(b"MIT").unwrap();
                w.finish().unwrap();
            }
            buf
        }
        ArchiveKind::TarGz => {
            use flate2::write::GzEncoder;
            use flate2::Compression;
            let mut gz = GzEncoder::new(Vec::new(), Compression::default());
            {
                let mut builder = tar::Builder::new(&mut gz);
                let mut header = tar::Header::new_gnu();
                header.set_size(binary.len() as u64);
                header.set_mode(0o755);
                header.set_cksum();
                builder.append_data(&mut header, exe, binary).unwrap();
                let license = b"MIT";
                let mut lheader = tar::Header::new_gnu();
                lheader.set_size(license.len() as u64);
                lheader.set_mode(0o644);
                lheader.set_cksum();
                builder
                    .append_data(&mut lheader, "LICENSE", &license[..])
                    .unwrap();
                builder.finish().unwrap();
            }
            gz.finish().unwrap()
        }
    }
}

type Routes = HashMap<String, (&'static str, Vec<u8>)>;

/// Spawn a minimal HTTP/1.1 server serving the given routes; returns the bound port.
fn spawn_server(routes: Routes) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let routes = Arc::new(routes);
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let routes = routes.clone();
            thread::spawn(move || {
                let mut buf = Vec::new();
                let mut chunk = [0u8; 1024];
                // Read until end of request headers.
                loop {
                    match stream.read(&mut chunk) {
                        Ok(0) => break,
                        Ok(n) => {
                            buf.extend_from_slice(&chunk[..n]);
                            if buf.windows(4).any(|w| w == b"\r\n\r\n") {
                                break;
                            }
                        }
                        Err(_) => break,
                    }
                }
                let request = String::from_utf8_lossy(&buf);
                let path = request
                    .lines()
                    .next()
                    .and_then(|line| line.split_whitespace().nth(1))
                    .map(|p| p.split('?').next().unwrap_or(p).to_string())
                    .unwrap_or_default();
                let response = match routes.get(&path) {
                    Some((content_type, body)) => {
                        let mut resp = format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            body.len()
                        )
                        .into_bytes();
                        resp.extend_from_slice(body);
                        resp
                    }
                    None => {
                        b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                            .to_vec()
                    }
                };
                let _ = stream.write_all(&response);
                let _ = stream.flush();
            });
        }
    });
    port
}

fn release_json(assets: &[String]) -> Vec<u8> {
    let asset_objs: Vec<Value> = assets
        .iter()
        .map(|name| serde_json::json!({ "name": name }))
        .collect();
    serde_json::to_vec(&serde_json::json!({
        "tag_name": TAG,
        "draft": false,
        "prerelease": false,
        "assets": asset_objs,
    }))
    .unwrap()
}

fn base_routes_for(assets: &[String]) -> Routes {
    let mut routes: Routes = HashMap::new();
    let json = release_json(assets);
    routes.insert(
        format!("/repos/{REPO}/releases/latest"),
        ("application/json", json.clone()),
    );
    routes.insert(
        format!("/repos/{REPO}/releases/tags/{TAG}"),
        ("application/json", json),
    );
    routes
}

fn temp_root(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "telex-upgrade-it-{}-{name}-{:?}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn run_upgrade(port: u16, root: &PathBuf) -> std::process::Output {
    run_upgrade_args(port, root, &[])
}

fn run_upgrade_args(port: u16, root: &PathBuf, extra: &[&str]) -> std::process::Output {
    let mut args: Vec<String> = vec![
        "--json".into(),
        "upgrade".into(),
        "--repo".into(),
        REPO.into(),
        "--root".into(),
        root.to_string_lossy().into_owned(),
        "--skip-drain".into(),
    ];
    args.extend(extra.iter().map(|s| s.to_string()));
    Command::new(telex_bin())
        .args(&args)
        .env("TELEX_UPGRADE_API_BASE", format!("http://127.0.0.1:{port}"))
        .env(
            "TELEX_UPGRADE_DOWNLOAD_BASE",
            format!("http://127.0.0.1:{port}"),
        )
        // Prevent any launcher re-dispatch of the subprocess.
        .env(install::LAUNCHER_GUARD_ENV, "1")
        // Deterministic install root regardless of the host environment.
        .env("TELEX_INSTALL_ROOT", root.to_string_lossy().to_string())
        .output()
        .expect("run telex upgrade")
}

/// Routes for a full, valid release of the test's own binary (archive + sidecar + release JSON).
fn happy_routes() -> (Routes, String, ArchiveKind) {
    let (target, kind) = current_target().expect("current platform is a supported release target");
    let binary = std::fs::read(telex_bin()).unwrap();
    let archive = pack(kind, &binary);
    let archive_name = asset_name(TAG, target, kind);
    let sidecar_name = format!("{archive_name}.sha256");
    let sidecar = format!("{}  {archive_name}", sha256_hex(&archive)).into_bytes();
    let mut routes = base_routes_for(&[archive_name.clone(), sidecar_name.clone()]);
    routes.insert(
        format!("/{REPO}/releases/download/{TAG}/{archive_name}"),
        ("application/octet-stream", archive),
    );
    routes.insert(
        format!("/{REPO}/releases/download/{TAG}/{sidecar_name}"),
        ("text/plain", sidecar),
    );
    (routes, archive_name, kind)
}

#[test]
fn release_upgrade_downloads_verifies_installs_and_switches() {
    let (target, kind) = current_target().expect("current platform is a supported release target");
    let binary = std::fs::read(telex_bin()).unwrap();
    let archive = pack(kind, &binary);
    let archive_name = asset_name(TAG, target, kind);
    let sidecar_name = format!("{archive_name}.sha256");
    let sidecar = format!("{}  {archive_name}", sha256_hex(&archive)).into_bytes();

    let mut routes = base_routes_for(&[archive_name.clone(), sidecar_name.clone()]);
    routes.insert(
        format!("/{REPO}/releases/download/{TAG}/{archive_name}"),
        ("application/octet-stream", archive),
    );
    routes.insert(
        format!("/{REPO}/releases/download/{TAG}/{sidecar_name}"),
        ("text/plain", sidecar),
    );
    let port = spawn_server(routes);

    let root = temp_root("happy");
    let output = run_upgrade(port, &root);
    assert!(
        output.status.success(),
        "upgrade failed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let v: Value = serde_json::from_slice(&output.stdout).expect("upgrade emits JSON");
    assert_eq!(v["upgrade"], true);
    assert_eq!(v["release"]["tag"], TAG);
    assert_eq!(v["release"]["verified"], true);
    assert_eq!(v["release"]["asset"], archive_name);
    assert_eq!(v["switch"]["switched_to"], TAG);

    // The versioned layout now has the release installed and current points at it.
    let current = std::fs::read_to_string(root.join("current")).unwrap();
    assert_eq!(current.trim(), TAG);
    let installed_binary = root.join("versions").join(TAG).join(install::exe_name());
    assert!(
        installed_binary.is_file(),
        "expected installed binary at {}",
        installed_binary.display()
    );

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn release_upgrade_fails_closed_on_checksum_mismatch() {
    let (target, kind) = current_target().expect("supported target");
    let binary = std::fs::read(telex_bin()).unwrap();
    let archive = pack(kind, &binary);
    let archive_name = asset_name(TAG, target, kind);
    let sidecar_name = format!("{archive_name}.sha256");
    // Serve a wrong checksum.
    let bad = format!("{}  {archive_name}", "0".repeat(64)).into_bytes();

    let mut routes = base_routes_for(&[archive_name.clone(), sidecar_name.clone()]);
    routes.insert(
        format!("/{REPO}/releases/download/{TAG}/{archive_name}"),
        ("application/octet-stream", archive),
    );
    routes.insert(
        format!("/{REPO}/releases/download/{TAG}/{sidecar_name}"),
        ("text/plain", bad),
    );
    let port = spawn_server(routes);

    let root = temp_root("mismatch");
    let output = run_upgrade(port, &root);
    assert!(!output.status.success(), "upgrade should fail closed");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("checksum mismatch"),
        "expected checksum-mismatch error, got: {stderr}"
    );
    // Nothing was installed / switched.
    assert!(!root.join("current").exists());
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn release_upgrade_fails_closed_on_missing_sidecar() {
    let (target, kind) = current_target().expect("supported target");
    let binary = std::fs::read(telex_bin()).unwrap();
    let archive = pack(kind, &binary);
    let archive_name = asset_name(TAG, target, kind);

    // Release advertises only the archive, no sidecar.
    let mut routes = base_routes_for(&[archive_name.clone()]);
    routes.insert(
        format!("/{REPO}/releases/download/{TAG}/{archive_name}"),
        ("application/octet-stream", archive),
    );
    let port = spawn_server(routes);

    let root = temp_root("no-sidecar");
    let output = run_upgrade(port, &root);
    assert!(!output.status.success(), "upgrade should fail closed");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("missing checksum sidecar"),
        "expected missing-sidecar error, got: {stderr}"
    );
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn release_upgrade_reports_unsupported_platform_asset() {
    // Release exists but has no asset for this platform.
    let routes = base_routes_for(&["telex-v9.9.9-some-other-target.zip".to_string()]);
    let port = spawn_server(routes);

    let root = temp_root("no-asset");
    let output = run_upgrade(port, &root);
    assert!(!output.status.success(), "upgrade should fail closed");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("no asset for this platform"),
        "expected missing-asset error, got: {stderr}"
    );
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn release_upgrade_times_out_on_a_hanging_server() {
    // A server that accepts the connection but never responds must not hang telex forever:
    // the read timeout (overridden low for the test) makes `telex upgrade` fail fast.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    thread::spawn(move || {
        for stream in listener.incoming() {
            if let Ok(s) = stream {
                // Hold the connection open without ever writing a response.
                std::mem::forget(s);
            }
        }
    });

    let root = temp_root("timeout");
    let start = std::time::Instant::now();
    let output = Command::new(telex_bin())
        .args([
            "--json",
            "upgrade",
            "--repo",
            REPO,
            "--root",
            &root.to_string_lossy(),
            "--skip-drain",
        ])
        .env("TELEX_UPGRADE_API_BASE", format!("http://127.0.0.1:{port}"))
        .env(
            "TELEX_UPGRADE_DOWNLOAD_BASE",
            format!("http://127.0.0.1:{port}"),
        )
        .env("TELEX_UPGRADE_READ_TIMEOUT_MS", "500")
        .env(install::LAUNCHER_GUARD_ENV, "1")
        .env("TELEX_INSTALL_ROOT", root.to_string_lossy().to_string())
        .output()
        .expect("run telex upgrade against a hanging server");
    let elapsed = start.elapsed();
    assert!(!output.status.success(), "upgrade should fail on timeout");
    assert!(
        elapsed < std::time::Duration::from_secs(20),
        "upgrade should fail fast on a hanging server, took {elapsed:?}"
    );
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn release_upgrade_explicit_version_installs_by_tag() {
    // `telex upgrade --version <tag>` resolves via /releases/tags and installs that release.
    let (routes, archive_name, _kind) = happy_routes();
    let port = spawn_server(routes);
    let root = temp_root("explicit-version");
    let output = run_upgrade_args(port, &root, &["--version", TAG]);
    assert!(
        output.status.success(),
        "explicit-version upgrade failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let v: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(v["release"]["tag"], TAG);
    assert_eq!(v["release"]["asset"], archive_name);
    assert_eq!(v["switch"]["switched_to"], TAG);
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn release_upgrade_already_current_then_force_reinstalls() {
    let (routes, _archive_name, _kind) = happy_routes();
    let port = spawn_server(routes);
    let root = temp_root("already-current");

    // First install succeeds and switches.
    let first = run_upgrade(port, &root);
    assert!(first.status.success(), "first upgrade should succeed");

    // Second run resolves the same latest tag -> already current, no reinstall, distinct shape.
    let second = run_upgrade(port, &root);
    assert!(second.status.success(), "already-current run should exit 0");
    let v: Value = serde_json::from_slice(&second.stdout).unwrap();
    assert_eq!(v["upgrade"], false);
    assert_eq!(v["status"], "already_current");
    assert_eq!(v["tag"], TAG);
    assert!(
        v.get("release").is_none(),
        "already-current output omits the release envelope"
    );

    // --force reinstalls even though it is already current.
    let forced = run_upgrade_args(port, &root, &["--force"]);
    assert!(forced.status.success(), "forced reinstall should succeed");
    let fv: Value = serde_json::from_slice(&forced.stdout).unwrap();
    assert_eq!(fv["upgrade"], true);
    assert_eq!(fv["switch"]["switched_to"], TAG);
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn release_upgrade_times_out_on_a_slow_drip_server() {
    // A server that dribbles the discovery body one byte at a time, never finishing, must be
    // bounded by the total request timeout (overridden low), not kept alive indefinitely.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            thread::spawn(move || {
                // Read the request, then send headers promising a large body but never finish it.
                let mut buf = [0u8; 512];
                let _ = stream.read(&mut buf);
                let _ = stream.write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 1000000\r\n\r\n",
                );
                let _ = stream.flush();
                loop {
                    if stream.write_all(b" ").is_err() {
                        break;
                    }
                    let _ = stream.flush();
                    thread::sleep(std::time::Duration::from_millis(50));
                }
            });
        }
    });

    let root = temp_root("slow-drip");
    let start = std::time::Instant::now();
    let output = Command::new(telex_bin())
        .args([
            "--json",
            "upgrade",
            "--repo",
            REPO,
            "--root",
            &root.to_string_lossy(),
            "--skip-drain",
        ])
        .env("TELEX_UPGRADE_API_BASE", format!("http://127.0.0.1:{port}"))
        .env(
            "TELEX_UPGRADE_DOWNLOAD_BASE",
            format!("http://127.0.0.1:{port}"),
        )
        // Total request deadline low enough that a slow-drip transfer is cut off quickly.
        .env("TELEX_UPGRADE_TIMEOUT_MS", "1500")
        .env(install::LAUNCHER_GUARD_ENV, "1")
        .env("TELEX_INSTALL_ROOT", root.to_string_lossy().to_string())
        .output()
        .expect("run telex upgrade against a slow-drip server");
    let elapsed = start.elapsed();
    assert!(!output.status.success(), "upgrade should fail on slow drip");
    assert!(
        elapsed < std::time::Duration::from_secs(20),
        "upgrade should be bounded by the total timeout, took {elapsed:?}"
    );
    std::fs::remove_dir_all(&root).ok();
}
