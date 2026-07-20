use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const EXPERIMENTAL_NAMESPACE_KEY: &str = "operator-station-spike";
pub const EXPERIMENTAL_NAMESPACE_URN: &str = "urn:telex:experimental:operator-station-spike:v1";
pub const ESCALATION_KIND: &str = "operator-station-spike.escalation";
pub const HUMAN_REPLY_KIND: &str = "operator-station-spike.human-reply";

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StationMessage {
    pub id: i64,
    pub thread_id: i64,
    pub parent_id: Option<i64>,
    pub from: Option<String>,
    pub to: String,
    pub cc: Vec<String>,
    pub kind: String,
    pub attention: String,
    pub requires_disposition: bool,
    pub subject: Option<String>,
    pub body: String,
    #[serde(rename = "metadata")]
    pub metadata_raw: Option<String>,
    pub sent_at_ms: i64,
    pub created_at_ms: Option<i64>,
    pub delivered_to: String,
    pub primary_to: String,
    pub delivery_role: String,
    pub requires_disposition_for_current_recipient: bool,
    pub latest_disposition: Option<String>,
    pub actionable: bool,
    pub ack_pending: bool,
    pub source_references: Vec<SourceReference>,
    pub metadata_error: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DispositionRecord {
    pub id: i64,
    pub message_id: i64,
    pub recipient: String,
    pub state: String,
    pub note: Option<String>,
    pub by_principal: Option<String>,
    pub at_ms: i64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadEntry {
    pub message: StationMessage,
    pub dispositions: Vec<DispositionRecord>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadView {
    pub message: StationMessage,
    pub dispositions: Vec<DispositionRecord>,
    pub thread: Vec<ThreadEntry>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceReference {
    pub id: i64,
    pub thread_id: i64,
    pub from: Option<String>,
    pub to: Option<String>,
    pub subject: Option<String>,
    pub sent_at_ms: Option<i64>,
    pub store_fingerprint: Option<String>,
    pub availability: SourceAvailability,
}

impl SourceReference {
    pub fn matches_message(&self, message: &StationMessage) -> bool {
        self.id == message.id
            && self.thread_id == message.thread_id
            && self
                .from
                .as_ref()
                .is_some_and(|from| message.from.as_ref() == Some(from))
            && self.to.as_ref().is_some_and(|to| &message.to == to)
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum SourceAvailability {
    Available,
    UnavailableInCurrentStore,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SentReceipt {
    pub receipt: String,
    pub id: i64,
    pub thread_id: i64,
    pub parent_id: Option<i64>,
    pub to: String,
    pub from: Option<String>,
    pub attention: Option<String>,
    pub requires_disposition: Option<bool>,
    pub occupied: Option<bool>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AddressOccupancy {
    pub address: String,
    pub occupied: bool,
    pub age_secs: f64,
    pub occupant: Option<String>,
    pub station_health: Option<String>,
    pub pending_unconsumed_count: Option<i64>,
    pub live_waiters_count: Option<i64>,
    pub error: Option<String>,
    pub refreshed_at_ms: i64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Diagnostic {
    pub at_ms: i64,
    pub level: String,
    pub code: String,
    pub detail: String,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum CourierPhase {
    Starting,
    Backfilling,
    Attaching,
    Armed,
    AckPending,
    Backoff,
    Paused,
    Stopping,
    Stopped,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CourierState {
    pub phase: CourierPhase,
    pub detail: Option<String>,
    pub persistent: bool,
    pub consecutive_daemon_hung: u8,
    pub current_waiter_pid: Option<u32>,
    pub last_exit_code: Option<i32>,
    pub ack_pending_message_id: Option<i64>,
}

impl Default for CourierState {
    fn default() -> Self {
        Self {
            phase: CourierPhase::Starting,
            detail: None,
            persistent: false,
            consecutive_daemon_hung: 0,
            current_waiter_pid: None,
            last_exit_code: None,
            ack_pending_message_id: None,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StationConfigView {
    pub station_address: String,
    pub ingress_address: String,
    pub store_fingerprint: String,
    pub telex_version: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AddressStatusView {
    pub address: String,
    pub occupied: bool,
    pub health: String,
    pub detail: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StationRuntimeStatusView {
    pub phase: String,
    pub detail: Option<String>,
    pub courier_state: String,
    pub station: Option<AddressStatusView>,
    pub ingress: Option<AddressStatusView>,
    pub diagnostics: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StationStateView {
    pub config: StationConfigView,
    pub messages: Vec<StationMessage>,
    pub status: StationRuntimeStatusView,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceReferenceView {
    pub id: i64,
    pub thread_id: i64,
    pub from: Option<String>,
    pub to: String,
    pub subject: Option<String>,
    pub sent_at_ms: i64,
    pub store_fingerprint: Option<String>,
    pub resolution: String,
    pub message: Option<StationMessage>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FrontendThreadView {
    pub selected: StationMessage,
    pub thread: Vec<ThreadEntry>,
    pub sources: Vec<SourceReferenceView>,
    pub raw_metadata: Option<String>,
}

impl StationMessage {
    pub fn parse_metadata(&mut self, active_fingerprint: &str) {
        self.source_references.clear();
        self.metadata_error = None;
        let Some(raw) = self.metadata_raw.as_deref() else {
            return;
        };
        match parse_source_references(raw, active_fingerprint) {
            Ok(Some(references)) => self.source_references = references,
            Ok(None) => {}
            Err(error) => self.metadata_error = Some(error),
        }
    }

    pub fn is_unresolved_primary_disposition(&self, station_address: &str) -> bool {
        self.to == station_address
            && self.requires_disposition
            && !self
                .latest_disposition
                .as_deref()
                .is_some_and(is_terminal_disposition)
    }

    pub fn toast_eligible(&self, station_address: &str) -> bool {
        if self.to != station_address
            || self.delivery_role != "to"
            || self.delivered_to != station_address
        {
            return false;
        }
        match self.attention.as_str() {
            "interrupt" => true,
            "next-checkpoint" if self.requires_disposition_for_current_recipient => true,
            "fyi" => false,
            _ => self.kind == ESCALATION_KIND && self.requires_disposition_for_current_recipient,
        }
    }
}

pub fn is_terminal_disposition(value: &str) -> bool {
    matches!(value, "handled" | "rejected" | "closed")
}

impl CourierPhase {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Starting => "starting",
            Self::Backfilling => "backfilling",
            Self::Attaching => "attaching",
            Self::Armed => "armed",
            Self::AckPending => "ack-pending",
            Self::Backoff => "backoff",
            Self::Paused => "paused",
            Self::Stopping => "stopping",
            Self::Stopped => "stopped",
        }
    }
}

fn parse_source_references(
    raw_metadata: &str,
    active_fingerprint: &str,
) -> Result<Option<Vec<SourceReference>>, String> {
    let value: Value = serde_json::from_str(raw_metadata)
        .map_err(|error| format!("metadata is not valid JSON: {error}"))?;
    let extension = value
        .get("extensions")
        .and_then(|extensions| extensions.get(EXPERIMENTAL_NAMESPACE_KEY))
        .and_then(Value::as_str);
    if extension != Some(EXPERIMENTAL_NAMESPACE_URN) {
        return Ok(None);
    }
    let expected_schema = format!("{EXPERIMENTAL_NAMESPACE_URN}#escalation");
    if value.get("dataschema").and_then(Value::as_str) != Some(expected_schema.as_str()) {
        return Ok(None);
    }
    let source_values = value
        .get("ext")
        .and_then(|ext| ext.get(EXPERIMENTAL_NAMESPACE_KEY))
        .and_then(|extension| extension.get("sourceMessages"))
        .and_then(Value::as_array)
        .ok_or_else(|| "experimental metadata is missing ext sourceMessages".to_string())?;

    source_values
        .iter()
        .map(|source| {
            let id = required_i64(source, "id")?;
            let thread_id = required_i64(source, "threadId")?;
            let store_fingerprint = optional_string(source, "storeFingerprint")?;
            let availability = if store_fingerprint.as_deref() == Some(active_fingerprint) {
                SourceAvailability::Available
            } else {
                SourceAvailability::UnavailableInCurrentStore
            };
            Ok(SourceReference {
                id,
                thread_id,
                from: optional_string(source, "from")?,
                to: optional_string(source, "to")?,
                subject: optional_string(source, "subject")?,
                sent_at_ms: optional_i64(source, "sentAtMs")?,
                store_fingerprint,
                availability,
            })
        })
        .collect::<Result<Vec<_>, String>>()
        .map(Some)
}

fn required_i64(value: &Value, name: &str) -> Result<i64, String> {
    value
        .get(name)
        .and_then(Value::as_i64)
        .ok_or_else(|| format!("experimental source is missing numeric {name}"))
}

fn optional_i64(value: &Value, name: &str) -> Result<Option<i64>, String> {
    match value.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_i64()
            .map(Some)
            .ok_or_else(|| format!("experimental source {name} must be numeric")),
    }
}

fn optional_string(value: &Value, name: &str) -> Result<Option<String>, String> {
    match value.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_str()
            .map(|value| Some(value.to_string()))
            .ok_or_else(|| format!("experimental source {name} must be a string")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(attention: &str, kind: &str, required: bool, role: &str) -> StationMessage {
        StationMessage {
            id: 1,
            thread_id: 1,
            parent_id: None,
            from: Some("attention:rob".into()),
            to: "operator:rob".into(),
            cc: vec![],
            kind: kind.into(),
            attention: attention.into(),
            requires_disposition: required,
            subject: Some("Decision".into()),
            body: "Choose".into(),
            metadata_raw: None,
            sent_at_ms: 1,
            created_at_ms: Some(1),
            delivered_to: "operator:rob".into(),
            primary_to: "operator:rob".into(),
            delivery_role: role.into(),
            requires_disposition_for_current_recipient: required,
            latest_disposition: None,
            actionable: required,
            ack_pending: false,
            source_references: vec![],
            metadata_error: None,
        }
    }

    #[test]
    fn toast_policy_is_narrow_and_primary_only() {
        assert!(message("interrupt", "note", false, "to").toast_eligible("operator:rob"));
        assert!(message("next-checkpoint", "note", true, "to").toast_eligible("operator:rob"));
        assert!(message("background", ESCALATION_KIND, true, "to").toast_eligible("operator:rob"));
        assert!(!message("fyi", ESCALATION_KIND, true, "to").toast_eligible("operator:rob"));
        assert!(!message("interrupt", "note", false, "cc").toast_eligible("operator:rob"));
        assert!(!message("background", "note", false, "to").toast_eligible("operator:rob"));
    }

    #[test]
    fn source_resolution_requires_exact_experimental_namespace_and_fingerprint() {
        let metadata = serde_json::json!({
            "extensions": {
                EXPERIMENTAL_NAMESPACE_KEY: EXPERIMENTAL_NAMESPACE_URN
            },
            "dataschema": format!("{EXPERIMENTAL_NAMESPACE_URN}#escalation"),
            "ext": {
                EXPERIMENTAL_NAMESPACE_KEY: {
                    "sourceMessages": [{
                        "id": 123,
                        "threadId": 120,
                        "from": "worker:a",
                        "storeFingerprint": "sha256:active"
                    }]
                }
            }
        })
        .to_string();
        let available = parse_source_references(&metadata, "sha256:active")
            .unwrap()
            .unwrap();
        assert_eq!(available[0].availability, SourceAvailability::Available);
        let unavailable = parse_source_references(&metadata, "sha256:other")
            .unwrap()
            .unwrap();
        assert_eq!(
            unavailable[0].availability,
            SourceAvailability::UnavailableInCurrentStore
        );

        let production_looking = metadata.replace(EXPERIMENTAL_NAMESPACE_KEY, "operator-station");
        assert!(
            parse_source_references(&production_looking, "sha256:active")
                .unwrap()
                .is_none()
        );

        let other_schema = metadata.replace("#escalation", "#other");
        assert!(parse_source_references(&other_schema, "sha256:active")
            .unwrap()
            .is_none());

        let mut missing_schema: Value = serde_json::from_str(&metadata).unwrap();
        missing_schema.as_object_mut().unwrap().remove("dataschema");
        assert!(
            parse_source_references(&missing_schema.to_string(), "sha256:active")
                .unwrap()
                .is_none()
        );

        let foreign_namespace = metadata.replace(
            EXPERIMENTAL_NAMESPACE_URN,
            "urn:telex:experimental:foreign:v1",
        );
        assert!(parse_source_references(&foreign_namespace, "sha256:active")
            .unwrap()
            .is_none());
    }

    #[test]
    fn source_reference_matches_documented_identity_fields() {
        let message = message("interrupt", ESCALATION_KIND, true, "to");
        let matching = SourceReference {
            id: message.id,
            thread_id: message.thread_id,
            from: message.from.clone(),
            to: Some(message.to.clone()),
            subject: message.subject.clone(),
            sent_at_ms: Some(message.sent_at_ms),
            store_fingerprint: Some("sha256:active".into()),
            availability: SourceAvailability::Available,
        };
        assert!(matching.matches_message(&message));

        let mut mismatched = matching;
        mismatched.from = Some("worker:other".into());
        assert!(!mismatched.matches_message(&message));

        let missing_from = SourceReference {
            from: None,
            ..mismatched.clone()
        };
        assert!(!missing_from.matches_message(&message));

        let missing_to = SourceReference {
            from: message.from.clone(),
            to: None,
            ..mismatched
        };
        assert!(!missing_to.matches_message(&message));
    }

    #[test]
    fn opaque_metadata_is_double_parsed_and_malformed_recognized_data_is_visible() {
        let outer_message_metadata = serde_json::json!({
            "metadata": "{\"extensions\":{\"other\":\"urn:other\"}}"
        });
        let raw = outer_message_metadata["metadata"].as_str().unwrap();
        assert!(parse_source_references(raw, "sha256:x").unwrap().is_none());

        let malformed = format!(
            r#"{{"extensions":{{"{EXPERIMENTAL_NAMESPACE_KEY}":"{EXPERIMENTAL_NAMESPACE_URN}"}},"dataschema":"{EXPERIMENTAL_NAMESPACE_URN}#escalation","ext":{{"{EXPERIMENTAL_NAMESPACE_KEY}":{{}}}}}}"#
        );
        assert!(parse_source_references(&malformed, "sha256:x").is_err());
    }

    #[test]
    fn station_message_serializes_frontend_contract_in_camel_case() {
        let mut message = message("interrupt", "note", false, "to");
        message.metadata_raw = Some("{\"opaque\":true}".into());
        let value = serde_json::to_value(message).unwrap();
        assert_eq!(value["threadId"], 1);
        assert_eq!(value["metadata"], "{\"opaque\":true}");
        assert!(value.get("metadataRaw").is_none());
        assert!(value.get("requiresDisposition").is_some());
    }
}
