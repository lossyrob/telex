//! Address filter: a case-insensitive substring matched against a message's `from`/`to`
//! addresses (and against an address-directory entry's address). Read-only and pure, so
//! it is easy to unit-test.

use telex::model::MessageRow;

/// Does `message` match the active address `filter` (case-insensitive substring on
/// `from_addr` or `to_addr`)? An empty filter matches everything.
pub fn message_matches(filter: &str, message: &MessageRow) -> bool {
    let needle = filter.trim().to_lowercase();
    if needle.is_empty() {
        return true;
    }
    message
        .from_addr
        .as_deref()
        .is_some_and(|s| s.to_lowercase().contains(&needle))
        || message.to_addr.to_lowercase().contains(&needle)
}

/// Does an address string match the active `filter` (case-insensitive substring)?
pub fn address_matches(filter: &str, address: &str) -> bool {
    let needle = filter.trim().to_lowercase();
    needle.is_empty() || address.to_lowercase().contains(&needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(from: Option<&str>, to: &str) -> MessageRow {
        MessageRow {
            id: 1,
            thread_id: 1,
            parent_id: None,
            from_addr: from.map(str::to_string),
            to_addr: to.to_string(),
            cc: None,
            kind: "note".into(),
            attention: "background".into(),
            requires_disposition: false,
            subject: None,
            body: String::new(),
            metadata: None,
            sent_at_ms: 0,
            created_at_ms: 0,
        }
    }

    #[test]
    fn empty_filter_matches_all() {
        assert!(message_matches("", &msg(Some("a"), "b")));
        assert!(address_matches("  ", "anything"));
    }

    #[test]
    fn matches_from_or_to_case_insensitively() {
        let m = msg(Some("workstream:impl-215"), "orchestrator:tx-1");
        assert!(message_matches("IMPL", &m));
        assert!(message_matches("orch", &m));
        assert!(!message_matches("node:ci", &m));
    }

    #[test]
    fn handles_missing_from() {
        let m = msg(None, "node:demo");
        assert!(message_matches("demo", &m));
        assert!(!message_matches("missing", &m));
    }
}
