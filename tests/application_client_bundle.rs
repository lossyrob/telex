use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::path::Path;

const EXPECTED_MANIFEST_BYTE_LENGTH: usize = 2_423;
const EXPECTED_MANIFEST_SHA256: &str =
    "23af4f331aed82480cb41da9ef827328d4d1a0ea65183f5c27f4cac59157c286";
const EXPECTED_PATHS: [&str; 5] = [
    "docs/design/DECISIONS.md",
    "docs/design/application-client.md",
    "docs/design/history/application-client-issue-12-original.md",
    "docs/design/index.md",
    "docs/notes/application-client/requirements-crosswalk.md",
];
const APPLICATION_CLIENT_OWNED_PATHS: [&str; 3] = [
    "docs/design/application-client.md",
    "docs/notes/application-client/requirements-crosswalk.md",
    "docs/design/history/application-client-issue-12-original.md",
];

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn is_lowercase_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn manifest_file<'a>(files: &'a [serde_json::Value], path: &str) -> &'a serde_json::Value {
    files
        .iter()
        .find(|file| file["path"].as_str() == Some(path))
        .unwrap_or_else(|| panic!("manifest is missing required path {path}"))
}

#[test]
fn application_client_bundle_matches_file_bytes() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let manifest_path = root.join("docs/design/application-client.bundle.json");
    let manifest_bytes = std::fs::read(&manifest_path).unwrap();
    let manifest: serde_json::Value = serde_json::from_slice(&manifest_bytes).unwrap();
    assert_eq!(manifest_bytes.len(), EXPECTED_MANIFEST_BYTE_LENGTH);
    assert_eq!(sha256_hex(&manifest_bytes), EXPECTED_MANIFEST_SHA256);
    assert_eq!(manifest["schemaVersion"], 1);
    assert_eq!(manifest["checkpointScope"], "design-only");

    let files = manifest["files"].as_array().unwrap();
    assert_eq!(files.len(), EXPECTED_PATHS.len());
    let mut actual_paths = BTreeSet::new();
    for file in files {
        let entry = file
            .as_object()
            .expect("manifest file entry must be an object");
        assert_eq!(entry.len(), 3, "manifest file entry has unexpected fields");
        let relative = file["path"]
            .as_str()
            .expect("manifest file path must be a string");
        assert!(
            actual_paths.insert(relative),
            "manifest contains duplicate path {relative}"
        );
        assert!(
            file["byteLength"].as_u64().is_some(),
            "{relative} byteLength must be an unsigned integer"
        );
        let expected_hash = file["sha256"]
            .as_str()
            .expect("manifest file sha256 must be a string");
        assert!(
            is_lowercase_sha256(expected_hash),
            "{relative} sha256 must be 64 lowercase hexadecimal characters"
        );
    }
    let expected_paths = EXPECTED_PATHS.into_iter().collect::<BTreeSet<_>>();
    assert_eq!(actual_paths, expected_paths);

    let historical = manifest["historicalIssue12"]
        .as_object()
        .expect("historicalIssue12 must be an object");
    assert_eq!(
        historical.len(),
        3,
        "historicalIssue12 has unexpected fields"
    );
    let historical_path = historical["path"]
        .as_str()
        .expect("historicalIssue12 path must be a string");
    let historical_length = historical["byteLength"]
        .as_u64()
        .expect("historicalIssue12 byteLength must be an unsigned integer");
    let historical_hash = historical["sha256"]
        .as_str()
        .expect("historicalIssue12 sha256 must be a string");
    assert!(is_lowercase_sha256(historical_hash));
    let historical_file = manifest_file(files, historical_path);
    assert_eq!(
        historical_file["byteLength"].as_u64(),
        Some(historical_length)
    );
    assert_eq!(historical_file["sha256"].as_str(), Some(historical_hash));

    // Shared design files remain publication-time evidence. Only these owned
    // sources require a controlled publication revision when their bytes change.
    for relative in APPLICATION_CLIENT_OWNED_PATHS {
        let file = manifest_file(files, relative);
        let expected_length = file["byteLength"].as_u64().unwrap();
        let expected_hash = file["sha256"].as_str().unwrap();
        let bytes = std::fs::read(root.join(relative)).unwrap();
        assert_eq!(
            bytes.len() as u64,
            expected_length,
            "{relative} byte length drifted; an Application Client-owned source change requires a controlled publication revision"
        );
        assert_eq!(
            sha256_hex(&bytes),
            expected_hash,
            "{relative} SHA-256 drifted; an Application Client-owned source change requires a controlled publication revision"
        );
    }
}
