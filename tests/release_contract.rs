//! Machine-checked release contract.
//!
//! The release asset naming, checksum format, and `telex version --json` surface are
//! a stable contract that the installers (`install.sh` / `install.ps1`) and the
//! downstream `telex upgrade` release-discovery node depend on. This test couples
//! those facts to the code that produces them, so a one-sided change (renaming a
//! target, dropping a `version --json` field, changing the archive grammar) fails at
//! repo-test time rather than silently at a user's install or upgrade.
//!
//! See `docs/developing/releasing.md` for the human-facing contract description.

use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Command;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read(rel: &str) -> String {
    let path = repo_root().join(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

/// Extract every `- target: <triple>` value from the release workflow build matrix.
fn release_matrix_targets() -> Vec<String> {
    let yml = read(".github/workflows/release.yml");
    let mut targets = Vec::new();
    for line in yml.lines() {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix("- target:") {
            targets.push(rest.trim().to_string());
        }
    }
    assert!(
        !targets.is_empty(),
        "release.yml build matrix should declare at least one target"
    );
    targets
}

/// Extract the literal `target="<triple>"` assignments from install.sh case arms.
fn install_sh_targets() -> Vec<String> {
    extract_all(&read("install.sh"), "target=\"", '"')
}

/// Extract the literal `$target = '<triple>'` assignments from install.ps1 switch arms.
fn install_ps1_targets() -> Vec<String> {
    extract_all(&read("install.ps1"), "$target = '", '\'')
}

/// Collect every substring that follows `marker` up to the next `close` char.
fn extract_all(haystack: &str, marker: &str, close: char) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = haystack;
    while let Some(idx) = rest.find(marker) {
        let after = &rest[idx + marker.len()..];
        if let Some(end) = after.find(close) {
            out.push(after[..end].to_string());
            rest = &after[end + 1..];
        } else {
            break;
        }
    }
    out
}

#[test]
fn installer_targets_are_a_subset_of_the_release_matrix() {
    let matrix = release_matrix_targets();
    let mut installer_targets = install_sh_targets();
    installer_targets.extend(install_ps1_targets());
    assert!(
        !installer_targets.is_empty(),
        "installers should reference at least one target triple"
    );
    for t in &installer_targets {
        assert!(
            matrix.contains(t),
            "installer target `{t}` is not built by release.yml matrix {matrix:?}; \
             every platform an installer can request must be produced by the release workflow"
        );
    }
}

#[test]
fn archive_name_grammar_is_consistent_across_workflow_and_installers() {
    // The asset grammar is telex-<tag>-<target>.{zip,tar.gz}. Couple the grammar
    // fragments across the producing workflow and the consuming installers so a
    // rename on one side breaks this test, not a user's download.
    let release = read(".github/workflows/release.yml");
    let install_sh = read("install.sh");
    let install_ps1 = read("install.ps1");

    // Unix tar.gz grammar.
    assert!(
        release.contains("telex-${tag}-${target}.tar.gz"),
        "release.yml should package Unix assets as telex-${{tag}}-${{target}}.tar.gz"
    );
    assert!(
        install_sh.contains("telex-${tag}-${target}.tar.gz"),
        "install.sh should download telex-${{tag}}-${{target}}.tar.gz"
    );
    // Windows zip grammar.
    assert!(
        release.contains("telex-$tag-$target.zip"),
        "release.yml should package Windows assets as telex-$tag-$target.zip"
    );
    assert!(
        install_ps1.contains("telex-$tag-$target.zip"),
        "install.ps1 should download telex-$tag-$target.zip"
    );
}

#[test]
fn checksum_sidecar_format_is_the_contracted_shape() {
    // Contract: each asset has a sibling `<asset>.sha256` whose first whitespace-
    // separated token is the lowercase hex SHA-256. Installers parse field 1 only.
    let release = read(".github/workflows/release.yml");
    // Windows: "$hash  $archive" written to "$archive.sha256".
    assert!(
        release.contains("\"$hash  $archive\"") && release.contains("$archive.sha256"),
        "release.yml Windows packaging should emit `<hash>  <archive>` to <archive>.sha256"
    );
    // Unix: `shasum -a 256 "${archive}" > "${archive}.sha256"` (hash is field 1).
    assert!(
        release.contains("shasum -a 256") && release.contains("${archive}.sha256"),
        "release.yml Unix packaging should emit a `shasum -a 256` sidecar <archive>.sha256"
    );
    // Installers must key off field 1 (the hash).
    let install_sh = read("install.sh");
    assert!(
        install_sh.contains("awk '{print $1}'"),
        "install.sh should parse the sha256 sidecar's first field as the hash"
    );
}

fn telex_bin() -> PathBuf {
    if let Some(path) = option_env!("CARGO_BIN_EXE_telex") {
        return PathBuf::from(path);
    }
    let exe = std::env::current_exe().expect("current test exe");
    let dir = exe.parent().expect("test exe dir");
    let target_dir = if dir.file_name().and_then(|n| n.to_str()) == Some("deps") {
        dir.parent().expect("target profile dir")
    } else {
        dir
    };
    let name = if cfg!(windows) { "telex.exe" } else { "telex" };
    target_dir.join(name)
}

#[test]
fn version_json_exposes_the_upgrade_contract_surface() {
    // Lock the `telex version --json` keys that the downstream release-upgrade node
    // consumes. Renaming or dropping one of these is a breaking change to that
    // contract and must be caught here, not by the future upgrade implementer.
    let bin = telex_bin();
    if !bin.exists() {
        // Binary not built in this test context (e.g. `cargo test` on a crate-only
        // filter). The static checks above still run; skip the runtime check.
        eprintln!("skipping version --json check: {} not built", bin.display());
        return;
    }
    let output = Command::new(&bin)
        .args(["--json", "version"])
        .output()
        .expect("run telex --json version");
    assert!(
        output.status.success(),
        "telex --json version exited with failure: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let v: Value =
        serde_json::from_slice(&output.stdout).expect("telex --json version emits valid JSON");

    // Package version (the binary's self-identified version).
    assert!(
        v["version"]["package_version"].is_string(),
        "version.package_version must be a string"
    );
    // Versioned-install layout the launcher/upgrade tooling reads.
    for key in ["root", "current_tag", "previous_tag", "active_tag"] {
        assert!(
            v["version"]["install"].get(key).is_some(),
            "version.install.{key} must be present for upgrade tooling"
        );
    }
    // Daemon protocol handshake surface.
    assert!(
        v["daemon_metadata"]["protocol_version"]["major"].is_number(),
        "daemon_metadata.protocol_version.major must be a number"
    );
    assert!(
        v["daemon_metadata"]["protocol_version"]["minor"].is_number(),
        "daemon_metadata.protocol_version.minor must be a number"
    );
    // Copilot bridge compatibility surface.
    assert!(
        v["copilot"]["bridge_protocol"].is_number(),
        "copilot.bridge_protocol must be a number"
    );
    assert!(
        v["copilot"]["min_compatible_plugin_version"].is_string(),
        "copilot.min_compatible_plugin_version must be a string"
    );
}

#[test]
fn workflow_and_cargo_agree_on_the_tag_convention() {
    // The release workflow derives asset versions from the git tag (github.ref_name)
    // and guards that the tag matches Cargo.toml's version. Assert the guard exists so
    // a future edit that removes it is caught here.
    let release = read(".github/workflows/release.yml");
    assert!(
        release.contains("Verify tag matches Cargo.toml version"),
        "release.yml must retain the tag<->Cargo.toml version-consistency guard"
    );
    assert!(
        Path::new(&repo_root().join("Cargo.toml")).exists(),
        "Cargo.toml must exist for the version guard to read"
    );
}
