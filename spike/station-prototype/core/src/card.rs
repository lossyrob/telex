//! The Station presentation convention carried in opaque Telex metadata.
//!
//! ```json
//! { "station": { "v": 0,
//!     "card": { "name": "impl-134", "harness": "claude-code", "host": "devbox-2",
//!               "repo": "lossyrob/telex", "branch": "feature/x", "cwd": "/home/rob/proj/telex",
//!               "session_id": "…", "session_name": "…", "terminal": "tmux main:3.1",
//!               "open_command": "ssh devbox-2 -t tmux attach -t main",
//!               "project_hint": "telex/station", "url": "https://…", "tags": ["impl"] },
//!     "replies": ["Approve", "Hold"] } }
//! ```
//!
//! Everything here is self-reported and unverified. It only changes how Station
//! *presents* a message; it can never override a Telex core field, select a
//! command, or change notification tier.

use crate::model::Card;
use serde_json::Value;

pub const NAMESPACE: &str = "station";
const MAX_FIELD: usize = 400;
const MAX_REPLIES: usize = 6;
const MAX_REPLY_LEN: usize = 120;

#[derive(Clone, Debug, Default)]
pub struct ParsedMetadata {
    pub card: Option<Card>,
    pub replies: Vec<String>,
    /// Remaining metadata Station does not interpret (pretty JSON or raw text).
    pub other: Option<String>,
}

fn clean(value: Option<&Value>) -> Option<String> {
    let text = match value? {
        Value::String(s) => s.trim().to_string(),
        Value::Number(n) => n.to_string(),
        _ => return None,
    };
    if text.is_empty() {
        return None;
    }
    // Inert text only: strip control characters and bound the length.
    let text: String = text.chars().filter(|c| !c.is_control()).take(MAX_FIELD).collect();
    Some(text)
}

pub fn parse(metadata: Option<&str>) -> ParsedMetadata {
    let Some(raw) = metadata.map(str::trim).filter(|m| !m.is_empty()) else {
        return ParsedMetadata::default();
    };
    let Ok(Value::Object(mut root)) = serde_json::from_str::<Value>(raw) else {
        return ParsedMetadata { other: Some(raw.chars().take(4000).collect()), ..Default::default() };
    };
    let station = root.remove(NAMESPACE);
    let mut parsed = ParsedMetadata::default();
    if let Some(Value::Object(station)) = station {
        if let Some(Value::Object(card)) = station.get("card") {
            let c = Card {
                name: clean(card.get("name")),
                harness: clean(card.get("harness")).map(|h| h.to_lowercase()),
                model: clean(card.get("model")),
                host: clean(card.get("host")),
                os: clean(card.get("os")),
                cwd: clean(card.get("cwd")),
                repo: clean(card.get("repo")),
                branch: clean(card.get("branch")),
                session_id: clean(card.get("session_id")),
                session_name: clean(card.get("session_name")),
                terminal: clean(card.get("terminal")),
                open_command: clean(card.get("open_command")),
                project_hint: clean(card.get("project_hint")),
                url: clean(card.get("url")),
                tags: match card.get("tags") {
                    Some(Value::Array(items)) => {
                        items.iter().filter_map(|v| clean(Some(v))).take(12).collect()
                    }
                    _ => Vec::new(),
                },
            };
            if !c.is_empty() {
                parsed.card = Some(c);
            }
        }
        if let Some(Value::Array(items)) = station.get("replies") {
            parsed.replies = items
                .iter()
                .filter_map(|v| clean(Some(v)))
                .map(|s| s.chars().take(MAX_REPLY_LEN).collect())
                .take(MAX_REPLIES)
                .collect();
        }
    }
    if !root.is_empty() {
        parsed.other = serde_json::to_string_pretty(&Value::Object(root)).ok();
    }
    parsed
}

/// Metadata Station attaches to human-authored messages so agents (and other
/// Stations) can tell a person wrote it. Opaque to Telex.
pub fn human_metadata(operator: &str) -> String {
    serde_json::json!({ NAMESPACE: { "v": 0, "author": "human", "operator": operator } }).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_card_replies_and_keeps_other_metadata_opaque() {
        let m = r#"{"station":{"v":0,"card":{"name":"impl-134","harness":"Claude-Code","host":"devbox-2","tags":["a","b"]},"replies":["Yes","No"]},"ticket":"T-1"}"#;
        let p = parse(Some(m));
        let card = p.card.unwrap();
        assert_eq!(card.name.as_deref(), Some("impl-134"));
        assert_eq!(card.harness.as_deref(), Some("claude-code"));
        assert_eq!(card.tags, vec!["a", "b"]);
        assert_eq!(p.replies, vec!["Yes", "No"]);
        assert!(p.other.unwrap().contains("ticket"));
    }

    #[test]
    fn tolerates_non_json_and_wrong_shapes() {
        assert!(parse(None).card.is_none());
        let p = parse(Some("not json"));
        assert!(p.card.is_none());
        assert_eq!(p.other.as_deref(), Some("not json"));
        let p = parse(Some(r#"{"station":{"card":{"name":{"nested":1},"host":7},"replies":"x"}}"#));
        let card = p.card.unwrap();
        assert!(card.name.is_none());
        assert_eq!(card.host.as_deref(), Some("7"));
        assert!(p.replies.is_empty());
    }

    #[test]
    fn strips_control_characters_and_bounds_length() {
        let long = "x".repeat(2000);
        let m = serde_json::json!({"station":{"card":{"name": format!("a\u{7}b{long}")}}}).to_string();
        let name = parse(Some(&m)).card.unwrap().name.unwrap();
        assert!(name.starts_with("abx"));
        assert_eq!(name.chars().count(), 400);
    }
}
