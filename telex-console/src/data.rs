//! Data access layer: a thin, read-only wrapper over the core `Backend` trait. All
//! backend calls funnel through here so the UI never touches `telex::backend` directly.

use std::sync::Arc;

use anyhow::Result;
use telex::backend::Backend;
use telex::model::{DeliveryRow, DispositionRow, InboxItem, MessageRow};

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

/// An address directory entry plus its resolved occupancy and undelivered backlog count.
#[derive(Clone, Debug)]
pub struct AddressEntry {
    pub address: AddressRow,
    pub occupancy: Occ,
    /// Count of messages queued to this address that have not yet been consumed
    /// and are not terminally dispositioned. `None` when the lookup failed.
    pub undelivered: Option<usize>,
}

/// Read-only store over a backend. Cheap to clone (shares the backend `Arc`), so it can
/// be handed to a background directory-refresh task.
#[derive(Clone)]
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

    /// Global feed page: up to `limit` messages with `id > cursor`, oldest first. Bounded so a
    /// large backlog (e.g. `--backfill all`) is drained in chunks rather than materialized whole.
    pub async fn feed_page(&self, cursor: i64, limit: i64) -> Result<Vec<MessageRow>> {
        self.backend.feed_page(cursor, limit).await
    }

    /// Address directory with per-address occupancy and undelivered backlog count. The backlog
    /// counts come from a single **read-only** bulk query (`undelivered_counts`), so the inspector
    /// never mutates the store. A failed occupancy lookup degrades that entry to `Unknown`.
    pub async fn addresses(&self) -> Result<Vec<AddressEntry>> {
        let window = liveness_window_secs();
        let rows = self.backend.list_addresses(None, false).await?;
        // One pure-SELECT query for all addresses' backlog (never materializes delivery rows).
        let counts: std::collections::HashMap<String, usize> = self
            .backend
            .undelivered_counts()
            .await
            .unwrap_or_default()
            .into_iter()
            .map(|(addr, n)| (addr, n.max(0) as usize))
            .collect();
        let mut out = Vec::with_capacity(rows.len());
        for address in rows {
            let occupancy = match self.backend.occupancy(&address.address, window).await {
                Ok(o) if o.occupied => Occ::Live,
                Ok(_) => Occ::Idle,
                Err(_) => Occ::Unknown,
            };
            let undelivered = Some(counts.get(&address.address).copied().unwrap_or(0));
            out.push(AddressEntry {
                address,
                occupancy,
                undelivered,
            });
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

    /// Delivery records for a single message (one per recipient that received it).
    pub async fn deliveries(&self, message_id: i64) -> Result<Vec<DeliveryRow>> {
        self.backend.deliveries_for(message_id).await
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
        backend
            .ensure_address("node:demo", None, None, None)
            .await
            .unwrap();
        backend
            .insert_message(&new_msg("node:demo", "me", "one"))
            .await
            .unwrap();
        backend
            .insert_message(&new_msg("node:demo", "me", "two"))
            .await
            .unwrap();
        (Store::new(backend.clone()), backend)
    }

    #[tokio::test]
    async fn feed_and_max_id() {
        let (store, _b) = seeded_store().await;
        assert_eq!(store.max_message_id().await.unwrap(), 2);
        let feed = store.feed_page(0, 1000).await.unwrap();
        assert_eq!(feed.len(), 2);
        // cursor past the first message yields only the second
        let tail = store.feed_page(1, 1000).await.unwrap();
        assert_eq!(tail.len(), 1);
        assert_eq!(tail[0].id, 2);
        // limit bounds the page
        let one = store.feed_page(0, 1).await.unwrap();
        assert_eq!(one.len(), 1);
        assert_eq!(one[0].id, 1);
    }

    #[tokio::test]
    async fn addresses_and_inbox() {
        let (store, _b) = seeded_store().await;
        let addrs = store.addresses().await.unwrap();
        assert!(addrs.iter().any(|a| a.address.address == "node:demo"));
        // never occupied in this test => Idle, not Unknown
        let demo = addrs
            .iter()
            .find(|a| a.address.address == "node:demo")
            .unwrap();
        assert_eq!(demo.occupancy, Occ::Idle);

        let inbox = store.address_inbox("node:demo", 50).await.unwrap();
        assert_eq!(inbox.len(), 2);
    }

    #[tokio::test]
    async fn undelivered_count_and_deliveries() {
        let (store, backend) = seeded_store().await;
        // Fan-out creates one pending delivery row per message at insert; both are undelivered.
        let addrs = store.addresses().await.unwrap();
        let demo = addrs
            .iter()
            .find(|a| a.address.address == "node:demo")
            .unwrap();
        assert_eq!(demo.undelivered, Some(2));

        // Read-only: computing the backlog (addresses -> undelivered_counts / list_addresses /
        // occupancy) must not consume or add delivery rows. Both fan-out rows stay pending.
        let d1 = store.deliveries(1).await.unwrap();
        assert_eq!(d1.len(), 1);
        assert_eq!(d1[0].recipient, "node:demo");
        assert!(d1[0].consumed_at_ms.is_none());
        assert_eq!(store.deliveries(2).await.unwrap().len(), 1);

        // Consume message 1 via the backend; the backlog drops and the row reads back consumed.
        backend
            .mark_delivered(1, "node:demo", Some("holderA"))
            .await
            .unwrap();
        let addrs = store.addresses().await.unwrap();
        let demo = addrs
            .iter()
            .find(|a| a.address.address == "node:demo")
            .unwrap();
        assert_eq!(demo.undelivered, Some(1));
        let dels = store.deliveries(1).await.unwrap();
        assert_eq!(dels.len(), 1);
        assert!(dels[0].consumed_at_ms.is_some());
    }
}
