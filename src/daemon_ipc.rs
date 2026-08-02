//! Daemon-scoped IPC protocol foundation.  This is separate from the legacy
//! address-keyed `ipc` module so P2 can add the daemon singleton surface without
//! rewriting the current resident-holder verbs.

use crate::model::DeliveryOutcome;
use serde::{Deserialize, Deserializer, Serialize};
use std::collections::BTreeSet;
use std::fmt;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt};

pub const PROTOCOL_MAJOR: u16 = 1;
pub const PROTOCOL_MINOR: u16 = 5;
pub const DAEMON_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const AUTH_POLICY_VERSION: u16 = 1;
pub const MAX_JSONL_FRAME_BYTES: usize = 1024 * 1024;
pub const MAX_MESSAGE_BODY_METADATA_BYTES: usize = MAX_JSONL_FRAME_BYTES - (64 * 1024);

pub const CAP_JSONL: &str = "jsonl_v1";
pub const CAP_ADMIN_CAP: &str = "admin_cap_v1";
pub const CAP_SAME_USER_PEER_AUTH: &str = "same_user_peer_auth_v1";
pub const CAP_STATUS_P2: &str = "status_p2";
pub const CAP_DRAIN_P2: &str = "drain_p2";
pub const CAP_MEMBERSHIP_P3: &str = "membership_p3";
pub const CAP_LIVENESS_P5: &str = "liveness_p5";
pub const CAP_STATUS_P5: &str = "status_p5";
pub const CAP_STATION_LIFECYCLE_P8: &str = "station_lifecycle_p8";
pub const CAP_WAIT_MIN_ATTENTION_P9: &str = "wait_min_attention_p9";
pub const CAP_WAIT_WAKE_ON_CC_P10: &str = "wait_wake_on_cc_p10";
/// Advertised (not required): the daemon honors `Register.on_deliver` and runs the generic
/// on-deliver exec push primitive. A client provisioning push delivery checks this / the
/// `push_registered` status to fail closed against an older daemon that would silently ignore
/// `on_deliver`.
pub const CAP_ON_DELIVER_EXEC: &str = "on_deliver_exec_v1";

/// Exit codes the on-deliver push handler (`telex copilot push`) returns and the daemon interprets.
/// Single source of truth for the handler<->daemon contract so the two sides cannot drift: exit 0 =
/// accepted, `ON_DELIVER_PERMANENT_EXIT` = permanent (dead-letter, e.g. too large),
/// `ON_DELIVER_DEFERRED_EXIT` = harness deferred because busy (held for the deferred backstop,
/// re-attempted by the idle drain -- issue #65 / ADR 0043), any other nonzero = transient retry.
pub const ON_DELIVER_PERMANENT_EXIT: i32 = 3;
pub const ON_DELIVER_DEFERRED_EXIT: i32 = 4;

/// Advertised (not required): the daemon understands the deferred on-deliver outcome (exit code
/// `ON_DELIVER_DEFERRED_EXIT`) and the `DrainDeferred` request (issue #65 / ADR 0043). Advertised
/// optionally so it never breaks the required-capability handshake with an older peer; a client can
/// check it to detect version skew (an older daemon maps exit 4 to a transient retry and ignores
/// `DrainDeferred`, which is bounded and self-resolves on daemon restart).
pub const CAP_ON_DELIVER_DEFERRED: &str = "on_deliver_deferred_v1";

/// Advertised (not required): the daemon owns durable station intents and reconciles them
/// (`Request::ReconcileIntents`, intent rows in status, `IntentRecoveryState`). Advertised rather
/// than required so a pre-P11 client still handshakes; a client that needs the behavior checks the
/// daemon minor against `RECONCILE_MIN_DAEMON_MINOR` and refuses to write an intent an older
/// daemon would never act on.
pub const CAP_STATION_INTENT: &str = "station_intent_v1";

pub const REQUIRED_CAPABILITIES: &[&str] = &[
    CAP_JSONL,
    CAP_ADMIN_CAP,
    CAP_SAME_USER_PEER_AUTH,
    CAP_STATUS_P2,
    CAP_DRAIN_P2,
    CAP_MEMBERSHIP_P3,
    CAP_LIVENESS_P5,
    CAP_STATUS_P5,
    CAP_STATION_LIFECYCLE_P8,
    CAP_WAIT_MIN_ATTENTION_P9,
    CAP_WAIT_WAKE_ON_CC_P10,
];

pub const ERROR_INCOMPATIBLE: &str = "Incompatible";
pub const ERROR_UNAUTHORIZED: &str = "Unauthorized";
pub const ERROR_NOT_RUNNING: &str = "DaemonNotRunning";
pub const ERROR_INTERNAL: &str = "Internal";
pub const ERROR_NEEDS_ATTACH: &str = "NeedsAttach";
pub const ERROR_AMBIGUOUS: &str = "Ambiguous";
pub const ERROR_UNSUPPORTED: &str = "Unsupported";
pub const ERROR_NOT_OWNER: &str = "NotOwner";
pub const REDACTED_SECRET: &str = "[redacted]";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProtocolVersion {
    pub major: u16,
    pub minor: u16,
}

pub const fn current_protocol_version() -> ProtocolVersion {
    ProtocolVersion {
        major: PROTOCOL_MAJOR,
        minor: PROTOCOL_MINOR,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum WatchPidRole {
    #[default]
    Anchor,
    Required,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WatchPidSpec {
    pub pid: u32,
    #[serde(default)]
    pub role: WatchPidRole,
}

impl WatchPidSpec {
    pub fn anchor(pid: u32) -> Self {
        Self {
            pid,
            role: WatchPidRole::Anchor,
        }
    }
}

impl<'de> Deserialize<'de> for WatchPidSpec {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Wire {
            LegacyPid(u32),
            Typed {
                pid: u32,
                #[serde(default)]
                role: WatchPidRole,
            },
        }

        match Wire::deserialize(deserializer)? {
            Wire::LegacyPid(pid) => Ok(WatchPidSpec::anchor(pid)),
            Wire::Typed { pid, role } => Ok(WatchPidSpec { pid, role }),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityScope {
    pub capability: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hello {
    pub protocol_version: ProtocolVersion,
    pub client_version: String,
    pub store_key: String,
    #[serde(default)]
    pub capabilities: Vec<String>,
    #[serde(default)]
    pub required_capabilities: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub capability_scopes: Vec<CapabilityScope>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HelloAck {
    pub protocol_version: ProtocolVersion,
    pub daemon_version: String,
    pub auth_policy_version: u16,
    pub accepted: bool,
    pub required_capabilities: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub capability_scopes: Vec<CapabilityScope>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Request {
    Register {
        store_key: String,
        address: String,
        session_id: String,
        occupant: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        description: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        scope: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tags: Option<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        watch_pids: Vec<WatchPidSpec>,
        #[serde(default)]
        recovery: bool,
        /// Optional harness-neutral on-deliver handler argv. When present, the daemon
        /// execs this command (message descriptor on stdin) after a message for this
        /// address is durably committed. The daemon never interprets the argv.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        on_deliver: Option<Vec<String>>,
        /// Explicitly replace the member's existing on-deliver handler. When true with
        /// `on_deliver = None`, clears push registration instead of preserving it during refresh.
        #[serde(default, skip_serializing_if = "is_false")]
        replace_on_deliver: bool,
        /// Optional on-deliver opt-in for live CC observer traffic. Applies only when
        /// `on_deliver` is present; defaults false for older clients.
        #[serde(default, skip_serializing_if = "is_false")]
        on_deliver_wake_on_cc: bool,
    },
    Detach {
        store_key: String,
        session_id: String,
        address: String,
    },
    StationStop {
        store_key: String,
        session_id: String,
        address: String,
        #[serde(default)]
        wait_grace_ms: u64,
    },
    Wait {
        store_key: String,
        session_id: String,
        address: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        attention: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        min_attention: Option<String>,
        #[serde(default, skip_serializing_if = "is_false")]
        wake_on_cc: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        timeout_ms: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        waiter_pid: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        waiter_start_time: Option<u64>,
    },
    Ack {
        store_key: String,
        session_id: String,
        address: String,
        message_id: i64,
    },
    Send {
        store_key: String,
        session_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        from_addr: Option<String>,
        to_addr: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cc: Option<String>,
        kind: String,
        attention: String,
        requires_disposition: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        subject: Option<String>,
        body: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        metadata: Option<String>,
    },
    Reply {
        store_key: String,
        session_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        from_addr: Option<String>,
        message_id: i64,
        kind: String,
        attention: String,
        requires_disposition: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        subject: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cc: Option<String>,
        body: String,
    },
    Status {
        #[serde(default)]
        store_key: Option<String>,
        #[serde(default)]
        detail: bool,
        #[serde(default)]
        proof: Option<String>,
    },
    SessionEnd {
        store_key: String,
        session_id: String,
        #[serde(default)]
        proof: Option<String>,
    },
    Reset {
        store_key: String,
        address: String,
        #[serde(default)]
        proof: Option<String>,
    },
    Drain {
        #[serde(default)]
        proof: Option<String>,
    },
    /// Idle-drain trigger (issue #65): clear the deferred-push skip for the session's on-deliver
    /// members and re-sweep their backlog, so messages deferred while the bridge was busy are
    /// re-attempted now that a root turn has stopped. Harness-neutral: the daemon only knows it
    /// should re-run the generic on-deliver sweep; the "busy/idle" concept lives entirely in the
    /// bridge. Durable state is revalidated by the sweep, so an already-acked message is skipped.
    DrainDeferred {
        store_key: String,
        session_id: String,
        #[serde(default)]
        proof: Option<String>,
    },
    /// Explicit station-intent reconciliation (issue #106 / ADR 0050). Admin-proofed exactly like
    /// `Drain`: reconciliation arms delivery and spawns push handler processes, so it must not be
    /// reachable from an unproofed request path. `scope` optionally narrows the pass to one store.
    ReconcileIntents {
        #[serde(default)]
        proof: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        scope: Option<String>,
    },
    Ping,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NeedsAttachReason {
    RestartLost,
    DeliberatelyDetached,
    /// A `pending` station intent exists for this binding: an attach is mid-flight (or crashed
    /// before finalizing). Explicit attach/resume is the way forward; the daemon will not act on a
    /// pending intent.
    PushIntentPending,
    /// A `live` push intent exists for this binding but could not be reconciled, so the daemon
    /// refused to create a *pull-only* member over it. This is the anti-downgrade signal.
    PushIntentUnrecoverable,
    /// Forward-compat catch-all so an older client deserializing a newer daemon's error does not
    /// fail on a reason it does not know.
    #[serde(other)]
    Unknown,
}

/// Recovery state of a station intent, as projected by the daemon.
///
/// Only `Pending`, `Live`, and `Revoked` are ever *persisted*; the rest are runtime projections
/// held in the daemon's in-memory intent index, so a transient probe failure never rewrites
/// durable state. `Unknown` is the forward-compat catch-all, matching `StationHealth`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum IntentRecoveryState {
    /// Written before `Register`, not yet finalized. Never reconciled.
    Pending,
    /// Finalized and eligible for reconciliation.
    #[default]
    Live,
    /// Reconciled into a live member during this daemon's lifetime.
    Restored,
    /// The predecessor's epoch lease is simply not stale yet. A **waiting** state, not an error:
    /// it retries at a fixed cadence, never enters the exponential ladder, and never counts toward
    /// quarantine. This is what makes the published crash-recovery bound derivable.
    DeferredLease,
    /// A live armed pull waiter owns the address. Pull-waiter precedence is preserved, so the
    /// intent waits rather than forcing the conflict.
    DeferredPullWaiter,
    /// The producer predates the probe verb, so its liveness is unprovable. Legacy, *not* failed:
    /// the documented manual resume path keeps working and no turn is ever blocked.
    LegacyProducer,
    /// The manifest or descriptor is structurally incompatible with this build.
    Incompatible,
    /// The producer, credential, or store selector could not be resolved, so the intent cannot be
    /// verified. Never "verified anyway".
    Unverifiable,
    /// A security check failed (ownership, permissions, containment, reparse point).
    Insecure,
    /// Too many consecutive genuine failures; retried on a slow cadence so one wedged intent can
    /// never consume the pass budget.
    Quarantined,
    /// Explicitly revoked locally (detach, session end, fallback downgrade).
    Tombstoned,
    /// A durable detach tombstone exists for this binding. Highest precedence of all: an
    /// explicitly detached station never auto-returns.
    Revoked,
    /// Another owner holds the address and is not stale.
    OwnershipConflict,
    /// Forward-compat catch-all. Never produced intentionally.
    #[serde(other)]
    Unknown,
}

impl IntentRecoveryState {
    /// Precedence when several states apply at once, highest first. Encoded as a total order so
    /// the projection is deterministic rather than dependent on evaluation order.
    pub fn precedence(self) -> u8 {
        match self {
            IntentRecoveryState::Revoked => 13,
            IntentRecoveryState::Tombstoned => 12,
            IntentRecoveryState::Insecure => 11,
            IntentRecoveryState::Incompatible => 10,
            IntentRecoveryState::OwnershipConflict => 9,
            IntentRecoveryState::Quarantined => 8,
            IntentRecoveryState::Unverifiable => 7,
            IntentRecoveryState::LegacyProducer => 6,
            IntentRecoveryState::DeferredPullWaiter => 5,
            IntentRecoveryState::DeferredLease => 4,
            IntentRecoveryState::Pending => 3,
            IntentRecoveryState::Restored => 2,
            IntentRecoveryState::Live => 1,
            IntentRecoveryState::Unknown => 0,
        }
    }

    /// The higher-precedence of two states.
    pub fn max(self, other: Self) -> Self {
        if other.precedence() > self.precedence() {
            other
        } else {
            self
        }
    }

    /// Whether this is a *waiting* state rather than an error, so status and the drain report can
    /// project "waiting for the predecessor's lease to go stale" instead of "failing".
    pub fn is_waiting(self) -> bool {
        matches!(
            self,
            IntentRecoveryState::DeferredLease
                | IntentRecoveryState::DeferredPullWaiter
                | IntentRecoveryState::Pending
        )
    }

    /// Whether the daemon considers this intent recoverable without operator action.
    pub fn is_recoverable(self) -> bool {
        matches!(
            self,
            IntentRecoveryState::Live
                | IntentRecoveryState::Restored
                | IntentRecoveryState::DeferredLease
                | IntentRecoveryState::DeferredPullWaiter
        )
    }
}

/// One intent row in the daemon status projection.
///
/// Carries evidence, not just a state name, matching the `*_since_ms` / `*_for_ms` / `*_count`
/// idiom already used by `MemberStatus`. Never carries a secret, a raw argv, or a credential path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IntentStatus {
    pub store_key: String,
    pub session_id: String,
    pub address: String,
    pub state: IntentRecoveryState,
    pub generation: u64,
    #[serde(default)]
    pub delivery_mode: DeliveryMode,
    #[serde(default)]
    pub wake_on_cc: bool,
    /// Whether a `MemberRecord` currently exists for this binding. When it does, `StationHealth`
    /// and `PushDeliveryHealth` stay authoritative and this row is supplementary; when it does
    /// not, this row is the only projection of the binding.
    #[serde(default)]
    pub has_member: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cc_watermark_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_attempt_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_success_ms: Option<i64>,
    #[serde(default)]
    pub attempts: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub producer_verified_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_attempt_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recovery_latency_ms: Option<i64>,
    /// When the cached index this row was projected from was last refreshed, so a reader can tell
    /// "no live intent" from "the index has not been refreshed since the last pass".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub index_as_of_ms: Option<i64>,
}

/// Outcome counts for one reconciliation pass. Published on the trigger/report seam so callers
/// (upgrade, rollback, tests) can await a pass rather than poll a clock.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReconcileReport {
    /// Monotonically increasing pass sequence number.
    pub pass_seq: u64,
    pub scanned: usize,
    pub restored: usize,
    pub refreshed_no_op: usize,
    pub deferred_lease: usize,
    pub deferred_pull_waiter: usize,
    pub failed: usize,
    pub skipped: usize,
    /// Intents in terminal/inert states (revoked, tombstoned, insecure, incompatible, legacy,
    /// unverifiable, quarantined). Surfaced, never retried on the fast cadence.
    pub inert: usize,
    /// Whether the scope holds more than the per-scope write cap. Reported, never acted on.
    pub over_cap: bool,
    pub observed_count: usize,
    pub duration_ms: u64,
    /// True when the pass stopped on `RECONCILE_PASS_DEADLINE` rather than sweeping the scope.
    pub deadline_reached: bool,
    pub index_as_of_ms: i64,
}

/// Pre/post-drain intent signal, computed from in-memory state only (members + the cached intent
/// index). No directory scan, no probe, no network I/O — so producing it can never push a graceful
/// drain past `--drain-timeout-ms`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DrainIntentReport {
    /// Intents a compatible successor is expected to restore automatically.
    pub recoverable: usize,
    /// Intents that need operator action (unverifiable, insecure, quarantined).
    pub degraded: usize,
    /// Intents this build cannot reconcile (schema or descriptor incompatibility, legacy producer).
    pub incompatible: usize,
    /// Intents whose state is not yet known to the index.
    pub unknown: usize,
    pub over_cap: bool,
    pub observed_count: usize,
    /// Index freshness, so an operator sees staleness rather than assuming the report is live.
    pub index_as_of_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Response {
    Registered {
        lease_epoch: i64,
        owner_instance_id: String,
    },
    Message {
        id: i64,
        thread_id: i64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        parent_id: Option<i64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        from_addr: Option<String>,
        to_addr: String,
        delivered_to: String,
        primary_to: String,
        #[serde(default)]
        cc: Vec<String>,
        delivery_role: String,
        kind: String,
        attention: String,
        requires_disposition: bool,
        requires_disposition_for_current_recipient: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        subject: Option<String>,
        body: String,
        sent_at_ms: i64,
        buffered_at_ms: i64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        lease_epoch: Option<i64>,
    },
    Sent {
        receipt: SentReceipt,
    },
    Timeout,
    PresenceEnded,
    StatusReport {
        status: DaemonStatus,
    },
    /// One completed reconciliation pass.
    Reconciled {
        report: ReconcileReport,
    },
    Pong {
        protocol_version: ProtocolVersion,
        daemon_version: String,
        instance_id: String,
    },
    Ack {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        delivery_outcome: Option<DeliveryOutcome>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        address: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message_id: Option<i64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        lease_epoch: Option<i64>,
        /// Pre-drain station-intent signal, present on the `Drain` ack. Additive and optional, so
        /// an older client deserializing a newer daemon's ack simply ignores it.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        drain_intents: Option<DrainIntentReport>,
    },
    StationStopped {
        store_key: String,
        session_id: String,
        address: String,
        detached: bool,
        waiters_before: usize,
        waiters_after: usize,
        #[serde(default)]
        live_waiters: Vec<LiveWaiterStatus>,
        /// Whether the stopped station had a registered on-deliver push handler. Station stop
        /// releases address membership + records a detach tombstone, but does NOT unload the
        /// in-session bridge extension; the CLI warns and points at `telex copilot detach`.
        #[serde(default)]
        push_registered: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        lease_epoch: Option<i64>,
    },
    Error {
        code: String,
        message: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        needs_attach_reason: Option<NeedsAttachReason>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SentReceipt {
    pub receipt: String,
    pub id: i64,
    pub thread_id: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<i64>,
    pub to: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attention: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requires_disposition: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub occupied: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DaemonStatus {
    pub protocol_version: ProtocolVersion,
    pub daemon_version: String,
    pub instance_id: String,
    pub singleton_key: String,
    #[serde(default)]
    pub stores: Vec<StoreStatus>,
    #[serde(default)]
    pub backoff: Vec<String>,
    #[serde(default)]
    pub recent_errors: Vec<RecentErrorStatus>,
    #[serde(default)]
    pub epoch_by_address: Vec<EpochStatus>,
    #[serde(default)]
    pub members: Vec<MemberStatus>,
    #[serde(default)]
    pub live_waiters: Vec<LiveWaiterStatus>,
    #[serde(default)]
    pub retention: Vec<RetentionStatus>,
    #[serde(default)]
    pub idle_stations: IdleStationStatus,
    #[serde(default)]
    pub deaf_stations: DeafStationStatus,
    /// Station-intent rows, including intent-only rows with no member. Part of the authenticated
    /// (`detail: true`) projection only — never the uncapped `status_minimal` projection.
    #[serde(default)]
    pub intents: Vec<IntentStatus>,
    /// When the cached intent index was last refreshed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intent_index_as_of_ms: Option<i64>,
    /// Whether the intent scope holds more than the per-scope write cap.
    #[serde(default)]
    pub intent_over_cap: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoreStatus {
    pub store_key: String,
    pub kind: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EpochStatus {
    pub store_key: String,
    pub address: String,
    pub lease_epoch: i64,
    pub owner_instance_id: String,
    pub idle: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemberStatus {
    pub store_key: String,
    pub backend: String,
    pub session_id: String,
    pub address: String,
    pub occupant: String,
    pub host: String,
    pub waiters: usize,
    #[serde(default)]
    pub live_waiters_count: usize,
    #[serde(default)]
    pub pending_unconsumed_count: i64,
    /// Inbound messages that require THIS station's disposition (primary recipient,
    /// `requires_disposition`, not terminal, unconsumed) — the actionable backlog, distinct from
    /// `pending_unconsumed_count` which also counts no-disposition notes and, on a shared address,
    /// traffic this station is not responsible for dispositioning.
    #[serde(default)]
    pub inbound_actionable_count: i64,
    #[serde(default)]
    pub station_health: StationHealth,
    /// Configured delivery path, separate from whether that path is currently healthy/armed.
    #[serde(default)]
    pub delivery_mode: DeliveryMode,
    /// Structured push-delivery health for a registered push station (see `PushDeliveryHealth`).
    #[serde(default)]
    pub push_delivery: PushDeliveryHealth,
    /// On-deliver messages whose re-push is currently suppressed (dead-lettered as unpushable, or
    /// past the `ON_DELIVER_MAX_REPUSH` attempt cap). They stay durable/readable via `telex inbox`;
    /// surfaced here as a persistent signal (not only in the rolling recent-errors buffer).
    #[serde(default)]
    pub push_suppressed_count: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub health_detail: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_waiter_exit_at_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_waiter_outcome: Option<WaiterOutcome>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_waiter_exit_code: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_waiter_detail: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_waiter_pid: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_delivered_message_id: Option<i64>,
    /// Whether this member registered a daemon on-deliver push handler (bridge push is active).
    #[serde(default)]
    pub push_registered: bool,
    /// Whether the push handler is opted into live CC observer traffic.
    #[serde(default)]
    pub push_wake_on_cc: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub push_cc_after_ms: Option<i64>,
    /// Count of this member's messages currently deferred-until-idle (bridge was busy). Distinct
    /// from accepted-unacked and failed-transient push state, so `telex status` can diagnose why a
    /// message has not arrived as a turn yet (issue #65).
    #[serde(default)]
    pub push_deferred_count: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unattended_since_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unattended_for_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deaf_since_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deaf_for_ms: Option<i64>,
    #[serde(default)]
    pub deaf_warn: bool,
    #[serde(default)]
    pub live_waiters: Vec<LiveWaiterStatus>,
    #[serde(default)]
    pub watch_pids: Vec<WatchPidStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tags: Option<String>,
    pub lease_epoch: i64,
    pub owner_instance_id: String,
    pub idle: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum WaiterOutcome {
    Message,
    IdleTimeout,
    PresenceEnded,
    AbnormalExit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum StationHealth {
    Armed,
    RecentlyDelivered,
    #[default]
    Unattended,
    UnattendedWithBacklog,
    /// A registered on-deliver push station: covered by a push path rather than a `telex wait`
    /// waiter, so it must not be reported `unattended`. Delivery *confidence* is carried separately
    /// by `push_delivery` (delivering / probing / stale_accepted) — this value only asserts the
    /// station is push-covered, not that a turn was confirmed seen.
    AttendedPush,
    /// Both push and pull coverage are active. New protocol peers reject this state at both
    /// entry points; the value remains a defensive tripwire for version skew and races.
    CoverageConflict,
    Idle,
    /// Forward-compat catch-all so an older client deserializing a newer daemon's status does not
    /// fail on a value it does not know. Never produced intentionally.
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryMode {
    Push,
    Pull,
    Conflict,
    /// Forward-compat/default value for status emitted by an older daemon.
    #[default]
    #[serde(other)]
    Unknown,
}

/// Push-delivery health for a member with a registered on-deliver handler, derived from the
/// daemon's own push-attempt outcomes (never from harness-specific bridge state). Reported as a
/// structured field so consumers read push health from a typed value rather than free-text detail.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PushDeliveryHealth {
    /// No on-deliver push handler registered (pull station or plain member).
    #[default]
    NotRegistered,
    /// Push handler registered and there is no undelivered backlog.
    NoBacklog,
    /// Backlog exists and a recent push attempt was accepted by the harness (bridge live).
    Delivering,
    /// Backlog exists but no push attempt is recorded yet (e.g. just after a daemon restart, before
    /// the next sweep). Not confidently attended and not deaf; resolves on the next sweep.
    Probing,
    /// The last accepted push's backstop has elapsed with no fresh accept and no failure yet — an
    /// earlier-than-deaf hint that a previously-live bridge may have gone away (e.g. session suspend).
    StaleAccepted,
    /// Backlog exists and push attempts are failing (bridge unreachable) — drives the deaf signal.
    Failing,
    /// Forward-compat catch-all (see `StationHealth::Unknown`).
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LiveWaiterStatus {
    pub waiter_id: u64,
    pub store_key: String,
    pub session_id: String,
    pub address: String,
    pub pid: u32,
    pub alive: bool,
    pub started_at_ms: i64,
    #[serde(default)]
    pub start_time: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attention: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_attention: Option<String>,
    #[serde(default)]
    pub wake_on_cc: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cc_after_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WatchPidStatus {
    pub pid: u32,
    pub role: WatchPidRole,
    pub alive: bool,
    #[serde(default)]
    pub start_time: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecentErrorStatus {
    pub at_ms: i64,
    pub kind: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetentionStatus {
    pub store_key: String,
    pub delivery_rows: i64,
    pub warn: bool,
    pub warn_threshold: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct IdleStationStatus {
    pub count: usize,
    pub warn: bool,
    pub warn_threshold: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct DeafStationStatus {
    pub count: usize,
    pub warn: bool,
    pub warn_threshold_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CompatibilityRow {
    pub protocol_major: u16,
    pub protocol_minor: u16,
    pub min_client_version: &'static str,
    pub min_daemon_version: &'static str,
    pub required_capabilities: &'static [&'static str],
    pub unknown_required_capability_error: &'static str,
    pub unknown_operation_error: &'static str,
}

pub const COMPATIBILITY_TABLE: &[CompatibilityRow] = &[CompatibilityRow {
    protocol_major: PROTOCOL_MAJOR,
    protocol_minor: PROTOCOL_MINOR,
    min_client_version: "0.1.0",
    min_daemon_version: "0.1.0",
    required_capabilities: REQUIRED_CAPABILITIES,
    unknown_required_capability_error: ERROR_INCOMPATIBLE,
    unknown_operation_error: ERROR_INCOMPATIBLE,
}];

#[derive(Debug)]
pub enum HandshakeError {
    Verify(String),
    Io(std::io::Error),
    Json(serde_json::Error),
    FrameTooLarge { max_bytes: usize },
    MalformedFrame(String),
    Eof,
    Rejected(String),
}

impl fmt::Display for HandshakeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HandshakeError::Verify(e) => write!(f, "server authentication failed: {e}"),
            HandshakeError::Io(e) => write!(f, "IPC I/O failed: {e}"),
            HandshakeError::Json(e) => write!(f, "IPC JSON framing failed: {e}"),
            HandshakeError::FrameTooLarge { max_bytes } => {
                write!(f, "IPC JSONL frame exceeded {max_bytes} bytes")
            }
            HandshakeError::MalformedFrame(e) => write!(f, "IPC JSONL frame malformed: {e}"),
            HandshakeError::Eof => write!(f, "IPC peer closed the connection"),
            HandshakeError::Rejected(reason) => write!(f, "daemon rejected handshake: {reason}"),
        }
    }
}

impl std::error::Error for HandshakeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            HandshakeError::Io(e) => Some(e),
            HandshakeError::Json(e) => Some(e),
            _ => None,
        }
    }
}

impl From<std::io::Error> for HandshakeError {
    fn from(value: std::io::Error) -> Self {
        HandshakeError::Io(value)
    }
}

impl From<serde_json::Error> for HandshakeError {
    fn from(value: serde_json::Error) -> Self {
        HandshakeError::Json(value)
    }
}

pub fn daemon_capabilities() -> Vec<String> {
    let mut caps: Vec<String> = REQUIRED_CAPABILITIES
        .iter()
        .map(|s| (*s).to_string())
        .collect();
    // Advertised-but-optional so it never breaks the required-capability handshake with an
    // older peer; provisioning code gates on it (and on `push_registered`) explicitly.
    caps.push(CAP_ON_DELIVER_EXEC.to_string());
    // Advertised-but-optional (issue #65): lets a client detect a daemon that understands the
    // deferred outcome + `DrainDeferred`, so version skew against an older daemon is diagnosable.
    caps.push(CAP_ON_DELIVER_DEFERRED.to_string());
    // Advertised-but-optional (issue #106): durable station intents plus daemon-owned
    // reconciliation. A client that would write an intent checks this (and the daemon minor)
    // rather than assuming a connected daemon will ever act on one.
    caps.push(CAP_STATION_INTENT.to_string());
    caps
}

pub fn daemon_required_capabilities() -> Vec<String> {
    REQUIRED_CAPABILITIES
        .iter()
        .map(|s| (*s).to_string())
        .collect()
}

pub fn client_hello(store_key: impl Into<String>) -> Hello {
    Hello {
        protocol_version: current_protocol_version(),
        client_version: DAEMON_VERSION.to_string(),
        store_key: store_key.into(),
        capabilities: daemon_capabilities(),
        required_capabilities: daemon_required_capabilities(),
        capability_scopes: Vec::new(),
    }
}

pub fn evaluate_hello(hello: &Hello) -> HelloAck {
    let required = daemon_required_capabilities();
    let capabilities = daemon_capabilities();
    let caps: BTreeSet<&str> = capabilities.iter().map(String::as_str).collect();
    let client_caps: BTreeSet<&str> = hello.capabilities.iter().map(String::as_str).collect();

    let reason = if hello.protocol_version.major != PROTOCOL_MAJOR {
        Some(format!(
            "protocol major mismatch: client {}, daemon {}",
            hello.protocol_version.major, PROTOCOL_MAJOR
        ))
    } else if let Some(cap) = hello
        .required_capabilities
        .iter()
        .find(|cap| !caps.contains(cap.as_str()))
    {
        Some(format!("unknown required capability: {cap}"))
    } else {
        required
            .iter()
            .find(|cap| !client_caps.contains(cap.as_str()))
            .map(|cap| format!("client missing required capability: {cap}"))
    };

    HelloAck {
        protocol_version: current_protocol_version(),
        daemon_version: DAEMON_VERSION.to_string(),
        auth_policy_version: AUTH_POLICY_VERSION,
        accepted: reason.is_none(),
        required_capabilities: required,
        reason,
        capability_scopes: Vec::new(),
    }
}

pub fn error_response(code: &str, message: impl Into<String>) -> Response {
    Response::Error {
        code: code.to_string(),
        message: message.into(),
        needs_attach_reason: None,
    }
}

pub fn unauthorized(message: impl Into<String>) -> Response {
    error_response(ERROR_UNAUTHORIZED, message.into())
}

pub fn incompatible(message: impl Into<String>) -> Response {
    error_response(ERROR_INCOMPATIBLE, message.into())
}

pub fn needs_attach(message: impl Into<String>) -> Response {
    error_response(ERROR_NEEDS_ATTACH, message.into())
}

pub fn needs_attach_with_reason(message: impl Into<String>, reason: NeedsAttachReason) -> Response {
    Response::Error {
        code: ERROR_NEEDS_ATTACH.to_string(),
        message: message.into(),
        needs_attach_reason: Some(reason),
    }
}

pub fn ambiguous(message: impl Into<String>) -> Response {
    error_response(ERROR_AMBIGUOUS, message.into())
}

/// `ERROR_INCOMPATIBLE` carrying a typed reason, so a client can render an actionable recovery
/// path instead of parsing free text. Used by the anti-downgrade guard.
pub fn incompatible_with_reason(message: impl Into<String>, reason: NeedsAttachReason) -> Response {
    Response::Error {
        code: ERROR_INCOMPATIBLE.to_string(),
        message: message.into(),
        needs_attach_reason: Some(reason),
    }
}

pub fn unsupported(message: impl Into<String>) -> Response {
    error_response(ERROR_UNSUPPORTED, message.into())
}

pub fn internal(message: impl Into<String>) -> Response {
    error_response(ERROR_INTERNAL, message.into())
}

fn is_false(value: &bool) -> bool {
    !*value
}

pub fn redact_secrets(message: impl Into<String>, secrets: &[&str]) -> String {
    let mut redacted = message.into();
    for secret in secrets {
        if !secret.is_empty() {
            redacted = redacted.replace(secret, REDACTED_SECRET);
        }
    }
    redacted
}

pub async fn write_json_line<W, T>(writer: &mut W, value: &T) -> Result<(), HandshakeError>
where
    W: AsyncWrite + Unpin,
    T: Serialize,
{
    let mut line = serde_json::to_vec(value)?;
    line.push(b'\n');
    if line.len() > MAX_JSONL_FRAME_BYTES {
        return Err(HandshakeError::FrameTooLarge {
            max_bytes: MAX_JSONL_FRAME_BYTES,
        });
    }
    writer.write_all(&line).await?;
    writer.flush().await?;
    Ok(())
}

pub fn json_line_frame_len<T>(value: &T) -> Result<usize, HandshakeError>
where
    T: Serialize,
{
    let len = serde_json::to_vec(value)?.len().saturating_add(1);
    Ok(len)
}

pub async fn read_json_line<R, T>(reader: &mut R) -> Result<T, HandshakeError>
where
    R: AsyncBufRead + Unpin,
    T: for<'de> Deserialize<'de>,
{
    let mut line = Vec::new();
    loop {
        let available = reader.fill_buf().await?;
        if available.is_empty() {
            return if line.is_empty() {
                Err(HandshakeError::Eof)
            } else {
                Err(HandshakeError::MalformedFrame(
                    "EOF before newline terminator".to_string(),
                ))
            };
        }
        let take = available
            .iter()
            .position(|b| *b == b'\n')
            .map_or(available.len(), |pos| pos + 1);
        if line.len().saturating_add(take) > MAX_JSONL_FRAME_BYTES {
            return Err(HandshakeError::FrameTooLarge {
                max_bytes: MAX_JSONL_FRAME_BYTES,
            });
        }
        line.extend_from_slice(&available[..take]);
        reader.consume(take);
        if line.ends_with(b"\n") {
            break;
        }
    }
    if line.ends_with(b"\n") {
        line.pop();
    }
    if line.ends_with(b"\r") {
        line.pop();
    }
    if line.is_empty() {
        return Err(HandshakeError::MalformedFrame(
            "empty JSONL frame".to_string(),
        ));
    }
    Ok(serde_json::from_slice(&line)?)
}

pub async fn send_hello_after_verifier<W, F>(
    writer: &mut W,
    hello: &Hello,
    verifier: F,
) -> Result<(), HandshakeError>
where
    W: AsyncWrite + Unpin,
    F: FnOnce() -> Result<(), HandshakeError>,
{
    verifier()?;
    write_json_line(writer, hello).await
}

pub async fn client_handshake<R, W, F>(
    reader: &mut R,
    writer: &mut W,
    hello: &Hello,
    verifier: F,
) -> Result<HelloAck, HandshakeError>
where
    R: AsyncBufRead + Unpin,
    W: AsyncWrite + Unpin,
    F: FnOnce() -> Result<(), HandshakeError>,
{
    send_hello_after_verifier(writer, hello, verifier).await?;
    let ack: HelloAck = read_json_line(reader).await?;
    if ack.accepted {
        Ok(ack)
    } else {
        Err(HandshakeError::Rejected(ack.reason.unwrap_or_else(|| {
            "daemon returned accepted=false".to_string()
        })))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::pin::Pin;
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    };
    use std::task::{Context, Poll};
    use tokio::io::{AsyncWrite, BufReader};

    #[test]
    fn station_health_serde_roundtrip_and_forward_compat() {
        // Wire values are stable snake_case.
        assert_eq!(
            serde_json::to_value(StationHealth::AttendedPush).unwrap(),
            serde_json::json!("attended_push")
        );
        for h in [
            StationHealth::Armed,
            StationHealth::RecentlyDelivered,
            StationHealth::Unattended,
            StationHealth::UnattendedWithBacklog,
            StationHealth::AttendedPush,
            StationHealth::CoverageConflict,
            StationHealth::Idle,
        ] {
            let s = serde_json::to_value(h).unwrap();
            assert_eq!(serde_json::from_value::<StationHealth>(s).unwrap(), h);
        }
        // Forward-compat: an older client meeting a newer daemon's unknown value degrades to
        // `Unknown` instead of failing to deserialize the whole status.
        assert_eq!(
            serde_json::from_value::<StationHealth>(serde_json::json!("some_future_state"))
                .unwrap(),
            StationHealth::Unknown
        );
    }

    #[test]
    fn push_delivery_health_serde_roundtrip_and_forward_compat() {
        assert_eq!(
            serde_json::to_value(PushDeliveryHealth::StaleAccepted).unwrap(),
            serde_json::json!("stale_accepted")
        );
        for h in [
            PushDeliveryHealth::NotRegistered,
            PushDeliveryHealth::NoBacklog,
            PushDeliveryHealth::Delivering,
            PushDeliveryHealth::Probing,
            PushDeliveryHealth::StaleAccepted,
            PushDeliveryHealth::Failing,
        ] {
            let s = serde_json::to_value(h).unwrap();
            assert_eq!(serde_json::from_value::<PushDeliveryHealth>(s).unwrap(), h);
        }
        assert_eq!(
            serde_json::from_value::<PushDeliveryHealth>(serde_json::json!("future_push_state"))
                .unwrap(),
            PushDeliveryHealth::Unknown
        );
        // A member status missing the new fields deserializes with defaults (older daemon).
        let member: MemberStatus = serde_json::from_value(serde_json::json!({
            "store_key": "s", "backend": "sqlite", "session_id": "x", "address": "a",
            "occupant": "o", "host": "h", "waiters": 0, "lease_epoch": 1,
            "owner_instance_id": "i", "idle": false
        }))
        .unwrap();
        assert_eq!(member.push_delivery, PushDeliveryHealth::NotRegistered);
        assert_eq!(member.inbound_actionable_count, 0);
        assert_eq!(member.push_suppressed_count, 0);
        assert_eq!(member.delivery_mode, DeliveryMode::Unknown);
    }

    #[test]
    fn delivery_mode_serde_roundtrip_and_forward_compat() {
        for mode in [
            DeliveryMode::Push,
            DeliveryMode::Pull,
            DeliveryMode::Conflict,
        ] {
            let value = serde_json::to_value(mode).unwrap();
            assert_eq!(serde_json::from_value::<DeliveryMode>(value).unwrap(), mode);
        }
        assert_eq!(
            serde_json::from_value::<DeliveryMode>(serde_json::json!("future_mode")).unwrap(),
            DeliveryMode::Unknown
        );
    }

    #[test]
    fn register_replace_on_deliver_is_additive_and_defaults_false() {
        let old_wire = serde_json::json!({
            "op": "register",
            "store_key": "sqlite:/tmp/test.db",
            "address": "addr:a",
            "session_id": "s1",
            "occupant": "tester"
        });
        let request: Request = serde_json::from_value(old_wire).unwrap();
        assert!(matches!(
            request,
            Request::Register {
                replace_on_deliver: false,
                ..
            }
        ));

        let mut new_wire = serde_json::json!({
            "op": "register",
            "store_key": "sqlite:/tmp/test.db",
            "address": "addr:a",
            "session_id": "s1",
            "occupant": "tester",
            "replace_on_deliver": true
        });
        let request: Request = serde_json::from_value(new_wire.clone()).unwrap();
        assert!(matches!(
            request,
            Request::Register {
                replace_on_deliver: true,
                ..
            }
        ));
        new_wire["replace_on_deliver"] = serde_json::Value::Bool(true);
        assert_eq!(
            serde_json::to_value(request).unwrap()["replace_on_deliver"],
            new_wire["replace_on_deliver"]
        );
    }

    #[test]
    fn compatibility_table_explicitly_names_current_major_and_required_caps() {
        let row = COMPATIBILITY_TABLE
            .iter()
            .find(|row| row.protocol_major == PROTOCOL_MAJOR)
            .expect("current protocol major in table");
        assert_eq!(row.protocol_minor, PROTOCOL_MINOR);
        assert_eq!(row.required_capabilities, REQUIRED_CAPABILITIES);
        assert_eq!(row.unknown_required_capability_error, ERROR_INCOMPATIBLE);
    }

    #[test]
    fn hello_accepts_matching_protocol_and_required_caps() {
        let hello = client_hello("sqlite:C:\\store.db");
        let ack = evaluate_hello(&hello);
        assert!(ack.accepted, "unexpected rejection: {:?}", ack.reason);
        assert_eq!(ack.protocol_version, current_protocol_version());
        assert_eq!(ack.required_capabilities, daemon_required_capabilities());
    }

    #[test]
    fn hello_rejects_protocol_major_mismatch() {
        let mut hello = client_hello("store");
        hello.protocol_version.major = PROTOCOL_MAJOR + 1;
        let ack = evaluate_hello(&hello);
        assert!(!ack.accepted);
        assert!(ack
            .reason
            .as_deref()
            .unwrap_or_default()
            .contains("protocol major mismatch"));
    }

    #[test]
    fn hello_rejects_unknown_required_capability() {
        let mut hello = client_hello("store");
        hello
            .required_capabilities
            .push("future_required".to_string());
        let ack = evaluate_hello(&hello);
        assert!(!ack.accepted);
        assert!(ack
            .reason
            .as_deref()
            .unwrap_or_default()
            .contains("future_required"));
    }

    #[test]
    fn hello_rejects_client_missing_daemon_required_capability() {
        let mut hello = client_hello("store");
        hello.capabilities.retain(|cap| cap != CAP_ADMIN_CAP);
        let ack = evaluate_hello(&hello);
        assert!(!ack.accepted);
        assert!(ack
            .reason
            .as_deref()
            .unwrap_or_default()
            .contains(CAP_ADMIN_CAP));
    }

    #[test]
    fn hello_ignores_unknown_optional_capability() {
        let mut hello = client_hello("store");
        hello.capabilities.push("future_optional".to_string());
        let ack = evaluate_hello(&hello);
        assert!(ack.accepted, "optional cap should not reject: {:?}", ack);
    }

    #[test]
    fn watch_pid_spec_accepts_legacy_pid_and_typed_role() {
        let legacy: WatchPidSpec = serde_json::from_str("1234").unwrap();
        assert_eq!(legacy, WatchPidSpec::anchor(1234));

        let typed: WatchPidSpec =
            serde_json::from_str(r#"{"pid":5678,"role":"required"}"#).unwrap();
        assert_eq!(typed.pid, 5678);
        assert_eq!(typed.role, WatchPidRole::Required);
    }

    struct GuardedWriter<W> {
        inner: W,
        verified: Arc<AtomicBool>,
    }

    impl<W: AsyncWrite + Unpin> AsyncWrite for GuardedWriter<W> {
        fn poll_write(
            mut self: Pin<&mut Self>,
            cx: &mut Context<'_>,
            buf: &[u8],
        ) -> Poll<std::io::Result<usize>> {
            assert!(
                self.verified.load(Ordering::SeqCst),
                "Hello bytes were written before server-auth verifier ran"
            );
            Pin::new(&mut self.inner).poll_write(cx, buf)
        }

        fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
            Pin::new(&mut self.inner).poll_flush(cx)
        }

        fn poll_shutdown(
            mut self: Pin<&mut Self>,
            cx: &mut Context<'_>,
        ) -> Poll<std::io::Result<()>> {
            Pin::new(&mut self.inner).poll_shutdown(cx)
        }
    }

    #[tokio::test]
    async fn server_auth_verifier_runs_before_hello_bytes_are_written() {
        let (client, _server) = tokio::io::duplex(4096);
        let verified = Arc::new(AtomicBool::new(false));
        let mut writer = GuardedWriter {
            inner: client,
            verified: verified.clone(),
        };

        let hello = client_hello("store");

        send_hello_after_verifier(&mut writer, &hello, || {
            verified.store(true, Ordering::SeqCst);
            Ok(())
        })
        .await
        .unwrap();

        assert!(verified.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn jsonl_frame_size_and_malformed_edges_are_typed() {
        let at_limit_payload = format!(
            "\"{}\"\n",
            "a".repeat(MAX_JSONL_FRAME_BYTES.saturating_sub(3))
        );
        let mut at_limit = BufReader::new(at_limit_payload.as_bytes());
        let parsed: String = read_json_line(&mut at_limit).await.unwrap();
        assert_eq!(parsed.len(), MAX_JSONL_FRAME_BYTES - 3);

        let over_limit_payload = format!(
            "\"{}\"\n",
            "a".repeat(MAX_JSONL_FRAME_BYTES.saturating_sub(2))
        );
        let mut over_limit = BufReader::new(over_limit_payload.as_bytes());
        assert!(matches!(
            read_json_line::<_, String>(&mut over_limit).await,
            Err(HandshakeError::FrameTooLarge { .. })
        ));

        let mut malformed = BufReader::new(b"{not-json}\n".as_slice());
        assert!(matches!(
            read_json_line::<_, serde_json::Value>(&mut malformed).await,
            Err(HandshakeError::Json(_))
        ));

        let mut empty = BufReader::new(b"\n".as_slice());
        assert!(matches!(
            read_json_line::<_, serde_json::Value>(&mut empty).await,
            Err(HandshakeError::MalformedFrame(_))
        ));

        let mut eof_without_newline = BufReader::new(b"{\"ok\":true}".as_slice());
        assert!(matches!(
            read_json_line::<_, serde_json::Value>(&mut eof_without_newline).await,
            Err(HandshakeError::MalformedFrame(_))
        ));
    }
}
