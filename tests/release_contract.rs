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
use std::path::PathBuf;
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
        "release.yml build matrix should declare at least one `- target:` entry; \
         parser drift (e.g. a switch to YAML flow style) would zero this out"
    );
    targets
}

/// Extract `[package].version` from a Cargo.toml string, scoped to the `[package]`
/// section (robust to a future `[workspace.package]` block). Returns "" if absent,
/// which callers assert against so workspace-inheritance drift fails loudly.
fn package_version(cargo_toml: &str) -> String {
    let mut in_pkg = false;
    for line in cargo_toml.lines() {
        let t = line.trim_start();
        if t.starts_with('[') {
            in_pkg = t.starts_with("[package]");
            continue;
        }
        if in_pkg {
            if let Some(rest) = t.strip_prefix("version") {
                if let Some(val) = rest.trim_start().strip_prefix('=') {
                    return val.trim().trim_matches('"').to_string();
                }
            }
        }
    }
    String::new()
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
    let sh = install_sh_targets();
    let ps1 = install_ps1_targets();
    // Fail loudly if either extractor stops matching (e.g. a quoting reformat in an
    // installer) instead of silently degrading the subset check into a no-op.
    assert!(
        !sh.is_empty(),
        "install.sh target extraction returned nothing; the parser drifted from the file"
    );
    assert!(
        !ps1.is_empty(),
        "install.ps1 target extraction returned nothing; the parser drifted from the file"
    );
    // Pin the specific targets each installer must be able to fetch. This catches a
    // *single dropped arm* (e.g. removing the macOS Intel case), which the non-empty
    // guard above alone would miss.
    for expected in [
        "x86_64-unknown-linux-gnu",
        "aarch64-apple-darwin",
        "x86_64-apple-darwin",
    ] {
        assert!(
            sh.iter().any(|t| t == expected),
            "install.sh no longer resolves target `{expected}`; a platform arm was dropped"
        );
    }
    for expected in ["x86_64-pc-windows-msvc", "aarch64-pc-windows-msvc"] {
        assert!(
            ps1.iter().any(|t| t == expected),
            "install.ps1 no longer resolves target `{expected}`; a platform arm was dropped"
        );
    }
    for t in sh.iter().chain(ps1.iter()) {
        assert!(
            matrix.contains(t),
            "installer target `{t}` is not built by release.yml matrix {matrix:?}; \
             every platform an installer can request must be produced by the release workflow"
        );
    }
}

#[cfg(feature = "self-update")]
#[test]
fn in_binary_upgrade_targets_equal_the_release_matrix() {
    // Assert SET EQUALITY between telex::release::SUPPORTED_TARGETS and the release.yml build
    // matrix, so a matrix change in EITHER direction (a new or removed target) breaks this repo
    // test rather than a user's `telex upgrade`.
    use std::collections::BTreeSet;
    let matrix: BTreeSet<String> = release_matrix_targets().into_iter().collect();
    let supported: BTreeSet<String> = telex::release::SUPPORTED_TARGETS
        .iter()
        .map(|t| t.triple.to_string())
        .collect();
    assert_eq!(
        supported, matrix,
        "telex::release::SUPPORTED_TARGETS and the release.yml build matrix must be identical; \
         a target present in one but not the other means `telex upgrade` and the release workflow \
         disagree on platform support"
    );
    // Every installer-fetched target must be a self-update target too.
    let installers: BTreeSet<String> = install_sh_targets()
        .into_iter()
        .chain(install_ps1_targets())
        .collect();
    assert!(
        installers.is_subset(&supported),
        "installers fetch targets not covered by the in-binary self-update set: {:?}",
        installers.difference(&supported).collect::<Vec<_>>()
    );
}

#[test]
fn release_publish_verifies_a_checksum_sidecar_for_every_archive() {
    // The in-binary `telex upgrade` release path is fail-closed on a missing checksum. That
    // stance depends on every published archive actually having a `.sha256` sibling.
    // `fail_on_unmatched_files` only asserts the publish glob matched >=1 file, so the release
    // workflow must have an explicit sidecar-presence guard before publish. Assert its behavior.
    let release = read(".github/workflows/release.yml");
    assert!(
        release.contains("missing checksum sidecar for"),
        "release.yml must fail the publish job when an archive lacks a .sha256 sidecar; the \
         fail-closed in-binary upgrader depends on this guarantee"
    );
    assert!(
        release.contains("archive for orphan sidecar"),
        "release.yml must also fail publish on a lone .sha256 with no matching archive \
         (bidirectional pairing), so a packaging slip cannot strand a platform"
    );
    assert!(
        release.contains("${archive}.sha256"),
        "the sidecar guard must check for a same-name `${{archive}}.sha256` sibling"
    );
}

#[test]
fn source_metadata_invocation_grammar_is_stable() {
    // `telex upgrade` reads the downloaded binary's metadata by spawning
    // `telex --json version --root <path>`. That invocation grammar is a cross-version
    // contract (an older telex installs a newer one this way); assert it stays valid and
    // exposes every field the upgrade path reads.
    let bin = telex_bin();
    let tmp = std::env::temp_dir().join(format!("telex-argv-contract-{}", std::process::id()));
    let output = Command::new(&bin)
        .args(["--json", "version", "--root"])
        .arg(&tmp)
        .env("TELEX_LAUNCHER_ACTIVE", "1")
        .output()
        .expect("run telex --json version --root <path>");
    assert!(
        output.status.success(),
        "`telex --json version --root <path>` must succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let v: Value = serde_json::from_slice(&output.stdout).expect("valid version JSON");
    // Fields consumed by src/commands/upgrade.rs::source_metadata.
    assert!(v["version"]["package_version"].is_string());
    assert!(v["version"]["build_id"]
        .as_str()
        .is_some_and(|build_id| !build_id.is_empty()));
    assert!(v["version"]["supported_schema_min"].is_number());
    assert!(v["version"]["supported_schema_max"].is_number());
    assert!(v["daemon_metadata"]["protocol_version"]["major"].is_number());
    assert!(v["daemon_metadata"]["protocol_version"]["minor"].is_number());
    assert!(v["daemon_metadata"]["required_capabilities"].is_array());
    assert!(v["copilot"]["bridge_protocol"].is_number());
    assert!(v["copilot"]["min_compatible_plugin_version"].is_string());
    std::fs::remove_dir_all(&tmp).ok();
}

#[test]
fn version_flag_distinguishes_same_semver_builds() {
    let output = Command::new(telex_bin())
        .arg("--version")
        .output()
        .expect("run telex --version");
    assert!(
        output.status.success(),
        "telex --version exited with failure: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(
        text.contains(env!("CARGO_PKG_VERSION")),
        "version flag must retain the package version: {text:?}"
    );
    assert!(
        text.contains(env!("TELEX_BUILD_ID")),
        "version flag must expose the build identifier: {text:?}"
    );
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

    // The publish job's upload path and release `files:` glob must key off the same
    // telex-<version>-<target> prefix, so the build->publish artifact hand-off can
    // never silently ship an empty/partial asset set past fail_on_unmatched_files.
    // The build job sanitizes the ref (slash-safe for workflow_dispatch dry-runs);
    // on a real tag the sanitized version equals github.ref_name that publish uses.
    assert!(
        release.contains("path: dist/telex-${{ steps.ver.outputs.v }}-${{ matrix.target }}.${{ matrix.archive }}*"),
        "build must upload artifacts under the sanitized telex-<version>-<target> prefix"
    );
    assert!(
        release.contains("dist/telex-${{ github.ref_name }}-*"),
        "publish must select release assets by the telex-<ref_name>- prefix"
    );
    assert!(
        release.contains(r#"echo "v=${GITHUB_REF_NAME//\//-}""#),
        "build must sanitize '/' out of the ref for asset naming so workflow_dispatch \
         dry-runs on slash-containing branches still package correctly"
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
    // Cargo builds the `telex` binary before running integration tests and sets this
    // env var at compile time. Using env! (not option_env!) means a missing bin
    // target fails to compile rather than letting the runtime contract check
    // silently skip.
    PathBuf::from(env!("CARGO_BIN_EXE_telex"))
}

#[test]
fn version_json_exposes_the_upgrade_contract_surface() {
    // Lock the `telex version --json` keys that the downstream release-upgrade node
    // consumes. Renaming or dropping one of these is a breaking change to that
    // contract and must be caught here, not by the future upgrade implementer.
    let bin = telex_bin();
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
    let build_id = v["version"]["build_id"]
        .as_str()
        .expect("version.build_id must be a string");
    assert!(
        !build_id.is_empty(),
        "version.build_id must be a non-empty string"
    );
    assert_eq!(
        build_id,
        env!("TELEX_BUILD_ID"),
        "version.build_id must identify the binary compiled for this test"
    );
    let text_output = Command::new(&bin)
        .args(["--text", "version"])
        .output()
        .expect("run telex version");
    assert!(
        text_output.status.success(),
        "telex version exited with failure: {}",
        String::from_utf8_lossy(&text_output.stderr)
    );
    let text = String::from_utf8_lossy(&text_output.stdout);
    assert!(
        text.lines().any(|line| line == format!("build {build_id}")),
        "telex version text must expose the JSON build id; output was {text:?}"
    );
    // The install root is always a concrete path; assert a non-null string, not mere
    // key presence (a JSON null would satisfy is_some but break upgrade tooling).
    assert!(
        v["version"]["install"]["root"].is_string(),
        "version.install.root must be a non-null string"
    );
    // These tag/binary fields are legitimately null before the first versioned
    // install, so assert presence only.
    for key in ["current_tag", "previous_tag", "active_tag"] {
        assert!(
            v["version"]["install"].get(key).is_some(),
            "version.install.{key} must be present for upgrade tooling"
        );
    }
    // Schema-compatibility window the upgrade path checks before switching binaries.
    assert!(
        v["version"]["supported_schema_min"].is_number(),
        "version.supported_schema_min must be a number for upgrade compatibility checks"
    );
    assert!(
        v["version"]["supported_schema_max"].is_number(),
        "version.supported_schema_max must be a number for upgrade compatibility checks"
    );
    // Daemon protocol handshake surface.
    assert!(
        v["daemon_metadata"]["protocol_version"]["major"].is_number(),
        "daemon_metadata.protocol_version.major must be a number"
    );
    assert!(
        v["daemon_metadata"]["protocol_version"]["minor"].is_number(),
        "daemon_metadata.protocol_version.minor must be a number"
    );
    assert!(
        v["daemon_metadata"]["required_capabilities"]
            .as_array()
            .is_some_and(|c| !c.is_empty()),
        "daemon_metadata.required_capabilities must be a non-empty array (upgrade compat)"
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
fn release_workflow_enforces_the_version_and_publish_guards() {
    // Assert the guard BEHAVIOR, not just a job display name: a flipped condition,
    // a broken extraction, or a dropped tag-gate must fail this test.
    let release = read(".github/workflows/release.yml");

    // The version-consistency guard derives the version from the tag and fails on
    // mismatch.
    assert!(
        release.contains("GITHUB_REF_NAME#v"),
        "version guard must derive the release version from the tag (GITHUB_REF_NAME#v)"
    );
    assert!(
        release.contains(r#"[ "${tag}" != "${crate}" ]"#),
        "version guard must fail-branch when the tag does not equal the crate version \
         (a flipped `!=`->`=` would defeat the guard)"
    );
    assert!(
        release.contains("exit 1"),
        "version guard must exit non-zero on mismatch"
    );

    // verify-version and publish must both be gated to version tags, so a branch
    // workflow_dispatch never publishes a branch-named release.
    let tag_guard = "startsWith(github.ref, 'refs/tags/v')";
    assert_eq!(
        release.matches(tag_guard).count(),
        2,
        "exactly the verify-version and publish jobs must be gated with {tag_guard:?}"
    );

    // Both gates must also require a push event, so a workflow_dispatch pointed at a
    // tag ref stays a build-only dry-run and never publishes.
    assert_eq!(
        release.matches("github.event_name == 'push'").count(),
        2,
        "verify-version and publish must also require github.event_name == 'push' \
         so a workflow_dispatch on a tag ref cannot publish"
    );

    // publish must run only after verify-version AND the whole build matrix, so a
    // mismatched or partial build cannot publish.
    assert!(
        release.contains("needs: [verify-version, build]"),
        "publish must depend on [verify-version, build]"
    );

    assert!(
        release.contains("TELEX_BUILD_ID: ${{ github.sha }}"),
        "release builds must embed the exact release commit as the build identifier"
    );
    assert!(
        release.contains("Verify release binary build identity")
            && release.contains("metadata.version.build_id")
            && release.contains("does not match GITHUB_SHA"),
        "release workflow must reject an archive candidate whose binary build_id is not GITHUB_SHA"
    );

    // The [package].version the guard reads must be present and semver-shaped; if a
    // future refactor moved it to workspace inheritance this extraction would empty
    // out and fail here.
    let pkg = package_version(&read("Cargo.toml"));
    assert_eq!(
        pkg.split('.').count(),
        3,
        "Cargo.toml [package].version should be X.Y.Z, got {pkg:?}"
    );
}

/// Replicates the release.yml `awk` extraction of `[package].version` in Rust:
/// section-scoped, then strip quotes/whitespace and the `version=` prefix. Kept
/// deliberately separate from `package_version` so the parser-agreement test can
/// prove the two independent parsers resolve the same value on the real Cargo.toml.
fn awk_style_package_version(cargo_toml: &str) -> String {
    let mut in_pkg = false;
    for line in cargo_toml.lines() {
        if line.starts_with('[') {
            in_pkg = line.starts_with("[package]");
            continue;
        }
        // awk match: /^version[[:space:]]*=/
        let trimmed_key = line.trim_start_matches("version");
        let is_version_line =
            line.starts_with("version") && trimmed_key.trim_start().starts_with('=');
        if in_pkg && is_version_line {
            // gsub(/["[:space:]]/,"") then sub(/^version=/,"")
            let collapsed: String = line
                .chars()
                .filter(|c| *c != '"' && !c.is_whitespace())
                .collect();
            return collapsed.trim_start_matches("version=").to_string();
        }
    }
    String::new()
}

#[test]
fn workflow_awk_and_rust_version_parsers_agree() {
    // The release.yml awk gate and the Rust `package_version` helper are two
    // independent parsers of the same fact. If they diverge, the CI gate and this
    // test would disagree -- false confidence. Assert they resolve identically on the
    // real Cargo.toml, and that the result is non-empty (workspace-inheritance drift
    // would empty both).
    let cargo = read("Cargo.toml");
    let rust = package_version(&cargo);
    let awk = awk_style_package_version(&cargo);
    assert!(
        !rust.is_empty(),
        "Rust package_version parser resolved nothing"
    );
    assert_eq!(
        rust, awk,
        "the awk gate in release.yml and the Rust parser disagree on [package].version \
         ({awk:?} vs {rust:?}); keep the two extractions equivalent"
    );
}

#[test]
fn plugin_versions_track_the_crate_version() {
    // The Copilot plugin advertises a version that must move in lockstep with the
    // binary (version-matched plugin/binary compatibility). The release runbook
    // lists bumping these as a pre-cut step; enforce it here so a Cargo.toml bump
    // that forgets the plugin fails in CI, not at a user's `copilot plugin install`.
    let crate_version = package_version(&read("Cargo.toml"));
    assert!(
        !crate_version.is_empty(),
        "could not read [package].version from Cargo.toml"
    );

    let plugin: Value =
        serde_json::from_str(&read("copilot/plugin/plugin.json")).expect("plugin.json parses");
    assert_eq!(
        plugin["version"].as_str(),
        Some(crate_version.as_str()),
        "copilot/plugin/plugin.json version must match Cargo.toml [package].version"
    );

    let market: Value = serde_json::from_str(&read(".github/plugin/marketplace.json"))
        .expect("marketplace.json parses");
    assert_eq!(
        market["metadata"]["version"].as_str(),
        Some(crate_version.as_str()),
        "marketplace.json metadata.version must match Cargo.toml [package].version"
    );
    assert_eq!(
        market["plugins"][0]["version"].as_str(),
        Some(crate_version.as_str()),
        "marketplace.json plugin version must match Cargo.toml [package].version"
    );

    // The bootstrap skill's `--plugin-version <X>` example is a user-facing version
    // string; keep it in lockstep so a bump cannot leave a stale example behind.
    let skill = read("copilot/plugin/skills/telex/SKILL.md");
    assert!(
        skill.contains(&format!("--plugin-version {crate_version}")),
        "copilot/plugin/skills/telex/SKILL.md `--plugin-version` example must match \
         Cargo.toml [package].version ({crate_version})"
    );
}
