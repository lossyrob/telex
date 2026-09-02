use serde_json::Value;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;

const FIXTURE_MANIFEST: &str = "tests/fixtures/application-client-consumer/Cargo.toml";

#[test]
fn supported_consumer_profiles_resolve_exact_root_features() {
    for (profile, expected) in [
        ("sqlite", &["sqlite"][..]),
        ("postgres", &["postgres"][..]),
        ("entra", &["entra", "postgres"][..]),
        ("sqlite,postgres", &["postgres", "sqlite"][..]),
        ("sqlite,entra", &["entra", "postgres", "sqlite"][..]),
    ] {
        let actual = resolved_root_features(profile);
        assert_eq!(
            actual,
            expected.iter().map(|feature| feature.to_string()).collect(),
            "unexpected root telex features for {profile}"
        );
        assert!(
            !actual.contains("self-update"),
            "{profile} unexpectedly enabled self-update"
        );
    }
}

fn resolved_root_features(profile: &str) -> BTreeSet<String> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let output = Command::new(env!("CARGO"))
        .current_dir(&root)
        .args([
            "metadata",
            "--format-version",
            "1",
            "--manifest-path",
            FIXTURE_MANIFEST,
            "--no-default-features",
            "--features",
            profile,
        ])
        .output()
        .expect("run cargo metadata for application-client fixture");
    assert!(
        output.status.success(),
        "cargo metadata failed for {profile}: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let metadata: Value = serde_json::from_slice(&output.stdout).expect("parse cargo metadata");
    let root_manifest = canonical(&root.join("Cargo.toml"));
    let package = metadata["packages"]
        .as_array()
        .expect("metadata packages")
        .iter()
        .find(|package| {
            package["name"].as_str() == Some("telex")
                && package["manifest_path"]
                    .as_str()
                    .is_some_and(|path| canonical(Path::new(path)) == root_manifest)
        })
        .expect("root telex package in fixture metadata");
    let package_id = package["id"].as_str().expect("root telex package id");
    metadata["resolve"]["nodes"]
        .as_array()
        .expect("metadata resolve nodes")
        .iter()
        .find(|node| node["id"].as_str() == Some(package_id))
        .expect("root telex resolve node")["features"]
        .as_array()
        .expect("root telex resolved features")
        .iter()
        .map(|feature| feature.as_str().expect("feature name").to_string())
        .collect()
}

fn canonical(path: &Path) -> PathBuf {
    path.canonicalize()
        .unwrap_or_else(|error| panic!("canonicalize {}: {error}", path.display()))
}
