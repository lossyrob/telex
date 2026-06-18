//! Backend abstraction: one `Backend` trait with SQLite and Postgres implementations
//! selected at runtime. The ephemeral `wait` client never touches this — it speaks only
//! to the holder over local IPC. This is the "same semantic core, two backends" promise.

use anyhow::Result;
use async_trait::async_trait;

use crate::model::*;

#[cfg(feature = "postgres")]
pub mod postgres;
#[cfg(feature = "sqlite")]
pub mod sqlite;

/// What a backend can do, so the core can adapt behavior honestly.
#[derive(Clone, Copy, Debug)]
pub struct Capabilities {
    pub durable: bool,
    /// "native" (LISTEN/NOTIFY) or "poll".
    pub push: &'static str,
    /// "ttl" (heartbeat window) in v0 for both backends.
    pub lease: &'static str,
}

#[async_trait]
pub trait Backend: Send + Sync {
    fn kind(&self) -> &'static str;
    fn capabilities(&self) -> Capabilities;

    async fn init_schema(&self) -> Result<()>;

    // ---- addresses / directory ----
    async fn ensure_address(
        &self,
        address: &str,
        description: Option<&str>,
        scope: Option<&str>,
        tags: Option<&str>,
    ) -> Result<()>;
    async fn get_address(&self, address: &str) -> Result<Option<AddressRow>>;
    async fn set_address_status(&self, address: &str, status: &str) -> Result<bool>;
    async fn list_addresses(
        &self,
        scope: Option<&str>,
        include_retired: bool,
    ) -> Result<Vec<AddressRow>>;

    // ---- leases / liveness ----
    async fn claim_lease(&self, claim: &LeaseClaim, window_secs: i64) -> Result<LeaseOutcome>;
    async fn heartbeat(&self, address: &str) -> Result<()>;
    async fn release_lease(&self, address: &str, occupant: &str) -> Result<bool>;
    async fn get_lease(&self, address: &str) -> Result<Option<LeaseRow>>;
    async fn occupancy(&self, address: &str, window_secs: i64) -> Result<Occupancy>;

    // ---- messages ----
    async fn max_id(&self, address: &str) -> Result<i64>;
    async fn fetch_after(&self, address: &str, cursor: i64) -> Result<Vec<MessageRow>>;
    /// Record that `message_id` was handed to a waiter for `recipient` (the served address), so a
    /// later holder does not redeliver it. Durable: this is what turns the in-memory delivery
    /// cursor into a restart-survivable mark. `occupant` is optional audit context (which holder).
    async fn mark_delivered(
        &self,
        message_id: i64,
        recipient: &str,
        occupant: Option<&str>,
    ) -> Result<()>;
    /// Messages addressed to `address`, with id `<= upto_id`, that have NOT yet been delivered to a
    /// waiter and whose latest disposition for that recipient is not terminal, ordered by id. A
    /// holder enqueues these at startup so messages queued while the address was unoccupied are
    /// delivered when a holder returns — instead of being skipped past `max_id`. The `upto_id`
    /// high-water bound (the holder's start cursor) is what keeps the seeded backlog and the
    /// `fetch_after` drain (`id > cursor`) from overlapping, so nothing is delivered twice.
    async fn undelivered_backlog(&self, address: &str, upto_id: i64) -> Result<Vec<MessageRow>>;
    async fn insert_message(&self, m: &NewMessage) -> Result<MessageRow>;
    async fn get_message(&self, id: i64) -> Result<Option<MessageRow>>;
    async fn thread_messages(&self, thread_id: i64) -> Result<Vec<MessageRow>>;
    async fn inbox(&self, address: &str, include_all: bool, limit: i64) -> Result<Vec<InboxItem>>;
    async fn export(
        &self,
        address: Option<&str>,
        thread: Option<i64>,
        since: i64,
    ) -> Result<Vec<MessageRow>>;

    // ---- dispositions ----
    async fn insert_disposition(
        &self,
        message_id: i64,
        recipient: &str,
        state: &str,
        note: Option<&str>,
        by: Option<&str>,
    ) -> Result<DispositionRow>;
    async fn dispositions_for(&self, message_id: i64) -> Result<Vec<DispositionRow>>;

    /// Best-effort push signal (Postgres LISTEN/NOTIFY); a no-op where unsupported.
    async fn notify_new(&self, address: &str, id: i64, sent_at_ms: i64) -> Result<()>;
}

/// The backend kinds compiled into this build (for `telex backend kinds` / diagnostics).
pub fn available_kinds() -> &'static [&'static str] {
    &[
        #[cfg(feature = "sqlite")]
        "sqlite",
        #[cfg(feature = "postgres")]
        "postgres",
    ]
}
