//! Backend abstraction: one `Backend` trait with SQLite and Postgres implementations
//! selected at runtime. The ephemeral `wait` client never touches this — it speaks only
//! to the holder over local IPC. This is the "same semantic core, two backends" promise.

use anyhow::{bail, Result};
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

#[derive(Clone, Copy, Debug)]
pub struct WaitFetchOptions {
    pub wake_on_cc: bool,
    pub cc_after_ms: i64,
}

#[derive(Clone, Debug)]
pub struct WaitCandidate {
    pub message: MessageRow,
    pub notification_only: bool,
}

impl WaitCandidate {
    pub fn primary(message: MessageRow) -> Self {
        Self {
            message,
            notification_only: false,
        }
    }

    pub fn cc_notification(message: MessageRow) -> Self {
        Self {
            message,
            notification_only: true,
        }
    }
}

#[async_trait]
pub trait Backend: Send + Sync {
    fn kind(&self) -> &'static str;
    fn capabilities(&self) -> Capabilities;
    fn supports_wake_on_cc(&self) -> bool {
        false
    }

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

    // ---- leases / liveness (v0 API; kept for backward compat) ----
    async fn claim_lease(&self, claim: &LeaseClaim, window_secs: i64) -> Result<LeaseOutcome>;
    async fn heartbeat(&self, address: &str) -> Result<()>;
    /// Non-deleting release: clears ownership but retains the lease row and its epoch
    /// high-water so the monotonic sequence is preserved for the next claimant.
    async fn release_lease(&self, address: &str, occupant: &str) -> Result<bool>;
    async fn get_lease(&self, address: &str) -> Result<Option<LeaseRow>>;
    async fn occupancy(&self, address: &str, window_secs: i64) -> Result<Occupancy>;

    // ---- epoch-aware lease / delivery fence (P1 — SQLite only in this release) ----

    /// Claim delivery ownership at the next epoch using a compare-and-set.
    /// `liveness_window_secs` is a duration, not a caller-computed timestamp: durable
    /// backends compute the stale cutoff from their own clock inside the claim transaction.
    /// Returns `Claimed` or `AlreadyOwned`.
    /// Default implementation returns an unsupported error so non-SQLite backends
    /// compile without behavioral parity.
    async fn claim_epoch_lease(
        &self,
        _address: &str,
        _owner_instance_id: &str,
        _liveness_window_secs: i64,
    ) -> Result<EpochClaimResult> {
        bail!("claim_epoch_lease: not supported by this backend")
    }

    /// Epoch-guarded heartbeat.  Returns `true` if the row was updated (caller still
    /// owns the address at the given epoch), `false` if a higher epoch exists (self-demote).
    async fn heartbeat_epoch(
        &self,
        _address: &str,
        _owner_instance_id: &str,
        _lease_epoch: i64,
    ) -> Result<bool> {
        bail!("heartbeat_epoch: not supported by this backend")
    }

    /// Non-deleting release of epoch ownership: clears `owner_instance_id` and preserves
    /// `lease_epoch` so the next claimant continues the monotonic sequence.
    /// Returns `true` if the row was updated, `false` if the caller was not the owner.
    async fn release_epoch_lease(
        &self,
        _address: &str,
        _owner_instance_id: &str,
        _lease_epoch: i64,
    ) -> Result<bool> {
        bail!("release_epoch_lease: not supported by this backend")
    }

    /// Detach-specific release path. Durable backends should atomically release the epoch lease
    /// and record a terminal session/address tombstone so ack recovery can distinguish an
    /// intentional detach from membership lost during daemon restart.
    async fn release_epoch_lease_for_detach(
        &self,
        address: &str,
        owner_instance_id: &str,
        lease_epoch: i64,
        session_id: &str,
        reason: &str,
    ) -> Result<bool> {
        let released = self
            .release_epoch_lease(address, owner_instance_id, lease_epoch)
            .await?;
        if released {
            self.record_detach_tombstone(session_id, address, reason)
                .await?;
        }
        Ok(released)
    }

    /// Admin reset: clear durable epoch ownership for one address while preserving the
    /// `lease_epoch` high-water. Returns the preserved epoch when a row existed.
    async fn reset_epoch_lease(&self, _address: &str) -> Result<Option<i64>> {
        bail!("reset_epoch_lease: not supported by this backend")
    }

    /// Atomically check ownership and mark `(message_id, recipient)` as consumed.
    /// The ownership check and the mark are one `BEGIN IMMEDIATE` transaction on SQLite
    /// (`SELECT … FOR UPDATE` + mark on Postgres), so a between-check-and-mark ownership
    /// rotation is caught.  `NotOwner` has strict precedence over all other outcomes.
    async fn mark_consumed_if_current_owner(
        &self,
        _recipient: &str,
        _owner_instance_id: &str,
        _lease_epoch: i64,
        _message_id: i64,
    ) -> Result<DeliveryOutcome> {
        bail!("mark_consumed_if_current_owner: not supported by this backend")
    }

    /// Read (and advance) the durable backend clock high-water mark.  On SQLite this is
    /// a persisted `clock_hwm` row that never moves backward across restarts or wall-clock
    /// skew.  Primarily useful for tests that need a stable time reference.
    async fn durable_clock_now_ms(&self) -> Result<i64> {
        bail!("durable_clock_now_ms: not supported by this backend")
    }

    /// Count retained per-recipient delivery rows for Status retention reporting.
    async fn delivery_retention_count(&self) -> Result<i64> {
        bail!("delivery_retention_count: not supported by this backend")
    }

    /// Count pending, unconsumed deliveries for one recipient. Used only for health/status
    /// observability; command semantics still use `fetch_undelivered`.
    async fn pending_unconsumed_count(&self, _address: &str) -> Result<i64> {
        bail!("pending_unconsumed_count: not supported by this backend")
    }

    /// Count inbound messages that require THIS recipient's disposition and are still actionable:
    /// the recipient is the primary `to_addr`, `requires_disposition` is set, the delivery is not
    /// consumed, and the latest disposition for the recipient is not terminal. This is the
    /// actionable backlog — distinct from `pending_unconsumed_count`, which also counts
    /// no-disposition notes and, on a shared address, traffic this recipient did not need to act on.
    /// Health/status observability only.
    async fn inbound_actionable_count(&self, _address: &str) -> Result<i64> {
        bail!("inbound_actionable_count: not supported by this backend")
    }

    /// Both observability counts for one recipient in a single pass. The default calls the two
    /// methods separately; durable backends override to materialize pending delivery rows once and
    /// run both counts on one connection, avoiding duplicate materialization on the status/turn-guard
    /// hot path.
    async fn pending_and_actionable_counts(&self, address: &str) -> Result<(i64, i64)> {
        let pending = self.pending_unconsumed_count(address).await?;
        let actionable = self.inbound_actionable_count(address).await?;
        Ok((pending, actionable))
    }

    async fn record_detach_tombstone(
        &self,
        _session_id: &str,
        _address: &str,
        _reason: &str,
    ) -> Result<()> {
        Ok(())
    }

    async fn clear_detach_tombstone(&self, _session_id: &str, _address: &str) -> Result<()> {
        Ok(())
    }

    async fn detach_tombstone(
        &self,
        _session_id: &str,
        _address: &str,
    ) -> Result<Option<DetachTombstone>> {
        Ok(None)
    }

    // ---- messages ----
    /// The greatest message id across all addresses (0 if empty). Used by read-only
    /// consumers (e.g. the console) to seed a bounded backfill cursor for the global feed.
    async fn max_message_id(&self) -> Result<i64>;
    /// Record that `message_id` was handed to a waiter for `recipient` (the served address), so no
    /// holder redelivers it. Durable: this is the per-recipient delivery fact that makes delivery
    /// state survive holder restarts. `occupant` is optional audit context (which holder).
    async fn mark_delivered(
        &self,
        message_id: i64,
        recipient: &str,
        occupant: Option<&str>,
    ) -> Result<()>;
    /// Every message addressed to `address` that has NOT yet been consumed (agent-acked) AND whose
    /// latest disposition for that recipient is not terminal, ordered by id. This is the holder's
    /// single source of truth for "what still needs delivering": the live drain queues whatever this
    /// returns (deduped in-memory), so delivery never depends on a monotonic id cursor. On Postgres
    /// that is what closes the commit-order gap (issue #18) — a concurrently-committed lower id has
    /// no delivery record, so it is returned and delivered by the *live* holder, no restart required.
    /// The two do-not-deliver signals are a consumed delivery record (primary) and a terminal
    /// disposition (secondary, for messages recovered out-of-band via `telex inbox`); see DECISIONS 0013.
    async fn fetch_undelivered(&self, address: &str) -> Result<Vec<MessageRow>>;
    async fn fetch_wait_candidates(
        &self,
        address: &str,
        options: WaitFetchOptions,
    ) -> Result<Vec<WaitCandidate>> {
        let mut candidates: Vec<WaitCandidate> = self
            .fetch_undelivered(address)
            .await?
            .into_iter()
            .map(WaitCandidate::primary)
            .collect();
        if options.wake_on_cc {
            bail!("wake-on-cc wait candidates are not supported by this backend")
        }
        candidates.sort_by_key(|candidate| candidate.message.id);
        Ok(candidates)
    }
    async fn has_delivery_for_recipient(&self, _message_id: i64, _recipient: &str) -> Result<bool> {
        Ok(false)
    }
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
    /// Delivery records for a message (one per recipient that received it). Read side of
    /// `mark_delivered`; the source of truth for "was this delivered, when, to which holder."
    async fn deliveries_for(&self, message_id: i64) -> Result<Vec<DeliveryRow>>;

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
