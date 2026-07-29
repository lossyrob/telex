#[path = "../src/protocol.rs"]
#[allow(dead_code)]
mod protocol;

use chrono::DateTime;
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

fn validate(schema: &Value, instance: &Value, root: &Value, path: &str) -> Result<(), String> {
    if let Some(allowed) = schema.as_bool() {
        return if allowed {
            Ok(())
        } else {
            Err(format!("{path}: schema is false"))
        };
    }
    let object = schema
        .as_object()
        .ok_or_else(|| format!("{path}: schema is not an object or boolean"))?;

    if let Some(reference) = object.get("$ref").and_then(Value::as_str) {
        let pointer = reference
            .strip_prefix('#')
            .ok_or_else(|| format!("{path}: only local refs are supported"))?;
        let target = root
            .pointer(pointer)
            .ok_or_else(|| format!("{path}: unresolved ref {reference}"))?;
        return validate(target, instance, root, path);
    }

    if let Some(all_of) = object.get("allOf").and_then(Value::as_array) {
        for branch in all_of {
            validate(branch, instance, root, path)?;
        }
    }
    if let Some(condition) = object.get("if") {
        if validate(condition, instance, root, path).is_ok() {
            if let Some(then_schema) = object.get("then") {
                validate(then_schema, instance, root, path)?;
            }
        } else if let Some(else_schema) = object.get("else") {
            validate(else_schema, instance, root, path)?;
        }
    }
    if let Some(not_schema) = object.get("not") {
        if validate(not_schema, instance, root, path).is_ok() {
            return Err(format!("{path}: matched forbidden schema"));
        }
    }
    if let Some(any_of) = object.get("anyOf").and_then(Value::as_array) {
        if !any_of
            .iter()
            .any(|branch| validate(branch, instance, root, path).is_ok())
        {
            return Err(format!("{path}: did not match anyOf"));
        }
    }

    if let Some(expected) = object.get("const") {
        if instance != expected {
            return Err(format!("{path}: expected const {expected}, got {instance}"));
        }
    }
    if let Some(values) = object.get("enum").and_then(Value::as_array) {
        if !values.contains(instance) {
            return Err(format!("{path}: value {instance} is not in enum"));
        }
    }
    if let Some(kind) = object.get("type") {
        let matches = match kind {
            Value::String(name) => type_matches(name, instance),
            Value::Array(names) => names
                .iter()
                .filter_map(Value::as_str)
                .any(|name| type_matches(name, instance)),
            _ => false,
        };
        if !matches {
            return Err(format!("{path}: type mismatch for {kind}, got {instance}"));
        }
    }

    if let Some(required) = object.get("required").and_then(Value::as_array) {
        let instance_object = instance
            .as_object()
            .ok_or_else(|| format!("{path}: required applies to a non-object"))?;
        for name in required.iter().filter_map(Value::as_str) {
            if !instance_object.contains_key(name) {
                return Err(format!("{path}: missing required property {name}"));
            }
        }
    }
    if let Some(properties) = object.get("properties").and_then(Value::as_object) {
        if let Some(instance_object) = instance.as_object() {
            for (name, property_schema) in properties {
                if let Some(value) = instance_object.get(name) {
                    validate(property_schema, value, root, &format!("{path}.{name}"))?;
                }
            }
            if object.get("additionalProperties") == Some(&Value::Bool(false)) {
                for name in instance_object.keys() {
                    if !properties.contains_key(name) {
                        return Err(format!("{path}: unknown property {name}"));
                    }
                }
            }
        }
    }

    if let Some(items) = object.get("items") {
        if let Some(array) = instance.as_array() {
            for (index, value) in array.iter().enumerate() {
                validate(items, value, root, &format!("{path}[{index}]"))?;
            }
        }
    }
    if let Some(min_items) = object.get("minItems").and_then(Value::as_u64) {
        if instance.as_array().map_or(0, Vec::len) < min_items as usize {
            return Err(format!("{path}: fewer than {min_items} items"));
        }
    }
    if object.get("uniqueItems") == Some(&Value::Bool(true)) {
        if let Some(array) = instance.as_array() {
            let unique: BTreeSet<String> = array.iter().map(Value::to_string).collect();
            if unique.len() != array.len() {
                return Err(format!("{path}: duplicate array item"));
            }
        }
    }
    if let Some(min_length) = object.get("minLength").and_then(Value::as_u64) {
        if instance.as_str().map_or(0, |text| text.chars().count()) < min_length as usize {
            return Err(format!("{path}: string shorter than {min_length}"));
        }
    }
    if let Some(minimum) = object.get("minimum").and_then(Value::as_i64) {
        if let Some(value) = instance.as_i64() {
            if value < minimum {
                return Err(format!("{path}: {value} is less than {minimum}"));
            }
        }
    }
    if let Some(pattern) = object.get("pattern").and_then(Value::as_str) {
        let text = instance
            .as_str()
            .ok_or_else(|| format!("{path}: pattern applies to non-string"))?;
        if !pattern_matches(pattern, text) {
            return Err(format!("{path}: {text:?} does not match {pattern}"));
        }
    }
    if object.get("format").and_then(Value::as_str) == Some("date-time") {
        let text = instance
            .as_str()
            .ok_or_else(|| format!("{path}: date-time applies to non-string"))?;
        DateTime::parse_from_rfc3339(text)
            .map_err(|error| format!("{path}: invalid date-time: {error}"))?;
    }
    Ok(())
}

fn type_matches(expected: &str, value: &Value) -> bool {
    match expected {
        "object" => value.is_object(),
        "array" => value.is_array(),
        "string" => value.is_string(),
        "integer" => value.as_i64().is_some() || value.as_u64().is_some(),
        "number" => value.is_number(),
        "boolean" => value.is_boolean(),
        "null" => value.is_null(),
        other => panic!("unsupported schema type {other}"),
    }
}

fn pattern_matches(pattern: &str, text: &str) -> bool {
    match pattern {
        "^[^\\u0000-\\u001f\\u007f]+$" => {
            !text.is_empty() && !text.chars().any(|ch| ch <= '\u{1f}' || ch == '\u{7f}')
        }
        "^[a-z0-9][a-z0-9-]*(\\.[a-z0-9][a-z0-9-]*)+$" => {
            let parts: Vec<_> = text.split('.').collect();
            parts.len() >= 2
                && parts.iter().all(|part| {
                    !part.is_empty()
                        && part
                            .chars()
                            .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-')
                        && part
                            .chars()
                            .next()
                            .is_some_and(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit())
                })
        }
        "^[0-9a-f]{64}$" => {
            text.len() == 64
                && text
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        }
        "^[a-z0-9]+(?:-[a-z0-9]+)*$" => text.split('-').all(|part| {
            !part.is_empty()
                && part
                    .chars()
                    .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit())
        }),
        "^[0-9]+\\.[0-9]+\\.[0-9]+$" => {
            let parts: Vec<_> = text.split('.').collect();
            parts.len() == 3
                && parts
                    .iter()
                    .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
        }
        "^7\\.[0-9]+$" => text.strip_prefix("7.").is_some_and(|minor| {
            !minor.is_empty() && minor.bytes().all(|byte| byte.is_ascii_digit())
        }),
        "^(?![A-Za-z]:|[/\\\\]).+$" => {
            !text.is_empty()
                && !text.starts_with('/')
                && !text.starts_with('\\')
                && !(text.len() >= 2
                    && text.as_bytes()[0].is_ascii_alphabetic()
                    && text.as_bytes()[1] == b':')
        }
        "^[A-Z_][A-Z0-9_]*$" => {
            text.bytes()
                .next()
                .is_some_and(|byte| byte == b'_' || byte.is_ascii_uppercase())
                && text
                    .bytes()
                    .all(|byte| byte == b'_' || byte.is_ascii_uppercase() || byte.is_ascii_digit())
        }
        other => panic!("unsupported schema pattern {other}"),
    }
}

fn assert_valid(schema: &Value, instance: &Value, label: &str) {
    validate(schema, instance, schema, "$")
        .unwrap_or_else(|error| panic!("{label} failed schema validation: {error}"));
}

struct DetectorRun {
    result: Value,
    parsed: protocol::ValidatedResult,
}

fn run_detector(template_id: &str, parameters: Value, state: Value) -> DetectorRun {
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

    let mut child = Command::new("pwsh")
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
    DetectorRun { result, parsed }
}

fn fixture(template_id: &str, name: &str) -> String {
    path_text(
        templates_root()
            .join(template_id)
            .join("fixtures")
            .join(name),
    )
}

fn primary_parameters(template_id: &str) -> Value {
    match template_id {
        "github-pr" => json!({
            "fixturePath": fixture(template_id, "github-pr-ready.json"),
            "repository": "example/repo",
            "emitInitialSnapshot": true
        }),
        "github-pr-external-activity" => json!({
            "fixturePath": fixture(template_id, "activity.json"),
            "repository": "example/repo",
            "selfLogin": "self-login",
            "ignoredLogins": ["example-bot"],
            "emitInitialSnapshot": true
        }),
        "azure-devops-pr" => json!({
            "fixturePath": fixture(template_id, "azure-devops-pr-ready.json"),
            "organization": "example-organization",
            "project": "example-project",
            "repositoryId": "example-repository",
            "emitInitialSnapshot": true
        }),
        "http-json" => json!({
            "fixturePath": fixture(template_id, "condition-met.json"),
            "sourceId": "example",
            "fieldPath": "service.ready",
            "expectedValue": true,
            "emitInitialSnapshot": true
        }),
        "local-file-json" => json!({
            "inputPath": fixture(template_id, "ready.json"),
            "sourceId": "example",
            "field": "ready",
            "expectedValue": true,
            "emitInitialSnapshot": true
        }),
        "local-command" => json!({
            "command": [
                "pwsh",
                "-NoLogo",
                "-NoProfile",
                "-File",
                fixture(template_id, "condition-met.ps1")
            ],
            "workingDirectory": path_text(templates_root().join(template_id).join("fixtures")),
            "sourceId": "example",
            "conditionExitCodes": [0],
            "successExitCodes": [1],
            "commandTimeoutSeconds": 20,
            "maxOutputChars": 16384,
            "emitInitialSnapshot": true
        }),
        _ => unreachable!(),
    }
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
                "repository": "example/repo",
                "emitInitialSnapshot": true
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
                "emitInitialSnapshot": true
            }),
            json!({
                "fixturePath": fixture(template_id, "azure-devops-pr-neutral.json"),
                "organization": "example-organization",
                "project": "example-project",
                "repositoryId": "example-repository",
                "emitInitialCreatedEvent": true
            }),
            json!({
                "fixturePath": fixture(template_id, "azure-devops-pr-attention.json"),
                "organization": "example-organization",
                "project": "example-project",
                "repositoryId": "example-repository",
                "emitInitialSnapshot": true
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
    assert!(validate(&request_schema, &unknown_request, &request_schema, "$").is_err());

    assert_valid(
        &result_schema,
        &json!({"schemaVersion": 1, "outcome": "idle", "nextState": {}}),
        "canonical idle result",
    );
    assert!(validate(
        &result_schema,
        &json!({"schemaVersion": 1, "outcome": "event"}),
        &result_schema,
        "$"
    )
    .is_err());
    assert!(validate(
        &result_schema,
        &json!({"schemaVersion": 1, "outcome": "degraded", "nextState": {}}),
        &result_schema,
        "$"
    )
    .is_err());
}

#[test]
fn manifests_and_registration_samples_are_strict_and_consistent() {
    let manifest_schema = read_json(&templates_root().join("manifest.schema.json"));
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
        assert!(validate(&manifest_schema, &missing, &manifest_schema, "$").is_err());
        let mut unknown = manifest.clone();
        unknown["unexpected"] = Value::Bool(true);
        assert!(validate(&manifest_schema, &unknown, &manifest_schema, "$").is_err());

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
            assert_eq!(
                json_string_set(&registration["allowedEventKinds"]),
                allowed_kinds
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
                "fea74673f43e09fa6ac1b8b9cc706e8bae385ca057f34444d1b4401002f3fcac",
                "github-pr:42:fea74673f43e09fa6ac1b8b9",
            ),
        ),
        (
            "github-pr-external-activity",
            (
                "9653ad4bd3e56e3e7ad056cfe1fb27143ccfa2e242ce250d2c3394a812637e51",
                "github-pr-activity:43:9653ad4bd3e56e3e7ad056cf",
            ),
        ),
        (
            "azure-devops-pr",
            (
                "4c52efe282b9722ca630d6466a1da445f51df1e8703b1c2e7df78ddd2f682210",
                "azure-devops-pr:73:4c52efe282b9722ca630d646",
            ),
        ),
        (
            "http-json",
            (
                "e5fd747256369e7672370c914a0f20f416383660a4346f7a1edd00d6e06f43f4",
                "http-json:example:e5fd747256369e7672370c91",
            ),
        ),
        (
            "local-file-json",
            (
                "4e4cac22c77c33156c7c608fc1d967b621b931207e38f5770736c2135b163e5a",
                "local-file-json:example:4e4cac22c77c33156c7c608f",
            ),
        ),
        (
            "local-command",
            (
                "ddaeac79e5128de3babb442b29737124abfe1c1bcd14bafa9834b3538a1ea36a",
                "local-command:example:ddaeac79e5128de3babb442b",
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
            }
        }
        assert_eq!(
            emitted_kinds,
            json_string_set(&manifest(template_id)["allowedEventKinds"]),
            "{template_id} fixture event-kind union must equal manifest policy"
        );
    }
}

fn run_preflight(script: &str, arguments: &[String]) -> Output {
    Command::new("pwsh")
        .args(["-NoLogo", "-NoProfile", "-File"])
        .arg(templates_root().join("shared").join(script))
        .args(arguments)
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
fn fixtures_changelog_and_skill_guidance_remain_sanitized_and_current() {
    let mut fixture_files = Vec::new();
    for template_id in TEMPLATE_IDS {
        collect_files(
            &templates_root().join(template_id).join("fixtures"),
            &mut fixture_files,
        );
    }
    let forbidden = [
        "github.com/",
        "dev.azure.com/",
        "authorization:",
        "ghp_",
        "github_pat_",
        "bearer ey",
        "c:\\users\\",
        "/home/",
    ];
    for path in fixture_files {
        let text = fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("read fixture {}: {error}", path.display()));
        let lowered = text.to_ascii_lowercase();
        for fragment in forbidden {
            assert!(
                !lowered.contains(fragment),
                "{} contains unsanitized fragment {fragment}",
                path.display()
            );
        }
        if path.extension().and_then(|extension| extension.to_str()) == Some("json") {
            serde_json::from_str::<Value>(&text)
                .unwrap_or_else(|error| panic!("fixture {} JSON: {error}", path.display()));
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
    let anchors: BTreeSet<String> = readme
        .lines()
        .filter_map(|line| line.strip_prefix("## "))
        .map(heading_anchor)
        .collect();
    let skill = fs::read_to_string(templates_root().join("SKILL.md")).expect("template skill");
    let mut remainder = skill.as_str();
    let marker = "(README.md#";
    let mut link_count = 0;
    while let Some(start) = remainder.find(marker) {
        let after = &remainder[start + marker.len()..];
        let end = after.find(')').expect("README anchor closes");
        let anchor = &after[..end];
        assert!(
            anchors.contains(anchor),
            "SKILL link does not resolve: {anchor}"
        );
        link_count += 1;
        remainder = &after[end + 1..];
    }
    assert!(
        link_count >= 8,
        "SKILL must link to authoritative README sections"
    );
    assert!(templates_root()
        .join("RECONCILING-CUSTOMIZATIONS.md")
        .is_file());
}
