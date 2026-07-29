#[path = "../src/protocol.rs"]
#[allow(dead_code)]
mod protocol;

use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

const TEMPLATE_IDS: [&str; 6] = [
    "github-pr",
    "github-pr-external-activity",
    "azure-devops-pr",
    "http-json",
    "local-file-json",
    "local-command",
];

fn watcher_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn repository_root() -> PathBuf {
    watcher_root()
        .parent()
        .expect("telex-watcher has repository parent")
        .to_path_buf()
}

fn templates_root() -> PathBuf {
    watcher_root().join("templates")
}

fn read_json(path: &Path) -> Value {
    serde_json::from_slice(&fs::read(path).unwrap_or_else(|error| {
        panic!("read {}: {error}", path.display());
    }))
    .unwrap_or_else(|error| panic!("parse {}: {error}", path.display()))
}

fn path_text(path: impl AsRef<Path>) -> String {
    path.as_ref().to_string_lossy().into_owned()
}

fn sha256_file(path: &Path) -> String {
    let bytes = fs::read(path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn schema(name: &str) -> Value {
    read_json(
        &repository_root()
            .join("docs")
            .join("design")
            .join("schemas")
            .join(name),
    )
}

fn manifest(template_id: &str) -> Value {
    read_json(&templates_root().join(template_id).join("manifest.json"))
}

fn assert_valid(schema: &Value, instance: &Value, label: &str) {
    let validator = jsonschema::draft202012::options()
        .should_validate_formats(true)
        .build(schema)
        .unwrap_or_else(|error| panic!("{label} schema compilation failed: {error}"));
    if let Err(error) = validator.validate(instance) {
        panic!("{label} failed schema validation: {error}");
    }
}

fn is_valid(schema: &Value, instance: &Value) -> bool {
    jsonschema::draft202012::options()
        .should_validate_formats(true)
        .build(schema)
        .expect("compile JSON Schema")
        .is_valid(instance)
}

struct DetectorRun {
    result: Value,
    parsed: protocol::ValidatedResult,
    stderr: String,
}

fn run_detector(template_id: &str, parameters: Value, state: Value) -> DetectorRun {
    run_detector_with_env(template_id, parameters, state, &[])
}

fn inherit_test_baseline(command: &mut Command) {
    for key in [
        "PATH",
        "PATHEXT",
        "SystemRoot",
        "SystemDrive",
        "WINDIR",
        "ComSpec",
        "HOME",
        "USERPROFILE",
        "HOMEDRIVE",
        "HOMEPATH",
        "APPDATA",
        "LOCALAPPDATA",
        "TMP",
        "TEMP",
        "TMPDIR",
        "LANG",
        "LC_ALL",
    ] {
        if let Some(value) = std::env::var_os(key) {
            command.env(key, value);
        }
    }
}

fn run_detector_with_env(
    template_id: &str,
    parameters: Value,
    state: Value,
    environment: &[(&str, &str)],
) -> DetectorRun {
    let manifest = manifest(template_id);
    let script_path = templates_root().join(
        manifest["librarySource"]["path"]
            .as_str()
            .expect("manifest source path"),
    );
    let request = json!({
        "schemaVersion": 1,
        "attempt": {
            "id": "template-conformance-attempt",
            "now": "2026-07-29T12:00:00Z"
        },
        "watch": {
            "id": format!("template-conformance-{template_id}"),
            "parameters": parameters
        },
        "script": {
            "mode": "pinned",
            "sha256": manifest["librarySource"]["sha256"]
        },
        "state": state
    });
    assert_valid(
        &schema("watcher-detector-request-v1.schema.json"),
        &request,
        &format!("{template_id} request"),
    );

    let mut command = Command::new("pwsh");
    command
        .args(["-NoLogo", "-NoProfile", "-File"])
        .arg(&script_path)
        .current_dir(
            script_path
                .parent()
                .expect("detector script has parent directory"),
        )
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env_clear();
    inherit_test_baseline(&mut command);
    command.env("TELEX_WATCHER_TEST", "1");
    for (name, value) in environment {
        command.env(name, value);
    }
    let mut child = command
        .spawn()
        .unwrap_or_else(|error| panic!("start {}: {error}", script_path.display()));
    child
        .stdin
        .take()
        .expect("detector stdin")
        .write_all(request.to_string().as_bytes())
        .expect("write detector request");
    let output = child.wait_with_output().expect("wait for detector");
    assert!(
        output.status.success(),
        "{template_id} exited nonzero: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).lines().count(),
        1,
        "{template_id} must emit exactly one result line"
    );
    let result: Value = serde_json::from_slice(&output.stdout)
        .unwrap_or_else(|error| panic!("{template_id} result JSON: {error}"));
    assert_valid(
        &schema("watcher-detector-result-v1.schema.json"),
        &result,
        &format!("{template_id} result"),
    );
    let parsed = protocol::parse_result(&output.stdout)
        .unwrap_or_else(|error| panic!("{template_id} runtime parser: {error}"));
    DetectorRun {
        result,
        parsed,
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

fn fixture(template_id: &str, name: &str) -> String {
    path_text(
        templates_root()
            .join(template_id)
            .join("fixtures")
            .join(name),
    )
}

fn registration(template_id: &str, sample: &str) -> Value {
    read_json(
        &templates_root()
            .join(template_id)
            .join("registrations")
            .join(sample),
    )
}

fn primary_parameters(template_id: &str) -> Value {
    let mut parameters = registration(template_id, "pinned.json")["parameters"].clone();
    let object = parameters.as_object_mut().expect("registration parameters");
    match template_id {
        "github-pr" => {
            object.insert(
                "fixturePath".into(),
                fixture(template_id, "github-pr-ready.json").into(),
            );
            object.insert("repository".into(), "example/repo".into());
            object.insert("pullRequestNumber".into(), 42.into());
        }
        "github-pr-external-activity" => {
            object.insert(
                "fixturePath".into(),
                fixture(template_id, "activity.json").into(),
            );
            object.insert("repository".into(), "example/repo".into());
            object.insert("pullRequestNumber".into(), 43.into());
            object.insert("selfLogin".into(), "self-login".into());
        }
        "azure-devops-pr" => {
            object.insert(
                "fixturePath".into(),
                fixture(template_id, "azure-devops-pr-ready.json").into(),
            );
            object.insert("organization".into(), "example-organization".into());
            object.insert("project".into(), "example-project".into());
            object.insert("repositoryId".into(), "example-repository".into());
            object.insert("pullRequestId".into(), 73.into());
        }
        "http-json" => {
            object.insert(
                "fixturePath".into(),
                fixture(template_id, "condition-met.json").into(),
            );
            object.insert("sourceId".into(), "example".into());
            object.insert("authentication".into(), "none".into());
        }
        "local-file-json" => {
            object.insert(
                "inputPath".into(),
                fixture(template_id, "ready.json").into(),
            );
            object.insert("sourceId".into(), "example".into());
        }
        "local-command" => {
            object.insert(
                "command".into(),
                json!([
                    "pwsh",
                    "-NoLogo",
                    "-NoProfile",
                    "-File",
                    fixture(template_id, "condition-met.ps1")
                ]),
            );
            object.insert(
                "workingDirectory".into(),
                path_text(templates_root().join(template_id).join("fixtures")).into(),
            );
            object.insert("sourceId".into(), "example".into());
        }
        _ => unreachable!(),
    }
    parameters
}

fn event_variants(template_id: &str) -> Vec<Value> {
    match template_id {
        "github-pr" => vec![
            primary_parameters(template_id),
            json!({
                "fixturePath": fixture(template_id, "github-pr-neutral.json"),
                "repository": "example/repo",
                "emitInitialSnapshot": true
            }),
            json!({
                "fixturePath": fixture(template_id, "github-pr-attention.json"),
                "repository": "example/repo"
            }),
            json!({
                "fixturePath": fixture(template_id, "github-pr-terminal.json"),
                "repository": "example/repo"
            }),
        ],
        "github-pr-external-activity" => vec![primary_parameters(template_id)],
        "azure-devops-pr" => vec![
            primary_parameters(template_id),
            json!({
                "fixturePath": fixture(template_id, "azure-devops-pr-neutral.json"),
                "organization": "example-organization",
                "project": "example-project",
                "repositoryId": "example-repository",
                "emitInitialSnapshot": true,
                "emitInitialCreatedEvent": false
            }),
            json!({
                "fixturePath": fixture(template_id, "azure-devops-pr-neutral.json"),
                "organization": "example-organization",
                "project": "example-project",
                "repositoryId": "example-repository",
                "emitInitialCreatedEvent": true,
                "emitInitialSnapshot": false
            }),
            json!({
                "fixturePath": fixture(template_id, "azure-devops-pr-attention.json"),
                "organization": "example-organization",
                "project": "example-project",
                "repositoryId": "example-repository"
            }),
            json!({
                "fixturePath": fixture(template_id, "azure-devops-pr-terminal.json"),
                "organization": "example-organization",
                "project": "example-project",
                "repositoryId": "example-repository"
            }),
        ],
        _ => vec![primary_parameters(template_id)],
    }
}

fn json_string_set(value: &Value) -> BTreeSet<String> {
    value
        .as_array()
        .expect("JSON array")
        .iter()
        .map(|item| item.as_str().expect("string item").to_owned())
        .collect()
}

#[test]
fn canonical_schema_validator_enforces_load_bearing_constraints() {
    let request_schema = schema("watcher-detector-request-v1.schema.json");
    let result_schema = schema("watcher-detector-result-v1.schema.json");
    let request = json!({
        "schemaVersion": 1,
        "attempt": {"id": "attempt", "now": "2026-07-29T12:00:00Z"},
        "watch": {"id": "watch", "parameters": {}},
        "script": {"mode": "pinned", "sha256": "0".repeat(64)},
        "state": {}
    });
    assert_valid(&request_schema, &request, "canonical request");
    let mut unknown_request = request.clone();
    unknown_request["unexpected"] = Value::Bool(true);
    assert!(!is_valid(&request_schema, &unknown_request));

    assert_valid(
        &result_schema,
        &json!({"schemaVersion": 1, "outcome": "idle", "nextState": {}}),
        "canonical idle result",
    );
    assert!(!is_valid(
        &result_schema,
        &json!({"schemaVersion": 1, "outcome": "event"})
    ));
    assert!(!is_valid(
        &result_schema,
        &json!({"schemaVersion": 1, "outcome": "degraded", "nextState": {}})
    ));
    assert_valid(
        &json!({"type": "string", "pattern": "^provider-[0-9]{2}$"}),
        &json!("provider-71"),
        "arbitrary JSON Schema regex",
    );
}

#[test]
fn manifests_and_registration_samples_are_strict_and_consistent() {
    let manifest_schema = read_json(&templates_root().join("manifest.schema.json"));
    let attributes =
        fs::read_to_string(repository_root().join(".gitattributes")).expect("git attributes");
    assert!(
        attributes
            .lines()
            .any(|line| line == "telex-watcher/templates/** text eol=lf"),
        "template product files must have checkout-stable LF bytes"
    );
    let mut product_files = Vec::new();
    collect_files(&templates_root(), &mut product_files);
    for path in product_files {
        let bytes =
            fs::read(&path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
        assert!(
            !bytes.windows(2).any(|pair| pair == b"\r\n"),
            "{} must use LF line endings",
            path.display()
        );
    }
    let required_registration_fields = [
        "id",
        "detectorSchemaVersion",
        "command",
        "scriptPath",
        "workingDirectory",
        "scriptMode",
        "backendProfile",
        "sender",
        "target",
        "attention",
        "requiresDisposition",
        "intervalSeconds",
        "timeoutSeconds",
        "allowedEventKinds",
        "allowedEventKindPrefixes",
        "environmentAllowlist",
        "parameters",
        "initialState",
        "maxSafeDowntimeSeconds",
    ];

    for template_id in TEMPLATE_IDS {
        let manifest = manifest(template_id);
        assert_valid(
            &manifest_schema,
            &manifest,
            &format!("{template_id} manifest"),
        );
        let mut missing = manifest.clone();
        missing
            .as_object_mut()
            .expect("manifest object")
            .remove("templateVersion");
        assert!(!is_valid(&manifest_schema, &missing));
        let mut unknown = manifest.clone();
        unknown["unexpected"] = Value::Bool(true);
        assert!(!is_valid(&manifest_schema, &unknown));
        let mut relocated = manifest.clone();
        relocated["derivedFrom"]["reconciliation"] = json!("guidance/RECONCILE.md");
        assert_valid(
            &manifest_schema,
            &relocated,
            &format!("{template_id} relocated reconciliation"),
        );

        assert_eq!(manifest["templateId"], template_id);
        let source_path = templates_root().join(
            manifest["librarySource"]["path"]
                .as_str()
                .expect("source path"),
        );
        let source_digest = sha256_file(&source_path);
        assert_eq!(manifest["librarySource"]["sha256"], source_digest);
        let source_text = fs::read_to_string(&source_path).expect("detector source text");
        for support in manifest["librarySource"]["supportFiles"]
            .as_array()
            .expect("support files")
        {
            let support_path =
                templates_root().join(support["path"].as_str().expect("support path"));
            let digest = sha256_file(&support_path);
            assert_eq!(support["sha256"], digest);
            assert!(
                source_text.contains(&digest),
                "{template_id} must pin imported helper {digest}"
            );
        }

        let allowed_kinds = json_string_set(&manifest["allowedEventKinds"]);
        let allowed_credentials: BTreeSet<String> = manifest["credentials"]
            ["requiredEnvironmentVariables"]
            .as_array()
            .expect("required environment")
            .iter()
            .chain(
                manifest["credentials"]["optionalEnvironmentVariables"]
                    .as_array()
                    .expect("optional environment"),
            )
            .map(|value| value.as_str().expect("environment name").to_owned())
            .collect();
        for requirement in manifest["credentials"]["conditionalRequirements"]
            .as_array()
            .expect("conditional requirements")
        {
            assert!(
                json_string_set(&requirement["requires"]).is_subset(&allowed_credentials),
                "{template_id} conditionally requires an undeclared credential"
            );
        }

        for (sample, mode) in [
            ("pinned.json", "pinned"),
            ("development.json", "follow-path"),
        ] {
            let registration = read_json(
                &templates_root()
                    .join(template_id)
                    .join("registrations")
                    .join(sample),
            );
            let object = registration.as_object().expect("registration object");
            for field in required_registration_fields {
                assert!(
                    object.contains_key(field),
                    "{template_id}/{sample} missing {field}"
                );
            }
            assert_eq!(registration["scriptMode"], mode);
            assert_eq!(
                registration["command"]
                    .as_array()
                    .expect("command array")
                    .iter()
                    .find(|item| item.as_str() == registration["scriptPath"].as_str()),
                Some(&registration["scriptPath"])
            );
            let registration_kinds = json_string_set(&registration["allowedEventKinds"]);
            assert!(
                registration_kinds.is_subset(&allowed_kinds),
                "{template_id}/{sample} authorizes a kind outside the manifest"
            );
            let mut expected_kinds = allowed_kinds.clone();
            for (parameter, kind) in [
                ("emitInitialSnapshot", "github.pull-request.snapshot"),
                ("emitInitialSnapshot", "azure-devops.pull-request.snapshot"),
                (
                    "emitInitialCreatedEvent",
                    "azure-devops.pull-request.created",
                ),
            ] {
                if expected_kinds.contains(kind)
                    && registration["parameters"][parameter].as_bool() != Some(true)
                {
                    expected_kinds.remove(kind);
                }
            }
            assert_eq!(
                registration_kinds, expected_kinds,
                "{template_id}/{sample} must be least-privilege for synthetic kinds"
            );
            assert!(registration["allowedEventKindPrefixes"]
                .as_array()
                .expect("kind prefixes")
                .is_empty());
            assert_eq!(
                registration["intervalSeconds"],
                manifest["scheduling"]["recommendedIntervalSeconds"]
            );
            assert!(
                registration["intervalSeconds"].as_u64()
                    >= manifest["scheduling"]["minimumIntervalSeconds"].as_u64()
            );
            assert_eq!(
                registration["maxSafeDowntimeSeconds"],
                manifest["scheduling"]["maxSafeDowntimeSeconds"]
            );
            let registration_credentials = json_string_set(&registration["environmentAllowlist"]);
            assert!(
                registration_credentials.is_subset(&allowed_credentials),
                "{template_id}/{sample} uses an undeclared credential"
            );
            for requirement in manifest["credentials"]["conditionalRequirements"]
                .as_array()
                .expect("conditional requirements")
            {
                let parameter = requirement["when"]["parameter"]
                    .as_str()
                    .expect("condition parameter");
                if registration["parameters"][parameter] == requirement["when"]["equals"] {
                    assert!(
                        json_string_set(&requirement["requires"])
                            .is_subset(&registration_credentials),
                        "{template_id}/{sample} omits a conditionally required credential"
                    );
                }
            }
            assert_eq!(registration["scriptPath"], "<DETECTOR-PATH>");
            assert_eq!(registration["workingDirectory"], "<TEMPLATE-DIRECTORY>");
            assert_eq!(registration["initialState"], json!({}));
            assert!(
                !registration.to_string().contains("REPLACE-WITH-"),
                "{template_id}/{sample} contains a stale replacement marker"
            );
            if mode == "pinned" {
                assert_eq!(registration["scriptDigest"], source_digest);
            } else {
                assert!(!object.contains_key("scriptDigest"));
            }
        }
    }
}

#[test]
fn fixture_detectors_emit_declared_kinds_with_stable_ids_and_suppress_replay() {
    let pinned: BTreeMap<&str, (&str, &str)> = BTreeMap::from([
        (
            "github-pr",
            (
                "017b2629b85ccd4dcb4eca4898fa84cc2996b3e6dbbed61c4be59f65d5e30171",
                "github-pr:42:017b2629b85ccd4dcb4eca48",
            ),
        ),
        (
            "github-pr-external-activity",
            (
                "48028f1174c6ccebaed866ec7176f89177c17ce1a6be9e8c8076a8766cee375a",
                "github-pr-activity:43:48028f1174c6ccebaed866ec",
            ),
        ),
        (
            "azure-devops-pr",
            (
                "985685e50683ac11de09e32091149396ab6c176fbe7aca950020082072d9606a",
                "azure-devops-pr:73:985685e50683ac11de09e320",
            ),
        ),
        (
            "http-json",
            (
                "5c46396891a798218ffd568b8aa3e7f942f9362ce7a004d11a7c18157a03d04e",
                "http-json:example:5c46396891a798218ffd568b",
            ),
        ),
        (
            "local-file-json",
            (
                "5fa0d25475fe8e8c2a7ae9dcc8dac6f7d31586e54d15ed0e9101b681cf99ee48",
                "local-file-json:example:5fa0d25475fe8e8c2a7ae9dc",
            ),
        ),
        (
            "local-command",
            (
                "632c0d44266fc389e6947848fa3eddecc6519c34093045eba8727cb78db80357",
                "local-command:example:632c0d44266fc389e6947848",
            ),
        ),
    ]);

    for template_id in TEMPLATE_IDS {
        let mut emitted_kinds = BTreeSet::new();
        for (index, parameters) in event_variants(template_id).into_iter().enumerate() {
            let first = run_detector(template_id, parameters.clone(), json!({}));
            assert!(
                matches!(first.result["outcome"].as_str(), Some("event" | "terminal")),
                "{template_id} fixture variant {index} did not emit an event"
            );
            assert!(first.parsed.event.is_some());
            let cursor = first.result["nextState"]["cursor"]
                .as_str()
                .expect("event cursor")
                .to_owned();
            let event_id = first.result["event"]["id"]
                .as_str()
                .expect("event ID")
                .to_owned();
            emitted_kinds.insert(
                first.result["event"]["kind"]
                    .as_str()
                    .expect("event kind")
                    .to_owned(),
            );

            let repeated = run_detector(template_id, parameters.clone(), json!({}));
            assert_eq!(repeated.result["nextState"]["cursor"], cursor);
            assert_eq!(repeated.result["event"]["id"], event_id);
            if first.result["outcome"] == "event" {
                let replay =
                    run_detector(template_id, parameters, first.result["nextState"].clone());
                assert_eq!(replay.result["outcome"], "idle");
                assert!(replay.parsed.event.is_none());
                assert_eq!(replay.result["nextState"]["cursor"], cursor);
            }

            if index == 0 {
                let expected = pinned.get(template_id).expect("pinned evidence");
                assert_eq!(cursor, expected.0, "{template_id} cursor changed");
                assert_eq!(event_id, expected.1, "{template_id} event ID changed");
                if template_id == "azure-devops-pr" {
                    assert_eq!(
                        first.result["event"]["metadata"]["creationDate"],
                        "2026-07-19T11:00:00.0000000+00:00",
                        "Azure DevOps timestamps must retain their provider instant"
                    );
                }
            }
        }
        assert_eq!(
            emitted_kinds,
            json_string_set(&manifest(template_id)["allowedEventKinds"]),
            "{template_id} fixture event-kind union must equal manifest policy"
        );
    }
}

fn with_parameter(mut parameters: Value, name: &str, value: Value) -> Value {
    parameters
        .as_object_mut()
        .expect("parameter object")
        .insert(name.to_owned(), value);
    parameters
}

fn capture_path(name: &str) -> PathBuf {
    let directory = repository_root()
        .join("target")
        .join("template-conformance-artifacts");
    fs::create_dir_all(&directory).expect("create conformance artifact directory");
    let path = directory.join(name);
    if path.exists() {
        fs::remove_file(&path).expect("remove stale transport capture");
    }
    path
}

fn read_capture(path: &Path) -> Vec<Value> {
    let text = fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("read capture {}: {error}", path.display()));
    let records = text
        .lines()
        .map(|line| serde_json::from_str(line).expect("transport record JSON"))
        .collect();
    fs::remove_file(path).expect("remove transport capture");
    records
}

#[test]
fn canonical_cursor_is_independent_of_object_insertion_order() {
    let helper = path_text(templates_root().join("shared").join("DetectorCommon.psm1"));
    let script = format!(
        "Import-Module '{}'; \
         $a=[ordered]@{{z=1;a=[ordered]@{{b=2;a=1}};decimal=[decimal]'1234.5'}}; \
         $b=[ordered]@{{decimal=[decimal]'1234.5';a=[ordered]@{{a=1;b=2}};z=1}}; \
         $original=[Globalization.CultureInfo]::CurrentCulture; \
         try {{ \
             [Globalization.CultureInfo]::CurrentCulture=[Globalization.CultureInfo]::GetCultureInfo('fr-FR'); \
             $first=Get-OpaqueCursor $a; \
             [Globalization.CultureInfo]::CurrentCulture=[Globalization.CultureInfo]::GetCultureInfo('tr-TR'); \
             $second=Get-OpaqueCursor $b; \
             @($first,$second) | ConvertTo-Json -Compress \
         }} finally {{ [Globalization.CultureInfo]::CurrentCulture=$original }}",
        helper.replace('\'', "''")
    );
    let output = Command::new("pwsh")
        .args(["-NoLogo", "-NoProfile", "-Command", &script])
        .output()
        .expect("run canonical cursor probe");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let cursors: Value = serde_json::from_slice(&output.stdout).expect("cursor JSON");
    assert_eq!(cursors[0], cursors[1]);

    let utc = run_detector_with_env(
        "azure-devops-pr",
        primary_parameters("azure-devops-pr"),
        json!({}),
        &[("TZ", "UTC")],
    );
    let pacific = run_detector_with_env(
        "azure-devops-pr",
        primary_parameters("azure-devops-pr"),
        json!({}),
        &[("TZ", "Pacific/Auckland")],
    );
    assert_eq!(
        utc.result["nextState"]["cursor"], pacific.result["nextState"]["cursor"],
        "Azure DevOps cursor must not depend on the detector timezone"
    );
    assert_eq!(
        utc.result["event"]["metadata"]["creationDate"],
        pacific.result["event"]["metadata"]["creationDate"]
    );
}

#[test]
fn provider_transports_are_network_free_versioned_and_credential_safe() {
    for template_id in ["github-pr", "github-pr-external-activity"] {
        let capture = capture_path(&format!("{template_id}-transport.jsonl"));
        let parameters = with_parameter(
            primary_parameters(template_id),
            "testTransportCapturePath",
            path_text(&capture).into(),
        );
        assert!(!parameters.to_string().contains("github-test-token"));
        let run = run_detector_with_env(
            template_id,
            parameters,
            json!({}),
            &[("GH_TOKEN", "github-test-token")],
        );
        assert!(matches!(
            run.result["outcome"].as_str(),
            Some("event" | "terminal")
        ));
        let records = read_capture(&capture);
        assert_eq!(
            records.len(),
            manifest(template_id)["provider"]["callsPerAttempt"]
                .as_u64()
                .expect("call count") as usize
        );
        let record = &records[0];
        assert_eq!(record["transport"], "gh-cli");
        assert_eq!(record["executable"], "gh");
        assert_eq!(record["body"], Value::Null);
        assert_eq!(record["credentialEnvironment"], json!(["GH_TOKEN"]));
        assert!(!record.to_string().contains("github-test-token"));
        assert!(record["arguments"]
            .as_array()
            .expect("gh args")
            .iter()
            .any(|value| value == "--json"));
    }

    for (name, bearer, pat, environment, expected_authorization) in [
        (
            "ado-bearer",
            true,
            false,
            ("AZURE_DEVOPS_ACCESS_TOKEN", "ado-bearer-token"),
            "Bearer ado-bearer-token",
        ),
        (
            "ado-pat",
            false,
            true,
            ("AZURE_DEVOPS_EXT_PAT", "pat-token"),
            "Basic OnBhdC10b2tlbg==",
        ),
    ] {
        let capture = capture_path(&format!("{name}-transport.jsonl"));
        let mut parameters = primary_parameters("azure-devops-pr");
        parameters["allowBearerAuthentication"] = bearer.into();
        parameters["allowPatAuthentication"] = pat.into();
        parameters["testTransportCapturePath"] = path_text(&capture).into();
        assert!(!parameters.to_string().contains(environment.1));
        let run = run_detector_with_env("azure-devops-pr", parameters, json!({}), &[environment]);
        assert_eq!(run.result["outcome"], "event");
        let records = read_capture(&capture);
        assert_eq!(
            records.len(),
            manifest("azure-devops-pr")["provider"]["callsPerAttempt"]
                .as_u64()
                .expect("ADO call count") as usize
        );
        for record in records {
            let uri = record["uri"].as_str().expect("ADO URI");
            assert!(uri.starts_with("https://"));
            assert!(uri.contains("api-version=7.1"));
            assert_eq!(record["method"], "GET");
            assert_eq!(record["body"], Value::Null);
            assert_eq!(record["headers"]["Authorization"], expected_authorization);
            let mut without_headers = record.clone();
            without_headers["headers"] = json!({});
            assert!(!without_headers.to_string().contains(environment.1));
        }
    }

    for (name, authentication, header_name, environment, expected_header, expected_value) in [
        (
            "http-bearer",
            "bearer",
            None,
            Some(("HTTP_JSON_BEARER_TOKEN", "http-bearer-token")),
            "Authorization",
            "Bearer http-bearer-token",
        ),
        (
            "http-header",
            "header",
            Some("X-Api-Key"),
            Some(("HTTP_JSON_HEADER_VALUE", "http-header-token")),
            "X-Api-Key",
            "http-header-token",
        ),
    ] {
        let capture = capture_path(&format!("{name}-transport.jsonl"));
        let mut parameters = primary_parameters("http-json");
        parameters["authentication"] = authentication.into();
        parameters["url"] = "https://api.example.invalid/status".into();
        parameters["testTransportCapturePath"] = path_text(&capture).into();
        if let Some(header_name) = header_name {
            parameters["headerName"] = header_name.into();
        }
        let environment = environment.expect("HTTP credential");
        assert!(!parameters.to_string().contains(environment.1));
        let run = run_detector_with_env("http-json", parameters, json!({}), &[environment]);
        assert_eq!(run.result["outcome"], "event");
        let records = read_capture(&capture);
        assert_eq!(
            records.len(),
            manifest("http-json")["provider"]["callsPerAttempt"]
                .as_u64()
                .expect("HTTP call count") as usize
        );
        let record = &records[0];
        assert!(record["uri"]
            .as_str()
            .expect("HTTP URI")
            .starts_with("https://"));
        assert_eq!(record["method"], "GET");
        assert_eq!(record["body"], Value::Null);
        assert_eq!(record["maximumRedirection"], 0);
        assert_eq!(record["headers"][expected_header], expected_value);
        let mut without_headers = record.clone();
        without_headers["headers"] = json!({});
        assert!(!without_headers.to_string().contains(environment.1));
    }

    let capture = capture_path("http-none-transport.jsonl");
    let mut parameters = primary_parameters("http-json");
    parameters["authentication"] = "none".into();
    parameters["url"] = "https://api.example.invalid/status".into();
    parameters["testTransportCapturePath"] = path_text(&capture).into();
    let run = run_detector("http-json", parameters, json!({}));
    assert_eq!(run.result["outcome"], "event");
    let records = read_capture(&capture);
    assert_eq!(records.len(), 1);
    assert_eq!(records[0]["headers"], json!({}));
}

#[test]
fn credential_modes_fail_closed_without_the_manifest_required_environment() {
    let capture = capture_path("ado-missing-credential.jsonl");
    let mut ado = primary_parameters("azure-devops-pr");
    ado["testTransportCapturePath"] = path_text(&capture).into();
    let missing_ado = run_detector("azure-devops-pr", ado, json!({}));
    assert_eq!(missing_ado.result["outcome"], "degraded");
    assert!(missing_ado
        .stderr
        .contains(r#""code":"missing-credential""#));
    assert!(!capture.exists());

    let capture = capture_path("http-missing-credential.jsonl");
    let mut http = primary_parameters("http-json");
    http["authentication"] = "bearer".into();
    http["url"] = "https://api.example.invalid/status".into();
    http["testTransportCapturePath"] = path_text(&capture).into();
    let missing_http = run_detector("http-json", http, json!({}));
    assert_eq!(missing_http.result["outcome"], "degraded");
    assert!(missing_http
        .stderr
        .contains(r#""code":"missing-credential""#));
    assert!(!capture.exists());

    let capture = capture_path("ado-invalid-auth-mode.jsonl");
    let mut ado = primary_parameters("azure-devops-pr");
    ado["allowBearerAuthentication"] = false.into();
    ado["allowPatAuthentication"] = false.into();
    ado["testTransportCapturePath"] = path_text(&capture).into();
    let invalid_mode = run_detector("azure-devops-pr", ado, json!({}));
    assert_eq!(invalid_mode.result["outcome"], "degraded");
    assert!(invalid_mode
        .stderr
        .contains(r#""code":"credential-policy""#));
    assert!(!capture.exists());
}

#[test]
fn edge_case_classification_and_local_evidence_are_stable() {
    let waiting = run_detector(
        "azure-devops-pr",
        json!({
            "fixturePath": fixture("azure-devops-pr", "azure-devops-pr-waiting-for-author.json"),
            "organization": "example-organization",
            "project": "example-project",
            "repositoryId": "example-repository",
            "blockingReviewerVoteAtMost": -10
        }),
        json!({}),
    );
    assert_eq!(waiting.result["outcome"], "idle");
    assert!(waiting.parsed.event.is_none());

    let no_activity_parameters = json!({
        "fixturePath": fixture("github-pr-external-activity", "no-external-activity.json"),
        "repository": "example/repo",
        "selfLogin": "self-login",
        "ignoredLogins": ["example-bot"]
    });
    let no_activity = run_detector(
        "github-pr-external-activity",
        no_activity_parameters.clone(),
        json!({}),
    );
    assert_eq!(no_activity.result["outcome"], "idle");
    let no_activity_repeated = run_detector(
        "github-pr-external-activity",
        no_activity_parameters,
        no_activity.result["nextState"].clone(),
    );
    assert_eq!(no_activity_repeated.result["outcome"], "idle");
    assert_eq!(
        no_activity.result["nextState"]["cursor"],
        no_activity_repeated.result["nextState"]["cursor"]
    );

    let versionless = run_detector(
        "local-file-json",
        json!({
            "inputPath": fixture("local-file-json", "ready-versionless.json"),
            "sourceId": "versionless",
            "field": "ready",
            "expectedValue": true
        }),
        json!({}),
    );
    assert_eq!(versionless.result["outcome"], "event");
    assert_eq!(versionless.result["event"]["metadata"]["version"], "");

    let changing_parameters = json!({
        "command": [
            "pwsh", "-NoLogo", "-NoProfile", "-File",
            fixture("local-command", "condition-met-changing-output.ps1")
        ],
        "workingDirectory": path_text(templates_root().join("local-command").join("fixtures")),
        "sourceId": "changing-output",
        "conditionExitCodes": [0],
        "successExitCodes": [1],
        "commandTimeoutSeconds": 20,
        "maxOutputChars": 16384
    });
    let first = run_detector("local-command", changing_parameters.clone(), json!({}));
    let second = run_detector("local-command", changing_parameters.clone(), json!({}));
    assert_eq!(
        first.result["outcome"], "event",
        "portable local-command fixture failed: {}",
        first.stderr
    );
    assert_eq!(
        second.result["outcome"], "event",
        "portable local-command fixture failed: {}",
        second.stderr
    );
    assert_ne!(
        first.result["event"]["body"],
        second.result["event"]["body"]
    );
    assert_eq!(
        first.result["nextState"]["cursor"],
        second.result["nextState"]["cursor"]
    );
    assert_eq!(first.result["event"]["id"], second.result["event"]["id"]);
    let replay = run_detector(
        "local-command",
        changing_parameters,
        first.result["nextState"].clone(),
    );
    assert_eq!(replay.result["outcome"], "idle");
    assert!(replay.parsed.event.is_none());

    let environment = run_detector_with_env(
        "local-command",
        json!({
            "command": [
                "pwsh", "-NoLogo", "-NoProfile", "-File",
                fixture("local-command", "environment-check.ps1")
            ],
            "workingDirectory": path_text(templates_root().join("local-command").join("fixtures")),
            "sourceId": "environment",
            "conditionExitCodes": [0],
            "successExitCodes": [1],
            "commandTimeoutSeconds": 20,
            "maxOutputChars": 16384
        }),
        json!({}),
        &[("TELEX_ALLOWED_SENTINEL", "allowed")],
    );
    assert_eq!(environment.result["outcome"], "event");
}

#[test]
fn http_json_scalar_null_and_missing_semantics_are_distinct() {
    let null_parameters = json!({
        "fixturePath": fixture("http-json", "null-value.json"),
        "sourceId": "null",
        "fieldPath": "service.ready",
        "expectedValue": null,
        "authentication": "none"
    });
    let present_null = run_detector("http-json", null_parameters, json!({}));
    assert_eq!(present_null.result["outcome"], "event");

    let missing = run_detector(
        "http-json",
        json!({
            "fixturePath": fixture("http-json", "missing-field.json"),
            "sourceId": "missing",
            "fieldPath": "service.ready",
            "expectedValue": null,
            "authentication": "none"
        }),
        json!({}),
    );
    assert_eq!(missing.result["outcome"], "idle");

    let object_expected = run_detector(
        "http-json",
        json!({
            "fixturePath": fixture("http-json", "condition-met.json"),
            "sourceId": "object",
            "fieldPath": "service",
            "expectedValue": {"ready": true},
            "authentication": "none"
        }),
        json!({}),
    );
    assert_eq!(object_expected.result["outcome"], "degraded");
    assert!(object_expected
        .stderr
        .contains(r#""code":"configuration-invalid""#));
}

#[test]
fn local_file_json_expected_value_is_required() {
    let omitted = run_detector(
        "local-file-json",
        json!({
            "inputPath": fixture("local-file-json", "ready.json"),
            "sourceId": "omitted",
            "field": "ready"
        }),
        json!({}),
    );
    assert_eq!(omitted.result["outcome"], "degraded");
    assert!(omitted.result.get("nextState").is_none());
    assert!(omitted.stderr.contains(r#""code":"configuration-invalid""#));
    assert!(omitted
        .stderr
        .contains("parameters.expectedValue is required"));
}

fn run_preflight(script: &str, arguments: &[String]) -> Output {
    let mut test_arguments = vec!["-TestMode".to_string()];
    test_arguments.extend_from_slice(arguments);
    run_preflight_raw(script, &test_arguments, &[])
}

fn run_preflight_raw(script: &str, arguments: &[String], environment: &[(&str, &str)]) -> Output {
    let mut command = Command::new("pwsh");
    command
        .args(["-NoLogo", "-NoProfile", "-File"])
        .arg(templates_root().join("shared").join(script))
        .args(arguments)
        .env_clear();
    inherit_test_baseline(&mut command);
    for (name, value) in environment {
        command.env(name, value);
    }
    command
        .output()
        .unwrap_or_else(|error| panic!("run {script}: {error}"))
}

#[test]
fn pr_preflight_blocks_terminal_registration_and_closes_the_first_attempt_race() {
    let github_open = run_preflight(
        "github-pr-preflight.ps1",
        &[
            "-Repository".into(),
            "example/repo".into(),
            "-PullRequestNumber".into(),
            "42".into(),
            "-FixturePath".into(),
            fixture("github-pr", "github-pr-ready.json"),
            "-Now".into(),
            "2026-07-29T12:00:00Z".into(),
        ],
    );
    assert!(github_open.status.success());
    let github_evidence: Value =
        serde_json::from_slice(&github_open.stdout).expect("GitHub preflight evidence");
    assert_eq!(github_evidence["terminal"], false);
    for (template_id, terminal_fixture, number) in [
        ("github-pr", "github-pr-terminal.json", 42),
        ("github-pr-external-activity", "terminal.json", 43),
    ] {
        let evidence = if template_id == "github-pr" {
            github_evidence.clone()
        } else {
            let output = run_preflight(
                "github-pr-preflight.ps1",
                &[
                    "-Repository".into(),
                    "example/repo".into(),
                    "-PullRequestNumber".into(),
                    number.to_string(),
                    "-FixturePath".into(),
                    fixture(template_id, "activity.json"),
                    "-Now".into(),
                    "2026-07-29T12:00:00Z".into(),
                ],
            );
            assert!(output.status.success());
            serde_json::from_slice(&output.stdout).expect("custom GitHub preflight")
        };
        let result = run_detector(
            template_id,
            json!({
                "fixturePath": fixture(template_id, terminal_fixture),
                "repository": "example/repo",
                "pullRequestNumber": number,
                "selfLogin": "self-login",
                "ignoredLogins": []
            }),
            json!({"preflight": evidence}),
        );
        assert_eq!(result.result["outcome"], "terminal");
        assert!(result.result.get("event").is_none());
    }

    let github_terminal = run_preflight(
        "github-pr-preflight.ps1",
        &[
            "-Repository".into(),
            "example/repo".into(),
            "-PullRequestNumber".into(),
            "42".into(),
            "-FixturePath".into(),
            fixture("github-pr", "github-pr-terminal.json"),
            "-Now".into(),
            "2026-07-29T12:00:00Z".into(),
        ],
    );
    assert_eq!(github_terminal.status.code(), Some(3));
    assert_eq!(
        read_output_json(&github_terminal)["terminal"],
        Value::Bool(true)
    );

    let azure_open = run_preflight(
        "azure-devops-pr-preflight.ps1",
        &[
            "-Organization".into(),
            "example-organization".into(),
            "-Project".into(),
            "example-project".into(),
            "-RepositoryId".into(),
            "example-repository".into(),
            "-PullRequestId".into(),
            "73".into(),
            "-FixturePath".into(),
            fixture("azure-devops-pr", "azure-devops-pr-ready.json"),
            "-Now".into(),
            "2026-07-29T12:00:00Z".into(),
        ],
    );
    assert!(azure_open.status.success());
    let azure_evidence = read_output_json(&azure_open);
    let azure_race = run_detector(
        "azure-devops-pr",
        json!({
            "fixturePath": fixture("azure-devops-pr", "azure-devops-pr-terminal.json"),
            "organization": "example-organization",
            "project": "example-project",
            "repositoryId": "example-repository"
        }),
        json!({"preflight": azure_evidence}),
    );
    assert_eq!(azure_race.result["outcome"], "terminal");
    assert!(azure_race.result.get("event").is_none());

    let azure_terminal = run_preflight(
        "azure-devops-pr-preflight.ps1",
        &[
            "-Organization".into(),
            "example-organization".into(),
            "-Project".into(),
            "example-project".into(),
            "-RepositoryId".into(),
            "example-repository".into(),
            "-PullRequestId".into(),
            "73".into(),
            "-FixturePath".into(),
            fixture("azure-devops-pr", "azure-devops-pr-terminal.json"),
            "-Now".into(),
            "2026-07-29T12:00:00Z".into(),
        ],
    );
    assert_eq!(azure_terminal.status.code(), Some(3));
    assert_eq!(
        read_output_json(&azure_terminal)["terminal"],
        Value::Bool(true)
    );
}

#[test]
fn preflight_declared_terminal_is_eventless_and_mismatch_is_structured() {
    let github_open = run_preflight(
        "github-pr-preflight.ps1",
        &[
            "-Repository".into(),
            "example/repo".into(),
            "-PullRequestNumber".into(),
            "42".into(),
            "-FixturePath".into(),
            fixture("github-pr", "github-pr-ready.json"),
            "-Now".into(),
            "2026-07-29T12:00:00Z".into(),
        ],
    );
    let mut github_evidence = read_output_json(&github_open);
    github_evidence["terminal"] = true.into();
    let declared_terminal = run_detector(
        "github-pr",
        json!({
            "fixturePath": fixture("github-pr", "github-pr-ready.json"),
            "repository": "example/repo",
            "pullRequestNumber": 42,
            "emitInitialSnapshot": false
        }),
        json!({"preflight": github_evidence}),
    );
    assert_eq!(declared_terminal.result["outcome"], "terminal");
    assert!(declared_terminal.parsed.event.is_none());

    let github_activity_open = run_preflight(
        "github-pr-preflight.ps1",
        &[
            "-Repository".into(),
            "example/repo".into(),
            "-PullRequestNumber".into(),
            "43".into(),
            "-FixturePath".into(),
            fixture("github-pr-external-activity", "activity.json"),
            "-Now".into(),
            "2026-07-29T12:00:00Z".into(),
        ],
    );
    let mut github_activity_evidence = read_output_json(&github_activity_open);
    github_activity_evidence["terminal"] = true.into();
    let external_declared_terminal = run_detector(
        "github-pr-external-activity",
        json!({
            "fixturePath": fixture("github-pr-external-activity", "activity.json"),
            "repository": "example/repo",
            "pullRequestNumber": 43,
            "selfLogin": "self-login",
            "ignoredLogins": ["example-bot"]
        }),
        json!({"preflight": github_activity_evidence}),
    );
    assert_eq!(external_declared_terminal.result["outcome"], "terminal");
    assert!(external_declared_terminal.parsed.event.is_none());

    let azure_open = run_preflight(
        "azure-devops-pr-preflight.ps1",
        &[
            "-Organization".into(),
            "example-organization".into(),
            "-Project".into(),
            "example-project".into(),
            "-RepositoryId".into(),
            "example-repository".into(),
            "-PullRequestId".into(),
            "73".into(),
            "-FixturePath".into(),
            fixture("azure-devops-pr", "azure-devops-pr-ready.json"),
            "-Now".into(),
            "2026-07-29T12:00:00Z".into(),
        ],
    );
    let mut azure_evidence = read_output_json(&azure_open);
    azure_evidence["terminal"] = true.into();
    let declared_terminal = run_detector(
        "azure-devops-pr",
        json!({
            "fixturePath": fixture("azure-devops-pr", "azure-devops-pr-ready.json"),
            "organization": "example-organization",
            "project": "example-project",
            "repositoryId": "example-repository",
            "pullRequestId": 73
        }),
        json!({"preflight": azure_evidence}),
    );
    assert_eq!(declared_terminal.result["outcome"], "terminal");
    assert!(declared_terminal.parsed.event.is_none());

    let mismatched = json!({
        "schemaVersion": 1,
        "provider": "github",
        "templateIds": ["github-pr-external-activity"],
        "observedAt": "2026-07-29T12:00:00Z",
        "terminal": false,
        "state": "OPEN",
        "identity": {
            "repository": "example/repo",
            "pullRequestNumber": 42,
            "headSha": "0123456789abcdef0123456789abcdef01234567"
        }
    });
    let parameters = json!({
        "fixturePath": fixture("github-pr", "github-pr-ready.json"),
        "repository": "example/repo",
        "pullRequestNumber": 42
    });
    let first = run_detector(
        "github-pr",
        parameters.clone(),
        json!({"preflight": mismatched.clone()}),
    );
    let repeated = run_detector("github-pr", parameters, json!({"preflight": mismatched}));
    for run in [&first, &repeated] {
        assert_eq!(run.result["outcome"], "degraded");
        assert!(run.result.get("nextState").is_none());
        assert!(run
            .stderr
            .contains(r#""code":"preflight-identity-mismatch""#));
    }

    let mut external_mismatch = read_output_json(&github_activity_open);
    external_mismatch["templateIds"] = json!(["github-pr"]);
    let external_parameters = json!({
        "fixturePath": fixture("github-pr-external-activity", "activity.json"),
        "repository": "example/repo",
        "pullRequestNumber": 43,
        "selfLogin": "self-login",
        "ignoredLogins": ["example-bot"]
    });
    let first = run_detector(
        "github-pr-external-activity",
        external_parameters.clone(),
        json!({"preflight": external_mismatch.clone()}),
    );
    let repeated = run_detector(
        "github-pr-external-activity",
        external_parameters,
        json!({"preflight": external_mismatch}),
    );
    for run in [&first, &repeated] {
        assert_eq!(run.result["outcome"], "degraded");
        assert!(run.result.get("nextState").is_none());
        assert!(run
            .stderr
            .contains(r#""code":"preflight-identity-mismatch""#));
    }

    let parameter_fallback = run_detector(
        "github-pr",
        json!({
            "fixturePath": fixture("github-pr", "github-pr-ready.json"),
            "repository": "example/repo",
            "pullRequestNumber": 42,
            "preflight": {
                "terminal": true
            }
        }),
        json!({}),
    );
    assert_eq!(parameter_fallback.result["outcome"], "event");
    assert_eq!(
        parameter_fallback.result["event"]["kind"],
        "github.pull-request.ready-to-merge"
    );

    let invalid_timestamp = run_detector(
        "github-pr",
        json!({
            "fixturePath": fixture("github-pr", "github-pr-ready.json"),
            "repository": "example/repo",
            "pullRequestNumber": 42
        }),
        json!({
            "preflight": {
                "schemaVersion": 1,
                "provider": "github",
                "templateIds": ["github-pr"],
                "observedAt": "2026-07-29T12:00:00",
                "terminal": false,
                "state": "OPEN",
                "identity": {
                    "repository": "example/repo",
                    "pullRequestNumber": 42
                }
            }
        }),
    );
    assert_eq!(invalid_timestamp.result["outcome"], "degraded");
    assert!(invalid_timestamp
        .stderr
        .contains(r#""code":"preflight-identity-mismatch""#));
}

#[test]
fn preflight_test_seams_rfc3339_and_exit_codes_are_explicit() {
    let without_test_mode = run_preflight_raw(
        "github-pr-preflight.ps1",
        &[
            "-Repository".into(),
            "example/repo".into(),
            "-PullRequestNumber".into(),
            "42".into(),
            "-FixturePath".into(),
            fixture("github-pr", "github-pr-ready.json"),
        ],
        &[],
    );
    assert_eq!(without_test_mode.status.code(), Some(5));
    assert!(String::from_utf8_lossy(&without_test_mode.stderr).contains("test-mode-required"));

    let invalid_now = run_preflight(
        "github-pr-preflight.ps1",
        &[
            "-Repository".into(),
            "example/repo".into(),
            "-PullRequestNumber".into(),
            "42".into(),
            "-FixturePath".into(),
            fixture("github-pr", "github-pr-ready.json"),
            "-Now".into(),
            "not-a-timestamp".into(),
        ],
    );
    assert_eq!(invalid_now.status.code(), Some(5));
    assert!(String::from_utf8_lossy(&invalid_now.stderr).contains("invalid-rfc3339"));

    let missing_fixture = run_preflight(
        "azure-devops-pr-preflight.ps1",
        &[
            "-Organization".into(),
            "example-organization".into(),
            "-Project".into(),
            "example-project".into(),
            "-RepositoryId".into(),
            "example-repository".into(),
            "-PullRequestId".into(),
            "73".into(),
            "-FixturePath".into(),
            fixture("azure-devops-pr", "missing.json"),
        ],
    );
    assert_eq!(missing_fixture.status.code(), Some(5));
    assert!(String::from_utf8_lossy(&missing_fixture.stderr).contains("fixture-parse-failure"));

    let missing_credential = run_preflight_raw(
        "azure-devops-pr-preflight.ps1",
        &[
            "-Organization".into(),
            "example-organization".into(),
            "-Project".into(),
            "example-project".into(),
            "-RepositoryId".into(),
            "example-repository".into(),
            "-PullRequestId".into(),
            "73".into(),
            "-Authentication".into(),
            "bearer".into(),
        ],
        &[],
    );
    assert_eq!(missing_credential.status.code(), Some(4));
    assert!(String::from_utf8_lossy(&missing_credential.stderr)
        .contains("provider-auth-transport-failure"));
}

fn read_output_json(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).expect("preflight output JSON")
}

fn collect_files(root: &Path, files: &mut Vec<PathBuf>) {
    for entry in
        fs::read_dir(root).unwrap_or_else(|error| panic!("read {}: {error}", root.display()))
    {
        let path = entry.expect("directory entry").path();
        if path.is_dir() {
            collect_files(&path, files);
        } else {
            files.push(path);
        }
    }
}

fn collect_json_strings<'a>(value: &'a Value, strings: &mut Vec<&'a str>) {
    match value {
        Value::String(text) => strings.push(text),
        Value::Array(items) => {
            for item in items {
                collect_json_strings(item, strings);
            }
        }
        Value::Object(object) => {
            for item in object.values() {
                collect_json_strings(item, strings);
            }
        }
        _ => {}
    }
}

fn heading_anchor(heading: &str) -> String {
    let mut anchor = String::new();
    let mut previous_hyphen = false;
    for ch in heading.trim().chars() {
        if ch.is_ascii_alphanumeric() {
            anchor.push(ch.to_ascii_lowercase());
            previous_hyphen = false;
        } else if (ch == ' ' || ch == '-') && !previous_hyphen && !anchor.is_empty() {
            anchor.push('-');
            previous_hyphen = true;
        }
    }
    anchor.trim_end_matches('-').to_owned()
}

#[test]
fn fixtures_changelog_and_agent_guidance_remain_sanitized_and_current() {
    let fixture_forbidden = [
        "github.com/",
        "dev.azure.com/",
        "authorization:",
        "ghp_",
        "github_pat_",
        "bearer ey",
        "c:\\users\\",
        "/home/",
    ];
    let broad_forbidden = [
        "ghp_",
        "github_pat_",
        "bearer ey",
        "c:\\users\\",
        "/home/",
        "\u{fffd}",
        "replace-with-",
    ];
    let allowed_placeholders = BTreeSet::from([
        "<DETECTOR-PATH>",
        "<TEMPLATE-DIRECTORY>",
        "<INPUT-JSON-PATH>",
        "<OBSERVATIONAL-COMMAND>",
        "<COMMAND-WORKING-DIRECTORY>",
    ]);

    for template_id in TEMPLATE_IDS {
        let manifest = manifest(template_id);
        let api_version = manifest["provider"]["apiVersion"]
            .as_str()
            .expect("provider API version");
        let mut fixture_files = Vec::new();
        collect_files(
            &templates_root().join(template_id).join("fixtures"),
            &mut fixture_files,
        );
        for path in fixture_files {
            let text = fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("read fixture {}: {error}", path.display()));
            let lowered = text.to_ascii_lowercase();
            for fragment in fixture_forbidden {
                assert!(
                    !lowered.contains(fragment),
                    "{} contains unsanitized fragment {fragment}",
                    path.display()
                );
            }
            if path.extension().and_then(|extension| extension.to_str()) == Some("json") {
                let fixture: Value = serde_json::from_str(&text)
                    .unwrap_or_else(|error| panic!("fixture {} JSON: {error}", path.display()));
                assert_eq!(
                    fixture["apiVersion"],
                    api_version,
                    "{} API version must match the manifest",
                    path.display()
                );
                assert!(
                    fixture["capturedAgainst"]
                        .as_str()
                        .is_some_and(|value| !value.is_empty()),
                    "{} must declare capturedAgainst",
                    path.display()
                );
            } else {
                assert!(
                    text.lines()
                        .any(|line| line == format!("# apiVersion: {api_version}")),
                    "{} must declare fixture API version",
                    path.display()
                );
                assert!(
                    text.lines()
                        .any(|line| line.starts_with("# capturedAgainst: ")),
                    "{} must declare capturedAgainst",
                    path.display()
                );
            }
        }

        for sample in ["pinned.json", "development.json"] {
            let path = templates_root()
                .join(template_id)
                .join("registrations")
                .join(sample);
            let text = fs::read_to_string(&path).expect("registration text");
            let lowered = text.to_ascii_lowercase();
            for fragment in broad_forbidden {
                assert!(
                    !lowered.contains(fragment),
                    "{} contains unsanitized fragment {fragment}",
                    path.display()
                );
            }
            let registration: Value = serde_json::from_str(&text).expect("registration JSON");
            let mut strings = Vec::new();
            collect_json_strings(&registration, &mut strings);
            for value in strings {
                if value.starts_with('<') || value.ends_with('>') {
                    assert!(
                        allowed_placeholders.contains(value),
                        "{} contains undocumented placeholder {value}",
                        path.display()
                    );
                }
            }
        }

        let manifest_path = templates_root().join(template_id).join("manifest.json");
        let manifest_text = fs::read_to_string(&manifest_path).expect("manifest text");
        for fragment in broad_forbidden {
            assert!(
                !manifest_text.to_ascii_lowercase().contains(fragment),
                "{} contains unsanitized fragment {fragment}",
                manifest_path.display()
            );
        }
    }

    for helper in [
        "DetectorCommon.psm1",
        "BoundedCommand.psm1",
        "github-pr-preflight.ps1",
        "azure-devops-pr-preflight.ps1",
    ] {
        let path = templates_root().join("shared").join(helper);
        let text = fs::read_to_string(&path).expect("shared helper text");
        for fragment in broad_forbidden {
            assert!(
                !text.to_ascii_lowercase().contains(fragment),
                "{} contains unsanitized fragment {fragment}",
                path.display()
            );
        }
    }

    let changelog =
        fs::read_to_string(templates_root().join("CHANGELOG.md")).expect("template changelog");
    for template_id in TEMPLATE_IDS {
        let manifest = manifest(template_id);
        let version = manifest["templateVersion"]
            .as_str()
            .expect("template version");
        assert!(
            changelog.contains(&format!("`{template_id}` template version {version}")),
            "changelog missing {template_id} {version}"
        );
    }

    let readme = fs::read_to_string(templates_root().join("README.md")).expect("template README");
    assert!(
        readme.contains("[detector template checklist](AGENT.md)"),
        "README must link to the concise agent checklist"
    );
    let anchors: BTreeSet<String> = readme
        .lines()
        .filter_map(|line| line.strip_prefix("## "))
        .map(heading_anchor)
        .collect();
    let agent_guide =
        fs::read_to_string(templates_root().join("AGENT.md")).expect("template agent guide");
    let mut remainder = agent_guide.as_str();
    let marker = "(README.md#";
    let mut link_count = 0;
    while let Some(start) = remainder.find(marker) {
        let after = &remainder[start + marker.len()..];
        let end = after.find(')').expect("README anchor closes");
        let anchor = &after[..end];
        assert!(
            anchors.contains(anchor),
            "AGENT link does not resolve: {anchor}"
        );
        link_count += 1;
        remainder = &after[end + 1..];
    }
    assert!(
        link_count >= 8,
        "AGENT must link to authoritative README sections"
    );
    assert!(templates_root()
        .join("RECONCILING-CUSTOMIZATIONS.md")
        .is_file());
}
