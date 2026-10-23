use serde::ser::SerializeMap;
use serde::{Serialize, Serializer};
use serde_json::{json, Value};
use std::path::PathBuf;
use telex_watcher::protocol_v2::*;

fn cwd() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .canonicalize()
        .unwrap()
}

fn registration() -> Value {
    json!({
        "schemaVersion": 2,
        "id": "github-pr-162",
        "command": ["python", "detector.py", "", "$(not-interpolated)", "semi;colon"],
        "intervalSeconds": 300,
        "timeoutSeconds": 30,
        "backendProfile": "test-profile",
        "sender": "service:watcher/github",
        "target": "project:telex/node:162"
    })
}

fn request() -> Value {
    json!({
        "schemaVersion": 2,
        "attempt": {"id": "attempt-uuid", "now": "2026-07-30T18:00:00Z"},
        "watch": {
            "id": "github-pr-162", "activatedAt": "2026-07-30T17:45:00.123Z",
            "lastSuccessAt": null, "parameters": {"repo": "lossyrob/telex", "pullRequest": 162}
        },
        "state": null
    })
}

fn event() -> Value {
    json!({"kind": "Provider Vocabulary / Unrestricted", "subject": "Example", "body": "Line\none"})
}

fn result(outcome: &str) -> Value {
    json!({"schemaVersion": 2, "outcome": outcome})
}

fn reg(value: &Value) -> Result<Registration, ProtocolError> {
    parse_registration(&serde_json::to_vec(value).unwrap(), &cwd())
}

fn req(value: &Value) -> Result<DetectorRequest, ProtocolError> {
    parse_request(&serde_json::to_vec(value).unwrap())
}

fn res(value: &Value) -> Result<DetectorResult, ProtocolError> {
    parse_result(&serde_json::to_vec(value).unwrap())
}

#[test]
fn registration_defaults_capture_directory_without_changing_argv() {
    let input = registration();
    let registration = reg(&input).unwrap();
    assert_eq!(registration.working_directory, cwd());
    assert_eq!(registration.attention, "next-checkpoint");
    assert!(registration.requires_disposition);
    assert!(registration.environment_allowlist.is_empty());
    assert_eq!(registration.parameters, json!({}));
    assert_eq!(registration.initial_state, Value::Null);
    assert_eq!(
        registration.command,
        serde_json::from_value::<Vec<String>>(input["command"].clone()).unwrap()
    );
    let encoded = serde_json::to_value(&registration).unwrap();
    assert!(encoded.get("workingDirectory").is_some());
    assert!(encoded.get("working_directory").is_none());
    assert_eq!(reg(&encoded).unwrap(), registration);
}

#[test]
fn registration_enforces_schema_route_argv_and_product_bounds() {
    for (field, value) in [
        ("schemaVersion", json!(1)),
        ("schemaVersion", json!("2")),
        ("id", json!("Uppercase")),
        ("id", json!("_leading")),
        ("id", json!("a".repeat(129))),
        ("command", json!("python detector.py")),
        ("command", json!([])),
        ("command", json!([""])),
        ("command", json!(["python", "bad\0argument"])),
        ("intervalSeconds", json!(0)),
        ("intervalSeconds", json!(MAX_INTERVAL_SECONDS + 1)),
        ("timeoutSeconds", json!(0)),
        ("timeoutSeconds", json!(MAX_TIMEOUT_SECONDS + 1)),
        ("backendProfile", json!("")),
        ("backendProfile", json!(" spaced")),
        ("sender", json!("has space")),
        ("sender", json!("control\u{7f}")),
        ("target", json!("")),
        ("target", json!("bad\naddress")),
        ("attention", json!("urgent")),
        ("requiresDisposition", Value::Null),
        ("environmentAllowlist", json!(["NAME", "NAME"])),
        ("environmentAllowlist", json!(["1BAD"])),
        ("environmentAllowlist", json!(["BAD-NAME"])),
        ("environmentAllowlist", json!([""])),
    ] {
        let mut input = registration();
        input[field] = value;
        assert!(reg(&input).is_err(), "accepted invalid {field}: {input}");
    }
    for (interval, timeout) in [
        (MIN_INTERVAL_SECONDS, MIN_TIMEOUT_SECONDS),
        (MAX_INTERVAL_SECONDS, MAX_TIMEOUT_SECONDS),
    ] {
        let mut input = registration();
        input["intervalSeconds"] = json!(interval);
        input["timeoutSeconds"] = json!(timeout);
        input["environmentAllowlist"] = json!(["GH_TOKEN", "_lower123"]);
        input["requiresDisposition"] = json!(false);
        assert!(reg(&input).is_ok());
    }
}

#[test]
fn registration_requires_canonical_existing_absolute_directory() {
    let mut input = registration();
    input["workingDirectory"] = json!(cwd());
    assert_eq!(reg(&input).unwrap().working_directory, cwd());
    for path in [
        PathBuf::new(),
        PathBuf::from("relative"),
        cwd().join("does-not-exist-protocol-v2"),
        cwd().join("Cargo.toml"),
        PathBuf::from(format!(
            "{}{sep}src{sep}..",
            cwd().display(),
            sep = std::path::MAIN_SEPARATOR
        )),
    ] {
        input["workingDirectory"] = json!(path);
        assert!(reg(&input).is_err(), "accepted path {path:?}");
    }
    input["workingDirectory"] = Value::Null;
    assert!(reg(&input).is_err());
    assert!(parse_registration(
        &serde_json::to_vec(&registration()).unwrap(),
        &PathBuf::from(".")
    )
    .is_err());
}

#[test]
fn required_fields_and_strict_envelopes() {
    for field in [
        "schemaVersion",
        "id",
        "command",
        "intervalSeconds",
        "timeoutSeconds",
        "backendProfile",
        "sender",
        "target",
    ] {
        let mut input = registration();
        input.as_object_mut().unwrap().remove(field);
        assert!(reg(&input).is_err(), "registration allowed missing {field}");
    }
    for field in ["schemaVersion", "attempt", "watch", "state"] {
        let mut input = request();
        input.as_object_mut().unwrap().remove(field);
        assert!(req(&input).is_err(), "request allowed missing {field}");
    }
    for field in ["id", "activatedAt", "lastSuccessAt", "parameters"] {
        let mut input = request();
        input["watch"].as_object_mut().unwrap().remove(field);
        assert!(req(&input).is_err(), "watch allowed missing {field}");
    }
    let mut input = request();
    input["attempt"]["extra"] = json!(true);
    assert!(req(&input).is_err());
    let mut input = request();
    input["watch"]["extra"] = json!(true);
    assert!(req(&input).is_err());
    for field in [
        "script",
        "manifest",
        "scriptPath",
        "action",
        "eventKindAllowlist",
    ] {
        let mut input = registration();
        input[field] = json!({});
        assert!(reg(&input).is_err());
        let mut input = request();
        input[field] = json!({});
        assert!(req(&input).is_err());
    }
    for field in ["schemaVersion", "outcome"] {
        let mut input = result("idle");
        input.as_object_mut().unwrap().remove(field);
        assert!(res(&input).is_err());
    }
}

#[test]
fn timestamps_require_utc_precision_and_real_calendar_dates() {
    for value in [
        "2024-02-29T23:59:59Z",
        "2026-01-01T00:00:00.1Z",
        "2026-01-01T00:00:00.12Z",
        "2026-01-01T00:00:00.123Z",
    ] {
        assert!(validate_timestamp(value).is_ok(), "{value}");
        let mut input = request();
        input["attempt"]["now"] = json!(value);
        input["watch"]["activatedAt"] = json!(value);
        input["watch"]["lastSuccessAt"] = json!(value);
        assert!(req(&input).is_ok());
    }
    for value in [
        "",
        "not-a-date",
        "2025-02-29T00:00:00Z",
        "2026-04-31T00:00:00Z",
        "2026-13-01T00:00:00Z",
        "2026-01-00T00:00:00Z",
        "2026-01-01T24:00:00Z",
        "2026-01-01T00:00:60Z",
        "2026-01-01T00:00:00+00:00",
        "2026-01-01t00:00:00z",
        "2026-01-01T00:00:00.Z",
        "2026-01-01T00:00:00.1234Z",
        "2026-01-01 00:00:00Z",
        "2026-01-01T00:00:00Z\n",
        "２０２６-01-01T00:00:00Z",
    ] {
        assert!(validate_timestamp(value).is_err(), "{value}");
        for (object, field) in [
            ("attempt", "now"),
            ("watch", "activatedAt"),
            ("watch", "lastSuccessAt"),
        ] {
            let mut input = request();
            input[object][field] = json!(value);
            assert!(req(&input).is_err(), "{object}.{field} accepted {value}");
        }
    }
    validate_timestamp(&now_timestamp()).unwrap();
}

#[test]
fn outcome_matrix_and_null_state_preservation() {
    for outcome in ["idle", "event", "terminal", "degraded"] {
        for has_event in [false, true] {
            for state in [None, Some(Value::Null), Some(json!({"cursor": 1}))] {
                let mut input = result(outcome);
                if has_event {
                    input["event"] = event();
                }
                if let Some(value) = &state {
                    input["nextState"] = value.clone();
                }
                let valid = match outcome {
                    "idle" => !has_event,
                    "event" => has_event,
                    "terminal" => true,
                    "degraded" => !has_event && state.is_none(),
                    _ => unreachable!(),
                };
                let parsed = res(&input);
                assert_eq!(parsed.is_ok(), valid, "{input}");
                if let Ok(parsed) = parsed {
                    assert_eq!(parsed.next_state, state);
                    let encoded = serde_json::to_value(parsed).unwrap();
                    assert_eq!(encoded.get("nextState"), input.get("nextState"));
                }
            }
        }
    }
    for outcome in ["idle", "event", "terminal", "degraded"] {
        let mut input = result(outcome);
        input["event"] = Value::Null;
        assert!(
            res(&input).is_err(),
            "explicit null event accepted for {outcome}"
        );
    }
    assert!(res(&result("unknown")).is_err());
}

#[test]
fn detector_cannot_author_identity_route_policy_or_action() {
    for field in [
        "id",
        "eventId",
        "eventSequence",
        "operationId",
        "backend",
        "backendProfile",
        "sender",
        "target",
        "attention",
        "requiresDisposition",
        "intervalSeconds",
        "timeoutSeconds",
        "action",
    ] {
        let mut input = result("event");
        input["event"] = event();
        input["event"][field] = json!("not-owned");
        assert!(res(&input).is_err(), "event accepted {field}");
        input["event"].as_object_mut().unwrap().remove(field);
        input[field] = json!("not-owned");
        assert!(res(&input).is_err(), "result accepted {field}");
    }
    let mut input = result("event");
    input["event"] = event();
    input["event"]["metadata"] = json!({"target": "opaque, not routing", "action": "data"});
    assert!(res(&input).is_ok());
}

#[test]
fn recursive_duplicates_are_rejected_before_information_is_lost() {
    for raw in [
        r#"{"schemaVersion":2,"schemaVersion":2,"outcome":"idle"}"#,
        r#"{"schemaVersion":2,"outcome":"idle","nextState":{"a":1,"a":2}}"#,
        r#"{"schemaVersion":2,"outcome":"idle","nextState":[{"a":1,"\u0061":2}]}"#,
        r#"{"schemaVersion":2,"outcome":"event","event":{"kind":"x","subject":"s","body":"","metadata":{"nested":{"x":1,"x":2}}}}"#,
    ] {
        assert!(
            matches!(
                parse_result(raw.as_bytes()),
                Err(ProtocolError::Canonicalization(_))
            ),
            "{raw}"
        );
        assert!(serde_json::from_str::<DetectorResult>(raw).is_err());
    }
    let raw = String::from_utf8(serde_json::to_vec(&registration()).unwrap()).unwrap();
    let raw = raw.replacen('{', r#"{"parameters":{"x":0,"x":1},"#, 1);
    assert!(matches!(
        parse_registration(raw.as_bytes(), &cwd()),
        Err(ProtocolError::Canonicalization(_))
    ));
    let raw = String::from_utf8(serde_json::to_vec(&request()).unwrap()).unwrap();
    let raw = raw.replace(r#""state":null"#, r#""state":{"a":[{"x":1,"x":2}]}"#);
    assert!(matches!(
        parse_request(raw.as_bytes()),
        Err(ProtocolError::Canonicalization(_))
    ));
}

#[test]
fn invalid_unicode_numbers_and_noncanonical_values_are_classified() {
    for value in [
        r#""\ud800""#,
        r#""\udfff""#,
        r#""\uffff""#,
        r#""\ufdd0""#,
        r#""\udbff\udfff""#,
        "1e400",
        "1e-400",
        "9007199254740992",
        "-9007199254740992",
        "99999999999999999999999999999999999999",
    ] {
        let raw = format!(r#"{{"schemaVersion":2,"outcome":"idle","nextState":{value}}}"#);
        assert!(
            matches!(
                parse_result(raw.as_bytes()),
                Err(ProtocolError::Canonicalization(_))
            ),
            "{value}"
        );
    }
    for value in [
        "1e30",
        "5e-324",
        "9007199254740991",
        "-9007199254740991",
        "-0.0",
        r#""\ud83d\ude00""#,
    ] {
        let raw = format!(r#"{{"schemaVersion":2,"outcome":"idle","nextState":{value}}}"#);
        assert!(parse_result(raw.as_bytes()).is_ok(), "{value}");
    }
    assert!(matches!(
        parse_result(&[0xff]),
        Err(ProtocolError::Canonicalization(_))
    ));
    for raw in [
        b"{}".as_slice(),
        b"[]",
        b"{",
        b"null",
        b"{} {}",
        b"{\"schemaVersion\":2,\"outcome\":\"idle\"} chatter",
    ] {
        assert!(
            matches!(parse_result(raw), Err(ProtocolError::Malformed(_))),
            "{raw:?}"
        );
    }
}

#[test]
fn canonicalizer_matches_rfc8785_number_escaping_and_utf16_order() {
    let value = json!({
        "numbers": [333333333.3333333, 1e30, 4.50, 2e-3, 1e-27, -0.0],
        "string": "\u{20ac}$\u{c}\nA'B\"\\\"/",
        "literals": [Value::Null, json!(true), json!(false)]
    });
    assert_eq!(
        String::from_utf8(canonical_bytes(&value).unwrap()).unwrap(),
        "{\"literals\":[null,true,false],\"numbers\":[333333333.3333333,1e+30,4.5,0.002,1e-27,0],\"string\":\"€$\\f\\nA'B\\\"\\\\\\\"/\"}"
    );
    let value = json!({"\u{e000}": 1, "\u{1f600}": 2, "\r": 3, "a": 4});
    assert_eq!(
        String::from_utf8(canonical_bytes(&value).unwrap()).unwrap(),
        "{\"\\r\":3,\"a\":4,\"😀\":2,\"\u{e000}\":1}"
    );
    assert_eq!(
        hash_value(&json!({})).unwrap(),
        "sha256:44136fa355b3678a1146ad16f7e8649e94fb4fc21fe77e8310c060f61caaff8a"
    );
    assert_eq!(
        hash_value(&json!({"b":2,"a":1})).unwrap(),
        hash_value(&json!({"a":1,"b":2})).unwrap()
    );
    assert_ne!(
        hash_value(&json!("é")).unwrap(),
        hash_value(&json!("e\u{301}")).unwrap()
    );
}

#[test]
fn canonicalizer_rejects_nonfinite_and_duplicate_custom_serializers() {
    struct Duplicate;
    impl Serialize for Duplicate {
        fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
            let mut map = serializer.serialize_map(Some(2))?;
            map.serialize_entry("same", &1)?;
            map.serialize_entry("same", &2)?;
            map.end()
        }
    }
    for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert!(matches!(
            canonical_bytes(&value),
            Err(ProtocolError::Canonicalization(_))
        ));
    }
    assert!(matches!(
        canonical_bytes(&Duplicate),
        Err(ProtocolError::Canonicalization(_))
    ));
    assert!(matches!(
        canonical_bytes(&u64::MAX),
        Err(ProtocolError::Canonicalization(_))
    ));
}

#[test]
fn limits_count_utf8_bytes_not_characters() {
    for (field, cap) in [
        ("kind", MAX_KIND_BYTES),
        ("subject", MAX_SUBJECT_BYTES),
        ("body", MAX_BODY_BYTES),
    ] {
        let mut input = result("event");
        input["event"] = event();
        input["event"][field] = json!("é".repeat(cap / 2));
        assert!(res(&input).is_ok(), "exact cap {field}");
        input["event"][field] = json!(format!("{}x", "é".repeat(cap / 2)));
        assert!(
            matches!(res(&input), Err(ProtocolError::OutputLimit(_))),
            "{field}"
        );
    }
    let mut registration = registration();
    registration["parameters"] = json!("x".repeat(MAX_PARAMETERS_BYTES - 2));
    assert!(reg(&registration).is_ok());
    registration["parameters"] = json!("x".repeat(MAX_PARAMETERS_BYTES - 1));
    assert!(matches!(
        reg(&registration),
        Err(ProtocolError::OutputLimit(_))
    ));
    registration["parameters"] = json!({});
    registration["initialState"] = json!("x".repeat(MAX_STATE_BYTES - 1));
    assert!(matches!(
        reg(&registration),
        Err(ProtocolError::OutputLimit(_))
    ));
    let mut input = result("event");
    input["event"] = event();
    input["event"]["metadata"] = json!({"a": "x".repeat(MAX_METADATA_BYTES - 8)});
    assert!(res(&input).is_ok());
    input["event"]["metadata"] = json!({"a": "x".repeat(MAX_METADATA_BYTES - 7)});
    assert!(matches!(res(&input), Err(ProtocolError::OutputLimit(_))));
    assert!(matches!(
        parse_result(&vec![b' '; MAX_STDOUT_BYTES + 1]),
        Err(ProtocolError::OutputLimit(_))
    ));
}

#[test]
fn event_field_shapes_and_controls_match_v2() {
    let mut input = result("event");
    input["event"] = event();
    for value in [json!(null), json!([]), json!("metadata"), json!(42)] {
        input["event"]["metadata"] = value;
        assert!(res(&input).is_err());
    }
    input["event"]["metadata"] = json!({});
    for field in ["kind", "subject"] {
        for value in ["", "tab\tvalue", "line\nvalue", "\u{7f}"] {
            input["event"][field] = json!(value);
            assert!(res(&input).is_err());
        }
        input["event"][field] = json!("opaque plain vocabulary");
    }
    input["event"]["body"] = json!("body may contain\n\tcontrols\u{0}");
    assert!(res(&input).is_ok());
}

#[test]
fn committed_example_inputs_match_protocol() {
    parse_request(include_bytes!("../field-validation/github-pr/request.json")).unwrap();
    parse_registration(
        include_bytes!("../field-validation/github-pr/registration.json"),
        &cwd(),
    )
    .unwrap();
}

#[test]
fn executable_schema_corpus_matches_concrete_protocol_models() {
    let corpus: Value = serde_json::from_slice(include_bytes!(
        "../field-validation/github-pr/protocol-schema-cases.json"
    ))
    .unwrap();
    for case in corpus["cases"].as_array().unwrap() {
        let model = case["model"].as_str().unwrap();
        let mut input = corpus["templates"][model].clone();
        if let Some(changes) = case["set"].as_object() {
            for (pointer, value) in changes {
                let (parent, key) = pointer.rsplit_once('/').unwrap();
                let key = key.replace("~1", "/").replace("~0", "~");
                input
                    .pointer_mut(parent)
                    .unwrap()
                    .as_object_mut()
                    .unwrap()
                    .insert(key, value.clone());
            }
        }
        if let Some(removals) = case["remove"].as_array() {
            for pointer in removals {
                let (parent, key) = pointer.as_str().unwrap().rsplit_once('/').unwrap();
                let key = key.replace("~1", "/").replace("~0", "~");
                assert!(input
                    .pointer_mut(parent)
                    .unwrap()
                    .as_object_mut()
                    .unwrap()
                    .remove(&key)
                    .is_some());
            }
        }
        let parsed = match model {
            "registration" => reg(&input).map(|value| serde_json::to_value(value).unwrap()),
            "request" => req(&input).map(|value| serde_json::to_value(value).unwrap()),
            "result" => res(&input).map(|value| serde_json::to_value(value).unwrap()),
            _ => panic!("unknown protocol model {model}"),
        };
        assert_eq!(
            parsed.is_ok(),
            case["protocolValid"].as_bool().unwrap(),
            "case {}: {parsed:?}",
            case["name"],
        );
        if model == "result" {
            if let Ok(serialized) = parsed {
                assert_eq!(
                    serialized.get("nextState"),
                    input.get("nextState"),
                    "case {} changed omission/null semantics",
                    case["name"],
                );
            }
        }
    }
}
