use anyhow::{anyhow, bail, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

pub const SCHEMA_VERSION: u32 = 1;
pub const MAX_STDOUT_BYTES: usize = 256 * 1024;
pub const MAX_STDERR_BYTES: usize = 64 * 1024;
pub const MAX_STATE_BYTES: usize = 256 * 1024;
pub const MAX_SUBJECT_BYTES: usize = 512;
pub const MAX_BODY_BYTES: usize = 128 * 1024;
pub const MAX_METADATA_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DetectorRequest {
    pub schema_version: u32,
    pub attempt: AttemptRequest,
    pub watch: WatchRequest,
    pub script: ScriptRequest,
    pub state: Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct AttemptRequest {
    pub id: String,
    pub now: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct WatchRequest {
    pub id: String,
    pub parameters: Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct ScriptRequest {
    pub mode: String,
    pub sha256: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DetectorResult {
    pub schema_version: u32,
    pub outcome: Outcome,
    #[serde(default)]
    pub next_state: Option<Value>,
    #[serde(default)]
    pub event: Option<DetectorEvent>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Outcome {
    Idle,
    Event,
    Terminal,
    Degraded,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DetectorEvent {
    pub id: String,
    pub kind: String,
    pub subject: String,
    pub body: String,
    #[serde(default)]
    pub metadata: Value,
}

#[derive(Debug, Clone)]
pub struct ValidatedResult {
    pub outcome: Outcome,
    pub next_state: Option<Value>,
    pub event: Option<DetectorEvent>,
}

pub fn parse_result(stdout: &[u8]) -> Result<ValidatedResult> {
    if stdout.len() > MAX_STDOUT_BYTES {
        bail!("detector stdout exceeded {MAX_STDOUT_BYTES} bytes");
    }
    let result: DetectorResult = serde_json::from_slice(stdout)
        .map_err(|error| anyhow!("invalid detector JSON: {error}"))?;
    validate_result(result)
}

pub fn validate_result(result: DetectorResult) -> Result<ValidatedResult> {
    if result.schema_version != SCHEMA_VERSION {
        bail!(
            "unsupported detector result schemaVersion {}; expected {}",
            result.schema_version,
            SCHEMA_VERSION
        );
    }
    if let Some(state) = &result.next_state {
        ensure_json_cap("nextState", state, MAX_STATE_BYTES)?;
    }

    match result.outcome {
        Outcome::Idle => {
            if result.event.is_some() {
                bail!("idle result must not contain event");
            }
        }
        Outcome::Event => {
            if result.event.is_none() {
                bail!("event result requires event");
            }
        }
        Outcome::Terminal => {}
        Outcome::Degraded => {
            if result.event.is_some() || result.next_state.is_some() {
                bail!("degraded result must not contain event or nextState");
            }
        }
    }

    if let Some(event) = &result.event {
        validate_event(event)?;
    }

    Ok(ValidatedResult {
        outcome: result.outcome,
        next_state: result.next_state,
        event: result.event,
    })
}

pub fn validate_event(event: &DetectorEvent) -> Result<()> {
    ensure_nonempty("event.id", &event.id, 512)?;
    ensure_nonempty("event.kind", &event.kind, 256)?;
    if !event.kind.contains('.') || event.kind.starts_with('.') || event.kind.ends_with('.') {
        bail!("event.kind must be a namespaced kind");
    }
    ensure_nonempty("event.subject", &event.subject, MAX_SUBJECT_BYTES)?;
    if event.body.len() > MAX_BODY_BYTES {
        bail!("event.body exceeded {MAX_BODY_BYTES} UTF-8 bytes");
    }
    ensure_json_cap("event.metadata", &event.metadata, MAX_METADATA_BYTES)?;
    Ok(())
}

pub fn state_json(value: &Value) -> Result<String> {
    ensure_json_cap("state", value, MAX_STATE_BYTES)?;
    Ok(serde_json::to_string(value)?)
}

pub fn parse_state(raw: &str) -> Result<Value> {
    let value =
        serde_json::from_str(raw).map_err(|error| anyhow!("invalid stored state: {error}"))?;
    ensure_json_cap("state", &value, MAX_STATE_BYTES)?;
    Ok(value)
}

pub fn hash_value(value: &Value) -> Result<String> {
    Ok(sha256(&serde_json::to_vec(value)?))
}

pub fn sha256(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut text = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write;
        let _ = write!(text, "{byte:02x}");
    }
    text
}

pub fn normalized_envelope(
    watch_id: &str,
    sender: &str,
    target: &str,
    event: &DetectorEvent,
) -> Result<Value> {
    let metadata = match &event.metadata {
        Value::Object(map) => Value::Object(map.clone()),
        other => other.clone(),
    };
    let mut envelope = BTreeMap::new();
    envelope.insert("eventId", Value::String(event.id.clone()));
    envelope.insert("kind", Value::String(event.kind.clone()));
    envelope.insert("metadata", metadata);
    envelope.insert("schemaVersion", Value::from(SCHEMA_VERSION));
    envelope.insert("sender", Value::String(sender.to_string()));
    envelope.insert("subject", Value::String(event.subject.clone()));
    envelope.insert("target", Value::String(target.to_string()));
    envelope.insert("watchId", Value::String(watch_id.to_string()));
    envelope.insert("body", Value::String(event.body.clone()));
    Ok(serde_json::to_value(envelope)?)
}

pub fn send_metadata(
    watch_id: &str,
    attempt_id: &str,
    script_mode: &str,
    script_digest: &str,
    event: &DetectorEvent,
) -> Result<String> {
    let value = serde_json::json!({
        "schemaVersion": SCHEMA_VERSION,
        "watchId": watch_id,
        "eventId": event.id,
        "attemptId": attempt_id,
        "script": { "mode": script_mode, "sha256": script_digest },
        "detector": event.metadata,
    });
    ensure_json_cap("normalized metadata", &value, MAX_METADATA_BYTES)?;
    Ok(serde_json::to_string(&value)?)
}

fn ensure_nonempty(field: &str, value: &str, cap: usize) -> Result<()> {
    if value.trim().is_empty() {
        bail!("{field} must not be empty");
    }
    if value.len() > cap {
        bail!("{field} exceeded {cap} UTF-8 bytes");
    }
    if value.chars().any(char::is_control) {
        bail!("{field} must not contain control characters");
    }
    Ok(())
}

fn ensure_json_cap(field: &str, value: &Value, cap: usize) -> Result<()> {
    let bytes = serde_json::to_vec(value)?;
    if bytes.len() > cap {
        bail!("{field} exceeded {cap} serialized bytes");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_event_policy_fields() {
        let raw = br#"{
          "schemaVersion": 1,
          "outcome": "event",
          "event": {
            "id": "provider:1",
            "kind": "watch.test",
            "subject": "test",
            "body": "body",
            "attention": "interrupt"
          }
        }"#;
        assert!(parse_result(raw).is_err());
    }

    #[test]
    fn rejects_degraded_state_advancement() {
        let raw = br#"{
          "schemaVersion": 1,
          "outcome": "degraded",
          "nextState": {"cursor": 2}
        }"#;
        assert!(parse_result(raw).is_err());
    }

    #[test]
    fn event_hash_is_stable_for_the_same_normalized_envelope() {
        let event = DetectorEvent {
            id: "provider:1".into(),
            kind: "watch.test".into(),
            subject: "test".into(),
            body: "body".into(),
            metadata: serde_json::json!({"provider": "fixture"}),
        };
        let first = normalized_envelope("watch", "sender", "target", &event).unwrap();
        let second = normalized_envelope("watch", "sender", "target", &event).unwrap();
        assert_eq!(hash_value(&first).unwrap(), hash_value(&second).unwrap());
    }

    fn base_event() -> DetectorEvent {
        DetectorEvent {
            id: "provider:1".into(),
            kind: "watch.test".into(),
            subject: "subject".into(),
            body: "body".into(),
            metadata: serde_json::json!({}),
        }
    }

    #[test]
    fn stdout_over_cap_is_rejected_before_parsing() {
        let oversize = vec![b' '; MAX_STDOUT_BYTES + 1];
        let error = parse_result(&oversize).unwrap_err().to_string();
        assert!(error.contains("exceeded"), "{error}");
    }

    #[test]
    fn event_subject_over_cap_is_rejected() {
        let mut event = base_event();
        event.subject = "s".repeat(MAX_SUBJECT_BYTES + 1);
        assert!(validate_event(&event).is_err());
    }

    #[test]
    fn event_body_over_cap_is_rejected() {
        let mut event = base_event();
        event.body = "b".repeat(MAX_BODY_BYTES + 1);
        assert!(validate_event(&event).is_err());
    }

    #[test]
    fn event_metadata_over_cap_is_rejected() {
        let mut event = base_event();
        event.metadata = serde_json::json!({ "blob": "m".repeat(MAX_METADATA_BYTES + 1) });
        assert!(validate_event(&event).is_err());
    }

    #[test]
    fn next_state_over_cap_is_rejected() {
        let big = serde_json::json!({ "blob": "x".repeat(MAX_STATE_BYTES + 1) });
        let result = DetectorResult {
            schema_version: SCHEMA_VERSION,
            outcome: Outcome::Idle,
            next_state: Some(big),
            event: None,
        };
        assert!(validate_result(result).is_err());
    }

    #[test]
    fn event_kind_must_be_namespaced() {
        for bad in ["nodot", ".leading", "trailing."] {
            let mut event = base_event();
            event.kind = bad.into();
            assert!(
                validate_event(&event).is_err(),
                "expected {bad} to be rejected"
            );
        }
        let mut event = base_event();
        event.kind = "watch.ok".into();
        assert!(validate_event(&event).is_ok());
    }

    #[test]
    fn event_fields_reject_control_characters() {
        let mut event = base_event();
        event.subject = "line\nbreak".into();
        assert!(validate_event(&event).is_err());
    }

    #[test]
    fn event_outcome_requires_event_and_idle_forbids_it() {
        let event_missing = DetectorResult {
            schema_version: SCHEMA_VERSION,
            outcome: Outcome::Event,
            next_state: None,
            event: None,
        };
        assert!(validate_result(event_missing).is_err());

        let idle_with_event = DetectorResult {
            schema_version: SCHEMA_VERSION,
            outcome: Outcome::Idle,
            next_state: None,
            event: Some(base_event()),
        };
        assert!(validate_result(idle_with_event).is_err());
    }

    #[test]
    fn unsupported_schema_version_is_rejected() {
        let raw = br#"{ "schemaVersion": 2, "outcome": "idle" }"#;
        assert!(parse_result(raw).is_err());
    }

    #[test]
    fn terminal_with_malformed_event_is_rejected() {
        let mut event = base_event();
        event.kind = "nonamespace".into();
        let result = DetectorResult {
            schema_version: SCHEMA_VERSION,
            outcome: Outcome::Terminal,
            next_state: None,
            event: Some(event),
        };
        assert!(validate_result(result).is_err());
    }
}
