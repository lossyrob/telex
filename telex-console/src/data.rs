//! Data access layer: a thin, read-only wrapper over the core `Backend` trait. All
//! backend calls funnel through here so the UI never touches `telex::backend` directly.

use std::sync::Arc;

use anyhow::Result;
use telex::backend::Backend;
use telex::model::{DispositionRow, InboxItem, MessageRow};

pub use telex::model::AddressRow;

/// Liveness window (seconds) used to decide whether an address is currently occupied.
/// Mirrors the core default (`TELEX_LIVENESS_WINDOW_SECS`, default 15).
fn liveness_window_secs() -> i64 {
    std::env::var("TELEX_LIVENESS_WINDOW_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(15)
}

/// Occupancy state of an address, with `Unknown` reserved for lookup failures so one bad
/// address never breaks the directory view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Occ {
    Live,
    Idle,
    Unknown,
}

/// An address directory entry plus its resolved occupancy.
#[derive(Clone, Debug)]
pub struct AddressEntry {
    pub address: AddressRow,
    pub occupancy: Occ,
}

/// Read-only store over a backend.
pub struct Store {
    backend: Arc<dyn Backend>,
}

impl Store {
    pub fn new(backend: Arc<dyn Backend>) -> Self {
        Self { backend }
    }

    pub fn kind(&self) -> &'static str {
        self.backend.kind()
    }

    /// Greatest message id across all addresses (for seeding the feed backfill cursor).
    pub async fn max_message_id(&self) -> Result<i64> {
        self.backend.max_message_id().await
    }

    /// Global feed: all messages with `id > cursor`, oldest first.
    pub async fn feed_since(&self, cursor: i64) -> Result<Vec<MessageRow>> {
        self.backend.export(None, None, cursor).await
    }

    /// Address directory with per-address occupancy. A failed occupancy lookup degrades
    /// that entry to `Occ::Unknown` rather than failing the whole call.
    pub async fn addresses(&self) -> Result<Vec<AddressEntry>> {
        let window = liveness_window_secs();
        let rows = self.backend.list_addresses(None, false).await?;
        let mut out = Vec::with_capacity(rows.len());
        for address in rows {
            let occupancy = match self.backend.occupancy(&address.address, window).await {
                Ok(o) if o.occupied => Occ::Live,
                Ok(_) => Occ::Idle,
                Err(_) => Occ::Unknown,
            };
            out.push(AddressEntry { address, occupancy });
        }
        Ok(out)
    }

    /// Recent messages addressed to `address`, with disposition/actionable rollups.
    pub async fn address_inbox(&self, address: &str, limit: i64) -> Result<Vec<InboxItem>> {
        self.backend.inbox(address, true, limit).await
    }

    /// All messages in a thread (root id == `thread_id`), oldest first.
    pub async fn thread(&self, thread_id: i64) -> Result<Vec<MessageRow>> {
        self.backend.thread_messages(thread_id).await
    }

    /// Disposition history for a single message.
    pub async fn dispositions(&self, message_id: i64) -> Result<Vec<DispositionRow>> {
        self.backend.dispositions_for(message_id).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use telex::backend::sqlite::SqliteBackend;
    use telex::model::{Attention, NewMessage};

    fn new_msg(to: &str, from: &str, subject: &str) -> NewMessage {
        NewMessage {
            parent_id: None,
            from_addr: Some(from.into()),
            to_addr: to.into(),
            cc: None,
            kind: "note".into(),
            attention: Attention::Background,
            requires_disposition: false,
            subject: Some(subject.into()),
            body: "body".into(),
            metadata: None,
            sent_at_ms: 1_000,
        }
    }

    async fn seeded_store() -> (Store, Arc<dyn Backend>) {
        let b = SqliteBackend::open(":memory:").unwrap();
        b.init_schema().await.unwrap();
        let backend: Arc<dyn Backend> = Arc::new(b);
        backend.ensure_address("node:demo", None, None, None).await.unwrap();
        backend.insert_message(&new_msg("node:demo", "me", "one")).await.unwrap();
        backend.insert_message(&new_msg("node:demo", "me", "two")).await.unwrap();
        (Store::new(backend.clone()), backend)
    }

    #[tokio::test]
    async fn feed_and_max_id() {
        let (store, _b) = seeded_store().await;
        assert_eq!(store.max_message_id().await.unwrap(), 2);
        let feed = store.feed_since(0).await.unwrap();
        assert_eq!(feed.len(), 2);
        // cursor past the first message yields only the second
        let tail = store.feed_since(1).await.unwrap();
        assert_eq!(tail.len(), 1);
        assert_eq!(tail[0].id, 2);
    }

    #[tokio::test]
    async fn addresses_and_inbox() {
        let (store, _b) = seeded_store().await;
        let addrs = store.addresses().await.unwrap();
        assert!(addrs.iter().any(|a| a.address.address == "node:demo"));
        // never occupied in this test => Idle, not Unknown
        let demo = addrs.iter().find(|a| a.address.address == "node:demo").unwrap();
        assert_eq!(demo.occupancy, Occ::Idle);

        let inbox = store.address_inbox("node:demo", 50).await.unwrap();
        assert_eq!(inbox.len(), 2);
    }
}

