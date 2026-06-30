//! Daemon-scoped IPC protocol foundation.  This is separate from the legacy
//! address-keyed `ipc` module so P2 can add the daemon singleton surface without
//! rewriting the current resident-holder verbs.

use crate::model::DeliveryOutcome;
use serde::{Deserialize, Deserializer, Serialize};
use std::collections::BTreeSet;
use std::fmt;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt};

pub const PROTOCOL_MAJOR: u16 = 1;
pub const PROTOCOL_MINOR: u16 = 3;
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
pub enum WatchPidRole {
    Anchor,
    Required,
}

impl Default for WatchPidRole {
    fn default() -> Self {
        Self::Anchor
    }
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
    Ping,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NeedsAttachReason {
    RestartLost,
    DeliberatelyDetached,
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
    #[serde(default)]
    pub station_health: StationHealth,
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
    Idle,
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
    REQUIRED_CAPABILITIES
        .iter()
        .map(|s| (*s).to_string())
        .collect()
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
    } else if let Some(cap) = required
        .iter()
        .find(|cap| !client_caps.contains(cap.as_str()))
    {
        Some(format!("client missing required capability: {cap}"))
    } else {
        None
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
