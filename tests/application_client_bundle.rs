use sha2::{Digest, Sha256};
use std::path::Path;

#[test]
fn application_client_bundle_matches_file_bytes() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let manifest_path = root.join("docs/design/application-client.bundle.json");
    let manifest_bytes = std::fs::read(&manifest_path).unwrap();
    let manifest: serde_json::Value = serde_json::from_slice(&manifest_bytes).unwrap();
    let manifest_hash = Sha256::digest(&manifest_bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    assert_eq!(
        manifest_hash,
        "5079239067d8c4944a3396897616113b00fadbcd6e6495b21a3f636255cdd623"
    );
    let actual_paths = manifest["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|file| file["path"].as_str().unwrap())
        .collect::<std::collections::BTreeSet<_>>();
    let expected_paths = [
        "docs/design/DECISIONS.md",
        "docs/design/application-client.md",
        "docs/design/history/application-client-issue-12-original.md",
        "docs/design/index.md",
        "docs/notes/application-client/requirements-crosswalk.md",
    ]
    .into_iter()
    .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(actual_paths, expected_paths);

    for file in manifest["files"].as_array().unwrap() {
        let relative = file["path"].as_str().unwrap();
        let expected_length = file["byteLength"].as_u64().unwrap();
        let expected_hash = file["sha256"].as_str().unwrap();
        let bytes = std::fs::read(root.join(relative)).unwrap();
        let actual_hash = Sha256::digest(&bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        assert_eq!(
            bytes.len() as u64,
            expected_length,
            "{relative} byte length drifted; regenerate application-client.bundle.json"
        );
        assert_eq!(
            actual_hash, expected_hash,
            "{relative} SHA-256 drifted; regenerate application-client.bundle.json"
        );
    }
}
