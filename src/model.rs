//! Core semantic types for Telex: attention levels, disposition states, and the
//! row/record structs that flow between the backend and the commands. The thin
//! semantic core lives in the client/library (here and in the commands), not in
//! backend-specific triggers.

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64
}

/// How urgently a recipient should be woken. Note: "interrupt" means "deliver at the
/// next turn boundary," not "preempt the running model" — agent-wake latency dominates.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum Attention {
    Interrupt,
    NextCheckpoint,
    #[default]
    Background,
    Fyi,
}

impl Attention {
    pub fn as_str(self) -> &'static str {
        match self {
            Attention::Interrupt => "interrupt",
            Attention::NextCheckpoint => "next-checkpoint",
            Attention::Background => "background",
            Attention::Fyi => "fyi",
        }
    }

    pub fn parse(s: &str) -> Result<Self> {
        Ok(match s.trim().to_ascii_lowercase().as_str() {
            "interrupt" => Attention::Interrupt,
            "next-checkpoint" | "next_checkpoint" | "checkpoint" => Attention::NextCheckpoint,
            "background" | "bg" => Attention::Background,
            "fyi" => Attention::Fyi,
            other => bail!(
                "unknown attention '{other}' (expected interrupt|next-checkpoint|background|fyi)"
            ),
        })
    }

    /// Whether a message of this attention is, by default, treated as actionable
    /// (worth waking the agent for via `wait`).
    pub fn is_actionable_default(self) -> bool {
        matches!(self, Attention::Interrupt | Attention::NextCheckpoint)
    }
}

/// What a recipient did with a message after delivery.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Disposition {
    Acknowledged,
    Handled,
    Deferred,
    Rejected,
    Closed,
    Escalated,
}

impl Disposition {
    pub fn as_str(self) -> &'static str {
        match self {
            Disposition::Acknowledged => "acknowledged",
            Disposition::Handled => "handled",
            Disposition::Deferred => "deferred",
            Disposition::Rejected => "rejected",
            Disposition::Closed => "closed",
            Disposition::Escalated => "escalated",
        }
    }

    pub fn parse(s: &str) -> Result<Self> {
        Ok(match s.trim().to_ascii_lowercase().as_str() {
            "acknowledged" | "ack" => Disposition::Acknowledged,
            "handled" | "handle" => Disposition::Handled,
            "deferred" | "defer" => Disposition::Deferred,
            "rejected" | "reject" => Disposition::Rejected,
            "closed" | "close" => Disposition::Closed,
            "escalated" | "escalate" => Disposition::Escalated,
            other => bail!("unknown disposition '{other}'"),
        })
    }

    /// Terminal dispositions remove a message from the actionable inbox.
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Disposition::Handled | Disposition::Rejected | Disposition::Closed
        )
    }

    pub fn is_terminal_str(s: &str) -> bool {
        TERMINAL_DISPOSITIONS.contains(&s)
    }
}

/// The terminal disposition states: a message whose latest disposition is one of these is done
/// and drops out of the actionable inbox (and out of the holder's restart backlog). Single source
/// of truth so the Rust check (`Disposition::is_terminal_str`) and the SQL backends that test
/// terminality inline cannot drift apart.
pub const TERMINAL_DISPOSITIONS: [&str; 3] = ["handled", "rejected", "closed"];

/// SQL value-list fragment of the terminal disposition states, e.g. `'handled','rejected','closed'`,
/// for inlining into a backend query. The values are fixed internal literals (never user input), so
/// interpolating them is injection-safe; deriving the fragment from `TERMINAL_DISPOSITIONS` keeps the
/// backends in lockstep with the canonical Rust definition.
pub fn terminal_dispositions_sql_list() -> String {
    TERMINAL_DISPOSITIONS
        .iter()
        .map(|s| format!("'{s}'"))
        .collect::<Vec<_>>()
        .join(",")
}

pub const STATUS_ACTIVE: &str = "active";
pub const STATUS_RETIRED: &str = "retired";

#[derive(Clone, Debug, Serialize)]
pub struct AddressRow {
    pub address: String,
    pub description: Option<String>,
    pub scope: Option<String>,
    pub tags: Option<String>,
    pub status: String,
    pub created_at_ms: i64,
}

#[derive(Clone, Debug, Serialize)]
pub struct LeaseRow {
    pub address: String,
    pub occupant: Option<String>,
    pub host: Option<String>,
    pub principal: Option<String>,
    pub description: Option<String>,
    pub tags: Option<String>,
    pub scope: Option<String>,
    pub pid: Option<i64>,
    pub since_ms: i64,
    pub heartbeat_at_ms: i64,
    /// Monotonic epoch high-water mark; None for rows not yet migrated to v2 schema.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lease_epoch: Option<i64>,
    /// Owning daemon instance; None when the lease is released/unowned.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner_instance_id: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct DetachTombstone {
    pub session_id: String,
    pub address: String,
    pub reason: String,
    pub at_ms: i64,
}

/// A request to claim/refresh a lease on an address.
#[derive(Clone, Debug, Default)]
pub struct LeaseClaim {
    pub address: String,
    pub occupant: String,
    pub host: String,
    pub principal: String,
    pub description: Option<String>,
    pub tags: Option<String>,
    pub scope: Option<String>,
    pub pid: i64,
}

#[derive(Debug)]
pub enum LeaseOutcome {
    /// We now hold the lease.
    Claimed,
    /// A different, still-live occupant holds it.
    AlreadyOccupied(LeaseRow),
}

/// Outcome of an explicit agent ack delivered via `mark_consumed_if_current_owner`.
/// Outcome precedence (strictly ordered): NotOwner > AlreadyConsumed / AckNoOp > Marked.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DeliveryOutcome {
    /// The message was successfully marked as consumed for this recipient.
    Marked,
    /// The message was already consumed (idempotent success; current owner confirmed first).
    AlreadyConsumed,
    /// No delivery row exists for `(message_id, recipient)`; inserts nothing.
    /// Returned only after confirming current ownership.
    AckNoOp,
    /// Caller is not the current epoch owner. Precedence-winning and fatal for the caller:
    /// the daemon must self-demote on this outcome, even if the message is already consumed.
    NotOwner,
}

/// Successful result from `claim_epoch_lease`: the new epoch and the owner's identity.
#[derive(Clone, Debug)]
pub struct EpochClaimed {
    pub lease_epoch: i64,
    pub owner_instance_id: String,
    /// True when a daemon-aware claimant converted a legacy/non-epoch row
    /// (`lease_epoch IS NULL`) through the explicit cutover path.
    pub legacy_cutover: bool,
}

/// Outcome of an epoch lease claim attempt.
#[derive(Debug)]
pub enum EpochClaimResult {
    /// Caller now holds the lease at this epoch.
    Claimed(EpochClaimed),
    /// A different, still-live owner holds the lease.
    AlreadyOwned {
        lease_epoch: i64,
        owner_instance_id: String,
        lease_row: LeaseRow,
    },
}

#[derive(Clone, Debug, Serialize)]
pub struct Occupancy {
    pub occupied: bool,
    pub age_secs: f64,
    pub occupant: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct MessageRow {
    pub id: i64,
    pub thread_id: i64,
    pub parent_id: Option<i64>,
    pub from_addr: Option<String>,
    pub to_addr: String,
    pub cc: Option<String>,
    pub kind: String,
    pub attention: String,
    pub requires_disposition: bool,
    pub subject: Option<String>,
    pub body: String,
    pub metadata: Option<String>,
    pub sent_at_ms: i64,
    pub created_at_ms: i64,
}

/// Fields supplied when inserting a new message. `thread_id`/`id` are assigned by
/// the backend; if `parent_id` is set the backend inherits the parent's thread.
#[derive(Clone, Debug, Default)]
pub struct NewMessage {
    pub parent_id: Option<i64>,
    pub from_addr: Option<String>,
    pub to_addr: String,
    pub cc: Option<String>,
    pub kind: String,
    pub attention: Attention,
    pub requires_disposition: bool,
    pub subject: Option<String>,
    pub body: String,
    pub metadata: Option<String>,
    pub sent_at_ms: i64,
}

#[derive(Clone, Debug, Serialize)]
pub struct DispositionRow {
    pub id: i64,
    pub message_id: i64,
    pub recipient: String,
    pub state: String,
    pub note: Option<String>,
    pub by_principal: Option<String>,
    pub at_ms: i64,
}

/// A message plus its current disposition status, for inbox listing.
#[derive(Clone, Debug, Serialize)]
pub struct InboxItem {
    #[serde(flatten)]
    pub message: MessageRow,
    pub latest_disposition: Option<String>,
    pub actionable: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attention_roundtrip() {
        for a in [
            Attention::Interrupt,
            Attention::NextCheckpoint,
            Attention::Background,
            Attention::Fyi,
        ] {
            assert_eq!(Attention::parse(a.as_str()).unwrap(), a);
        }
        assert!(Attention::parse("nonsense").is_err());
    }

    #[test]
    fn disposition_terminality() {
        assert!(Disposition::Handled.is_terminal());
        assert!(Disposition::Closed.is_terminal());
        assert!(Disposition::Rejected.is_terminal());
        assert!(!Disposition::Acknowledged.is_terminal());
        assert!(!Disposition::Deferred.is_terminal());
        assert!(!Disposition::Escalated.is_terminal());
        assert!(Disposition::is_terminal_str("handled"));
        assert!(!Disposition::is_terminal_str("acknowledged"));
    }
}
