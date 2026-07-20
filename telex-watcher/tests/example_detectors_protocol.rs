#[path = "../src/protocol.rs"]
#[allow(dead_code)]
mod protocol;

use serde_json::{json, Value};
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

fn examples_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("examples")
        .join("detectors")
}

fn run_detector(
    script: &str,
    parameters: Value,
    state: Value,
) -> (Value, protocol::ValidatedResult) {
    let script_path = examples_root().join("scripts").join(script);
    let request = json!({
        "schemaVersion": 1,
        "attempt": { "id": "rust-fixture-attempt", "now": "2026-07-19T00:00:00Z" },
        "watch": { "id": "rust-fixture-watch", "parameters": parameters },
        "script": { "mode": "follow-path", "sha256": "fixture" },
        "state": state,
    });
    let mut child = Command::new("pwsh")
        .args(["-NoLogo", "-NoProfile", "-File"])
        .arg(script_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start PowerShell detector");
    child
        .stdin
        .take()
        .expect("detector stdin")
        .write_all(request.to_string().as_bytes())
        .expect("write detector request");
    let output = child.wait_with_output().expect("wait for detector");
    assert!(
        output.status.success(),
        "{script} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let raw: Value = serde_json::from_slice(&output.stdout).expect("detector JSON");
    let parsed = protocol::parse_result(&output.stdout).expect("version-1 protocol result");
    (raw, parsed)
}

fn assert_initial_event_then_idle(script: &str, parameters: Value) -> Value {
    let (first_raw, first) = run_detector(script, parameters.clone(), json!({}));
    assert_eq!(first_raw["outcome"], "event", "{script} initial snapshot");
    assert!(first.event.is_some(), "{script} event should parse");
    let state = first_raw["nextState"].clone();
    assert!(state["cursor"].is_string(), "{script} opaque cursor");

    let (second_raw, second) = run_detector(script, parameters, state.clone());
    assert_eq!(second_raw["outcome"], "idle", "{script} replay suppression");
    assert!(second.event.is_none(), "{script} idle should parse");
    assert_eq!(second_raw["nextState"], state, "{script} stable cursor");
    first_raw
}

#[test]
fn fixture_detectors_emit_protocol_results_and_suppress_replay() {
    let fixtures = examples_root().join("fixtures");
    let github = assert_initial_event_then_idle(
        "gh-pr-detector.ps1",
        json!({
            "fixturePath": fixtures.join("github-pr-ready.json"),
            "repository": "OWNER/REPOSITORY",
            "emitInitialSnapshot": true,
        }),
    );
    assert_eq!(
        github["event"]["kind"],
        "github.pull-request.ready-to-merge"
    );
    let github_snapshot = assert_initial_event_then_idle(
        "gh-pr-detector.ps1",
        json!({
            "fixturePath": fixtures.join("github-pr-neutral.json"),
            "repository": "OWNER/REPOSITORY",
            "emitInitialSnapshot": true,
        }),
    );
    assert_eq!(
        github_snapshot["event"]["kind"],
        "github.pull-request.snapshot"
    );
    let (github_baseline, _) = run_detector(
        "gh-pr-detector.ps1",
        json!({
            "fixturePath": fixtures.join("github-pr-neutral.json"),
            "repository": "OWNER/REPOSITORY",
            "emitInitialSnapshot": false,
        }),
        json!({}),
    );
    assert_eq!(github_baseline["outcome"], "idle");

    let custom = assert_initial_event_then_idle(
        "gh-pr-external-activity-detector.ps1",
        json!({
            "fixturePath": fixtures.join("github-pr-external-activity.json"),
            "selfLogin": "self-login",
            "ignoredLogins": ["example-bot"],
            "emitInitialSnapshot": true,
        }),
    );
    assert_eq!(
        custom["event"]["metadata"]["externalReviews"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        custom["event"]["metadata"]["externalReviews"][0]["author"],
        "external-reviewer"
    );
    assert_eq!(
        custom["event"]["metadata"]["externalComments"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        custom["event"]["metadata"]["externalComments"][0]["author"],
        "external-commenter"
    );

    let azure = assert_initial_event_then_idle(
        "azure-devops-pr-detector.ps1",
        json!({
            "fixturePath": fixtures.join("azure-devops-pr-ready.json"),
            "emitInitialSnapshot": true,
        }),
    );
    assert_eq!(
        azure["event"]["kind"],
        "azure-devops.pull-request.ready-to-merge"
    );
    let azure_snapshot = assert_initial_event_then_idle(
        "azure-devops-pr-detector.ps1",
        json!({
            "fixturePath": fixtures.join("azure-devops-pr-neutral.json"),
            "emitInitialSnapshot": true,
        }),
    );
    assert_eq!(
        azure_snapshot["event"]["kind"],
        "azure-devops.pull-request.snapshot"
    );
    let azure_created = assert_initial_event_then_idle(
        "azure-devops-pr-detector.ps1",
        json!({
            "fixturePath": fixtures.join("azure-devops-pr-neutral.json"),
            "emitInitialCreatedEvent": true,
        }),
    );
    assert_eq!(
        azure_created["event"]["kind"],
        "azure-devops.pull-request.created"
    );
    let (azure_baseline, _) = run_detector(
        "azure-devops-pr-detector.ps1",
        json!({
            "fixturePath": fixtures.join("azure-devops-pr-neutral.json"),
            "emitInitialSnapshot": false,
        }),
        json!({}),
    );
    assert_eq!(azure_baseline["outcome"], "idle");

    let (ambiguous_auth, ambiguous_auth_parsed) = run_detector(
        "azure-devops-pr-detector.ps1",
        json!({
            "organization": "AZURE-DEVOPS-ORGANIZATION",
            "project": "AZURE-DEVOPS-PROJECT",
            "repositoryId": "AZURE-DEVOPS-REPOSITORY-ID",
            "pullRequestId": 123,
            "allowBearerAuthentication": true,
            "allowPatAuthentication": true,
        }),
        json!({}),
    );
    assert_eq!(ambiguous_auth["outcome"], "degraded");
    assert!(ambiguous_auth_parsed.event.is_none());

    let file = assert_initial_event_then_idle(
        "file-json-detector.ps1",
        json!({
            "inputPath": fixtures.join("file-json-ready.json"),
            "readyField": "ready",
            "emitInitialSnapshot": true,
        }),
    );
    assert_eq!(file["event"]["kind"], "local.file-json.ready");
}
