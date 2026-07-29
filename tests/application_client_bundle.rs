use sha2::{Digest, Sha256};
use std::path::Path;

#[test]
fn application_client_bundle_matches_file_bytes() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let manifest_path = root.join("docs/design/application-client.bundle.json");
    let manifest_bytes = std::fs::read(&manifest_path).unwrap();
    let manifest: serde_json::Value = serde_json::from_slice(&manifest_bytes).unwrap();

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
