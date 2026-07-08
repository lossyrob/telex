//! Hidden daemon singleton foundation: singleton identity, endpoint naming, capability
//! file handling, connect-or-spawn, and a P2 JSONL server loop.

#[cfg(feature = "postgres")]
use crate::backend::postgres::{
    make_tls as make_postgres_tls, notify_channel_for_schema, sanitize_ident,
};
#[cfg(feature = "sqlite")]
use crate::backend::sqlite::SqliteBackend;
use crate::backend::{Backend, WaitFetchOptions};
use crate::daemon_ipc::{
    self as proto, current_protocol_version, read_json_line, write_json_line, DaemonStatus,
    DeafStationStatus, EpochStatus, HandshakeError, HelloAck, IdleStationStatus, LiveWaiterStatus,
    MemberStatus, NeedsAttachReason, PushDeliveryHealth, RecentErrorStatus, Request, Response,
    RetentionStatus, SentReceipt, StationHealth, StoreStatus, WaiterOutcome, WatchPidRole,
    WatchPidSpec, WatchPidStatus, ON_DELIVER_DEFERRED_EXIT, ON_DELIVER_PERMANENT_EXIT,
};
use crate::model::{
    cc_recipients, delivery_role, now_ms, requires_disposition_for_recipient, Attention,
    DeliveryOutcome, EpochClaimResult, MessageRow, NewMessage, STATUS_RETIRED,
};
#[cfg(feature = "postgres")]
use anyhow::Context;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::fmt;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tokio::io::BufReader;
#[cfg(feature = "postgres")]
use tokio::sync::mpsc;
use tokio::sync::Semaphore;
use tokio::sync::{Mutex as AsyncMutex, Notify};
#[cfg(feature = "postgres")]
use tokio_postgres::AsyncMessage;

pub const READINESS_TIMEOUT: Duration = Duration::from_secs(5);
pub const CONNECT_ATTEMPT_TIMEOUT: Duration = Duration::from_millis(500);
pub const BACKOFF_INITIAL: Duration = Duration::from_millis(50);
pub const BACKOFF_MAX: Duration = Duration::from_millis(500);
pub const CRASHLOOP_MAX: usize = 3;
pub const CRASHLOOP_WINDOW: Duration = Duration::from_secs(10);
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(5);
const RECENT_ERROR_LIMIT: usize = 32;
const DEFAULT_IDLE_TTL: Duration = Duration::from_secs(24 * 60 * 60);
const RECENT_DELIVERY_HEALTH_GRACE_MS: i64 = 2 * 60 * 1000;
const DEFAULT_RETENTION_WARN_ROWS: i64 = 100_000;
const DEFAULT_IDLE_STATION_WARN: usize = 1_000;
const DEFAULT_DEAF_WARN_MS: i64 = 2 * 60 * 1000;

pub type Result<T> = std::result::Result<T, DaemonError>;

#[cfg(windows)]
const WINDOWS_ELEVATION_MISMATCH_HINT: &str = "On Windows, this usually means the telex daemon and this process are running at different elevations (Administrator vs non-Administrator), so they cannot authenticate over the daemon named pipe. Stop the existing daemon from a matching-elevation terminal, or restart/attach from the same elevation as this session (for an elevated session, start telex from an Administrator terminal).";

fn daemon_handshake_eof_message() -> String {
    let mut message = "daemon closed the connection during handshake".to_string();
    #[cfg(windows)]
    {
        message.push_str("; ");
        message.push_str(WINDOWS_ELEVATION_MISMATCH_HINT);
    }
    message
}

#[derive(Debug)]
pub enum DaemonError {
    Io {
        action: &'static str,
        source: std::io::Error,
    },
    Json(serde_json::Error),
    Incompatible(String),
    Unauthorized(String),
    NotRunning(String),
    AlreadyRunning(String),
    Timeout(String),
    Unsupported {
        capability: &'static str,
        message: String,
    },
    Protocol(String),
}

fn verify_expected_peer_identity(
    actual_pid: u32,
    actual_start_time: Option<u64>,
    expected_pid: Option<u32>,
    expected_start_time: Option<u64>,
) -> Result<()> {
    if let Some(expected_pid) = expected_pid {
        if actual_pid != expected_pid {
            return Err(DaemonError::Unauthorized(format!(
                "server pid {actual_pid} does not match expected pid {expected_pid}"
            )));
        }
    }
    if let Some(expected_start_time) = expected_start_time {
        match actual_start_time {
            Some(actual_start_time) if actual_start_time == expected_start_time => {}
            Some(actual_start_time) => {
                return Err(DaemonError::Unauthorized(format!(
                    "server start time {actual_start_time} does not match expected start time {expected_start_time}"
                )));
            }
            None => {
                return Err(DaemonError::Unauthorized(
                    "server start time could not be verified".into(),
                ));
            }
        }
    }
    Ok(())
}

impl fmt::Display for DaemonError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DaemonError::Io { action, source } => write!(f, "{action}: {source}"),
            DaemonError::Json(e) => write!(f, "JSON framing failed: {e}"),
            DaemonError::Incompatible(msg) => write!(f, "incompatible daemon IPC: {msg}"),
            DaemonError::Unauthorized(msg) => write!(f, "unauthorized daemon IPC request: {msg}"),
            DaemonError::NotRunning(msg) => write!(f, "daemon is not running: {msg}"),
            DaemonError::AlreadyRunning(msg) => {
                write!(f, "daemon singleton already running: {msg}")
            }
            DaemonError::Timeout(msg) => write!(f, "daemon readiness timed out: {msg}"),
            DaemonError::Unsupported {
                capability,
                message,
            } => write!(f, "{capability} is unsupported on this platform: {message}"),
            DaemonError::Protocol(msg) => write!(f, "daemon IPC protocol error: {msg}"),
        }
    }
}

impl std::error::Error for DaemonError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            DaemonError::Io { source, .. } => Some(source),
            DaemonError::Json(e) => Some(e),
            _ => None,
        }
    }
}

impl From<serde_json::Error> for DaemonError {
    fn from(value: serde_json::Error) -> Self {
        DaemonError::Json(value)
    }
}

impl From<HandshakeError> for DaemonError {
    fn from(value: HandshakeError) -> Self {
        match value {
            HandshakeError::Verify(e) => DaemonError::Unauthorized(e),
            HandshakeError::Io(e) => DaemonError::Io {
                action: "daemon IPC",
                source: e,
            },
            HandshakeError::Json(e) => DaemonError::Json(e),
            HandshakeError::FrameTooLarge { max_bytes } => {
                DaemonError::Protocol(format!("daemon IPC frame exceeded {max_bytes} bytes"))
            }
            HandshakeError::MalformedFrame(e) => DaemonError::Protocol(e),
            HandshakeError::Eof => DaemonError::Protocol(daemon_handshake_eof_message()),
            HandshakeError::Rejected(reason) => DaemonError::Incompatible(reason),
        }
    }
}

fn io_err(action: &'static str, source: std::io::Error) -> DaemonError {
    DaemonError::Io { action, source }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SingletonKey {
    pub user_identity: String,
    pub config_root: PathBuf,
    pub protocol_major: u16,
}

impl SingletonKey {
    pub fn current() -> Result<Self> {
        let config_root = prepare_config_root()?;
        Ok(Self {
            user_identity: platform::current_user_identity()?,
            config_root,
            protocol_major: proto::PROTOCOL_MAJOR,
        })
    }

    pub fn from_parts(
        user_identity: impl Into<String>,
        config_root: impl Into<PathBuf>,
        protocol_major: u16,
    ) -> Self {
        Self {
            user_identity: user_identity.into(),
            config_root: config_root.into(),
            protocol_major,
        }
    }

    pub fn material(&self) -> String {
        format!(
            "user={};config_root={};protocol_major={}",
            self.user_identity,
            self.config_root.to_string_lossy(),
            self.protocol_major
        )
    }

    pub fn short_hash(&self) -> String {
        short_hash(self.material().as_bytes())
    }

    pub fn redacted_material(&self) -> String {
        format!(
            "user=<redacted>;config_root={};protocol_major={}",
            self.config_root.to_string_lossy(),
            self.protocol_major
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Endpoint {
    #[cfg(windows)]
    WindowsPipe(String),
    #[cfg(unix)]
    UnixSocket(PathBuf),
}

impl Endpoint {
    pub fn display(&self) -> String {
        match self {
            #[cfg(windows)]
            Endpoint::WindowsPipe(name) => name.clone(),
            #[cfg(unix)]
            Endpoint::UnixSocket(path) => path.to_string_lossy().into_owned(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DaemonPaths {
    pub singleton: SingletonKey,
    pub singleton_hash: String,
    pub run_dir: PathBuf,
    pub endpoint: Endpoint,
    pub cap_path: PathBuf,
}

impl DaemonPaths {
    pub fn current() -> Result<Self> {
        let singleton = SingletonKey::current()?;
        let run_dir = prepare_runtime_dir()?;
        Ok(Self::for_key(singleton, run_dir))
    }

    pub fn for_key(singleton: SingletonKey, run_dir: impl Into<PathBuf>) -> Self {
        let run_dir = run_dir.into();
        let singleton_hash = singleton.short_hash();
        #[cfg(windows)]
        let endpoint = Endpoint::WindowsPipe(format!(r"\\.\pipe\telex-daemon-{singleton_hash}"));
        #[cfg(unix)]
        let endpoint =
            Endpoint::UnixSocket(run_dir.join(format!("telex-daemon-{singleton_hash}.sock")));
        let cap_path = run_dir.join(format!("daemon-{singleton_hash}.cap"));
        Self {
            singleton,
            singleton_hash,
            run_dir,
            endpoint,
            cap_path,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapFile {
    pub instance_id: String,
    pub admin_cap: String,
    pub singleton_hash: String,
    pub protocol_major: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_pid: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_start_time: Option<u64>,
}

impl CapFile {
    pub fn redacted(&self) -> serde_json::Value {
        serde_json::json!({
            "instance_id": self.instance_id,
            "admin_cap": proto::REDACTED_SECRET,
            "singleton_hash": self.singleton_hash,
            "protocol_major": self.protocol_major,
            "server_pid": self.server_pid,
            "server_start_time": self.server_start_time,
        })
    }
}

fn cap_required_peer_identity(cap: &CapFile) -> Result<(u32, u64)> {
    let pid = cap.server_pid.ok_or_else(|| {
        DaemonError::Unauthorized("daemon capability file is missing server_pid".to_string())
    })?;
    let start_time = cap.server_start_time.ok_or_else(|| {
        DaemonError::Unauthorized("daemon capability file is missing server_start_time".to_string())
    })?;
    Ok((pid, start_time))
}

pub struct DaemonState {
    paths: DaemonPaths,
    instance_id: String,
    admin_cap: String,
    stores: Mutex<HashMap<String, StoreEntry>>,
    store_open_guard: AsyncMutex<()>,
    members: Mutex<BTreeMap<MemberKey, MemberRecord>>,
    waiters: Mutex<BTreeMap<WaiterKey, WaiterRecord>>,
    next_waiter_id: AtomicU64,
    recent_errors: Arc<Mutex<VecDeque<RecentErrorStatus>>>,
    ended_sessions: Mutex<BTreeMap<SessionKey, EndedSessionRecord>>,
    draining: AtomicBool,
    on_deliver: OnDeliverState,
}

#[derive(Clone)]
struct StoreEntry {
    kind: String,
    backend: Arc<dyn Backend>,
    notify: Arc<Notify>,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct MemberKey {
    store_key: String,
    session_id: String,
    address: String,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct SessionKey {
    store_key: String,
    session_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct WaiterKey {
    waiter_id: u64,
}

#[derive(Clone, Debug)]
struct MemberRecord {
    address: String,
    store_key: String,
    backend: String,
    session_id: String,
    occupant: String,
    host: String,
    waiters: usize,
    watch_pids: Vec<WatchPidRecord>,
    description: Option<String>,
    scope: Option<String>,
    tags: Option<String>,
    lease_epoch: i64,
    owner_instance_id: String,
    idle: bool,
    idle_rearmable: bool,
    unattended_since_ms: Option<i64>,
    unattended_with_backlog_since_ms: Option<i64>,
    last_waiter_exit_at_ms: Option<i64>,
    last_waiter_outcome: Option<WaiterOutcome>,
    last_waiter_exit_code: Option<i32>,
    last_waiter_detail: Option<String>,
    last_waiter_pid: Option<u32>,
    last_delivered_message_id: Option<i64>,
    /// Harness-neutral on-deliver handler argv registered for this address/session, if any.
    on_deliver: Option<Vec<String>>,
    on_deliver_wake_on_cc: bool,
    on_deliver_cc_after_ms: Option<i64>,
}

#[derive(Clone, Debug)]
struct WatchPidRecord {
    pid: u32,
    start_time: Option<u64>,
    role: WatchPidRole,
}

#[derive(Clone, Debug)]
struct WaiterRecord {
    waiter_id: u64,
    store_key: String,
    session_id: String,
    address: String,
    pid: u32,
    start_time: Option<u64>,
    started_at_ms: i64,
    attention: Option<String>,
    min_attention: Option<String>,
    wake_on_cc: bool,
    cc_after_ms: Option<i64>,
    timeout_ms: Option<u64>,
}

#[derive(Clone, Debug)]
struct EndedSessionRecord {
    at_ms: i64,
    reason: String,
    addresses: BTreeSet<String>,
    occupant: Option<String>,
}

impl DaemonState {
    async fn status(&self) -> DaemonStatus {
        self.status_with_thresholds(
            retention_warn_threshold(),
            idle_station_warn_threshold(),
            deaf_warn_threshold_ms(),
        )
        .await
    }

    fn status_minimal(&self) -> DaemonStatus {
        DaemonStatus {
            protocol_version: current_protocol_version(),
            daemon_version: proto::DAEMON_VERSION.to_string(),
            instance_id: self.instance_id.clone(),
            singleton_key: self.paths.singleton.redacted_material(),
            stores: Vec::new(),
            backoff: vec!["n/a: crashloop backoff is not persisted by the daemon".to_string()],
            recent_errors: Vec::new(),
            epoch_by_address: Vec::new(),
            members: Vec::new(),
            live_waiters: Vec::new(),
            retention: Vec::new(),
            idle_stations: IdleStationStatus::default(),
            deaf_stations: DeafStationStatus::default(),
        }
    }

    async fn status_with_thresholds(
        &self,
        retention_warn_threshold: i64,
        idle_station_warn_threshold: usize,
        deaf_warn_threshold_ms: i64,
    ) -> DaemonStatus {
        self.prune_dead_waiters();
        let store_entries: Vec<(String, StoreEntry)> = self
            .stores
            .lock()
            .unwrap()
            .iter()
            .map(|(store_key, entry)| (store_key.clone(), entry.clone()))
            .collect();
        let stores = store_entries
            .iter()
            .map(|(store_key, entry)| StoreStatus {
                store_key: store_key.clone(),
                kind: entry.kind.clone(),
            })
            .collect();
        let mut retention = Vec::new();
        for (store_key, entry) in &store_entries {
            match entry.backend.delivery_retention_count().await {
                Ok(delivery_rows) => retention.push(RetentionStatus {
                    store_key: store_key.clone(),
                    delivery_rows,
                    warn: delivery_rows >= retention_warn_threshold,
                    warn_threshold: retention_warn_threshold,
                }),
                Err(e) => self.push_recent_error(
                    "BackendDisconnect",
                    format!("retention count failed for {store_key}: {e:#}"),
                ),
            }
        }

        let live_waiters = self.live_waiter_statuses();
        let member_records: Vec<MemberRecord> =
            self.members.lock().unwrap().values().cloned().collect();
        let store_backends: HashMap<String, Arc<dyn Backend>> = store_entries
            .iter()
            .map(|(store_key, entry)| (store_key.clone(), entry.backend.clone()))
            .collect();
        // Both observability counts per unique (store, address), in a single backend pass that
        // materializes pending delivery rows once. `pending_unconsumed_count` counts all unconsumed
        // non-terminal deliveries; `inbound_actionable_count` counts only those requiring THIS
        // station's disposition (primary + requires_disposition), so no-disposition notes and, on a
        // shared address, traffic this station need not act on are separated out.
        let mut pending_counts: HashMap<(String, String), i64> = HashMap::new();
        let mut inbound_actionable_counts: HashMap<(String, String), i64> = HashMap::new();
        for member in &member_records {
            let key = (member.store_key.clone(), member.address.clone());
            if pending_counts.contains_key(&key) {
                continue;
            }
            let (pending, actionable) = match store_backends.get(&member.store_key) {
                Some(backend) => match backend.pending_and_actionable_counts(&member.address).await
                {
                    Ok(counts) => counts,
                    Err(e) => {
                        self.push_recent_error(
                            "BackendDisconnect",
                            format!(
                                "pending/actionable counts failed for {} {}: {e:#}",
                                member.store_key, member.address
                            ),
                        );
                        (0, 0)
                    }
                },
                None => (0, 0),
            };
            pending_counts.insert(key.clone(), pending);
            inbound_actionable_counts.insert(key, actionable);
        }
        // Push-delivery health + suppressed counts per member. Computed BEFORE taking the members
        // lock as a defensive measure: `push_delivery_health`/`push_suppressed_count` lock
        // `on_deliver.pushed`/`dead_lettered`. There is no strict inversion today
        // (`on_deliver_advance_cc_lower_bound` releases `pushed` before it locks `members`), but
        // computing these outside the members lock keeps the two mutexes decoupled so a future
        // members-then-pushed path cannot introduce one.
        let (push_health_by_key, push_suppressed_by_key) = {
            let now_inst = Instant::now();
            let mut health: HashMap<MemberKey, PushDeliveryHealth> = HashMap::new();
            let mut suppressed: HashMap<MemberKey, i64> = HashMap::new();
            for member in &member_records {
                let key = MemberKey {
                    store_key: member.store_key.clone(),
                    session_id: member.session_id.clone(),
                    address: member.address.clone(),
                };
                let pending = pending_counts
                    .get(&(member.store_key.clone(), member.address.clone()))
                    .copied()
                    .unwrap_or(0);
                health.insert(
                    key.clone(),
                    self.push_delivery_health(&key, pending, member.on_deliver.is_some(), now_inst),
                );
                suppressed.insert(key.clone(), self.push_suppressed_count(&key));
            }
            (health, suppressed)
        };
        let member_records: Vec<MemberRecord> = {
            let now = now_ms();
            let mut members = self.members.lock().unwrap();
            for member in members.values_mut() {
                let live_waiters_count = live_waiters
                    .iter()
                    .filter(|waiter| {
                        waiter.store_key == member.store_key
                            && waiter.session_id == member.session_id
                            && waiter.address == member.address
                    })
                    .count();
                let pending = pending_counts
                    .get(&(member.store_key.clone(), member.address.clone()))
                    .copied()
                    .unwrap_or(0);
                let push_health = push_health_by_key
                    .get(&MemberKey {
                        store_key: member.store_key.clone(),
                        session_id: member.session_id.clone(),
                        address: member.address.clone(),
                    })
                    .copied()
                    .unwrap_or(PushDeliveryHealth::NotRegistered);
                if !member.idle && live_waiters_count == 0 {
                    member.unattended_since_ms.get_or_insert(now);
                    // For a push station, "uncovered backlog" means push delivery is actually
                    // failing (bridge unreachable) — NOT merely that messages are pending while the
                    // bridge is delivering/probing. For a pull station, any pending is backlog.
                    let backlog_uncovered = if member.on_deliver.is_some() {
                        push_health == PushDeliveryHealth::Failing
                    } else {
                        pending > 0
                    };
                    if backlog_uncovered {
                        member.unattended_with_backlog_since_ms.get_or_insert(now);
                    } else {
                        member.unattended_with_backlog_since_ms = None;
                    }
                } else {
                    member.unattended_with_backlog_since_ms = None;
                }
            }
            members.values().cloned().collect()
        };
        let members: Vec<MemberStatus> = member_records
            .iter()
            .map(|member| {
                let pending = pending_counts
                    .get(&(member.store_key.clone(), member.address.clone()))
                    .copied()
                    .unwrap_or(0);
                let key = MemberKey {
                    store_key: member.store_key.clone(),
                    session_id: member.session_id.clone(),
                    address: member.address.clone(),
                };
                let push_health = push_health_by_key
                    .get(&key)
                    .copied()
                    .unwrap_or(PushDeliveryHealth::NotRegistered);
                let inbound_actionable = inbound_actionable_counts
                    .get(&(member.store_key.clone(), member.address.clone()))
                    .copied()
                    .unwrap_or(0);
                let push_suppressed = push_suppressed_by_key.get(&key).copied().unwrap_or(0);
                let mut status = member.status(
                    &live_waiters,
                    pending,
                    inbound_actionable,
                    push_health,
                    push_suppressed,
                    deaf_warn_threshold_ms,
                );
                status.push_deferred_count = self.on_deliver_deferred_count(&key);
                status
            })
            .collect();
        let epoch_by_address = members
            .iter()
            .map(|m| EpochStatus {
                store_key: m.store_key.clone(),
                address: m.address.clone(),
                lease_epoch: m.lease_epoch,
                owner_instance_id: m.owner_instance_id.clone(),
                idle: m.idle,
            })
            .collect();
        let idle_count = members.iter().filter(|m| m.idle).count();
        let deaf_count = members.iter().filter(|m| m.deaf_warn).count();
        DaemonStatus {
            protocol_version: current_protocol_version(),
            daemon_version: proto::DAEMON_VERSION.to_string(),
            instance_id: self.instance_id.clone(),
            singleton_key: self.paths.singleton.redacted_material(),
            stores,
            backoff: vec!["n/a: crashloop backoff is not persisted by the daemon".to_string()],
            recent_errors: self.recent_errors(),
            epoch_by_address,
            members,
            live_waiters,
            retention,
            idle_stations: IdleStationStatus {
                count: idle_count,
                warn: idle_count >= idle_station_warn_threshold,
                warn_threshold: idle_station_warn_threshold,
            },
            deaf_stations: DeafStationStatus {
                count: deaf_count,
                warn: deaf_count > 0,
                warn_threshold_ms: deaf_warn_threshold_ms,
            },
        }
    }

    fn check_admin_cap(&self, proof: Option<&str>) -> std::result::Result<(), Response> {
        match proof {
            Some(proof) if proof == self.admin_cap => Ok(()),
            Some(proof) => Err(proto::unauthorized(proto::redact_secrets(
                format!("invalid admin capability proof: {proof}"),
                &[proof, &self.admin_cap],
            ))),
            None => Err(proto::unauthorized("admin capability proof required")),
        }
    }

    async fn backend_for(
        &self,
        store_key: &str,
    ) -> std::result::Result<Arc<dyn Backend>, Response> {
        if let Some(entry) = self.stores.lock().unwrap().get(store_key).cloned() {
            return Ok(entry.backend);
        }

        let _open_guard = self.store_open_guard.lock().await;
        if let Some(entry) = self.stores.lock().unwrap().get(store_key).cloned() {
            return Ok(entry.backend);
        }

        let entry = open_store_entry(store_key, self.recent_errors.clone()).await?;
        let backend = entry.backend.clone();
        self.stores
            .lock()
            .unwrap()
            .insert(store_key.to_string(), entry);
        Ok(backend)
    }

    fn store_notify(&self, store_key: &str) -> Option<Arc<Notify>> {
        self.stores
            .lock()
            .unwrap()
            .get(store_key)
            .map(|entry| entry.notify.clone())
    }

    fn member_key(store_key: &str, session_id: &str, address: &str) -> MemberKey {
        MemberKey {
            store_key: store_key.to_string(),
            session_id: session_id.to_string(),
            address: address.to_string(),
        }
    }

    fn session_key(store_key: &str, session_id: &str) -> SessionKey {
        SessionKey {
            store_key: store_key.to_string(),
            session_id: session_id.to_string(),
        }
    }

    fn waiter_key(waiter_id: u64) -> WaiterKey {
        WaiterKey { waiter_id }
    }

    fn get_member(&self, store_key: &str, session_id: &str, address: &str) -> Option<MemberRecord> {
        self.members
            .lock()
            .unwrap()
            .get(&Self::member_key(store_key, session_id, address))
            .cloned()
    }

    fn session_members(&self, store_key: &str, session_id: &str) -> Vec<MemberRecord> {
        self.members
            .lock()
            .unwrap()
            .values()
            .filter(|m| m.store_key == store_key && m.session_id == session_id && !m.idle)
            .cloned()
            .collect()
    }

    /// Active members for a session across ALL stores. The idle drain (issue #65) uses this instead
    /// of the store-scoped variant: the `agentStop` drain hook is static and resolves the client's
    /// ambient store, which differs from a session attached with a named `--backend`/`--db`. Since
    /// the daemon is a per-user singleton holding every store's members, and a Copilot `session_id`
    /// is globally unique, matching by `session_id` alone drains the correct members regardless of
    /// which store the drain client resolved.
    fn session_members_any_store(&self, session_id: &str) -> Vec<MemberRecord> {
        self.members
            .lock()
            .unwrap()
            .values()
            .filter(|m| m.session_id == session_id && !m.idle)
            .cloned()
            .collect()
    }

    fn has_address_member(&self, store_key: &str, address: &str) -> bool {
        self.members
            .lock()
            .unwrap()
            .values()
            .any(|m| m.store_key == store_key && m.address == address && !m.idle)
    }

    fn note_backlog_for_unattended_address(&self, store_key: &str, address: &str) {
        let now = now_ms();
        let mut members = self.members.lock().unwrap();
        for member in members.values_mut().filter(|m| {
            m.store_key == store_key && m.address == address && !m.idle && m.waiters == 0
        }) {
            member.unattended_since_ms.get_or_insert(now);
            member.unattended_with_backlog_since_ms.get_or_insert(now);
        }
    }

    fn insert_member(&self, record: MemberRecord) {
        self.members.lock().unwrap().insert(
            Self::member_key(&record.store_key, &record.session_id, &record.address),
            record,
        );
    }

    fn mark_session_idle(
        &self,
        store_key: &str,
        session_id: &str,
        kind: &str,
        reason: &str,
        definite_end: bool,
    ) -> Vec<MemberRecord> {
        let mut affected = Vec::new();
        {
            let mut members = self.members.lock().unwrap();
            for member in members
                .values_mut()
                .filter(|m| m.store_key == store_key && m.session_id == session_id && !m.idle)
            {
                let prior = member.clone();
                member.idle = true;
                member.idle_rearmable = false;
                member.waiters = 0;
                member.unattended_since_ms = Some(now_ms());
                member.unattended_with_backlog_since_ms = None;
                if prior.waiters > 0 {
                    member.last_waiter_exit_at_ms = Some(now_ms());
                    member.last_waiter_outcome = Some(WaiterOutcome::PresenceEnded);
                    member.last_waiter_exit_code = Some(5);
                    member.last_waiter_detail = Some(presence_ended_detail(kind));
                }
                affected.push(prior);
            }
        }
        for member in &affected {
            self.push_recent_error(
                kind,
                format!(
                    "{kind}: marked idle store={} session={} address={} prior_occupant={} prior_waiters={}: {reason}",
                    member.store_key, member.session_id, member.address, member.occupant, member.waiters
                ),
            );
        }
        if definite_end && !affected.is_empty() {
            self.record_definite_session_end(store_key, session_id, kind, &affected);
        }
        affected
    }

    fn mark_address_idle(
        &self,
        store_key: &str,
        address: &str,
        kind: &str,
        reason: &str,
    ) -> Vec<MemberRecord> {
        let mut affected = Vec::new();
        {
            let mut members = self.members.lock().unwrap();
            for member in members
                .values_mut()
                .filter(|m| m.store_key == store_key && m.address == address && !m.idle)
            {
                let prior = member.clone();
                member.idle = true;
                member.idle_rearmable = false;
                member.waiters = 0;
                member.unattended_since_ms = Some(now_ms());
                member.unattended_with_backlog_since_ms = None;
                if prior.waiters > 0 {
                    member.last_waiter_exit_at_ms = Some(now_ms());
                    member.last_waiter_outcome = Some(WaiterOutcome::PresenceEnded);
                    member.last_waiter_exit_code = Some(5);
                    member.last_waiter_detail = Some(presence_ended_detail(kind));
                }
                affected.push(prior);
            }
        }
        for member in &affected {
            self.push_recent_error(
                kind,
                format!(
                    "{kind}: marked idle store={} session={} address={} prior_occupant={} prior_waiters={}: {reason}",
                    member.store_key, member.session_id, member.address, member.occupant, member.waiters
                ),
            );
        }
        affected
    }

    fn mark_member_idle(
        &self,
        store_key: &str,
        session_id: &str,
        address: &str,
        kind: &str,
        reason: &str,
    ) -> Option<MemberRecord> {
        let prior = {
            let mut members = self.members.lock().unwrap();
            members
                .get_mut(&Self::member_key(store_key, session_id, address))
                .and_then(|member| {
                    if member.idle {
                        None
                    } else {
                        let prior = member.clone();
                        member.idle = true;
                        member.idle_rearmable = kind == "IdleTtlReap";
                        member.waiters = 0;
                        member.unattended_since_ms = Some(now_ms());
                        member.unattended_with_backlog_since_ms = None;
                        if prior.waiters > 0 {
                            member.last_waiter_exit_at_ms = Some(now_ms());
                            member.last_waiter_outcome = Some(WaiterOutcome::PresenceEnded);
                            member.last_waiter_exit_code = Some(5);
                            member.last_waiter_detail = Some(presence_ended_detail(kind));
                        }
                        Some(prior)
                    }
                })
        };
        if let Some(member) = &prior {
            self.push_recent_error(
                kind,
                format!(
                    "{kind}: marked idle store={} session={} address={} prior_occupant={} prior_waiters={}: {reason}",
                    member.store_key, member.session_id, member.address, member.occupant, member.waiters
                ),
            );
        }
        prior
    }

    fn record_definite_session_end(
        &self,
        store_key: &str,
        session_id: &str,
        reason: &str,
        affected: &[MemberRecord],
    ) {
        let addresses = affected.iter().map(|m| m.address.clone()).collect();
        let occupant = affected.first().map(|m| m.occupant.clone());
        self.ended_sessions.lock().unwrap().insert(
            Self::session_key(store_key, session_id),
            EndedSessionRecord {
                at_ms: now_ms(),
                reason: reason.to_string(),
                addresses,
                occupant,
            },
        );
    }

    fn session_definite_end(
        &self,
        store_key: &str,
        session_id: &str,
    ) -> Option<EndedSessionRecord> {
        self.ended_sessions
            .lock()
            .unwrap()
            .get(&Self::session_key(store_key, session_id))
            .cloned()
    }

    fn check_session_id_reuse_tripwire(&self, record: &MemberRecord) {
        let ended = self.session_definite_end(&record.store_key, &record.session_id);
        let Some(ended) = ended else {
            return;
        };
        self.push_recent_error(
            "SessionIdReuse",
            format!(
                "SESSION_ID_REUSE_TRIPWIRE store={} session={} re-registered address={} occupant={} after definite_end reason={} at_ms={} prior_addresses={:?} prior_occupant={:?}",
                record.store_key,
                record.session_id,
                record.address,
                record.occupant,
                ended.reason,
                ended.at_ms,
                ended.addresses,
                ended.occupant
            ),
        );
    }

    fn clear_definite_session_end(&self, store_key: &str, session_id: &str) {
        self.ended_sessions
            .lock()
            .unwrap()
            .remove(&Self::session_key(store_key, session_id));
    }

    fn rearm_idle_member_if_allowed(
        &self,
        store_key: &str,
        session_id: &str,
        address: &str,
    ) -> Option<MemberRecord> {
        let key = Self::member_key(store_key, session_id, address);
        let mut members = self.members.lock().unwrap();
        let member = members.get_mut(&key)?;
        if !member.idle {
            if member.unattended_since_ms.is_none() {
                member.unattended_since_ms = Some(now_ms());
            }
            return Some(member.clone());
        }
        if !member.idle_rearmable {
            return None;
        }
        member.idle = false;
        member.idle_rearmable = false;
        member.unattended_since_ms = Some(now_ms());
        Some(member.clone())
    }

    fn remove_member(
        &self,
        store_key: &str,
        session_id: &str,
        address: &str,
    ) -> Option<MemberRecord> {
        self.members
            .lock()
            .unwrap()
            .remove(&Self::member_key(store_key, session_id, address))
    }

    fn remove_member_if_current(&self, record: &MemberRecord) -> bool {
        let key = Self::member_key(&record.store_key, &record.session_id, &record.address);
        let should_remove = {
            let mut members = self.members.lock().unwrap();
            let should = members.get(&key).is_some_and(|current| {
                current.lease_epoch == record.lease_epoch
                    && current.owner_instance_id == record.owner_instance_id
            });
            if should {
                members.remove(&key);
            }
            should
        };
        if should_remove {
            self.on_deliver_forget_member(&key);
        }
        should_remove
    }

    fn members_snapshot(&self) -> Vec<MemberRecord> {
        self.members.lock().unwrap().values().cloned().collect()
    }

    fn clear_members(&self) {
        self.members.lock().unwrap().clear();
        self.waiters.lock().unwrap().clear();
        self.on_deliver.pushed.lock().unwrap().clear();
    }

    fn push_recent_error(&self, kind: impl Into<String>, message: impl Into<String>) {
        push_recent_error_to_queue(&self.recent_errors, kind, message, &[&self.admin_cap]);
    }

    fn recent_errors(&self) -> Vec<RecentErrorStatus> {
        self.recent_errors.lock().unwrap().iter().cloned().collect()
    }

    fn begin_draining(&self) -> bool {
        !self.draining.swap(true, Ordering::SeqCst)
    }

    fn clear_draining(&self) {
        self.draining.store(false, Ordering::SeqCst);
    }

    fn is_draining(&self) -> bool {
        self.draining.load(Ordering::SeqCst)
    }

    fn live_waiter_statuses(&self) -> Vec<LiveWaiterStatus> {
        self.waiters
            .lock()
            .unwrap()
            .values()
            .map(WaiterRecord::status)
            .collect()
    }

    fn live_waiter_statuses_for(
        &self,
        store_key: &str,
        session_id: &str,
        address: &str,
    ) -> Vec<LiveWaiterStatus> {
        self.prune_dead_waiters();
        self.waiters
            .lock()
            .unwrap()
            .values()
            .filter(|waiter| {
                waiter.store_key == store_key
                    && waiter.session_id == session_id
                    && waiter.address == address
            })
            .map(WaiterRecord::status)
            .collect()
    }

    fn has_live_waiter_for(&self, store_key: &str, session_id: &str, address: &str) -> bool {
        self.prune_dead_waiters();
        self.waiters.lock().unwrap().values().any(|waiter| {
            waiter.store_key == store_key
                && waiter.session_id == session_id
                && waiter.address == address
        })
    }

    fn prune_dead_waiters(&self) {
        let mut removed = Vec::new();
        {
            let mut waiters = self.waiters.lock().unwrap();
            waiters.retain(|_, waiter| {
                let alive = waiter.pid == 0
                    || crate::session_watch::process_alive_with_start_time(
                        waiter.pid,
                        waiter.start_time,
                    );
                if !alive {
                    removed.push((
                        waiter.store_key.clone(),
                        waiter.session_id.clone(),
                        waiter.address.clone(),
                        waiter.pid,
                        waiter.started_at_ms,
                    ));
                }
                alive
            });
        }
        if removed.is_empty() {
            return;
        }
        let mut members = self.members.lock().unwrap();
        for (store_key, session_id, address, pid, started_at_ms) in removed {
            if let Some(member) =
                members.get_mut(&Self::member_key(&store_key, &session_id, &address))
            {
                let removed_at_ms = now_ms();
                member.waiters = member.waiters.saturating_sub(1);
                let terminal_recorded = member
                    .last_waiter_exit_at_ms
                    .map(|exit_at| exit_at >= started_at_ms)
                    .unwrap_or(false)
                    && member.last_waiter_pid == Some(pid);
                if !terminal_recorded {
                    member.last_waiter_exit_at_ms = Some(removed_at_ms);
                    member.last_waiter_outcome = Some(WaiterOutcome::AbnormalExit);
                    member.last_waiter_exit_code = None;
                    member.last_waiter_detail =
                        Some("waiter process exited before daemon response".to_string());
                    member.last_waiter_pid = Some(pid);
                }
                if !member.idle {
                    member.unattended_since_ms = Some(removed_at_ms);
                }
            }
        }
    }

    fn add_waiter(&self, mut waiter: WaiterRecord) -> u64 {
        let waiter_id = self.next_waiter_id.fetch_add(1, Ordering::SeqCst);
        waiter.waiter_id = waiter_id;
        let store_key = waiter.store_key.clone();
        let session_id = waiter.session_id.clone();
        let address = waiter.address.clone();
        self.waiters
            .lock()
            .unwrap()
            .insert(Self::waiter_key(waiter_id), waiter);
        if let Some(member) = self.members.lock().unwrap().get_mut(&Self::member_key(
            &store_key,
            &session_id,
            &address,
        )) {
            member.waiters = member.waiters.saturating_add(1);
            member.unattended_since_ms = None;
            member.unattended_with_backlog_since_ms = None;
        }
        waiter_id
    }

    fn remove_waiter(
        &self,
        store_key: &str,
        session_id: &str,
        address: &str,
        waiter_id: u64,
        record_abnormal_if_unreported: bool,
    ) {
        let removed = self
            .waiters
            .lock()
            .unwrap()
            .remove(&Self::waiter_key(waiter_id));
        if let Some(member) = self
            .members
            .lock()
            .unwrap()
            .get_mut(&Self::member_key(store_key, session_id, address))
        {
            member.waiters = member.waiters.saturating_sub(1);
            if let Some(waiter) = &removed {
                let terminal_recorded = member
                    .last_waiter_exit_at_ms
                    .map(|exit_at| exit_at >= waiter.started_at_ms)
                    .unwrap_or(false)
                    && (waiter.pid == 0 || member.last_waiter_pid == Some(waiter.pid));
                if record_abnormal_if_unreported && !member.idle && !terminal_recorded {
                    member.last_waiter_exit_at_ms = Some(now_ms());
                    member.last_waiter_outcome = Some(WaiterOutcome::AbnormalExit);
                    member.last_waiter_exit_code = None;
                    member.last_waiter_detail =
                        Some("waiter ended before daemon-authored terminal response".to_string());
                    member.last_waiter_pid = (waiter.pid != 0).then_some(waiter.pid);
                }
            }
            if record_abnormal_if_unreported && member.waiters == 0 && !member.idle {
                member.unattended_since_ms = Some(now_ms());
            }
        }
    }

    fn record_waiter_exit(
        &self,
        store_key: &str,
        session_id: &str,
        address: &str,
        outcome: WaiterOutcome,
        exit_code: Option<i32>,
        detail: Option<String>,
        pid: Option<u32>,
    ) {
        if let Some(member) = self
            .members
            .lock()
            .unwrap()
            .get_mut(&Self::member_key(store_key, session_id, address))
        {
            member.last_waiter_exit_at_ms = Some(now_ms());
            member.last_waiter_outcome = Some(outcome);
            member.last_waiter_exit_code = exit_code;
            member.last_waiter_detail = detail;
            member.last_waiter_pid = pid;
            if member.waiters == 0 && !member.idle {
                member.unattended_since_ms = Some(now_ms());
            }
        }
    }

    fn record_waiter_message_exit(
        &self,
        store_key: &str,
        session_id: &str,
        address: &str,
        message_id: i64,
        pid: Option<u32>,
    ) {
        if let Some(member) = self
            .members
            .lock()
            .unwrap()
            .get_mut(&Self::member_key(store_key, session_id, address))
        {
            member.last_waiter_exit_at_ms = Some(now_ms());
            member.last_waiter_outcome = Some(WaiterOutcome::Message);
            member.last_waiter_exit_code = Some(0);
            member.last_waiter_detail = None;
            member.last_waiter_pid = pid;
            member.last_delivered_message_id = Some(message_id);
            if member.waiters == 0 && !member.idle {
                member.unattended_since_ms = Some(now_ms());
            }
        }
    }
}

impl MemberRecord {
    fn status(
        &self,
        live_waiters: &[LiveWaiterStatus],
        pending_unconsumed_count: i64,
        inbound_actionable_count: i64,
        push_delivery: PushDeliveryHealth,
        push_suppressed_count: i64,
        deaf_warn_threshold_ms: i64,
    ) -> MemberStatus {
        let member_waiters: Vec<LiveWaiterStatus> = live_waiters
            .iter()
            .filter(|waiter| {
                waiter.store_key == self.store_key
                    && waiter.session_id == self.session_id
                    && waiter.address == self.address
            })
            .cloned()
            .collect();
        let live_waiters_count = member_waiters.len();
        let now = now_ms();
        let (station_health, health_detail) = self.station_health(
            live_waiters_count,
            pending_unconsumed_count,
            inbound_actionable_count,
            push_delivery,
        );
        let unattended_since_ms = if !self.idle && live_waiters_count == 0 {
            self.unattended_since_ms
                .or(self.last_waiter_exit_at_ms)
                .or(Some(now))
        } else {
            None
        };
        let unattended_for_ms = unattended_since_ms.map(|since| now.saturating_sub(since));
        let deaf_since_ms = if station_health == StationHealth::UnattendedWithBacklog {
            self.unattended_with_backlog_since_ms.or(Some(now))
        } else {
            None
        };
        let deaf_for_ms = deaf_since_ms.map(|since| now.saturating_sub(since));
        let deaf_warn = station_health == StationHealth::UnattendedWithBacklog
            && deaf_for_ms
                .map(|age| age >= deaf_warn_threshold_ms)
                .unwrap_or(false);
        MemberStatus {
            store_key: self.store_key.clone(),
            backend: self.backend.clone(),
            session_id: self.session_id.clone(),
            address: self.address.clone(),
            occupant: self.occupant.clone(),
            host: self.host.clone(),
            waiters: self.waiters,
            live_waiters_count,
            pending_unconsumed_count,
            inbound_actionable_count,
            station_health,
            push_delivery,
            push_suppressed_count,
            health_detail,
            last_waiter_exit_at_ms: self.last_waiter_exit_at_ms,
            last_waiter_outcome: self.last_waiter_outcome.clone(),
            last_waiter_exit_code: self.last_waiter_exit_code,
            last_waiter_detail: self.last_waiter_detail.clone(),
            last_waiter_pid: self.last_waiter_pid,
            last_delivered_message_id: self.last_delivered_message_id,
            push_registered: self.on_deliver.is_some(),
            push_wake_on_cc: self.on_deliver_wake_on_cc,
            push_cc_after_ms: self.on_deliver_cc_after_ms,
            // Filled in by the status builder, which has the deferred-attempt map; a bare
            // MemberRecord cannot see it.
            push_deferred_count: 0,
            unattended_since_ms,
            unattended_for_ms,
            deaf_since_ms,
            deaf_for_ms,
            deaf_warn,
            live_waiters: member_waiters,
            watch_pids: self.watch_pids.iter().map(WatchPidRecord::status).collect(),
            description: self.description.clone(),
            scope: self.scope.clone(),
            tags: self.tags.clone(),
            lease_epoch: self.lease_epoch,
            owner_instance_id: self.owner_instance_id.clone(),
            idle: self.idle,
        }
    }

    fn station_health(
        &self,
        live_waiters_count: usize,
        pending_unconsumed_count: i64,
        inbound_actionable_count: i64,
        push_delivery: PushDeliveryHealth,
    ) -> (StationHealth, Option<String>) {
        if self.idle {
            return (
                StationHealth::Idle,
                Some("station is marked idle".to_string()),
            );
        }
        if live_waiters_count > 0 {
            return (StationHealth::Armed, None);
        }
        if self.last_waiter_outcome == Some(WaiterOutcome::Message) {
            if let Some(exit_at) = self.last_waiter_exit_at_ms {
                if now_ms().saturating_sub(exit_at) <= RECENT_DELIVERY_HEALTH_GRACE_MS {
                    return (
                        StationHealth::RecentlyDelivered,
                        Some(format!(
                            "last waiter delivered message {} recently; agent may be handling before re-arm",
                            self.last_delivered_message_id
                                .map(|id| id.to_string())
                                .unwrap_or_else(|| "?".to_string())
                        )),
                    );
                }
            }
        }
        // A registered on-deliver push station has no `telex wait` waiter by design, so waiter
        // presence cannot decide its health. Use the daemon's own push-delivery health instead: it
        // is only "deaf" (unattended-with-backlog) when pushes are actually FAILING (bridge
        // unreachable). A delivering/probing/stale-accepted bridge is attended-via-push, never
        // reported `unattended`. Folds in #64 and the persistent false-deaf of #66.
        if self.on_deliver.is_some() {
            return match push_delivery {
                PushDeliveryHealth::Failing => (
                    StationHealth::UnattendedWithBacklog,
                    Some(format!(
                        "push bridge is not accepting delivery (unreachable); {pending_unconsumed_count} unconsumed, {inbound_actionable_count} awaiting this station's disposition"
                    )),
                ),
                PushDeliveryHealth::StaleAccepted => (
                    StationHealth::AttendedPush,
                    Some(format!(
                        "attended via push bridge (no waiter; expected in push mode); last push accepted but its backstop has elapsed with no fresh accept — bridge may have gone away (probing on next sweep); {inbound_actionable_count} awaiting disposition"
                    )),
                ),
                PushDeliveryHealth::Probing => (
                    StationHealth::AttendedPush,
                    Some(
                        "attended via push bridge (no waiter; expected in push mode); push delivery is being (re)attempted — health not yet confirmed".to_string(),
                    ),
                ),
                PushDeliveryHealth::Delivering | PushDeliveryHealth::NoBacklog => (
                    StationHealth::AttendedPush,
                    Some(format!(
                        "attended via push bridge (no waiter; expected in push mode); {inbound_actionable_count} awaiting this station's disposition"
                    )),
                ),
                // `on_deliver.is_some()` means push is registered, so these are not expected here;
                // match them explicitly so a future `PushDeliveryHealth` variant forces a decision
                // rather than silently inheriting `attended_push`.
                PushDeliveryHealth::NotRegistered | PushDeliveryHealth::Unknown => (
                    StationHealth::AttendedPush,
                    Some(
                        "registered push station; see push_delivery for delivery confidence"
                            .to_string(),
                    ),
                ),
            };
        }
        if pending_unconsumed_count > 0 {
            (
                StationHealth::UnattendedWithBacklog,
                Some(format!(
                    "station has {pending_unconsumed_count} pending unconsumed message(s) and no live waiter"
                )),
            )
        } else {
            (
                StationHealth::Unattended,
                Some("station has no live waiter".to_string()),
            )
        }
    }
}

impl WatchPidRecord {
    fn status(&self) -> WatchPidStatus {
        WatchPidStatus {
            pid: self.pid,
            role: self.role,
            alive: crate::session_watch::process_alive_with_start_time(self.pid, self.start_time),
            start_time: self.start_time,
        }
    }
}

impl WaiterRecord {
    fn status(&self) -> LiveWaiterStatus {
        LiveWaiterStatus {
            waiter_id: self.waiter_id,
            store_key: self.store_key.clone(),
            session_id: self.session_id.clone(),
            address: self.address.clone(),
            pid: self.pid,
            alive: self.pid == 0
                || crate::session_watch::process_alive_with_start_time(self.pid, self.start_time),
            started_at_ms: self.started_at_ms,
            start_time: self.start_time,
            attention: self.attention.clone(),
            min_attention: self.min_attention.clone(),
            wake_on_cc: self.wake_on_cc,
            cc_after_ms: self.cc_after_ms,
            timeout_ms: self.timeout_ms,
        }
    }
}

struct WaiterGuard {
    state: Arc<DaemonState>,
    store_key: String,
    session_id: String,
    address: String,
    waiter_id: u64,
    suppress_abnormal_on_drop: bool,
}

impl WaiterGuard {
    fn new(
        state: Arc<DaemonState>,
        store_key: &str,
        session_id: &str,
        address: &str,
        pid: Option<u32>,
        start_time: Option<u64>,
        attention: Option<String>,
        min_attention: Option<String>,
        wake_on_cc: bool,
        cc_after_ms: Option<i64>,
        timeout_ms: Option<u64>,
    ) -> Self {
        let pid = pid.unwrap_or(0);
        let waiter_id = state.add_waiter(WaiterRecord {
            waiter_id: 0,
            store_key: store_key.to_string(),
            session_id: session_id.to_string(),
            address: address.to_string(),
            pid,
            start_time,
            started_at_ms: now_ms(),
            attention,
            min_attention,
            wake_on_cc,
            cc_after_ms,
            timeout_ms,
        });
        Self {
            state,
            store_key: store_key.to_string(),
            session_id: session_id.to_string(),
            address: address.to_string(),
            waiter_id,
            suppress_abnormal_on_drop: false,
        }
    }

    fn suppress_abnormal_on_drop(&mut self) {
        self.suppress_abnormal_on_drop = true;
    }
}

impl Drop for WaiterGuard {
    fn drop(&mut self) {
        self.state.remove_waiter(
            &self.store_key,
            &self.session_id,
            &self.address,
            self.waiter_id,
            !self.suppress_abnormal_on_drop,
        );
    }
}

pub struct DaemonClient {
    reader: BufReader<tokio::io::ReadHalf<platform::ClientConn>>,
    writer: tokio::io::WriteHalf<platform::ClientConn>,
    pub ack: HelloAck,
    pub paths: DaemonPaths,
}

impl DaemonClient {
    pub async fn request(&mut self, request: &Request) -> Result<Response> {
        write_json_line(&mut self.writer, request).await?;
        let response: Response = read_json_line(&mut self.reader).await?;
        Ok(response)
    }
}

pub fn short_hash(bytes: &[u8]) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{h:016x}")[..12].to_string()
}

pub fn verify_admin_proof(
    expected: &str,
    proof: Option<&str>,
) -> std::result::Result<(), Response> {
    match proof {
        Some(proof) if proof == expected => Ok(()),
        Some(proof) => Err(proto::unauthorized(proto::redact_secrets(
            format!("invalid admin capability proof: {proof}"),
            &[proof, expected],
        ))),
        None => Err(proto::unauthorized("admin capability proof required")),
    }
}

pub fn read_cap_file(path: &Path) -> Result<CapFile> {
    let text = std::fs::read_to_string(path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            DaemonError::NotRunning(format!("capability file {} does not exist", path.display()))
        } else {
            io_err("reading daemon capability file", e)
        }
    })?;
    serde_json::from_str(&text).map_err(DaemonError::Json)
}

pub fn write_cap_file(path: &Path, cap: &CapFile) -> Result<()> {
    let parent = path.parent().ok_or_else(|| {
        DaemonError::Protocol(format!("cap path has no parent: {}", path.display()))
    })?;
    platform::ensure_owner_private_dir(parent)?;
    let json = serde_json::to_vec(cap)?;
    let tmp = sibling_tmp_path(path);
    platform::write_owner_only_file(&tmp, &json)?;
    match std::fs::rename(&tmp, path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            std::fs::remove_file(path)
                .map_err(|e| io_err("replacing daemon capability file", e))?;
            std::fs::rename(&tmp, path).map_err(|e| io_err("installing daemon capability file", e))
        }
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(io_err("installing daemon capability file", e))
        }
    }
}

fn sibling_tmp_path(path: &Path) -> PathBuf {
    let file_name = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("daemon.cap");
    path.with_file_name(format!(
        "{file_name}.{}.{}.tmp",
        std::process::id(),
        monotonic_nonce()
    ))
}

fn monotonic_nonce() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

pub fn local_status_metadata(paths: &DaemonPaths) -> serde_json::Value {
    serde_json::json!({
        "running": false,
        "protocol_version": current_protocol_version(),
        "daemon_version": proto::DAEMON_VERSION,
        "singleton_key": paths.singleton.redacted_material(),
        "singleton_hash": paths.singleton_hash,
        "endpoint": paths.endpoint.display(),
        "cap_path": paths.cap_path.to_string_lossy(),
    })
}

#[derive(Debug, Clone, Serialize)]
pub struct DaemonVersionMetadata {
    pub protocol_version: proto::ProtocolVersion,
    pub daemon_version: &'static str,
    pub auth_policy_version: u16,
    pub required_capabilities: &'static [&'static str],
    pub compatibility: &'static [proto::CompatibilityRow],
}

pub fn daemon_version_metadata() -> DaemonVersionMetadata {
    DaemonVersionMetadata {
        protocol_version: current_protocol_version(),
        daemon_version: proto::DAEMON_VERSION,
        auth_policy_version: proto::AUTH_POLICY_VERSION,
        required_capabilities: proto::REQUIRED_CAPABILITIES,
        compatibility: proto::COMPATIBILITY_TABLE,
    }
}

pub async fn connect_existing(store_key: &str) -> Result<DaemonClient> {
    let paths = DaemonPaths::current()?;
    let cap = read_cap_file(&paths.cap_path)?;
    let (server_pid, server_start_time) = cap_required_peer_identity(&cap)?;
    let conn = platform::connect(&paths.endpoint).await?;
    let expected_exe = canonical_current_exe()?;
    platform::verify_server_peer(
        &conn,
        &expected_exe,
        Some(server_pid),
        Some(server_start_time),
    )?;
    handshake_connected(conn, paths, store_key).await
}

pub async fn connect_or_spawn(store_key: &str) -> Result<DaemonClient> {
    let deadline = Instant::now() + READINESS_TIMEOUT;
    let mut launches: Vec<Instant> = Vec::new();
    let mut backoff = BACKOFF_INITIAL;
    let mut last_err: Option<DaemonError>;
    let existing_probe_deadline = Instant::now() + Duration::from_millis(250);

    loop {
        match tokio::time::timeout(CONNECT_ATTEMPT_TIMEOUT, connect_existing(store_key)).await {
            Ok(Ok(client)) => return Ok(client),
            Ok(Err(e @ (DaemonError::Unauthorized(_) | DaemonError::Incompatible(_)))) => {
                return Err(e)
            }
            Ok(Err(e)) => last_err = Some(e),
            Err(_) => last_err = Some(DaemonError::Timeout("connect attempt timed out".into())),
        }
        if Instant::now() >= existing_probe_deadline {
            break;
        }
        tokio::time::sleep(BACKOFF_INITIAL).await;
    }

    while Instant::now() < deadline {
        launches.retain(|t| t.elapsed() < CRASHLOOP_WINDOW);
        if launches.len() >= CRASHLOOP_MAX {
            return Err(DaemonError::Timeout(format!(
                "daemon failed readiness {CRASHLOOP_MAX} times within {:?}",
                CRASHLOOP_WINDOW
            )));
        }
        launches.push(Instant::now());
        spawn_daemon()?;

        loop {
            if Instant::now() >= deadline {
                break;
            }
            match tokio::time::timeout(CONNECT_ATTEMPT_TIMEOUT, connect_existing(store_key)).await {
                Ok(Ok(client)) => return Ok(client),
                Ok(Err(e)) => last_err = Some(e),
                Err(_) => last_err = Some(DaemonError::Timeout("connect attempt timed out".into())),
            }
            tokio::time::sleep(backoff).await;
            backoff = std::cmp::min(backoff.saturating_mul(2), BACKOFF_MAX);
        }
    }

    Err(DaemonError::Timeout(format!(
        "daemon did not return HelloAck before readiness timeout; last error: {}",
        last_err
            .map(|e| e.to_string())
            .unwrap_or_else(|| "none".to_string())
    )))
}

pub async fn request_connect_or_spawn(store_key: &str, request: &Request) -> Result<Response> {
    let deadline = Instant::now() + READINESS_TIMEOUT;
    loop {
        let mut client = connect_or_spawn(store_key).await?;
        let response = match client.request(request).await {
            Ok(response) => response,
            Err(e) => {
                if Instant::now() >= deadline {
                    return Err(e);
                }
                tokio::time::sleep(BACKOFF_INITIAL).await;
                continue;
            }
        };
        match &response {
            Response::Error { code, message, .. }
                if code == proto::ERROR_NOT_RUNNING && message.contains("draining") =>
            {
                if Instant::now() >= deadline {
                    return Ok(response);
                }
                tokio::time::sleep(BACKOFF_INITIAL).await;
            }
            _ => return Ok(response),
        }
    }
}

fn spawn_daemon() -> Result<()> {
    let exe = canonical_current_exe()?;
    spawn_daemon_process(&exe)
}

#[cfg(not(windows))]
fn spawn_daemon_process(exe: &Path) -> Result<()> {
    let mut command = std::process::Command::new(exe);
    command
        .arg("daemon")
        .arg("serve")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    configure_daemon_spawn(&mut command);
    command
        .spawn()
        .map(|_| ())
        .map_err(|e| io_err("spawning daemon", e))
}

#[cfg(not(windows))]
fn configure_daemon_spawn(_command: &mut std::process::Command) {}

#[cfg(windows)]
fn spawn_daemon_process(exe: &Path) -> Result<()> {
    use std::mem::zeroed;
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Foundation::{CloseHandle, FALSE};
    use windows_sys::Win32::System::Threading::{
        CreateProcessW, PROCESS_INFORMATION, STARTUPINFOW,
    };

    const DETACHED_PROCESS: u32 = 0x0000_0008;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    let command_line = format!("{} daemon serve", quote_windows_arg(&exe.to_string_lossy()));
    let mut command_line_wide: Vec<u16> = std::ffi::OsStr::new(&command_line)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let mut startup: STARTUPINFOW = unsafe { zeroed() };
    startup.cb = std::mem::size_of::<STARTUPINFOW>() as u32;
    let mut process_info: PROCESS_INFORMATION = unsafe { zeroed() };

    // SAFETY: `command_line_wide` is a mutable, null-terminated buffer as required by
    // CreateProcessW. `inherit_handles=FALSE` is the critical bit: daemon auto-spawn must not keep
    // a caller's redirected stdout/stderr pipes or job wait alive after the one-shot client exits.
    let ok = unsafe {
        CreateProcessW(
            std::ptr::null(),
            command_line_wide.as_mut_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            FALSE,
            DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW,
            std::ptr::null(),
            std::ptr::null(),
            &mut startup,
            &mut process_info,
        )
    };
    if ok == 0 {
        return Err(io_err("spawning daemon", std::io::Error::last_os_error()));
    }
    unsafe {
        CloseHandle(process_info.hThread);
        CloseHandle(process_info.hProcess);
    }
    Ok(())
}

#[cfg(windows)]
fn quote_windows_arg(arg: &str) -> String {
    let mut quoted = String::from("\"");
    let mut backslashes = 0;
    for ch in arg.chars() {
        match ch {
            '\\' => backslashes += 1,
            '"' => {
                quoted.push_str(&"\\".repeat(backslashes * 2 + 1));
                quoted.push('"');
                backslashes = 0;
            }
            _ => {
                if backslashes > 0 {
                    quoted.push_str(&"\\".repeat(backslashes));
                    backslashes = 0;
                }
                quoted.push(ch);
            }
        }
    }
    if backslashes > 0 {
        quoted.push_str(&"\\".repeat(backslashes * 2));
    }
    quoted.push('"');
    quoted
}

async fn handshake_connected(
    conn: platform::ClientConn,
    paths: DaemonPaths,
    store_key: &str,
) -> Result<DaemonClient> {
    let hello = proto::client_hello(store_key);
    let (read_half, mut write_half) = tokio::io::split(conn);
    let mut reader = BufReader::new(read_half);
    proto::send_hello_after_verifier(&mut write_half, &hello, || Ok(())).await?;
    let ack: HelloAck = read_json_line(&mut reader).await?;
    if !ack.accepted {
        return Err(DaemonError::Incompatible(
            ack.reason
                .clone()
                .unwrap_or_else(|| "accepted=false".to_string()),
        ));
    }
    Ok(DaemonClient {
        reader,
        writer: write_half,
        ack,
        paths,
    })
}

pub async fn serve() -> Result<()> {
    let paths = DaemonPaths::current()?;
    let mut listener = platform::Listener::bind(&paths.endpoint)?;
    let state = Arc::new(new_state(paths)?);
    let (drain_tx, mut drain_rx) = tokio::sync::mpsc::unbounded_channel::<()>();
    let heartbeat_task = tokio::spawn(heartbeat_loop(state.clone()));

    loop {
        tokio::select! {
            accepted = listener.accept() => {
                let conn = accepted?;
                listener.ready_for_next()?;
                let state = state.clone();
                let drain_tx = drain_tx.clone();
                tokio::spawn(async move {
                    match handle_client(conn, state).await {
                        Ok(ClientAction::Drain) => {
                            let _ = drain_tx.send(());
                        }
                        Ok(ClientAction::Continue) => {}
                        Err(e) => eprintln!("[daemon] client error: {e}"),
                    }
                });
            }
            _ = drain_rx.recv() => break,
        }
    }
    heartbeat_task.abort();
    Ok(())
}

fn new_state(paths: DaemonPaths) -> Result<DaemonState> {
    let instance_id = random_token("inst")?;
    let admin_cap = random_token("cap")?;
    let server_start_time = current_process_start_time_for_cap()?;
    let cap = CapFile {
        instance_id: instance_id.clone(),
        admin_cap: admin_cap.clone(),
        singleton_hash: paths.singleton_hash.clone(),
        protocol_major: paths.singleton.protocol_major,
        server_pid: Some(std::process::id()),
        server_start_time: server_start_time,
    };
    write_cap_file(&paths.cap_path, &cap)?;
    Ok(DaemonState {
        paths,
        instance_id,
        admin_cap,
        stores: Mutex::new(HashMap::new()),
        store_open_guard: AsyncMutex::new(()),
        members: Mutex::new(BTreeMap::new()),
        waiters: Mutex::new(BTreeMap::new()),
        next_waiter_id: AtomicU64::new(1),
        recent_errors: Arc::new(Mutex::new(VecDeque::new())),
        ended_sessions: Mutex::new(BTreeMap::new()),
        draining: AtomicBool::new(false),
        on_deliver: OnDeliverState::default(),
    })
}

fn current_process_start_time_for_cap() -> Result<Option<u64>> {
    let start_time = crate::session_watch::capture_process_start_time(std::process::id());
    if cfg!(any(target_os = "linux", windows)) && start_time.is_none() {
        return Err(DaemonError::Unsupported {
            capability: "daemon cap server_start_time",
            message: "current process start time could not be captured".to_string(),
        });
    }
    Ok(start_time)
}

async fn open_store_entry(
    store_key: &str,
    recent_errors: Arc<Mutex<VecDeque<RecentErrorStatus>>>,
) -> std::result::Result<StoreEntry, Response> {
    #[cfg(not(feature = "postgres"))]
    let _ = &recent_errors;

    if let Some(path) = store_key.strip_prefix("sqlite:") {
        let path = Path::new(path);
        if !path.is_absolute() {
            return Err(proto::unsupported(format!(
                "sqlite store key must contain an absolute path, got {store_key}"
            )));
        }
        #[cfg(feature = "sqlite")]
        {
            let backend = SqliteBackend::open_locked(&path.to_string_lossy())
                .map_err(|e| proto::unsupported(format!("opening SQLite store: {e:#}")))?;
            backend
                .init_schema()
                .await
                .map_err(|e| proto::unsupported(format!("initializing SQLite store: {e:#}")))?;
            return Ok(StoreEntry {
                kind: backend.kind().to_string(),
                backend: Arc::new(backend),
                notify: Arc::new(Notify::new()),
            });
        }
        #[cfg(not(feature = "sqlite"))]
        {
            return Err(proto::unsupported(
                "this telex build does not include the sqlite backend",
            ));
        }
    }

    if store_key.starts_with("postgres:") {
        #[cfg(feature = "postgres")]
        {
            let (profile_name, profile) = resolve_postgres_profile_for_store_key(store_key)?;
            let backend = crate::backend::postgres::PgBackend::connect_profile(profile.clone())
                .await
                .map_err(|e| {
                    proto::unsupported(format!(
                        "opening Postgres backend profile '{profile_name}': {e:#}"
                    ))
                })?;
            backend
                .init_schema()
                .await
                .map_err(|e| proto::unsupported(format!("initializing Postgres store: {e:#}")))?;
            let notify = Arc::new(Notify::new());
            spawn_postgres_notify_listener(store_key.to_string(), notify.clone(), recent_errors);
            return Ok(StoreEntry {
                kind: backend.kind().to_string(),
                backend: Arc::new(backend),
                notify,
            });
        }
        #[cfg(not(feature = "postgres"))]
        {
            return Err(proto::unsupported(
                "this telex build does not include the postgres backend",
            ));
        }
    }

    Err(proto::unsupported(format!(
        "daemon store key must be sqlite:<absolute-path> or postgres:<profile-target>, got {store_key}"
    )))
}

#[cfg(feature = "postgres")]
fn resolve_postgres_profile_for_store_key(
    store_key: &str,
) -> std::result::Result<(String, crate::profiles::BackendProfile), Response> {
    let cfg = crate::profiles::load()
        .map_err(|e| proto::unsupported(format!("loading backend profiles: {e:#}")))?;
    let mut matches = cfg
        .backends
        .into_iter()
        .filter(|(_, profile)| profile.kind == "postgres")
        .filter(|(_, profile)| crate::profiles::store_key(profile, None) == store_key)
        .collect::<Vec<_>>();
    match matches.len() {
        1 => Ok(matches.remove(0)),
        0 => Err(proto::unsupported(format!(
            "no configured Postgres backend profile matches store key {store_key}; add the profile on this host before attaching"
        ))),
        _ => {
            let names = matches
                .iter()
                .map(|(name, _)| name.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            Err(proto::unsupported(format!(
                "ambiguous Postgres backend profiles for store key {store_key}: {names}; refusing to choose one"
            )))
        }
    }
}

#[cfg(feature = "postgres")]
fn spawn_postgres_notify_listener(
    store_key: String,
    notify: Arc<Notify>,
    recent_errors: Arc<Mutex<VecDeque<RecentErrorStatus>>>,
) {
    tokio::spawn(async move {
        let mut backoff = BACKOFF_INITIAL;
        loop {
            let result = async {
                let (_, profile) = resolve_postgres_profile_for_store_key(&store_key)
                    .map_err(|response| anyhow::anyhow!("{response:?}"))?;
                let (cfg, schema) = crate::profiles::pg_connect_config(&profile).await?;
                run_postgres_notify_listener(&cfg, schema.as_deref(), notify.clone()).await
            }
            .await;
            match result {
                Ok(()) => {
                    push_recent_error_to_queue(
                        &recent_errors,
                        "NotifyDegraded",
                        format!("postgres LISTEN loop ended for {store_key}; reconnecting"),
                        &[],
                    );
                }
                Err(e) => {
                    push_recent_error_to_queue(
                        &recent_errors,
                        "NotifyDegraded",
                        format!("postgres LISTEN loop failed for {store_key}: {e:#}; reconnecting"),
                        &[],
                    );
                }
            }
            tokio::time::sleep(backoff).await;
            backoff = (backoff * 2).min(BACKOFF_MAX);
        }
    });
}

#[cfg(feature = "postgres")]
enum PgListenEvent {
    Message(AsyncMessage),
    Error(tokio_postgres::Error),
    Closed,
}

fn push_recent_error_to_queue(
    recent_errors: &Arc<Mutex<VecDeque<RecentErrorStatus>>>,
    kind: impl Into<String>,
    message: impl Into<String>,
    redactions: &[&str],
) {
    let mut errors = recent_errors.lock().unwrap();
    let message = proto::redact_secrets(message.into(), redactions);
    errors.push_back(RecentErrorStatus {
        at_ms: now_ms(),
        kind: kind.into(),
        message,
    });
    while errors.len() > RECENT_ERROR_LIMIT {
        errors.pop_front();
    }
}

#[cfg(feature = "postgres")]
async fn run_postgres_notify_listener(
    cfg: &tokio_postgres::Config,
    schema: Option<&str>,
    notify: Arc<Notify>,
) -> anyhow::Result<()> {
    let (client, mut connection) = cfg
        .connect(make_postgres_tls()?)
        .await
        .context("connecting postgres LISTEN client")?;
    let (tx, mut rx) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        loop {
            let event = match std::future::poll_fn(|cx| connection.poll_message(cx)).await {
                Some(Ok(message)) => PgListenEvent::Message(message),
                Some(Err(e)) => PgListenEvent::Error(e),
                None => PgListenEvent::Closed,
            };
            let terminal = matches!(event, PgListenEvent::Error(_) | PgListenEvent::Closed);
            if tx.send(event).is_err() || terminal {
                break;
            }
        }
    });

    client
        .batch_execute("SET SESSION CHARACTERISTICS AS TRANSACTION ISOLATION LEVEL READ COMMITTED")
        .await
        .context("pinning LISTEN client READ COMMITTED isolation")?;
    if let Some(schema) = schema {
        let schema = sanitize_ident(schema)?;
        client
            .batch_execute(&format!("SET search_path TO {schema}, public;"))
            .await
            .context("setting LISTEN client search_path")?;
    }
    let notify_channel = notify_channel_for_schema(schema)?;
    client
        .batch_execute(&format!("LISTEN {notify_channel};"))
        .await
        .context("subscribing postgres LISTEN channel")?;
    loop {
        match rx.recv().await {
            Some(PgListenEvent::Message(AsyncMessage::Notification(notification)))
                if notification.channel() == notify_channel =>
            {
                notify.notify_waiters();
            }
            Some(PgListenEvent::Message(_)) => {}
            Some(PgListenEvent::Error(e)) => return Err(e.into()),
            Some(PgListenEvent::Closed) | None => return Ok(()),
        }
    }
}

enum ClientAction {
    Continue,
    Drain,
}

/// Max concurrent on-deliver handler processes across the daemon.
const ON_DELIVER_MAX_CONCURRENCY: usize = 8;
/// Max messages a single per-member sweep will (re)push per tick, so a large backlog cannot
/// starve fresh commit-time pushes; the remainder is delivered on subsequent sweeps.
const ON_DELIVER_SWEEP_BATCH: usize = 64;
/// Wall-clock budget for a single on-deliver handler process.
const ON_DELIVER_TIMEOUT: Duration = Duration::from_secs(30);
/// Base cooldown before re-pushing a still-undelivered message whose last push **failed** (bridge
/// unreachable); it doubles per attempt up to `ON_DELIVER_RETRY_MAX`, so a transiently dead bridge
/// recovers quickly without hammering. A push the harness **accepted** instead waits on the much
/// longer `ON_DELIVER_ACCEPTED_BACKSTOP`, because an accepted turn is already queued in the
/// session; its real re-delivery trigger is re-provision (reattach/reload), which clears the push
/// record via `on_deliver_forget_member` and re-delivers the un-acked backlog. "Pushed" is still an
/// attempt record, not terminal suppression -- a crash/reload after accept-but-before-ack
/// re-delivers on re-provision, and the backstop covers a silent in-session drop -- so a message is
/// never stranded.
const ON_DELIVER_RETRY_BASE: Duration = Duration::from_secs(15);
/// Ceiling for the per-message re-push backoff (also the steady-state retry interval).
const ON_DELIVER_RETRY_MAX: Duration = Duration::from_secs(300);
/// Re-push interval for a still-unacked message whose last push was **accepted**. An accepted turn
/// sits in the live session's queue (or was seen but not yet acked, which the agent-stop turn-guard
/// nudges), so re-pushing it on the fast failure backoff would just inject duplicate turns. Its
/// real re-delivery trigger is a re-provision (reattach or bridge reload -> re-deliver the un-acked
/// backlog); this long backstop only guards the rare case where a continuously-held session
/// silently drops the queued turn without any reload/reattach.
const ON_DELIVER_ACCEPTED_BACKSTOP: Duration = Duration::from_secs(300);
/// After this many attempts on the same still-unacked message, surface a degraded status.
const ON_DELIVER_DEGRADED_AFTER: u32 = 6;
/// Re-attempt cooldown for a message the harness **deferred** (busy -- issue #65). The idle drain
/// is the prompt re-delivery trigger; this backstop only bounds the latency if the drain signal is
/// missed (hook did not fire / raced), so a busy bridge is not re-hit every heartbeat while the
/// turn runs. Invariant: `HEARTBEAT_INTERVAL <= ON_DELIVER_DEFERRED_BACKSTOP <
/// ON_DELIVER_ACCEPTED_BACKSTOP` (fallback re-attempt is bounded, but deferred is re-checked sooner
/// than a genuinely-queued accepted turn). Enforced by `on_deliver_backstop_invariants` in tests.
/// The permanent (dead-letter) and deferred exit codes are defined in `daemon_ipc` as the single
/// source of truth for the handler<->daemon contract.
const ON_DELIVER_DEFERRED_BACKSTOP: Duration = Duration::from_secs(30);
/// Hard cap on total push attempts for one still-unacked message. Past this, re-push is suppressed
/// (the message stays durably queued and readable via `telex inbox`; surfaced as a suppressed
/// count in status) so a never-acked message cannot be re-pushed forever. An explicit re-provision
/// (reattach/reload -> `on_deliver_forget_member`) resets the budget and re-delivers the backlog.
/// With the 300s accepted backstop this is ~2h of a live-but-never-acking session before suppression.
const ON_DELIVER_MAX_REPUSH: u32 = 24;

/// Backoff before re-pushing a still-undelivered message that has already been attempted
/// `attempts` times: `ON_DELIVER_RETRY_BASE` doubling per attempt, capped at
/// `ON_DELIVER_RETRY_MAX`.
fn on_deliver_backoff(attempts: u32) -> Duration {
    let steps = attempts.saturating_sub(1).min(5);
    ON_DELIVER_RETRY_BASE
        .checked_mul(1u32 << steps)
        .unwrap_or(ON_DELIVER_RETRY_MAX)
        .min(ON_DELIVER_RETRY_MAX)
}

/// The delay before a still-unacked message is eligible for re-push. A **failed** push (bridge
/// unreachable) uses the fast, growing `on_deliver_backoff` so a transiently dead bridge recovers
/// quickly. An **accepted** push (already queued in the live session) uses the long
/// `ON_DELIVER_ACCEPTED_BACKSTOP`: re-delivery of an accepted message is normally driven by
/// re-provision (reattach/reload clears the push record and re-delivers the backlog), so this
/// timer only needs to backstop the rare accept-but-silently-dropped-while-held case.
fn on_deliver_redelivery_delay(attempt: &PushAttempt) -> Duration {
    if attempt.deferred {
        ON_DELIVER_DEFERRED_BACKSTOP
    } else if attempt.accepted {
        ON_DELIVER_ACCEPTED_BACKSTOP
    } else {
        on_deliver_backoff(attempt.attempts)
    }
}

/// Identifies one in-flight on-deliver exec so concurrent commit + sweep paths do not
/// double-spawn a handler for the same (address, message).
#[derive(Clone, PartialEq, Eq, Hash)]
struct OnDeliverKey {
    store_key: String,
    address: String,
    message_id: i64,
}

/// One still-unacked message's push bookkeeping: when it was last attempted, how many times,
/// and whether the last attempt was **accepted** by the harness (`session.send` returned ok) vs
/// **failed** (bridge unreachable). Accepted and failed pushes back off very differently: a failed
/// push retries fast (`on_deliver_backoff`) to recover a transiently dead bridge, while an accepted
/// push is already in the live session's queue and is only re-pushed on a long backstop
/// (`ON_DELIVER_ACCEPTED_BACKSTOP`) -- its real re-delivery trigger is re-provision
/// (reattach/reload -> `on_deliver_forget_member`), not this timer.
#[derive(Clone, Copy)]
struct PushAttempt {
    last: Instant,
    attempts: u32,
    accepted: bool,
    /// The harness deferred this push because it was busy (issue #65). Mutually exclusive with
    /// `accepted`: a deferred message was not sent, so it is neither queued-in-session nor a
    /// failure. It uses `ON_DELIVER_DEFERRED_BACKSTOP` and is cleared by the idle drain.
    deferred: bool,
    notification_only: bool,
    notification_lower_bound: Option<i64>,
    /// Once this message has been accepted, it needs no further delivery: it is a CC notification
    /// or a primary message that does not require this recipient's disposition (an informational
    /// note). Such a message is skipped forever after a single accepted push, exactly like an
    /// accepted CC notification, so no-disposition traffic never enters an unbounded re-push pool.
    skip_after_accept: bool,
}

#[derive(Clone, Copy)]
struct InflightAttempt {
    notification_only: bool,
    notification_lower_bound: Option<i64>,
}

#[derive(Clone)]
struct OnDeliverCandidate {
    member_key: MemberKey,
    argv: Vec<String>,
    address: String,
    notification_only: bool,
}

/// Daemon-side liveness state for the generic on-deliver exec primitive. This is a
/// best-effort push notifier: it never marks messages delivered or consumed (that stays
/// agent-driven via `Ack`), so a failed or missing push only leaves the message durably
/// queued, exactly like an unarmed pull station. `pushed` records the last attempt per
/// still-undelivered `(member, message_id)` so re-pushes back off (fast after a failed push, a
/// long backstop after an accepted one) while the message stays unacked, and is pruned to the
/// currently-undelivered set on each sweep; `dead_lettered`
/// holds messages a handler reported as permanently unpushable (skipped from further pushes,
/// surfaced via a degraded status, still durably queued); `inflight` prevents a commit-path
/// and a sweep-path from racing the same message. `generations` fences lifecycle resets: each
/// member has a generation that is bumped on re-provision (`on_deliver_forget_member`), captured
/// when a push is spawned, and re-checked on completion, so an in-flight push launched before a
/// reset cannot write its stale outcome into the fresh generation's attempt map.
struct OnDeliverState {
    sem: Arc<Semaphore>,
    inflight: Mutex<HashMap<OnDeliverKey, InflightAttempt>>,
    pushed: Mutex<HashMap<MemberKey, HashMap<i64, PushAttempt>>>,
    dead_lettered: Mutex<HashMap<MemberKey, BTreeSet<i64>>>,
    /// Per-member idle-drain generation (issue #65). Bumped each time `DrainDeferred` runs for a
    /// member. A push captures the generation when it begins; if the generation advances while the
    /// push is inflight, a drain fired before the deferred attempt was recorded and could not clear
    /// or re-sweep it (nothing was recorded yet, and the inflight guard blocked the drain's sweep).
    /// The push then clears + re-sweeps itself so the message is re-attempted promptly instead of
    /// waiting for the deferred backstop. Distinct from `generations` (which fences lifecycle resets).
    drain_gen: Mutex<HashMap<MemberKey, u64>>,
    generations: Mutex<HashMap<MemberKey, u64>>,
}

impl Default for OnDeliverState {
    fn default() -> Self {
        Self {
            sem: Arc::new(Semaphore::new(ON_DELIVER_MAX_CONCURRENCY)),
            inflight: Mutex::new(HashMap::new()),
            pushed: Mutex::new(HashMap::new()),
            dead_lettered: Mutex::new(HashMap::new()),
            drain_gen: Mutex::new(HashMap::new()),
            generations: Mutex::new(HashMap::new()),
        }
    }
}

impl DaemonState {
    /// Non-idle members attending `address` that registered an on-deliver handler.
    fn on_deliver_candidates(&self, store_key: &str, address: &str) -> Vec<OnDeliverCandidate> {
        self.members
            .lock()
            .unwrap()
            .iter()
            .filter(|(_k, m)| {
                m.store_key == store_key
                    && m.address == address
                    && !m.idle
                    && m.on_deliver.is_some()
            })
            .map(|(k, m)| OnDeliverCandidate {
                member_key: k.clone(),
                argv: m.on_deliver.clone().unwrap_or_default(),
                address: m.address.clone(),
                notification_only: false,
            })
            .collect()
    }

    fn on_deliver_cc_candidates(
        &self,
        store_key: &str,
        row: &MessageRow,
    ) -> Vec<OnDeliverCandidate> {
        self.members
            .lock()
            .unwrap()
            .iter()
            .filter(|(_k, m)| {
                m.store_key == store_key
                    && !m.idle
                    && m.on_deliver.is_some()
                    && m.on_deliver_wake_on_cc
                    && m.on_deliver_cc_after_ms
                        .is_some_and(|lower| row.created_at_ms >= lower)
                    && delivery_role(&m.address, &row.to_addr, row.cc.as_deref()) == "cc"
            })
            .map(|(k, m)| OnDeliverCandidate {
                member_key: k.clone(),
                argv: m.on_deliver.clone().unwrap_or_default(),
                address: m.address.clone(),
                notification_only: true,
            })
            .collect()
    }

    /// Whether a re-push of `(member, id)` should be skipped right now: true if the message was
    /// dead-lettered (permanent failure), has hit the `ON_DELIVER_MAX_REPUSH` attempt cap
    /// (suppressed), was accepted and needs no further delivery (`skip_after_accept`), or while its
    /// last attempt is still inside its re-delivery delay (`on_deliver_redelivery_delay`: fast
    /// backoff after a failed push, long backstop after an accepted one). A never-attempted or
    /// delay-elapsed message under the cap is eligible.
    fn on_deliver_should_skip(&self, member: &MemberKey, id: i64, now: Instant) -> bool {
        if self
            .on_deliver
            .dead_lettered
            .lock()
            .unwrap()
            .get(member)
            .is_some_and(|s| s.contains(&id))
        {
            return true;
        }
        self.on_deliver
            .pushed
            .lock()
            .unwrap()
            .get(member)
            .and_then(|m| m.get(&id))
            .is_some_and(|a| {
                a.attempts >= ON_DELIVER_MAX_REPUSH
                    || (a.accepted && (a.notification_only || a.skip_after_accept))
                    || now.saturating_duration_since(a.last) < on_deliver_redelivery_delay(a)
            })
    }

    /// Record one push attempt for `(member, id)` -- its outcome (accepted / deferred-busy / failed)
    /// and time -- and return the current attempt count. The outcome selects the re-delivery delay:
    /// a failed push retries fast to recover a dead bridge; an accepted push waits on the long
    /// backstop (re-delivery is otherwise re-provision-driven); a **deferred** push waits on the
    /// deferred backstop and is cleared promptly by the idle drain. A deferred attempt does **not**
    /// increment `attempts`, so it never inflates the failed-backoff or trips the degraded-status
    /// threshold -- deferring while a long turn runs is normal, not degradation (issue #65).
    fn on_deliver_record_attempt(
        &self,
        member: &MemberKey,
        id: i64,
        now: Instant,
        accepted: bool,
        deferred: bool,
        notification_only: bool,
        notification_lower_bound: Option<i64>,
        skip_after_accept: bool,
    ) -> u32 {
        debug_assert!(
            !(accepted && deferred),
            "a push attempt cannot be both accepted and deferred; they are mutually exclusive outcomes"
        );
        let mut map = self.on_deliver.pushed.lock().unwrap();
        let entry = map
            .entry(member.clone())
            .or_default()
            .entry(id)
            .or_insert(PushAttempt {
                last: now,
                attempts: 0,
                accepted: false,
                deferred: false,
                notification_only,
                notification_lower_bound,
                skip_after_accept,
            });
        if !deferred {
            entry.attempts = entry.attempts.saturating_add(1);
        }
        entry.last = now;
        entry.accepted = accepted;
        entry.deferred = deferred;
        entry.notification_only = notification_only;
        entry.notification_lower_bound = notification_lower_bound;
        entry.skip_after_accept = skip_after_accept;
        entry.attempts
    }

    /// Clear the deferred-until-idle skip for a member's messages so the next sweep re-attempts them
    /// (issue #65 idle drain). Only **deferred** attempts are removed; accepted attempts (genuinely
    /// queued turns) are left untouched so the drain never re-injects a duplicate of a queued turn.
    /// Returns the number of deferred entries cleared -- 0 is the common no-work fast path.
    fn on_deliver_clear_deferred(&self, member: &MemberKey) -> usize {
        let mut map = self.on_deliver.pushed.lock().unwrap();
        let Some(attempts) = map.get_mut(member) else {
            return 0;
        };
        let before = attempts.len();
        attempts.retain(|_id, a| !a.deferred);
        let cleared = before - attempts.len();
        if attempts.is_empty() {
            map.remove(member);
        }
        cleared
    }

    /// Count of a member's messages currently deferred-until-idle (for `telex status` diagnosis).
    fn on_deliver_deferred_count(&self, member: &MemberKey) -> i64 {
        self.on_deliver
            .pushed
            .lock()
            .unwrap()
            .get(member)
            .map(|attempts| attempts.values().filter(|a| a.deferred).count() as i64)
            .unwrap_or(0)
    }

    /// Current idle-drain generation for a member (0 if never drained). A push captures this at
    /// start; if it advances before the push records its outcome, a drain raced the inflight push.
    fn on_deliver_drain_gen(&self, member: &MemberKey) -> u64 {
        self.on_deliver
            .drain_gen
            .lock()
            .unwrap()
            .get(member)
            .copied()
            .unwrap_or(0)
    }

    /// Advance a member's idle-drain generation (called by `DrainDeferred` per member).
    fn on_deliver_bump_drain_gen(&self, member: &MemberKey) {
        let mut map = self.on_deliver.drain_gen.lock().unwrap();
        let entry = map.entry(member.clone()).or_insert(0);
        *entry = entry.saturating_add(1);
    }

    /// Push-delivery health for a member with a registered on-deliver handler, derived only from
    /// the daemon's own push-attempt outcomes (harness-neutral: never reads the bridge registry).
    /// `pending` is the member's undelivered/unconsumed count. See `PushDeliveryHealth`.
    fn push_delivery_health(
        &self,
        member: &MemberKey,
        pending: i64,
        on_deliver_registered: bool,
        now: Instant,
    ) -> PushDeliveryHealth {
        if !on_deliver_registered {
            return PushDeliveryHealth::NotRegistered;
        }
        if pending <= 0 {
            return PushDeliveryHealth::NoBacklog;
        }
        let pushed = self.on_deliver.pushed.lock().unwrap();
        let member_attempts = match pushed.get(member) {
            Some(attempts) if !attempts.is_empty() => attempts,
            // Backlog exists but nothing attempted yet (e.g. just after a daemon restart, before
            // the next sweep, or between commit and first push). Not confidently attended, not deaf.
            _ => return PushDeliveryHealth::Probing,
        };
        // Consider only push-relevant attempts: a message that was accepted and needs no further
        // delivery (`skip_after_accept`: a CC notification or a no-disposition note) is done, not
        // current work, so it must not keep the station looking `stale_accepted`/`delivering` once
        // its backstop elapses. If the only attempts are such completed ones, there is no live push
        // work outstanding -> attended, no backlog.
        let mut relevant = member_attempts
            .values()
            .filter(|a| !(a.accepted && a.skip_after_accept))
            .peekable();
        if relevant.peek().is_none() {
            return PushDeliveryHealth::NoBacklog;
        }
        // Classify by the FRESHEST relevant attempt, not "any accept in the window": otherwise a
        // stale accept on message A (still inside its 300s backstop) would mask a fresh failure on
        // message B, reporting `delivering` while the bridge is actually unreachable and delaying
        // deaf detection by up to a backstop. Ties on `last` are broken toward a failure
        // (`!accepted` sorts high), so equal-timestamp completions never flip health nondeterministically
        // from the unordered attempt map.
        let freshest = relevant.max_by_key(|a| (a.last, !a.accepted));
        match freshest {
            Some(attempt) if attempt.accepted => {
                if now.saturating_duration_since(attempt.last) < ON_DELIVER_ACCEPTED_BACKSTOP {
                    PushDeliveryHealth::Delivering
                } else {
                    PushDeliveryHealth::StaleAccepted
                }
            }
            Some(_) => PushDeliveryHealth::Failing,
            None => PushDeliveryHealth::Probing,
        }
    }

    /// Count of a member's on-deliver messages whose re-push is currently suppressed: dead-lettered
    /// (permanently unpushable) plus those that have hit the `ON_DELIVER_MAX_REPUSH` attempt cap.
    /// They stay durably queued/readable; this is the persistent operator-visible signal. Counts by
    /// distinct message id so a message that is both dead-lettered and capped is not double-counted.
    fn push_suppressed_count(&self, member: &MemberKey) -> i64 {
        let mut suppressed: BTreeSet<i64> = self
            .on_deliver
            .dead_lettered
            .lock()
            .unwrap()
            .get(member)
            .cloned()
            .unwrap_or_default();
        if let Some(attempts) = self.on_deliver.pushed.lock().unwrap().get(member) {
            for (id, attempt) in attempts {
                if attempt.attempts >= ON_DELIVER_MAX_REPUSH {
                    suppressed.insert(*id);
                }
            }
        }
        suppressed.len() as i64
    }

    /// Prune a member's push-attempt and dead-letter records to only ids still in `keep` (the
    /// currently-undelivered set), so both maps stay bounded as messages are acked/consumed.
    fn on_deliver_retain_pushed(&self, member: &MemberKey, keep: &BTreeSet<i64>) {
        {
            let mut map = self.on_deliver.pushed.lock().unwrap();
            if let Some(attempts) = map.get_mut(member) {
                attempts.retain(|id, _| keep.contains(id));
                if attempts.is_empty() {
                    map.remove(member);
                }
            }
        }
        let mut dead = self.on_deliver.dead_lettered.lock().unwrap();
        if let Some(set) = dead.get_mut(member) {
            set.retain(|id| keep.contains(id));
            if set.is_empty() {
                dead.remove(member);
            }
        }
    }

    fn on_deliver_try_begin(
        &self,
        key: OnDeliverKey,
        notification_only: bool,
        notification_lower_bound: Option<i64>,
    ) -> bool {
        let mut inflight = self.on_deliver.inflight.lock().unwrap();
        if inflight.contains_key(&key) {
            return false;
        }
        inflight.insert(
            key,
            InflightAttempt {
                notification_only,
                notification_lower_bound,
            },
        );
        true
    }

    fn on_deliver_end(&self, key: &OnDeliverKey) {
        self.on_deliver.inflight.lock().unwrap().remove(key);
    }

    /// Current push generation for a member (0 if never reset). Captured when a push is spawned so
    /// a completion from before a re-provision reset can be detected and discarded.
    fn on_deliver_generation(&self, member: &MemberKey) -> u64 {
        self.on_deliver
            .generations
            .lock()
            .unwrap()
            .get(member)
            .copied()
            .unwrap_or(0)
    }

    /// Drop a member's push dedup state so a later re-bind re-pushes still-undelivered messages
    /// (lifecycle-scoped dedup). Called on member removal and on (re-)register. Bumps the member's
    /// push generation so any in-flight push spawned before this reset is fenced: its later
    /// completion is ignored instead of writing stale outcome/backoff into the fresh attempt map.
    fn on_deliver_forget_member(&self, member: &MemberKey) {
        self.on_deliver.pushed.lock().unwrap().remove(member);
        self.on_deliver.dead_lettered.lock().unwrap().remove(member);
        self.on_deliver.drain_gen.lock().unwrap().remove(member);
        let mut generations = self.on_deliver.generations.lock().unwrap();
        let generation = generations.entry(member.clone()).or_insert(0);
        *generation = generation.wrapping_add(1);
    }

    fn on_deliver_advance_cc_lower_bound(&self, member_key: &MemberKey, lower_bound: i64) {
        let safe_lower_bound = {
            let pushed = self.on_deliver.pushed.lock().unwrap();
            let earliest_unaccepted = pushed.get(member_key).and_then(|attempts| {
                attempts
                    .values()
                    .filter_map(|attempt| {
                        (attempt.notification_only && !attempt.accepted)
                            .then_some(attempt.notification_lower_bound.unwrap_or(lower_bound))
                    })
                    .min()
            });
            let earliest_inflight = self
                .on_deliver
                .inflight
                .lock()
                .unwrap()
                .iter()
                .filter_map(|(key, attempt)| {
                    (key.store_key == member_key.store_key
                        && key.address == member_key.address
                        && attempt.notification_only)
                        .then_some(attempt.notification_lower_bound.unwrap_or(lower_bound))
                })
                .min();
            let earliest_blocking = earliest_unaccepted
                .into_iter()
                .chain(earliest_inflight)
                .min();
            let highest_accepted = pushed.get(member_key).and_then(|attempts| {
                attempts
                    .values()
                    .filter_map(|attempt| {
                        if !attempt.notification_only || !attempt.accepted {
                            return None;
                        }
                        let ts = attempt.notification_lower_bound.unwrap_or(lower_bound);
                        if earliest_blocking.map_or(true, |blocking| ts < blocking) {
                            Some(ts)
                        } else {
                            None
                        }
                    })
                    .max()
            });
            let candidate = highest_accepted.unwrap_or(lower_bound);
            earliest_blocking
                .and_then(|id| id.checked_sub(1))
                .map_or(candidate, |ceiling| candidate.min(ceiling))
        };
        if let Some(member) = self.members.lock().unwrap().get_mut(member_key) {
            if member.on_deliver_wake_on_cc {
                member.on_deliver_cc_after_ms = Some(
                    member
                        .on_deliver_cc_after_ms
                        .map_or(safe_lower_bound, |current| current.max(safe_lower_bound)),
                );
            }
        }
    }

    /// Mark `(member, id)` permanently unpushable (the handler reported a non-retryable failure,
    /// e.g. a message too large for the harness frame). It is skipped from further pushes and
    /// pruned once the message leaves the undelivered set; it is never marked delivered/consumed,
    /// so it stays durably queued and readable via `telex inbox`.
    fn on_deliver_dead_letter(&self, member: &MemberKey, id: i64) {
        self.on_deliver
            .dead_lettered
            .lock()
            .unwrap()
            .entry(member.clone())
            .or_default()
            .insert(id);
    }

    /// Fast-path push on durable commit: fire the handler for the just-committed primary
    /// recipient, plus opted-in live CC observer recipients whose lower bound admits this message.
    fn fire_on_deliver_on_commit(self: &Arc<Self>, store_key: &str, row: &MessageRow) {
        for candidate in self
            .on_deliver_candidates(store_key, &row.to_addr)
            .into_iter()
            .chain(self.on_deliver_cc_candidates(store_key, row))
        {
            self.spawn_on_deliver(
                candidate.member_key,
                candidate.argv,
                store_key.to_string(),
                candidate.address,
                row.clone(),
                candidate.notification_only,
            );
        }
    }

    /// Spawn one on-deliver handler exec for a (member, message), rate-limited by the per-message
    /// backoff and the in-flight guard. Never blocks the caller.
    fn spawn_on_deliver(
        self: &Arc<Self>,
        member_key: MemberKey,
        argv: Vec<String>,
        store_key: String,
        address: String,
        row: MessageRow,
        notification_only: bool,
    ) {
        if argv.is_empty() {
            return;
        }
        let id = row.id;
        if self.on_deliver_should_skip(&member_key, id, Instant::now()) {
            return;
        }
        let notification_lower_bound = notification_only.then_some(row.created_at_ms);
        // A CC notification, or a primary message that does not require this recipient's
        // disposition, needs no re-delivery once accepted: one push is enough. This keeps
        // no-disposition traffic out of the unbounded re-push pool (mirrors the accepted-CC rule).
        let skip_after_accept = notification_only
            || !requires_disposition_for_recipient(
                row.requires_disposition,
                &address,
                &row.to_addr,
            );
        let key = OnDeliverKey {
            store_key: store_key.clone(),
            address: address.clone(),
            message_id: id,
        };
        if !self.on_deliver_try_begin(key.clone(), notification_only, notification_lower_bound) {
            return;
        }
        // Capture the member's drain generation before spawning. If a `DrainDeferred` runs while
        // this push is inflight, the generation advances; the deferred outcome below detects that
        // and self-re-sweeps, since the drain could neither clear (nothing recorded yet) nor sweep
        // (the inflight guard blocked it) this message.
        let drain_gen_at_start = self.on_deliver_drain_gen(&member_key);
        let descriptor = on_deliver_descriptor_json(&store_key, &address, &row);
        let sem = self.on_deliver.sem.clone();
        let state = self.clone();
        // Capture the member's push generation at spawn. If a re-provision resets the member
        // (bumping the generation) while this push is in flight, its completion is discarded below
        // rather than writing stale outcome/backoff into the fresh generation's attempt map.
        let spawn_generation = self.on_deliver_generation(&member_key);
        tokio::spawn(async move {
            let (outcome, stderr) = run_on_deliver(sem, argv, descriptor).await;
            if state.on_deliver_generation(&member_key) != spawn_generation {
                // A re-provision reset this member's push state after we started; discard this
                // stale completion (free the in-flight slot so the fresh generation re-pushes).
                state.on_deliver_end(&key);
                return;
            }
            if outcome == RunOutcome::Permanent {
                // Dead-letter: stop retrying a structurally unpushable message. It stays durably
                // queued (never marked consumed) and is readable via `telex inbox`.
                state.on_deliver_dead_letter(&member_key, id);
                state.on_deliver_end(&key);
                let detail = stderr.map(|s| format!(": {s}")).unwrap_or_default();
                state.push_recent_error(
                    "OnDeliverDeadLettered",
                    format!(
                        "on-deliver permanently failed (not retried) store={store_key} address={address} message_id={id}{detail}; message stays durable, read it via `telex inbox`"
                    ),
                );
            } else {
                // Record the attempt with its outcome so the next re-push uses the right delay
                // (accepted -> long backstop; deferred-busy -> deferred backstop, cleared by the
                // idle drain; failed -> fast backoff). The message leaves the attempt map only once
                // it is acked (retain sweep).
                let deferred = outcome == RunOutcome::Deferred;
                let attempts = state.on_deliver_record_attempt(
                    &member_key,
                    id,
                    Instant::now(),
                    outcome == RunOutcome::Ok,
                    deferred,
                    notification_only,
                    notification_lower_bound,
                    skip_after_accept,
                );
                state.on_deliver_end(&key);
                if outcome == RunOutcome::Ok {
                    if let Some(lower_bound) = notification_lower_bound {
                        state.on_deliver_advance_cc_lower_bound(&member_key, lower_bound);
                    }
                }
                if outcome == RunOutcome::Transient {
                    let detail = stderr.map(|s| format!(": {s}")).unwrap_or_default();
                    state.push_recent_error(
                        "OnDeliverFailed",
                        format!(
                            "on-deliver handler failed store={store_key} address={address} message_id={id}{detail}"
                        ),
                    );
                }
                // A deferred push is normal scheduling, not degradation: it did not increment
                // `attempts`, so it cannot trip the degraded threshold or spam recent errors.
                if !deferred && attempts == ON_DELIVER_DEGRADED_AFTER {
                    state.push_recent_error(
                        "OnDeliverDegraded",
                        format!(
                            "on-deliver still unacked after {attempts} attempts store={store_key} address={address} message_id={id}; the bridge may be unloaded/unreachable or the agent has not acked"
                        ),
                    );
                }
                // A never-acked message must not be re-pushed forever. A deferred outcome does not
                // increment `attempts`, so only real (accepted/failed) attempts can hit the cap.
                if !deferred && attempts == ON_DELIVER_MAX_REPUSH {
                    // Hard cap reached: suppress further re-push (the message stays durable/readable
                    // and is surfaced as a suppressed count in status). A re-provision resets it.
                    state.push_recent_error(
                        "OnDeliverSuppressed",
                        format!(
                            "on-deliver re-push suppressed after {attempts} attempts store={store_key} address={address} message_id={id}; it stays durable/readable via `telex inbox` and re-delivers on reattach/reload"
                        ),
                    );
                }
                // Inflight/drain race (issue #65): if a `DrainDeferred` ran while this push was
                // inflight, it saw no deferred entry (recorded just now) and its sweep hit the
                // inflight guard, so the drain missed this message. Now that the push has ended and
                // the deferred skip is set, self-re-sweep so the message is re-attempted promptly
                // instead of waiting out `ON_DELIVER_DEFERRED_BACKSTOP`. A subsequent re-defer
                // records no new drain, so this fires at most once per drain (no busy-loop).
                if deferred && state.on_deliver_drain_gen(&member_key) != drain_gen_at_start {
                    state.on_deliver_clear_deferred(&member_key);
                    if let Some(member) = state.get_member(
                        &member_key.store_key,
                        &member_key.session_id,
                        &member_key.address,
                    ) {
                        spawn_on_deliver_backlog(state.clone(), member);
                    }
                }
            }
        });
    }
}

/// Serialize a harness-neutral message descriptor fed to the on-deliver handler on stdin.
/// The daemon exposes only transport facts; it never learns what the handler does with them.
fn on_deliver_descriptor_json(store_key: &str, address: &str, row: &MessageRow) -> String {
    let delivery_role = delivery_role(address, &row.to_addr, row.cc.as_deref());
    let requires_disposition_for_current_recipient =
        requires_disposition_for_recipient(row.requires_disposition, address, &row.to_addr);
    serde_json::json!({
        "message_id": row.id,
        "thread_id": row.thread_id,
        "store_key": store_key,
        "address": address,
        "delivered_to": address,
        "primary_to": row.to_addr,
        "cc": cc_recipients(row.cc.as_deref()),
        "delivery_role": delivery_role,
        "from": row.from_addr,
        "kind": row.kind,
        "attention": row.attention,
        "requires_disposition": row.requires_disposition,
        "requires_disposition_for_current_recipient": requires_disposition_for_current_recipient,
        "subject": row.subject,
        "body": row.body,
    })
    .to_string()
}

/// Outcome of one on-deliver handler exec.
#[derive(Clone, Copy, PartialEq, Eq)]
enum RunOutcome {
    /// The handler accepted the push (exit 0).
    Ok,
    /// The harness deferred the push because it was busy (`ON_DELIVER_DEFERRED_EXIT`) -- not sent,
    /// not a failure; held at the deferred backstop and re-attempted by the idle drain.
    Deferred,
    /// A retryable failure (nonzero exit, spawn error, timeout) -- retried on backoff.
    Transient,
    /// A permanent, non-retryable failure (`ON_DELIVER_PERMANENT_EXIT`) -- dead-lettered.
    Permanent,
}

/// Exec one on-deliver handler process: descriptor on stdin, bounded concurrency, bounded
/// wall-clock. Returns (outcome, bounded-stderr-tail-on-failure). The daemon treats the argv
/// opaquely, distinguishing only a permanent exit code so it can dead-letter that message.
async fn run_on_deliver(
    sem: Arc<Semaphore>,
    argv: Vec<String>,
    descriptor: String,
) -> (RunOutcome, Option<String>) {
    if argv.is_empty() {
        return (RunOutcome::Transient, None);
    }
    let _permit = match sem.acquire().await {
        Ok(permit) => permit,
        Err(_) => return (RunOutcome::Transient, None),
    };
    let mut cmd = tokio::process::Command::new(&argv[0]);
    cmd.args(&argv[1..])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    configure_on_deliver_spawn(&mut cmd);
    let mut child = match cmd.spawn() {
        Ok(child) => child,
        Err(e) => return (RunOutcome::Transient, Some(format!("spawn failed: {e}"))),
    };
    if let Some(mut stdin) = child.stdin.take() {
        use tokio::io::AsyncWriteExt;
        let _ = stdin.write_all(descriptor.as_bytes()).await;
        let _ = stdin.write_all(b"\n").await;
        // stdin drops here, closing the pipe so the handler sees EOF.
    }
    let stderr_pipe = child.stderr.take();
    match tokio::time::timeout(ON_DELIVER_TIMEOUT, child.wait()).await {
        Ok(Ok(status)) if status.success() => (RunOutcome::Ok, None),
        Ok(Ok(status)) => {
            let outcome = match status.code() {
                Some(ON_DELIVER_PERMANENT_EXIT) => RunOutcome::Permanent,
                Some(ON_DELIVER_DEFERRED_EXIT) => RunOutcome::Deferred,
                _ => RunOutcome::Transient,
            };
            (outcome, read_bounded_stderr(stderr_pipe).await)
        }
        Ok(Err(e)) => (RunOutcome::Transient, Some(format!("wait failed: {e}"))),
        Err(_) => {
            let _ = child.start_kill();
            (RunOutcome::Transient, Some("handler timed out".to_string()))
        }
    }
}

#[cfg(windows)]
fn configure_on_deliver_spawn(command: &mut tokio::process::Command) {
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    // The daemon is a background process. Without this flag, Windows can briefly create a console
    // window for every on-deliver helper (`telex copilot push`), which appears as a desktop flash
    // whenever a message is sent.
    command.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
fn configure_on_deliver_spawn(_command: &mut tokio::process::Command) {}

/// Read a bounded tail of a finished child's stderr for diagnostics. No message bodies flow
/// through stderr; `telex copilot push` writes only short error lines.
async fn read_bounded_stderr(pipe: Option<tokio::process::ChildStderr>) -> Option<String> {
    use tokio::io::AsyncReadExt;
    let pipe = pipe?;
    let mut buf = Vec::new();
    let _ = pipe.take(4096).read_to_end(&mut buf).await;
    let text = String::from_utf8_lossy(&buf);
    let text = text.trim();
    if text.is_empty() {
        None
    } else {
        Some(text.chars().take(400).collect())
    }
}

/// Reconciliation sweep for one member with a handler: (re)push any durably-undelivered
/// message not already pushed. Reuses `fetch_undelivered` (the holder's source of truth for
/// "still needs delivering") so backlog that arrived while the bridge was down is delivered
/// on the next tick or on re-bind. Best effort; never touches delivery/consumption state.
async fn on_deliver_sweep_member(
    state: Arc<DaemonState>,
    backend: &Arc<dyn Backend>,
    member: &MemberRecord,
) {
    let argv = match &member.on_deliver {
        Some(argv) if !argv.is_empty() => argv.clone(),
        _ => return,
    };
    let candidates = match backend
        .fetch_wait_candidates(
            &member.address,
            WaitFetchOptions {
                wake_on_cc: member.on_deliver_wake_on_cc,
                cc_after_ms: member.on_deliver_cc_after_ms.unwrap_or_default(),
            },
        )
        .await
    {
        Ok(rows) => rows,
        Err(e) => {
            state.push_recent_error(
                "OnDeliverSweep",
                format!(
                    "fetch_wait_candidates failed store={} address={}: {e:#}",
                    member.store_key, member.address
                ),
            );
            return;
        }
    };
    let member_key = MemberKey {
        store_key: member.store_key.clone(),
        session_id: member.session_id.clone(),
        address: member.address.clone(),
    };
    let keep: BTreeSet<i64> = candidates
        .iter()
        .map(|candidate| candidate.message.id)
        .collect();
    state.on_deliver_retain_pushed(&member_key, &keep);
    let now = Instant::now();
    let mut fired = 0usize;
    for candidate in candidates {
        if state.on_deliver_should_skip(&member_key, candidate.message.id, now) {
            continue;
        }
        state.spawn_on_deliver(
            member_key.clone(),
            argv.clone(),
            member.store_key.clone(),
            member.address.clone(),
            candidate.message,
            candidate.notification_only,
        );
        fired += 1;
        if fired >= ON_DELIVER_SWEEP_BATCH {
            break;
        }
    }
}

/// Spawn a one-shot backlog sweep for a member (used on register/re-bind and per heartbeat
/// tick), so it never blocks the registration response or the heartbeat cycle.
fn spawn_on_deliver_backlog(state: Arc<DaemonState>, member: MemberRecord) {
    tokio::spawn(async move {
        if let Ok(backend) = state.backend_for(&member.store_key).await {
            on_deliver_sweep_member(state.clone(), &backend, &member).await;
        }
    });
}

async fn heartbeat_loop(state: Arc<DaemonState>) {
    loop {
        tokio::time::sleep(HEARTBEAT_INTERVAL).await;
        heartbeat_members_once(state.clone()).await;
    }
}

async fn heartbeat_members_once(state: Arc<DaemonState>) {
    if state.is_draining() {
        return;
    }
    state.prune_dead_waiters();
    let members = state.members_snapshot();
    for member in members {
        if state
            .get_member(&member.store_key, &member.session_id, &member.address)
            .is_none()
        {
            continue;
        }
        if member.idle {
            continue;
        }
        if let Some(reason) = watch_pid_reap_reason(&member.watch_pids) {
            let _ = end_session_members(
                state.clone(),
                member.store_key.clone(),
                member.session_id.clone(),
                "WatchPidDeath",
                &reason,
            )
            .await;
            continue;
        }
        let backend = match state.backend_for(&member.store_key).await {
            Ok(backend) => backend,
            Err(Response::Error { code, message, .. }) => {
                state.push_recent_error(
                    "BackendDisconnect",
                    format!(
                        "heartbeat skipped for {} {} epoch {}: {code}: {message}",
                        member.store_key, member.address, member.lease_epoch
                    ),
                );
                continue;
            }
            Err(other) => {
                state.push_recent_error(
                    "BackendDisconnect",
                    format!(
                        "heartbeat skipped for {} {} epoch {}: unexpected backend response {other:?}",
                        member.store_key, member.address, member.lease_epoch
                    ),
                );
                continue;
            }
        };
        match backend
            .heartbeat_epoch(
                &member.address,
                &member.owner_instance_id,
                member.lease_epoch,
            )
            .await
        {
            Ok(true) => {
                if member.on_deliver.is_some() {
                    spawn_on_deliver_backlog(state.clone(), member.clone());
                }
            }
            Ok(false) => {
                self_demote_member(&state, &member, "epoch heartbeat returned 0 rows");
            }
            Err(e) => {
                state.push_recent_error(
                    "BackendDisconnect",
                    format!(
                        "heartbeat failed for {} {} epoch {}: {e:#}",
                        member.store_key, member.address, member.lease_epoch
                    ),
                );
            }
        }
    }
}

fn presence_ended_detail(kind: &str) -> String {
    match kind {
        "IdleTtlReap" => "idle-ttl-reap",
        "SessionEnd" => "session-end",
        "StationStop" => "station-stop",
        "Reset" => "reset",
        "WatchPidDeath" => "watch-pid-death",
        _ => "presence-ended",
    }
    .to_string()
}

fn watch_pid_reap_reason(watch_pids: &[WatchPidRecord]) -> Option<String> {
    if watch_pids.is_empty() {
        return None;
    }
    let mut anchors_seen = false;
    let mut anchor_alive = false;
    for watch in watch_pids {
        let alive =
            crate::session_watch::process_alive_with_start_time(watch.pid, watch.start_time);
        match watch.role {
            WatchPidRole::Anchor => {
                anchors_seen = true;
                anchor_alive |= alive;
            }
            WatchPidRole::Required if !alive => {
                return Some(format!(
                    "required watch pid {} is dead or reused",
                    watch.pid
                ));
            }
            WatchPidRole::Required => {}
        }
    }
    if anchors_seen && !anchor_alive {
        return Some("all anchor watch pids are dead or reused".to_string());
    }
    None
}

fn capture_watch_pids(watch_pids: Vec<WatchPidSpec>) -> Vec<WatchPidRecord> {
    watch_pids
        .into_iter()
        .filter(|watch| watch.pid != 0)
        .map(|watch| WatchPidRecord {
            pid: watch.pid,
            start_time: crate::session_watch::capture_process_start_time(watch.pid),
            role: watch.role,
        })
        .collect()
}

fn self_demote_member(state: &DaemonState, member: &MemberRecord, reason: impl AsRef<str>) {
    let reason = reason.as_ref();
    if state.remove_member_if_current(member) {
        state.push_recent_error(
            "NotOwner",
            format!(
                "self-demoted {} session={} address={} epoch={} owner={}: {}",
                member.store_key,
                member.session_id,
                member.address,
                member.lease_epoch,
                member.owner_instance_id,
                reason
            ),
        );
    }
}

async fn prove_current_owner(
    state: &DaemonState,
    backend: &Arc<dyn Backend>,
    member: &MemberRecord,
    context: &str,
) -> std::result::Result<(), Response> {
    match backend
        .heartbeat_epoch(
            &member.address,
            &member.owner_instance_id,
            member.lease_epoch,
        )
        .await
    {
        Ok(true) => Ok(()),
        Ok(false) => {
            self_demote_member(
                state,
                member,
                format!("{context}: epoch heartbeat returned 0 rows"),
            );
            Err(needs_attach_for_missing_member(
                state,
                backend,
                &member.store_key,
                &member.session_id,
                &member.address,
                context,
            )
            .await)
        }
        Err(e) => Err(proto::internal(format!(
            "{context}: heartbeating {} at epoch {}: {e:#}",
            member.address, member.lease_epoch
        ))),
    }
}

async fn needs_attach_for_missing_member(
    state: &DaemonState,
    backend: &Arc<dyn Backend>,
    store_key: &str,
    session_id: &str,
    address: &str,
    operation: &str,
) -> Response {
    match backend.detach_tombstone(session_id, address).await {
        Ok(Some(tombstone)) => {
            return proto::needs_attach_with_reason(
                format!(
                    "session {session_id} deliberately detached from {address} in {store_key} by {} at {}; explicit attach required",
                    tombstone.reason, tombstone.at_ms
                ),
                NeedsAttachReason::DeliberatelyDetached,
            )
        }
        Ok(None) => {}
        Err(e) => {
            return proto::internal(format!(
                "checking detach tombstone for {operation} {session_id}/{address}: {e:#}"
            ))
        }
    }
    if let Some(ended) = state.session_definite_end(store_key, session_id) {
        return proto::needs_attach_with_reason(
            format!(
                "session {session_id} was definitely ended by {} at {}; deliberate re-attach required for {address} in {store_key}",
                ended.reason, ended.at_ms
            ),
            NeedsAttachReason::DeliberatelyDetached,
        );
    }
    state.push_recent_error(
        "NeedsAttach",
        format!("NeedsAttach operation={operation} store={store_key} session={session_id} address={address}"),
    );
    proto::needs_attach_with_reason(
        format!("session {session_id} is not attached to {address} in {store_key}"),
        NeedsAttachReason::RestartLost,
    )
}

async fn drain_members(state: Arc<DaemonState>) -> std::result::Result<(), Response> {
    state.begin_draining();
    let members = state.members_snapshot();
    for member in &members {
        let backend = match state.backend_for(&member.store_key).await {
            Ok(backend) => backend,
            Err(response) => {
                state.clear_draining();
                return Err(response);
            }
        };
        match backend
            .release_epoch_lease(
                &member.address,
                &member.owner_instance_id,
                member.lease_epoch,
            )
            .await
        {
            Ok(true) => {}
            Ok(false) => {
                state.push_recent_error(
                    "NotOwner",
                    format!(
                        "drain release found non-owner for {} {} epoch {} owner {}",
                        member.store_key,
                        member.address,
                        member.lease_epoch,
                        member.owner_instance_id
                    ),
                );
            }
            Err(e) => {
                state.clear_draining();
                return Err(proto::internal(format!(
                    "drain release failed for {} at epoch {}: {e:#}",
                    member.address, member.lease_epoch
                )));
            }
        }
    }
    state.clear_members();
    Ok(())
}

async fn handle_client(
    conn: platform::ServerConn,
    state: Arc<DaemonState>,
) -> Result<ClientAction> {
    platform::verify_client_peer(&conn)?;
    let (read_half, mut write_half) = tokio::io::split(conn);
    let mut reader = BufReader::new(read_half);

    let hello: proto::Hello = match read_json_line(&mut reader).await {
        Ok(hello) => hello,
        Err(e) => {
            write_json_line(
                &mut write_half,
                &proto::error_response(proto::ERROR_INCOMPATIBLE, e.to_string()),
            )
            .await?;
            return Ok(ClientAction::Continue);
        }
    };
    let ack = proto::evaluate_hello(&hello);
    write_json_line(&mut write_half, &ack).await?;
    if !ack.accepted {
        return Ok(ClientAction::Continue);
    }

    let request: Request = match read_json_line(&mut reader).await {
        Ok(req) => req,
        Err(HandshakeError::Eof) => return Ok(ClientAction::Continue),
        Err(e) => {
            write_json_line(
                &mut write_half,
                &proto::error_response(
                    proto::ERROR_INCOMPATIBLE,
                    format!("unknown or invalid request frame: {e}"),
                ),
            )
            .await?;
            return Ok(ClientAction::Continue);
        }
    };

    let (response, action) = handle_request(state, request).await;
    write_json_line(&mut write_half, &response).await?;
    Ok(action)
}

async fn handle_request(state: Arc<DaemonState>, request: Request) -> (Response, ClientAction) {
    let response = match request {
        Request::Ping => Response::Pong {
            protocol_version: current_protocol_version(),
            daemon_version: proto::DAEMON_VERSION.to_string(),
            instance_id: state.instance_id.clone(),
        },
        Request::Status { detail, proof, .. } => {
            if detail {
                if let Err(response) = state.check_admin_cap(proof.as_deref()) {
                    return (response, ClientAction::Continue);
                }
                Response::StatusReport {
                    status: state.status().await,
                }
            } else {
                Response::StatusReport {
                    status: state.status_minimal(),
                }
            }
        }
        Request::SessionEnd {
            store_key,
            session_id,
            proof,
        } => {
            if let Err(response) = state.check_admin_cap(proof.as_deref()) {
                return (response, ClientAction::Continue);
            }
            session_end(state.clone(), store_key, session_id).await
        }
        Request::Reset {
            store_key,
            address,
            proof,
        } => {
            if let Err(response) = state.check_admin_cap(proof.as_deref()) {
                return (response, ClientAction::Continue);
            }
            reset_station(state.clone(), store_key, address).await
        }
        Request::DrainDeferred {
            store_key,
            session_id,
            proof,
        } => {
            if let Err(response) = state.check_admin_cap(proof.as_deref()) {
                return (response, ClientAction::Continue);
            }
            drain_deferred(state.clone(), store_key, session_id).await
        }
        Request::Drain { proof } => {
            if let Err(response) = state.check_admin_cap(proof.as_deref()) {
                return (response, ClientAction::Continue);
            }
            if let Err(response) = drain_members(state.clone()).await {
                return (response, ClientAction::Continue);
            }
            return (
                Response::Ack {
                    message: Some("draining".to_string()),
                    delivery_outcome: None,
                    address: None,
                    message_id: None,
                    lease_epoch: None,
                },
                ClientAction::Drain,
            );
        }
        Request::Register {
            store_key,
            address,
            session_id,
            occupant,
            description,
            scope,
            tags,
            watch_pids,
            recovery,
            on_deliver,
            on_deliver_wake_on_cc,
        } => {
            register_member(
                state.clone(),
                store_key,
                address,
                session_id,
                occupant,
                description,
                scope,
                tags,
                watch_pids,
                recovery,
                on_deliver,
                on_deliver_wake_on_cc,
            )
            .await
        }
        Request::Detach {
            store_key,
            session_id,
            address,
        } => detach_member(state.clone(), store_key, session_id, address).await,
        Request::StationStop {
            store_key,
            session_id,
            address,
            wait_grace_ms,
        } => station_stop(state.clone(), store_key, session_id, address, wait_grace_ms).await,
        Request::Wait {
            store_key,
            session_id,
            address,
            attention,
            min_attention,
            wake_on_cc,
            timeout_ms,
            waiter_pid,
            waiter_start_time,
        } => {
            wait_for_message(
                state.clone(),
                store_key,
                session_id,
                address,
                attention,
                min_attention,
                wake_on_cc,
                timeout_ms,
                waiter_pid,
                waiter_start_time,
            )
            .await
        }
        Request::Ack {
            store_key,
            session_id,
            address,
            message_id,
        } => ack_message(state.clone(), store_key, session_id, address, message_id).await,
        Request::Send {
            store_key,
            session_id,
            from_addr,
            to_addr,
            cc,
            kind,
            attention,
            requires_disposition,
            subject,
            body,
            metadata,
        } => {
            send_message(
                state.clone(),
                store_key,
                session_id,
                from_addr,
                to_addr,
                cc,
                kind,
                attention,
                requires_disposition,
                subject,
                body,
                metadata,
            )
            .await
        }
        Request::Reply {
            store_key,
            session_id,
            from_addr,
            message_id,
            kind,
            attention,
            requires_disposition,
            subject,
            cc,
            body,
        } => {
            reply_message(
                state.clone(),
                store_key,
                session_id,
                from_addr,
                message_id,
                kind,
                attention,
                requires_disposition,
                subject,
                cc,
                body,
            )
            .await
        }
    };
    (response, ClientAction::Continue)
}

async fn register_member(
    state: Arc<DaemonState>,
    store_key: String,
    address: String,
    session_id: String,
    occupant: String,
    description: Option<String>,
    scope: Option<String>,
    tags: Option<String>,
    watch_pids: Vec<WatchPidSpec>,
    recovery: bool,
    on_deliver: Option<Vec<String>>,
    on_deliver_wake_on_cc: bool,
) -> Response {
    if state.is_draining() {
        return proto::error_response(proto::ERROR_NOT_RUNNING, "daemon is draining");
    }
    let watch_pids = capture_watch_pids(watch_pids);

    if let Some(existing) = state.get_member(&store_key, &session_id, &address) {
        let backend = match state.backend_for(&store_key).await {
            Ok(backend) => backend,
            Err(response) => return response,
        };
        match backend
            .heartbeat_epoch(&address, &existing.owner_instance_id, existing.lease_epoch)
            .await
        {
            Ok(true) => {
                let mut refreshed = existing.clone();
                refreshed.occupant = occupant;
                refreshed.description = description;
                refreshed.scope = scope;
                refreshed.tags = tags;
                let preserving_on_deliver = on_deliver.is_none() && existing.on_deliver.is_some();
                refreshed.watch_pids = if preserving_on_deliver {
                    existing.watch_pids.clone()
                } else {
                    watch_pids
                };
                refreshed.idle = false;
                refreshed.idle_rearmable = false;
                // Preserve an already-registered push handler and its liveness predicates when a
                // generic recovery/refresh re-registers with `on_deliver = None` (e.g. a `telex
                // wait` re-attach); only an explicit re-provision replaces them, so a pull
                // re-attach cannot silently disarm or process-anchor the Copilot bridge.
                refreshed.on_deliver = on_deliver.clone().or_else(|| existing.on_deliver.clone());
                if on_deliver.is_some() {
                    refreshed.on_deliver_wake_on_cc = on_deliver_wake_on_cc;
                    refreshed.on_deliver_cc_after_ms =
                        match on_deliver_cc_lower_bound(&backend, &address, on_deliver_wake_on_cc)
                            .await
                        {
                            Ok(value) => value,
                            Err(response) => return response,
                        };
                } else {
                    refreshed.on_deliver_wake_on_cc = existing.on_deliver_wake_on_cc;
                    refreshed.on_deliver_cc_after_ms = existing.on_deliver_cc_after_ms;
                }
                state.check_session_id_reuse_tripwire(&refreshed);
                if !recovery {
                    state.clear_definite_session_end(&store_key, &session_id);
                }
                state.insert_member(refreshed.clone());
                // Reset the push retry state and re-scan backlog only on an explicit
                // (re-)provision; a plain refresh that merely preserved the handler keeps its
                // backoff intact (the per-heartbeat sweep still delivers any backlog).
                if on_deliver.is_some() {
                    state.on_deliver_forget_member(&MemberKey {
                        store_key: refreshed.store_key.clone(),
                        session_id: refreshed.session_id.clone(),
                        address: refreshed.address.clone(),
                    });
                    spawn_on_deliver_backlog(state.clone(), refreshed.clone());
                }
                return Response::Registered {
                    lease_epoch: refreshed.lease_epoch,
                    owner_instance_id: refreshed.owner_instance_id,
                };
            }
            Ok(false) => {
                self_demote_member(
                    &state,
                    &existing,
                    "register refresh: epoch heartbeat returned 0 rows",
                );
            }
            Err(e) => {
                return proto::internal(format!(
                    "refreshing epoch lease for {address} at epoch {}: {e:#}",
                    existing.lease_epoch
                ));
            }
        }
    }

    if let Some(conflict) = state
        .members
        .lock()
        .unwrap()
        .values()
        .find(|m| m.store_key == store_key && m.address == address && !m.idle)
        .cloned()
    {
        return proto::error_response(
            proto::ERROR_INCOMPATIBLE,
            format!(
                "address {} is already attended by session {} in this daemon",
                conflict.address, conflict.session_id
            ),
        );
    }

    let backend = match state.backend_for(&store_key).await {
        Ok(backend) => backend,
        Err(response) => return response,
    };
    if recovery {
        match backend.detach_tombstone(&session_id, &address).await {
            Ok(Some(tombstone)) => {
                return proto::needs_attach_with_reason(
                    format!(
                        "session {session_id} deliberately detached from {address} in {store_key} by {} at {}; explicit attach required",
                        tombstone.reason, tombstone.at_ms
                    ),
                    NeedsAttachReason::DeliberatelyDetached,
                )
            }
            Ok(None) => {}
            Err(e) => {
                return proto::internal(format!(
                    "checking detach tombstone before recovery register {session_id}/{address}: {e:#}"
                ))
            }
        }
    }
    if let Err(e) = backend
        .ensure_address(
            &address,
            description.as_deref(),
            scope.as_deref(),
            tags.as_deref(),
        )
        .await
    {
        return proto::internal(format!("ensuring address {address}: {e:#}"));
    }

    let claimed = match backend
        .claim_epoch_lease(&address, &state.instance_id, liveness_window_secs())
        .await
    {
        Ok(EpochClaimResult::Claimed(claimed)) => claimed,
        Ok(EpochClaimResult::AlreadyOwned {
            lease_epoch,
            owner_instance_id,
            lease_row,
        }) => {
            return proto::error_response(
                proto::ERROR_INCOMPATIBLE,
                format!(
                    "address {} is already owned at epoch {} by {} ({:?})",
                    address, lease_epoch, owner_instance_id, lease_row.occupant
                ),
            );
        }
        Err(e) => return proto::unsupported(format!("claiming epoch lease for {address}: {e:#}")),
    };
    if claimed.legacy_cutover {
        state.push_recent_error(
            "LegacyCutover",
            format!(
                "claimed legacy/non-epoch lease row store={store_key} address={address} at epoch {}",
                claimed.lease_epoch
            ),
        );
    }
    if recovery {
        match backend.detach_tombstone(&session_id, &address).await {
            Ok(Some(tombstone)) => {
                let _ = backend
                    .release_epoch_lease(&address, &claimed.owner_instance_id, claimed.lease_epoch)
                    .await;
                return proto::needs_attach_with_reason(
                    format!(
                        "session {session_id} deliberately detached from {address} in {store_key} by {} at {}; explicit attach required",
                        tombstone.reason, tombstone.at_ms
                    ),
                    NeedsAttachReason::DeliberatelyDetached,
                );
            }
            Ok(None) => {}
            Err(e) => {
                let _ = backend
                    .release_epoch_lease(&address, &claimed.owner_instance_id, claimed.lease_epoch)
                    .await;
                return proto::internal(format!(
                    "checking detach tombstone after recovery claim {session_id}/{address}: {e:#}"
                ));
            }
        }
    } else if let Err(e) = backend.clear_detach_tombstone(&session_id, &address).await {
        let _ = backend
            .release_epoch_lease(&address, &claimed.owner_instance_id, claimed.lease_epoch)
            .await;
        state.push_recent_error(
            "DetachTombstone",
            format!(
                "failed to clear detach tombstone store={store_key} session={session_id} address={address}: {e:#}"
            ),
        );
        return proto::internal(format!(
            "registering {address}: failed to clear durable detach tombstone for session {session_id}: {e:#}"
        ));
    }
    let effective_on_deliver_wake_on_cc = on_deliver.is_some() && on_deliver_wake_on_cc;
    let on_deliver_cc_after_ms = match on_deliver_cc_lower_bound(
        &backend,
        &address,
        effective_on_deliver_wake_on_cc,
    )
    .await
    {
        Ok(value) => value,
        Err(response) => {
            let _ = backend
                .release_epoch_lease(&address, &claimed.owner_instance_id, claimed.lease_epoch)
                .await;
            return response;
        }
    };
    let record = MemberRecord {
        address: address.clone(),
        store_key: store_key.clone(),
        backend: backend.kind().to_string(),
        session_id: session_id.clone(),
        occupant,
        host: crate::config::hostname(),
        waiters: 0,
        watch_pids,
        description,
        scope,
        tags,
        lease_epoch: claimed.lease_epoch,
        owner_instance_id: claimed.owner_instance_id.clone(),
        idle: false,
        idle_rearmable: false,
        unattended_since_ms: Some(now_ms()),
        unattended_with_backlog_since_ms: None,
        last_waiter_exit_at_ms: None,
        last_waiter_outcome: None,
        last_waiter_exit_code: None,
        last_waiter_detail: None,
        last_waiter_pid: None,
        last_delivered_message_id: None,
        on_deliver,
        on_deliver_wake_on_cc: effective_on_deliver_wake_on_cc,
        on_deliver_cc_after_ms,
    };
    state.check_session_id_reuse_tripwire(&record);
    if !recovery {
        state.clear_definite_session_end(&store_key, &session_id);
    }
    let backlog = if record.on_deliver.is_some() {
        Some(record.clone())
    } else {
        None
    };
    state.insert_member(record);
    if let Some(member) = backlog {
        state.on_deliver_forget_member(&MemberKey {
            store_key: member.store_key.clone(),
            session_id: member.session_id.clone(),
            address: member.address.clone(),
        });
        spawn_on_deliver_backlog(state.clone(), member);
    }
    Response::Registered {
        lease_epoch: claimed.lease_epoch,
        owner_instance_id: claimed.owner_instance_id,
    }
}

async fn on_deliver_cc_lower_bound(
    backend: &Arc<dyn Backend>,
    address: &str,
    wake_on_cc: bool,
) -> std::result::Result<Option<i64>, Response> {
    if !wake_on_cc {
        return Ok(None);
    }
    if !backend.supports_wake_on_cc() {
        return Err(proto::unsupported(format!(
            "on-deliver wake-on-cc is not supported by the {} backend",
            backend.kind()
        )));
    }
    match backend.durable_clock_now_ms().await {
        Ok(value) => Ok(Some(value)),
        Err(e) => Err(proto::internal(format!(
            "capturing on-deliver CC lower bound for {address}: {e:#}"
        ))),
    }
}

async fn session_end(state: Arc<DaemonState>, store_key: String, session_id: String) -> Response {
    end_session_members(
        state,
        store_key,
        session_id,
        "SessionEnd",
        "authoritative sessionEnd hook",
    )
    .await
}

/// Idle-drain (issue #65): the harness reports (via `telex copilot drain` on turn-stop) that a root
/// turn ended, so re-attempt any messages this session deferred while it was busy. Clears the
/// deferred skip for each of the session's on-deliver members and queues a backlog re-sweep; the
/// sweep revalidates durable state (`fetch_wait_candidates`), so a message acked before the drain
/// is no longer a candidate and is not re-injected. Non-blocking: it queues the sweeps and returns.
/// The sweep is queued for **every** on-deliver member (not only those with a cleared deferred
/// entry) so it (a) closes the race where the deferred attempt is recorded just after the drain
/// arrives, and (b) opportunistically re-attempts any message whose backstop elapsed now that the
/// bridge is idle. The client (`telex copilot drain`) already skips this call for sessions with no
/// bridge, so the per-member sweep only runs for real bridge sessions.
/// The sweep is queued for **every** on-deliver member (not only those with a cleared deferred
/// entry) so it (a) closes the race where the deferred attempt is recorded just after the drain
/// arrives, and (b) opportunistically re-attempts any message whose backstop elapsed now that the
/// bridge is idle. The client (`telex copilot drain`) already skips this call for sessions with no
/// bridge, so the per-member sweep only runs for real bridge sessions. Members are matched by
/// `session_id` across all stores (not the client's ambient store) so a session on a named
/// `--backend`/`--db` -- whose static drain hook resolves a different store -- still drains.
async fn drain_deferred(
    state: Arc<DaemonState>,
    _store_key: String,
    session_id: String,
) -> Response {
    let mut cleared = 0usize;
    let mut swept_members = 0usize;
    for member in state.session_members_any_store(&session_id) {
        if member.on_deliver.is_none() {
            continue;
        }
        let member_key = MemberKey {
            store_key: member.store_key.clone(),
            session_id: member.session_id.clone(),
            address: member.address.clone(),
        };
        // Advance the drain generation so a push that is inflight right now (its deferred attempt
        // not yet recorded) detects this drain on completion and self-re-sweeps.
        state.on_deliver_bump_drain_gen(&member_key);
        cleared += state.on_deliver_clear_deferred(&member_key);
        swept_members += 1;
        spawn_on_deliver_backlog(state.clone(), member);
    }
    Response::Ack {
        message: Some(format!(
            "drain deferred: cleared {cleared} deferred message(s), re-swept {swept_members} member(s)"
        )),
        delivery_outcome: None,
        address: None,
        message_id: None,
        lease_epoch: None,
    }
}

async fn end_session_members(
    state: Arc<DaemonState>,
    store_key: String,
    session_id: String,
    kind: &str,
    reason: &str,
) -> Response {
    let active = state.session_members(&store_key, &session_id);
    let release_error = release_definite_end_members(&state, &active, kind).await;
    if let Some(response) = release_error {
        return response;
    }
    let affected = state.mark_session_idle(&store_key, &session_id, kind, reason, true);
    if affected.is_empty() {
        state.push_recent_error(
            kind,
            format!("{kind} no-op store={store_key} session={session_id}: no active members"),
        );
    }
    Response::Ack {
        message: Some(presence_ended_detail(kind)),
        delivery_outcome: None,
        address: None,
        message_id: None,
        lease_epoch: None,
    }
}

async fn release_definite_end_members(
    state: &DaemonState,
    members: &[MemberRecord],
    reason: &str,
) -> Option<Response> {
    for member in members {
        let backend = match state.backend_for(&member.store_key).await {
            Ok(backend) => backend,
            Err(response) => return Some(response),
        };
        match backend
            .release_epoch_lease_for_detach(
                &member.address,
                &member.owner_instance_id,
                member.lease_epoch,
                &member.session_id,
                reason,
            )
            .await
        {
            Ok(true) => {}
            Ok(false) => {
                state.push_recent_error(
                    "NotOwner",
                    format!(
                        "{reason} durable release found non-owner for {} {} epoch {} owner {}",
                        member.store_key,
                        member.address,
                        member.lease_epoch,
                        member.owner_instance_id
                    ),
                );
            }
            Err(e) => {
                state.push_recent_error(
                    "BackendDisconnect",
                    format!(
                        "{reason} durable release failed for {} {} epoch {}: {e:#}",
                        member.store_key, member.address, member.lease_epoch
                    ),
                );
                return Some(proto::internal(format!(
                    "{reason} durable release failed for {} at epoch {}: {e:#}",
                    member.address, member.lease_epoch
                )));
            }
        }
    }
    None
}

async fn reset_station(state: Arc<DaemonState>, store_key: String, address: String) -> Response {
    let backend = match state.backend_for(&store_key).await {
        Ok(backend) => backend,
        Err(response) => return response,
    };
    let durable_epoch = match backend.reset_epoch_lease(&address).await {
        Ok(epoch) => epoch,
        Err(e) => {
            return proto::internal(format!(
                "resetting durable epoch lease for {address} in {store_key}: {e:#}"
            ))
        }
    };
    let affected =
        state.mark_address_idle(&store_key, &address, "Reset", "operator reset requested");
    if affected.is_empty() {
        state.push_recent_error(
            "Reset",
            format!("Reset no-op store={store_key} address={address}: no active member"),
        );
    }
    Response::Ack {
        message: Some("reset".to_string()),
        delivery_outcome: None,
        address: Some(address),
        message_id: None,
        lease_epoch: affected.first().map(|m| m.lease_epoch).or(durable_epoch),
    }
}

async fn station_stop(
    state: Arc<DaemonState>,
    store_key: String,
    session_id: String,
    address: String,
    wait_grace_ms: u64,
) -> Response {
    let waiters_before = state
        .live_waiter_statuses_for(&store_key, &session_id, &address)
        .len();
    // Snapshot whether this station had a registered on-deliver push handler BEFORE detach removes
    // the member — station stop releases membership + tombstones but does NOT unload the in-session
    // bridge, so the CLI warns and points at `telex copilot detach`.
    let push_registered = state
        .get_member(&store_key, &session_id, &address)
        .map(|m| m.on_deliver.is_some())
        .unwrap_or(false);

    // Let any blocked wait request return PresenceEnded instead of an error. Once the waiter
    // guard drops, we can remove membership durably via detach without racing an orphan waiter
    // that might receive nobody-read output.
    let _ = state.mark_member_idle(
        &store_key,
        &session_id,
        &address,
        "StationStop",
        "station stop requested",
    );

    wait_for_waiters_to_drain(&state, &store_key, &session_id, &address, wait_grace_ms).await;

    let detached = detach_member(
        state.clone(),
        store_key.clone(),
        session_id.clone(),
        address.clone(),
    )
    .await;
    let waiters_after_status = state.live_waiter_statuses_for(&store_key, &session_id, &address);
    let waiters_after = waiters_after_status.len();
    match detached {
        Response::Ack {
            message,
            lease_epoch,
            ..
        } => Response::StationStopped {
            store_key,
            session_id,
            address,
            detached: true,
            waiters_before,
            waiters_after,
            live_waiters: waiters_after_status,
            push_registered,
            message,
            lease_epoch,
        },
        Response::Error { .. } => detached,
        other => proto::internal(format!(
            "unexpected station-stop detach response: {other:?}"
        )),
    }
}

async fn wait_for_waiters_to_drain(
    state: &DaemonState,
    store_key: &str,
    session_id: &str,
    address: &str,
    wait_grace_ms: u64,
) {
    let deadline = Instant::now() + Duration::from_millis(wait_grace_ms);
    loop {
        if state
            .live_waiter_statuses_for(store_key, session_id, address)
            .is_empty()
        {
            return;
        }
        let now = Instant::now();
        if now >= deadline {
            return;
        }
        tokio::time::sleep(
            deadline
                .saturating_duration_since(now)
                .min(Duration::from_millis(25)),
        )
        .await;
    }
}

async fn detach_member(
    state: Arc<DaemonState>,
    store_key: String,
    session_id: String,
    address: String,
) -> Response {
    let member = state.get_member(&store_key, &session_id, &address);
    if let Some(member) = member {
        let backend = match state.backend_for(&store_key).await {
            Ok(backend) => backend,
            Err(response) => return response,
        };
        match backend
            .release_epoch_lease_for_detach(
                &address,
                &member.owner_instance_id,
                member.lease_epoch,
                &session_id,
                "Detach",
            )
            .await
        {
            Ok(true) => {
                state.remove_member(&store_key, &session_id, &address);
                state.record_definite_session_end(
                    &store_key,
                    &session_id,
                    "Detach",
                    &[member.clone()],
                );
                // Do NOT record the durable tombstone again here: `release_epoch_lease_for_detach`
                // above already wrote it atomically inside the lease-release transaction (see the
                // backend contract). A second, non-atomic write can race a concurrent explicit
                // re-attach's tombstone clear and recreate a stale tombstone for a freshly-live
                // station, which `telex copilot push` would then refuse permanently.
            }
            Ok(false) => {
                self_demote_member(
                    &state,
                    &member,
                    "detach release_epoch_lease returned 0 rows",
                );
                return proto::error_response(
                    proto::ERROR_NOT_OWNER,
                    format!("session {session_id} no longer owns {address} in {store_key}"),
                );
            }
            Err(e) => {
                state.push_recent_error(
                    "BackendDisconnect",
                    format!(
                        "detach release failed for {store_key} {address} epoch {}: {e:#}",
                        member.lease_epoch
                    ),
                );
                return proto::internal(format!(
                    "detaching {address} at epoch {}: durable release failed: {e:#}",
                    member.lease_epoch
                ));
            }
        }
        Response::Ack {
            message: Some("detached".to_string()),
            delivery_outcome: None,
            address: Some(address),
            message_id: None,
            lease_epoch: Some(member.lease_epoch),
        }
    } else {
        let backend = match state.backend_for(&store_key).await {
            Ok(backend) => backend,
            Err(response) => return response,
        };
        if let Err(e) = backend
            .record_detach_tombstone(&session_id, &address, "Detach")
            .await
        {
            return proto::internal(format!(
                "recording durable detach tombstone for {session_id}/{address}: {e:#}"
            ));
        }
        state.push_recent_error(
            "Detach",
            format!(
                "Detach recorded terminal tombstone store={store_key} session={session_id} address={address}: no active in-memory member"
            ),
        );
        Response::Ack {
            message: Some("not-attached".to_string()),
            delivery_outcome: None,
            address: Some(address),
            message_id: None,
            lease_epoch: None,
        }
    }
}

async fn wait_for_message(
    state: Arc<DaemonState>,
    store_key: String,
    session_id: String,
    address: String,
    attention: Option<String>,
    min_attention: Option<String>,
    wake_on_cc: bool,
    timeout_ms: Option<u64>,
    waiter_pid: Option<u32>,
    waiter_start_time: Option<u64>,
) -> Response {
    wait_for_message_with_idle_ttl(
        state,
        store_key,
        session_id,
        address,
        attention,
        min_attention,
        wake_on_cc,
        timeout_ms,
        waiter_pid,
        waiter_start_time,
        idle_ttl_duration(),
    )
    .await
}

async fn wait_for_message_with_idle_ttl(
    state: Arc<DaemonState>,
    store_key: String,
    session_id: String,
    address: String,
    attention: Option<String>,
    min_attention: Option<String>,
    wake_on_cc: bool,
    timeout_ms: Option<u64>,
    waiter_pid: Option<u32>,
    waiter_start_time: Option<u64>,
    idle_ttl: Duration,
) -> Response {
    if state.is_draining() {
        return proto::error_response(proto::ERROR_NOT_RUNNING, "daemon is draining");
    }

    if state
        .get_member(&store_key, &session_id, &address)
        .is_none()
    {
        let backend = match state.backend_for(&store_key).await {
            Ok(backend) => backend,
            Err(response) => return response,
        };
        return needs_attach_for_missing_member(
            &state,
            &backend,
            &store_key,
            &session_id,
            &address,
            "wait",
        )
        .await;
    }
    let backend = match state.backend_for(&store_key).await {
        Ok(backend) => backend,
        Err(response) => return response,
    };
    if wake_on_cc && !backend.supports_wake_on_cc() {
        return proto::unsupported(format!(
            "wake-on-cc wait candidates are not supported by the {} backend",
            backend.kind()
        ));
    }
    let cc_after_ms = if wake_on_cc {
        match backend.durable_clock_now_ms().await {
            Ok(value) => Some(value),
            Err(e) => {
                return proto::internal(format!("capturing CC lower bound for {address}: {e:#}"))
            }
        }
    } else {
        None
    };
    let deadline = timeout_ms.map(|ms| Instant::now() + Duration::from_millis(ms));
    let idle_deadline = Instant::now() + idle_ttl;
    if state.has_live_waiter_for(&store_key, &session_id, &address) {
        state.push_recent_error(
            "ConcurrentWaiter",
            format!(
                "rejected concurrent wait store={store_key} session={session_id} address={address}: one live waiter is already armed"
            ),
        );
        return Response::PresenceEnded;
    }
    let (prior_unattended_since, prior_deaf_since) = state
        .get_member(&store_key, &session_id, &address)
        .map(|member| {
            (
                member.unattended_since_ms,
                member.unattended_with_backlog_since_ms,
            )
        })
        .unwrap_or((None, None));
    let waiter_pid_for_status = waiter_pid;
    let mut waiter_guard = WaiterGuard::new(
        state.clone(),
        &store_key,
        &session_id,
        &address,
        waiter_pid,
        waiter_start_time,
        attention.clone(),
        min_attention.clone(),
        wake_on_cc,
        cc_after_ms,
        timeout_ms,
    );
    let parsed_min_attention = match min_attention.as_deref().map(Attention::parse).transpose() {
        Ok(value) => value,
        Err(e) => {
            waiter_guard.suppress_abnormal_on_drop();
            return proto::error_response(proto::ERROR_INCOMPATIBLE, e.to_string());
        }
    };
    loop {
        let store_notification = state
            .store_notify(&store_key)
            .map(|notify| notify.notified_owned());
        if state.is_draining() {
            waiter_guard.suppress_abnormal_on_drop();
            return proto::error_response(proto::ERROR_NOT_RUNNING, "daemon is draining");
        }
        let current = match state.rearm_idle_member_if_allowed(&store_key, &session_id, &address) {
            Some(member) => member,
            None => {
                if state
                    .get_member(&store_key, &session_id, &address)
                    .is_some_and(|member| member.idle)
                {
                    return Response::PresenceEnded;
                }
                waiter_guard.suppress_abnormal_on_drop();
                return needs_attach_for_missing_member(
                    &state,
                    &backend,
                    &store_key,
                    &session_id,
                    &address,
                    "wait",
                )
                .await;
            }
        };
        if current.idle {
            return Response::PresenceEnded;
        }
        let candidates = match backend
            .fetch_wait_candidates(
                &address,
                WaitFetchOptions {
                    wake_on_cc,
                    cc_after_ms: cc_after_ms.unwrap_or_default(),
                },
            )
            .await
        {
            Ok(rows) => rows,
            Err(e) => {
                let detail = format!("{e:#}");
                waiter_guard.suppress_abnormal_on_drop();
                return proto::internal(format!(
                    "fetching wait candidates for {address}: {detail}"
                ));
            }
        };
        if let Some(last_id) = current.last_delivered_message_id {
            if current.last_waiter_outcome == Some(WaiterOutcome::Message)
                && candidates.iter().any(|candidate| {
                    !candidate.notification_only && candidate.message.id == last_id
                })
            {
                state.push_recent_error(
                    "UnackedDelivery",
                    format!(
                        "rejected wait re-arm store={store_key} session={session_id} address={address}: previously delivered message {last_id} is still unacked"
                    ),
                );
                if let Some(member) = state
                    .members
                    .lock()
                    .unwrap()
                    .get_mut(&DaemonState::member_key(&store_key, &session_id, &address))
                {
                    member.unattended_since_ms = prior_unattended_since;
                    member.unattended_with_backlog_since_ms = prior_deaf_since;
                }
                waiter_guard.suppress_abnormal_on_drop();
                return Response::PresenceEnded;
            }
        }
        if let Some(candidate) = candidates.into_iter().find(|candidate| {
            wait_attention_matches(
                candidate.message.attention.as_str(),
                attention.as_deref(),
                parsed_min_attention,
            )
        }) {
            let row = candidate.message;
            let current = match state.get_member(&store_key, &session_id, &address) {
                Some(member) => member,
                None => {
                    return needs_attach_for_missing_member(
                        &state,
                        &backend,
                        &store_key,
                        &session_id,
                        &address,
                        "wait-delivery",
                    )
                    .await;
                }
            };
            if current.idle {
                return Response::PresenceEnded;
            }
            if let Err(response) =
                prove_current_owner(&state, &backend, &current, "wait delivery proof").await
            {
                waiter_guard.suppress_abnormal_on_drop();
                return response;
            }
            let cc = cc_recipients(row.cc.as_deref());
            let delivery_role =
                delivery_role(&address, &row.to_addr, row.cc.as_deref()).to_string();
            let requires_disposition_for_current_recipient = requires_disposition_for_recipient(
                row.requires_disposition,
                &address,
                &row.to_addr,
            );
            let response = Response::Message {
                id: row.id,
                thread_id: row.thread_id,
                parent_id: row.parent_id,
                from_addr: row.from_addr,
                to_addr: row.to_addr.clone(),
                delivered_to: address.clone(),
                primary_to: row.to_addr,
                cc,
                delivery_role,
                kind: row.kind,
                attention: row.attention,
                requires_disposition: row.requires_disposition,
                requires_disposition_for_current_recipient,
                subject: row.subject,
                body: row.body,
                sent_at_ms: row.sent_at_ms,
                buffered_at_ms: now_ms(),
                lease_epoch: Some(current.lease_epoch),
            };
            return match proto::json_line_frame_len(&response) {
                Ok(len) if len <= proto::MAX_JSONL_FRAME_BYTES => {
                    state.record_waiter_message_exit(
                        &store_key,
                        &session_id,
                        &address,
                        row.id,
                        waiter_pid,
                    );
                    response
                }
                Ok(len) => {
                    waiter_guard.suppress_abnormal_on_drop();
                    proto::error_response(
                        proto::ERROR_INCOMPATIBLE,
                        format!(
                            "message {} serializes to {len} bytes, exceeding IPC frame limit {}",
                            row.id,
                            proto::MAX_JSONL_FRAME_BYTES
                        ),
                    )
                }
                Err(e) => {
                    waiter_guard.suppress_abnormal_on_drop();
                    proto::internal(format!("sizing message {} IPC frame: {e}", row.id))
                }
            };
        }
        if let Some(deadline) = deadline {
            let now = Instant::now();
            if now >= deadline {
                state.record_waiter_exit(
                    &store_key,
                    &session_id,
                    &address,
                    WaiterOutcome::IdleTimeout,
                    Some(2),
                    None,
                    waiter_pid_for_status,
                );
                return Response::Timeout;
            }
            if now >= idle_deadline {
                state.mark_member_idle(
                    &store_key,
                    &session_id,
                    &address,
                    "IdleTtlReap",
                    "blocked wait exceeded idle TTL",
                );
                state.record_waiter_exit(
                    &store_key,
                    &session_id,
                    &address,
                    WaiterOutcome::PresenceEnded,
                    Some(5),
                    Some("idle-ttl-reap".to_string()),
                    waiter_pid_for_status,
                );
                return Response::PresenceEnded;
            }
            let remaining = deadline.saturating_duration_since(now);
            let ttl_remaining = idle_deadline.saturating_duration_since(now);
            sleep_until_next_poll_or_notify(
                store_notification,
                remaining.min(ttl_remaining).min(Duration::from_millis(100)),
            )
            .await;
        } else {
            let now = Instant::now();
            if now >= idle_deadline {
                state.mark_member_idle(
                    &store_key,
                    &session_id,
                    &address,
                    "IdleTtlReap",
                    "blocked wait exceeded idle TTL",
                );
                state.record_waiter_exit(
                    &store_key,
                    &session_id,
                    &address,
                    WaiterOutcome::PresenceEnded,
                    Some(5),
                    Some("idle-ttl-reap".to_string()),
                    waiter_pid_for_status,
                );
                return Response::PresenceEnded;
            }
            sleep_until_next_poll_or_notify(
                store_notification,
                idle_deadline
                    .saturating_duration_since(now)
                    .min(Duration::from_millis(250)),
            )
            .await;
        }
    }
}

async fn sleep_until_next_poll_or_notify(
    notification: Option<impl std::future::Future<Output = ()>>,
    duration: Duration,
) {
    if duration.is_zero() {
        return;
    }
    if let Some(notification) = notification {
        tokio::select! {
            _ = notification => {}
            _ = tokio::time::sleep(duration) => {}
        }
    } else {
        tokio::time::sleep(duration).await;
    }
}

fn wait_attention_matches(
    row_attention: &str,
    exact_attention: Option<&str>,
    min_attention: Option<Attention>,
) -> bool {
    if let Some(want) = exact_attention {
        if row_attention != want {
            return false;
        }
    }
    let Some(minimum) = min_attention else {
        return true;
    };
    Attention::parse(row_attention)
        .map(|actual| actual.meets_minimum(minimum))
        .unwrap_or(false)
}

async fn ack_message(
    state: Arc<DaemonState>,
    store_key: String,
    session_id: String,
    address: String,
    message_id: i64,
) -> Response {
    if state.is_draining() {
        return proto::error_response(proto::ERROR_NOT_RUNNING, "daemon is draining");
    }

    let backend = match state.backend_for(&store_key).await {
        Ok(backend) => backend,
        Err(response) => return response,
    };
    let member = match state.get_member(&store_key, &session_id, &address) {
        Some(member) => member,
        None => {
            match backend.detach_tombstone(&session_id, &address).await {
                Ok(Some(tombstone)) => {
                    return proto::needs_attach_with_reason(
                        format!(
                            "session {session_id} deliberately detached from {address} in {store_key} by {} at {}; explicit attach required",
                            tombstone.reason, tombstone.at_ms
                        ),
                        NeedsAttachReason::DeliberatelyDetached,
                    );
                }
                Ok(None) => {}
                Err(e) => {
                    return proto::internal(format!(
                        "checking detach tombstone for {session_id}/{address}: {e:#}"
                    ))
                }
            }
            if let Some(ended) = state.session_definite_end(&store_key, &session_id) {
                return proto::needs_attach_with_reason(
                    format!(
                        "session {session_id} was definitely ended by {} at {}; deliberate re-attach required for {address} in {store_key}",
                        ended.reason, ended.at_ms
                    ),
                    NeedsAttachReason::DeliberatelyDetached,
                );
            }
            state.push_recent_error(
                "NeedsAttach",
                format!("Ack NeedsAttach store={store_key} session={session_id} address={address} message_id={message_id}"),
            );
            match backend.has_delivery_for_recipient(message_id, &address).await {
                Ok(true) => {
                    return proto::needs_attach_with_reason(
                        format!(
                            "session {session_id} lost membership for pending message {message_id} to {address} in {store_key}; restart re-attach may recover"
                        ),
                        NeedsAttachReason::RestartLost,
                    )
                }
                Ok(false) => {
                    return proto::needs_attach(format!(
                        "session {session_id} is not attached to {address} in {store_key}"
                    ))
                }
                Err(e) => {
                    return proto::internal(format!(
                        "checking delivery recovery eligibility for {message_id}/{address}: {e:#}"
                    ))
                }
            }
        }
    };
    match backend
        .mark_consumed_if_current_owner(
            &address,
            &member.owner_instance_id,
            member.lease_epoch,
            message_id,
        )
        .await
    {
        Ok(outcome) => {
            if outcome == DeliveryOutcome::NotOwner {
                self_demote_member(
                    &state,
                    &member,
                    "ack mark_consumed_if_current_owner returned NotOwner",
                );
            }
            Response::Ack {
                message: Some("ack".to_string()),
                delivery_outcome: Some(outcome),
                address: Some(address),
                message_id: Some(message_id),
                lease_epoch: Some(member.lease_epoch),
            }
        }
        Err(e) => proto::unsupported(format!("acking message {message_id}: {e:#}")),
    }
}

fn validate_message_payload_size(
    body: &str,
    subject: Option<&str>,
    metadata: Option<&str>,
) -> std::result::Result<(), Response> {
    let bytes = body
        .len()
        .saturating_add(subject.map(str::len).unwrap_or(0))
        .saturating_add(metadata.map(str::len).unwrap_or(0));
    if bytes > proto::MAX_MESSAGE_BODY_METADATA_BYTES {
        return Err(proto::error_response(
            proto::ERROR_INCOMPATIBLE,
            format!(
                "message body/subject/metadata is {bytes} bytes; limit is {} bytes",
                proto::MAX_MESSAGE_BODY_METADATA_BYTES
            ),
        ));
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn send_message(
    state: Arc<DaemonState>,
    store_key: String,
    session_id: String,
    from_addr: Option<String>,
    to_addr: String,
    cc: Option<String>,
    kind: String,
    attention: String,
    requires_disposition: bool,
    subject: Option<String>,
    body: String,
    metadata: Option<String>,
) -> Response {
    let attention = match Attention::parse(&attention) {
        Ok(attention) => attention,
        Err(e) => return proto::incompatible(e.to_string()),
    };
    if let Err(response) =
        validate_message_payload_size(&body, subject.as_deref(), metadata.as_deref())
    {
        return response;
    }
    let backend = match state.backend_for(&store_key).await {
        Ok(backend) => backend,
        Err(response) => return response,
    };
    let from = match resolve_sender(
        &state,
        &backend,
        &store_key,
        &session_id,
        from_addr.as_deref(),
    )
    .await
    {
        Ok(from) => from,
        Err(response) => return response,
    };
    match backend.get_address(&to_addr).await {
        Ok(Some(addr)) if addr.status == STATUS_RETIRED => {
            return proto::error_response(
                proto::ERROR_INCOMPATIBLE,
                format!("address {to_addr} is retired"),
            )
        }
        Ok(Some(_)) => {}
        Ok(None) => {
            if let Err(e) = backend.ensure_address(&to_addr, None, None, None).await {
                return proto::internal(format!("ensuring destination {to_addr}: {e:#}"));
            }
        }
        Err(e) => return proto::internal(format!("checking destination {to_addr}: {e:#}")),
    }
    let new = NewMessage {
        parent_id: None,
        from_addr: Some(from.clone()),
        to_addr: to_addr.clone(),
        cc,
        kind,
        attention,
        requires_disposition,
        subject,
        body,
        metadata,
        sent_at_ms: now_ms(),
    };
    let row = match backend.insert_message(&new).await {
        Ok(row) => row,
        Err(e) => return proto::internal(format!("inserting message: {e:#}")),
    };
    if let Err(e) = backend.notify_new(&to_addr, row.id, row.sent_at_ms).await {
        state.push_recent_error(
            "NotifyDegraded",
            format!(
                "notify_new failed store={store_key} address={to_addr} message_id={}: {e:#}; polling fallback remains active",
                row.id
            ),
        );
    }
    state.note_backlog_for_unattended_address(&store_key, &to_addr);
    state.fire_on_deliver_on_commit(&store_key, &row);
    let occupied = state.has_address_member(&store_key, &to_addr);
    Response::Sent {
        receipt: SentReceipt {
            receipt: if occupied {
                "delivered".to_string()
            } else {
                "queued-unoccupied".to_string()
            },
            id: row.id,
            thread_id: row.thread_id,
            parent_id: row.parent_id,
            to: to_addr,
            from: Some(from),
            attention: Some(row.attention),
            requires_disposition: Some(row.requires_disposition),
            occupied: Some(occupied),
        },
    }
}

#[allow(clippy::too_many_arguments)]
async fn reply_message(
    state: Arc<DaemonState>,
    store_key: String,
    session_id: String,
    from_addr: Option<String>,
    message_id: i64,
    kind: String,
    attention: String,
    requires_disposition: bool,
    subject: Option<String>,
    cc: Option<String>,
    body: String,
) -> Response {
    let attention = match Attention::parse(&attention) {
        Ok(attention) => attention,
        Err(e) => return proto::incompatible(e.to_string()),
    };
    if let Err(response) = validate_message_payload_size(&body, subject.as_deref(), None) {
        return response;
    }
    let backend = match state.backend_for(&store_key).await {
        Ok(backend) => backend,
        Err(response) => return response,
    };
    let from = match resolve_sender(
        &state,
        &backend,
        &store_key,
        &session_id,
        from_addr.as_deref(),
    )
    .await
    {
        Ok(from) => from,
        Err(response) => return response,
    };
    let parent = match backend.get_message(message_id).await {
        Ok(Some(parent)) => parent,
        Ok(None) => {
            return proto::error_response(
                proto::ERROR_INCOMPATIBLE,
                format!("message {message_id} not found"),
            )
        }
        Err(e) => return proto::internal(format!("loading parent message {message_id}: {e:#}")),
    };
    let to = match parent.from_addr.clone() {
        Some(to) if !to.trim().is_empty() => to,
        _ => {
            return proto::error_response(
                proto::ERROR_INCOMPATIBLE,
                format!("message {message_id} has no from address to reply to"),
            )
        }
    };
    if let Err(e) = backend.ensure_address(&to, None, None, None).await {
        return proto::internal(format!("ensuring reply destination {to}: {e:#}"));
    }
    let subject = subject.or_else(|| parent.subject.as_ref().map(|s| format!("Re: {s}")));
    let new = NewMessage {
        parent_id: Some(parent.id),
        from_addr: Some(from.clone()),
        to_addr: to.clone(),
        cc,
        kind,
        attention,
        requires_disposition,
        subject,
        body,
        metadata: None,
        sent_at_ms: now_ms(),
    };
    let row = match backend.insert_message(&new).await {
        Ok(row) => row,
        Err(e) => return proto::internal(format!("inserting reply: {e:#}")),
    };
    if let Err(e) = backend.notify_new(&to, row.id, row.sent_at_ms).await {
        state.push_recent_error(
            "NotifyDegraded",
            format!(
                "notify_new failed store={store_key} address={to} message_id={}: {e:#}; polling fallback remains active",
                row.id
            ),
        );
    }
    state.note_backlog_for_unattended_address(&store_key, &to);
    state.fire_on_deliver_on_commit(&store_key, &row);
    let occupied = state.has_address_member(&store_key, &to);
    Response::Sent {
        receipt: SentReceipt {
            receipt: if occupied {
                "delivered".to_string()
            } else {
                "queued-unoccupied".to_string()
            },
            id: row.id,
            thread_id: row.thread_id,
            parent_id: row.parent_id,
            to,
            from: Some(from),
            attention: None,
            requires_disposition: None,
            occupied: Some(occupied),
        },
    }
}

async fn resolve_sender(
    state: &DaemonState,
    backend: &Arc<dyn Backend>,
    store_key: &str,
    session_id: &str,
    from_addr: Option<&str>,
) -> std::result::Result<String, Response> {
    let from_addr = from_addr.filter(|addr| !addr.trim().is_empty());
    if let Some(addr) = from_addr {
        if let Some(member) = state
            .get_member(store_key, session_id, addr)
            .filter(|m| !m.idle)
        {
            return Ok(member.address);
        }
        return Err(needs_attach_for_missing_member(
            state,
            backend,
            store_key,
            session_id,
            addr,
            "send-reply-explicit-from",
        )
        .await);
    }
    let members = state.session_members(store_key, session_id);
    match members.as_slice() {
        [] => {
            state.push_recent_error(
                "NeedsAttach",
                format!("Send/Reply NeedsAttach store={store_key} session={session_id}"),
            );
            Err(proto::needs_attach_with_reason(
                format!("session {session_id} has no attached address in {store_key}"),
                NeedsAttachReason::RestartLost,
            ))
        }
        [one] => Ok(one.address.clone()),
        many => Err(proto::ambiguous(format!(
            "session {session_id} attends multiple addresses in {store_key}: {}",
            many.iter()
                .map(|m| m.address.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ))),
    }
}

fn liveness_window_secs() -> i64 {
    std::env::var("TELEX_LIVENESS_WINDOW_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(15)
}

fn idle_ttl_duration() -> Duration {
    idle_ttl_duration_from_env(false)
}

fn idle_ttl_duration_from_env(allow_subday: bool) -> Duration {
    std::env::var("TELEX_IDLE_TTL_MS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .map(|ms| clamp_idle_ttl(Duration::from_millis(ms), allow_subday))
        .unwrap_or(DEFAULT_IDLE_TTL)
}

fn clamp_idle_ttl(duration: Duration, allow_subday: bool) -> Duration {
    if allow_subday || duration >= DEFAULT_IDLE_TTL {
        duration
    } else {
        DEFAULT_IDLE_TTL
    }
}

fn retention_warn_threshold() -> i64 {
    std::env::var("TELEX_RETENTION_WARN_ROWS")
        .ok()
        .and_then(|s| s.parse::<i64>().ok())
        .unwrap_or(DEFAULT_RETENTION_WARN_ROWS)
}

fn idle_station_warn_threshold() -> usize {
    std::env::var("TELEX_IDLE_STATION_WARN")
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(DEFAULT_IDLE_STATION_WARN)
}

fn deaf_warn_threshold_ms() -> i64 {
    std::env::var("TELEX_DEAF_WARN_MS")
        .ok()
        .and_then(|s| s.parse::<i64>().ok())
        .filter(|ms| *ms >= 0)
        .unwrap_or(DEFAULT_DEAF_WARN_MS)
}

#[cfg(all(test, feature = "sqlite"))]
mod p3_tests {
    use super::*;
    use crate::model::{DeliveryOutcome, NewMessage};
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEST_SEQ: AtomicU64 = AtomicU64::new(1);

    fn test_state(label: &str) -> Arc<DaemonState> {
        let seq = TEST_SEQ.fetch_add(1, Ordering::SeqCst);
        let root = std::env::current_dir()
            .unwrap()
            .join("target")
            .join("daemon-p3-tests")
            .join(format!("{label}-{seq}"));
        std::fs::create_dir_all(&root).unwrap();
        let singleton =
            SingletonKey::from_parts("test-user", root.join("config"), proto::PROTOCOL_MAJOR);
        Arc::new(DaemonState {
            paths: DaemonPaths::for_key(singleton, root.join("run")),
            instance_id: format!("inst-{label}-{seq}"),
            admin_cap: format!("cap-{label}-{seq}"),
            stores: Mutex::new(HashMap::new()),
            store_open_guard: AsyncMutex::new(()),
            members: Mutex::new(BTreeMap::new()),
            waiters: Mutex::new(BTreeMap::new()),
            next_waiter_id: AtomicU64::new(1),
            recent_errors: Arc::new(Mutex::new(VecDeque::new())),
            ended_sessions: Mutex::new(BTreeMap::new()),
            draining: AtomicBool::new(false),
            on_deliver: OnDeliverState::default(),
        })
    }

    fn store_key(label: &str) -> String {
        let seq = TEST_SEQ.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::current_dir()
            .unwrap()
            .join("target")
            .join("daemon-p3-tests")
            .join("stores");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("{label}-{}-{seq}.db", std::process::id()));
        format!("sqlite:{}", path.to_string_lossy())
    }

    fn legacy_null_epoch_store_key(label: &str) -> String {
        let seq = TEST_SEQ.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::current_dir()
            .unwrap()
            .join("target")
            .join("daemon-p3-tests")
            .join("stores");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("{label}-{}-{seq}.db", std::process::id()));
        let c = rusqlite::Connection::open(&path).unwrap();
        c.execute_batch(
            "CREATE TABLE leases (
                address           TEXT PRIMARY KEY,
                occupant          TEXT,
                host              TEXT,
                principal         TEXT,
                description       TEXT,
                tags              TEXT,
                scope             TEXT,
                pid               INTEGER,
                since_ms          INTEGER NOT NULL,
                heartbeat_at_ms   INTEGER NOT NULL,
                lease_epoch       INTEGER,
                owner_instance_id TEXT
            );
            INSERT INTO leases(address, occupant, host, principal, since_ms, heartbeat_at_ms, lease_epoch, owner_instance_id)
            VALUES ('addr:legacy', 'legacy-holder', 'host', 'principal', 10, 20, NULL, 'legacy-owner');",
        )
        .unwrap();
        format!("sqlite:{}", path.to_string_lossy())
    }

    fn register_req(store: &str, session: &str, address: &str) -> Request {
        Request::Register {
            store_key: store.to_string(),
            address: address.to_string(),
            session_id: session.to_string(),
            occupant: format!("occupant-{session}"),
            description: Some("test member".to_string()),
            scope: Some("scope:test".to_string()),
            tags: Some("p3".to_string()),
            watch_pids: vec![WatchPidSpec::anchor(42)],
            recovery: false,
            on_deliver: None,
            on_deliver_wake_on_cc: false,
        }
    }

    #[tokio::test]
    async fn register_stores_on_deliver_and_lists_candidate() {
        let state = test_state("on-deliver-register");
        let store = store_key("on-deliver-register");
        let mut req = register_req(&store, "s1", "addr:a");
        if let Request::Register { on_deliver, .. } = &mut req {
            *on_deliver = Some(vec!["handler".to_string(), "--flag".to_string()]);
        }
        let resp = request(state.clone(), req).await;
        assert!(matches!(resp, Response::Registered { .. }));
        let member = state.get_member(&store, "s1", "addr:a").unwrap();
        assert_eq!(
            member.on_deliver,
            Some(vec!["handler".to_string(), "--flag".to_string()])
        );
        let candidates = state.on_deliver_candidates(&store, "addr:a");
        assert_eq!(candidates.len(), 1);
        assert_eq!(
            candidates[0].argv,
            vec!["handler".to_string(), "--flag".to_string()]
        );
    }

    #[tokio::test]
    async fn register_without_on_deliver_has_no_candidates() {
        let state = test_state("on-deliver-none");
        let store = store_key("on-deliver-none");
        let resp = request(state.clone(), register_req(&store, "s1", "addr:a")).await;
        assert!(matches!(resp, Response::Registered { .. }));
        assert!(state.on_deliver_candidates(&store, "addr:a").is_empty());
    }

    #[test]
    fn on_deliver_descriptor_has_transport_fields() {
        let row = MessageRow {
            id: 5,
            thread_id: 2,
            parent_id: None,
            from_addr: Some("role:snd".to_string()),
            to_addr: "role:rcv".to_string(),
            cc: None,
            kind: "note".to_string(),
            attention: "interrupt".to_string(),
            requires_disposition: true,
            subject: Some("subj".to_string()),
            body: "hello body".to_string(),
            metadata: None,
            sent_at_ms: 0,
            created_at_ms: 0,
        };
        let json = on_deliver_descriptor_json("sqlite:/x", "role:rcv", &row);
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["message_id"], 5);
        assert_eq!(v["address"], "role:rcv");
        assert_eq!(v["delivered_to"], "role:rcv");
        assert_eq!(v["primary_to"], "role:rcv");
        assert_eq!(v["delivery_role"], "to");
        assert_eq!(v["from"], "role:snd");
        assert_eq!(v["attention"], "interrupt");
        assert_eq!(v["requires_disposition"], true);
        assert_eq!(v["requires_disposition_for_current_recipient"], true);
        assert_eq!(v["body"], "hello body");
    }

    fn exit_zero_argv() -> Vec<String> {
        #[cfg(windows)]
        {
            vec!["cmd".into(), "/c".into(), "exit".into(), "0".into()]
        }
        #[cfg(unix)]
        {
            vec!["sh".into(), "-c".into(), "exit 0".into()]
        }
    }

    fn exit_one_argv() -> Vec<String> {
        #[cfg(windows)]
        {
            vec!["cmd".into(), "/c".into(), "exit".into(), "1".into()]
        }
        #[cfg(unix)]
        {
            vec!["sh".into(), "-c".into(), "exit 1".into()]
        }
    }

    fn exit_three_argv() -> Vec<String> {
        #[cfg(windows)]
        {
            vec!["cmd".into(), "/c".into(), "exit".into(), "3".into()]
        }
        #[cfg(unix)]
        {
            vec!["sh".into(), "-c".into(), "exit 3".into()]
        }
    }

    fn exit_four_argv() -> Vec<String> {
        #[cfg(windows)]
        {
            vec!["cmd".into(), "/c".into(), "exit".into(), "4".into()]
        }
        #[cfg(unix)]
        {
            vec!["sh".into(), "-c".into(), "exit 4".into()]
        }
    }

    fn record_stdin_argv(path: &std::path::Path) -> Vec<String> {
        let path = path.to_string_lossy().to_string();
        #[cfg(windows)]
        {
            let escaped = path.replace('\'', "''");
            vec![
                "powershell".into(),
                "-NoProfile".into(),
                "-Command".into(),
                format!(
                    "[IO.File]::WriteAllText('{escaped}', [Console]::In.ReadToEnd(), [Text.UTF8Encoding]::new($false))"
                ),
            ]
        }
        #[cfg(unix)]
        {
            vec!["tee".into(), path]
        }
    }

    async fn insert_to(state: &Arc<DaemonState>, store: &str, address: &str) -> i64 {
        insert_message_to(state, store, address, None).await
    }

    async fn insert_message_to(
        state: &Arc<DaemonState>,
        store: &str,
        address: &str,
        cc: Option<&str>,
    ) -> i64 {
        let backend = match state.backend_for(store).await {
            Ok(backend) => backend,
            Err(e) => panic!("backend_for failed: {e:?}"),
        };
        backend
            .insert_message(&NewMessage {
                to_addr: address.to_string(),
                cc: cc.map(str::to_string),
                from_addr: Some("addr:snd".to_string()),
                kind: "note".to_string(),
                attention: Attention::Interrupt,
                body: "hello".to_string(),
                sent_at_ms: now_ms(),
                ..Default::default()
            })
            .await
            .expect("insert_message")
            .id
    }

    fn wait_for_file(path: &std::path::Path, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if path.exists() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        false
    }

    #[tokio::test]
    async fn on_deliver_fires_and_marks_pushed_on_success() {
        let state = test_state("on-deliver-fires");
        let store = store_key("on-deliver-fires");
        let mut req = register_req(&store, "rcv", "addr:rcv");
        if let Request::Register { on_deliver, .. } = &mut req {
            *on_deliver = Some(exit_zero_argv());
        }
        assert!(matches!(
            request(state.clone(), req).await,
            Response::Registered { .. }
        ));
        // A message that requires this recipient's disposition: an accepted-but-unacked push of it
        // stays re-pushable (Namra #1) so disposition-required work is not stranded within a
        // lifecycle. (No-disposition notes instead skip forever after accept — see
        // `no_disposition_message_skipped_forever_after_accept`.)
        let id = insert_requires_disposition_to(&state, &store, "addr:rcv").await;
        let row = state
            .backend_for(&store)
            .await
            .unwrap()
            .get_message(id)
            .await
            .unwrap()
            .unwrap();
        state.fire_on_deliver_on_commit(&store, &row);
        let member_key = MemberKey {
            store_key: store.clone(),
            session_id: "rcv".to_string(),
            address: "addr:rcv".to_string(),
        };
        let mut pushed = false;
        for _ in 0..100 {
            if state.on_deliver_should_skip(&member_key, id, Instant::now()) {
                pushed = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert!(
            pushed,
            "a successful on-deliver handler should record a push attempt (backed off)"
        );
        // Regression (Namra #1): a successful push is an ATTEMPT, not terminal suppression.
        // While the message stays undelivered/unacked it must become re-pushable after the
        // backoff window, so a crash/reload after accept-but-before-ack cannot strand it.
        assert!(
            !state.on_deliver_should_skip(
                &member_key,
                id,
                Instant::now() + Duration::from_secs(600)
            ),
            "an accepted-but-unacked message must be re-pushable after its backoff"
        );
    }

    #[tokio::test]
    async fn on_deliver_default_does_not_push_cc_observer() {
        let state = test_state("on-deliver-no-cc-default");
        let store = store_key("on-deliver-no-cc-default");
        let root = std::env::current_dir()
            .unwrap()
            .join("target")
            .join("daemon-p3-tests")
            .join("on-deliver-no-cc-default");
        std::fs::create_dir_all(&root).unwrap();
        let cc_descriptor = root.join("cc.json");
        let _ = std::fs::remove_file(&cc_descriptor);

        let mut primary = register_req(&store, "primary", "addr:primary");
        if let Request::Register { on_deliver, .. } = &mut primary {
            *on_deliver = Some(exit_zero_argv());
        }
        assert!(matches!(
            request(state.clone(), primary).await,
            Response::Registered { .. }
        ));
        let mut observer = register_req(&store, "observer", "addr:observer");
        if let Request::Register { on_deliver, .. } = &mut observer {
            *on_deliver = Some(record_stdin_argv(&cc_descriptor));
        }
        assert!(matches!(
            request(state.clone(), observer).await,
            Response::Registered { .. }
        ));

        let id = insert_message_to(&state, &store, "addr:primary", Some("addr:observer")).await;
        let row = state
            .backend_for(&store)
            .await
            .unwrap()
            .get_message(id)
            .await
            .unwrap()
            .unwrap();
        state.fire_on_deliver_on_commit(&store, &row);
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(
            !cc_descriptor.exists(),
            "CC observer should not receive push without wake-on-cc"
        );
    }

    #[tokio::test]
    async fn on_deliver_wake_on_cc_pushes_live_cc_without_replay() {
        let state = test_state("on-deliver-cc-wake");
        let store = store_key("on-deliver-cc-wake");
        let root = std::env::current_dir()
            .unwrap()
            .join("target")
            .join("daemon-p3-tests")
            .join("on-deliver-cc-wake");
        std::fs::create_dir_all(&root).unwrap();
        let descriptor_path = root.join("cc.json");
        let _ = std::fs::remove_file(&descriptor_path);

        // Historical CC is visible but predates the push lower bound captured below.
        let historical =
            insert_message_to(&state, &store, "addr:primary", Some("addr:observer")).await;

        let mut observer = register_req(&store, "observer", "addr:observer");
        if let Request::Register {
            on_deliver,
            on_deliver_wake_on_cc,
            ..
        } = &mut observer
        {
            *on_deliver = Some(record_stdin_argv(&descriptor_path));
            *on_deliver_wake_on_cc = true;
        }
        assert!(matches!(
            request(state.clone(), observer).await,
            Response::Registered { .. }
        ));
        let member = state
            .get_member(&store, "observer", "addr:observer")
            .unwrap();
        assert!(member.on_deliver_wake_on_cc);
        assert!(member.on_deliver_cc_after_ms.is_some());
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(
            !descriptor_path.exists(),
            "historical CC {historical} should not replay after push wake registration"
        );

        let live = insert_message_to(&state, &store, "addr:primary", Some("addr:observer")).await;
        let row = state
            .backend_for(&store)
            .await
            .unwrap()
            .get_message(live)
            .await
            .unwrap()
            .unwrap();
        let member_key = MemberKey {
            store_key: store.clone(),
            session_id: "observer".to_string(),
            address: "addr:observer".to_string(),
        };
        let member = state
            .get_member(&store, "observer", "addr:observer")
            .unwrap();
        assert!(
            row.created_at_ms > member.on_deliver_cc_after_ms.unwrap(),
            "live row {} must be newer than lower bound {:?}",
            row.created_at_ms,
            member.on_deliver_cc_after_ms
        );
        assert_eq!(state.on_deliver_cc_candidates(&store, &row).len(), 1);
        state.fire_on_deliver_on_commit(&store, &row);
        let mut attempted = false;
        for _ in 0..100 {
            if state.on_deliver_should_skip(&member_key, live, Instant::now()) {
                attempted = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        assert!(attempted, "live CC should record an on-deliver attempt");
        assert!(
            wait_for_file(&descriptor_path, Duration::from_secs(3)),
            "live CC should push to opted-in on-deliver handler"
        );
        let descriptor: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&descriptor_path).unwrap()).unwrap();
        assert_eq!(descriptor["message_id"], live);
        assert_eq!(descriptor["address"], "addr:observer");
        assert_eq!(descriptor["delivery_role"], "cc");
        assert_eq!(descriptor["primary_to"], "addr:primary");
        assert_eq!(
            descriptor["requires_disposition_for_current_recipient"],
            false
        );
        let advanced = state
            .get_member(&store, "observer", "addr:observer")
            .unwrap();
        assert!(
            advanced.on_deliver_cc_after_ms.unwrap() >= row.created_at_ms,
            "accepted CC notification should advance push lower bound"
        );

        std::fs::remove_file(&descriptor_path).unwrap();
        spawn_on_deliver_backlog(
            state.clone(),
            state
                .get_member(&store, "observer", "addr:observer")
                .unwrap(),
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(
            !descriptor_path.exists(),
            "accepted notification-only CC must not be replayed by backlog sweep"
        );

        let mut reprovision = register_req(&store, "observer", "addr:observer");
        if let Request::Register {
            on_deliver,
            on_deliver_wake_on_cc,
            ..
        } = &mut reprovision
        {
            *on_deliver = Some(record_stdin_argv(&descriptor_path));
            *on_deliver_wake_on_cc = true;
        }
        assert!(matches!(
            request(state.clone(), reprovision).await,
            Response::Registered { .. }
        ));
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(
            !descriptor_path.exists(),
            "re-provision should advance lower bound and not replay old CC"
        );
    }

    #[tokio::test]
    async fn on_deliver_failure_backs_off_but_stays_retryable() {
        let state = test_state("on-deliver-fails");
        let store = store_key("on-deliver-fails");
        let mut req = register_req(&store, "rcv", "addr:rcv");
        if let Request::Register { on_deliver, .. } = &mut req {
            *on_deliver = Some(exit_one_argv());
        }
        assert!(matches!(
            request(state.clone(), req).await,
            Response::Registered { .. }
        ));
        let id = insert_to(&state, &store, "addr:rcv").await;
        let row = state
            .backend_for(&store)
            .await
            .unwrap()
            .get_message(id)
            .await
            .unwrap()
            .unwrap();
        state.fire_on_deliver_on_commit(&store, &row);
        let member_key = MemberKey {
            store_key: store.clone(),
            session_id: "rcv".to_string(),
            address: "addr:rcv".to_string(),
        };
        // The failed attempt is recorded and backed off (no every-heartbeat hammering)...
        let mut recorded = false;
        for _ in 0..100 {
            if state.on_deliver_should_skip(&member_key, id, Instant::now()) {
                recorded = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert!(
            recorded,
            "a failed on-deliver attempt must be recorded and backed off"
        );
        // ...but a failed push stays retryable past the backoff window.
        assert!(
            !state.on_deliver_should_skip(
                &member_key,
                id,
                Instant::now() + Duration::from_secs(600)
            ),
            "a failed push must remain retryable after backoff"
        );
    }

    #[tokio::test]
    async fn on_deliver_forget_member_clears_pushed() {
        let state = test_state("on-deliver-forget");
        let store = store_key("on-deliver-forget");
        let member_key = MemberKey {
            store_key: store.clone(),
            session_id: "s1".to_string(),
            address: "addr:a".to_string(),
        };
        let now = Instant::now();
        state.on_deliver_record_attempt(&member_key, 7, now, false, false, false, None, false);
        assert!(state.on_deliver_should_skip(&member_key, 7, now));
        state.on_deliver_forget_member(&member_key);
        assert!(
            !state.on_deliver_should_skip(&member_key, 7, now),
            "forgetting a member must clear its push attempt state so a rebind re-pushes"
        );
    }

    // ---- #66 bridge-liveness / self-stop hardening regression tests ----------------------------

    async fn register_push_member(
        state: &Arc<DaemonState>,
        store: &str,
        session: &str,
        addr: &str,
    ) {
        let mut req = register_req(store, session, addr);
        if let Request::Register { on_deliver, .. } = &mut req {
            // Empty argv: the member is push_registered (on_deliver.is_some()) but the daemon never
            // execs a handler, so the register-time backlog sweep records no attempt and the test
            // fully controls the push-attempt map via `on_deliver_record_attempt`.
            *on_deliver = Some(Vec::new());
        }
        assert!(
            matches!(
                request(state.clone(), req).await,
                Response::Registered { .. }
            ),
            "push member should register"
        );
    }

    async fn insert_requires_disposition_to(
        state: &Arc<DaemonState>,
        store: &str,
        addr: &str,
    ) -> i64 {
        let backend = state.backend_for(store).await.unwrap();
        backend
            .insert_message(&NewMessage {
                to_addr: addr.to_string(),
                from_addr: Some("addr:peer".to_string()),
                kind: "note".to_string(),
                attention: Attention::Interrupt,
                requires_disposition: true,
                body: "please handle".to_string(),
                sent_at_ms: now_ms(),
                ..Default::default()
            })
            .await
            .expect("insert requires_disposition message")
            .id
    }

    fn member_status<'a>(status: &'a DaemonStatus, addr: &str) -> &'a MemberStatus {
        status
            .members
            .iter()
            .find(|m| m.address == addr && !m.idle)
            .expect("member present in status")
    }

    fn mk(store: &str, session: &str, addr: &str) -> MemberKey {
        MemberKey {
            store_key: store.to_string(),
            session_id: session.to_string(),
            address: addr.to_string(),
        }
    }

    /// A live push bridge (recent accepted push) with backlog is reported attended-via-push, never
    /// `unattended`/deaf. Folds in #64 and the persistent false-deaf of #66.
    #[tokio::test]
    async fn live_push_bridge_is_attended_not_deaf() {
        let state = test_state("push-attended");
        let store = store_key("push-attended");
        register_push_member(&state, &store, "s1", "addr:a").await;
        let id = insert_requires_disposition_to(&state, &store, "addr:a").await;
        state.on_deliver_record_attempt(
            &mk(&store, "s1", "addr:a"),
            id,
            Instant::now(),
            true,
            false,
            false,
            None,
            false,
        );

        let status = state.status().await;
        let m = member_status(&status, "addr:a");
        assert!(m.push_registered);
        assert_eq!(m.push_delivery, PushDeliveryHealth::Delivering);
        assert_eq!(m.station_health, StationHealth::AttendedPush);
        assert!(!m.deaf_warn, "a live push bridge must not be flagged deaf");
        assert!(m.deaf_since_ms.is_none());
        assert_eq!(m.pending_unconsumed_count, 1);
        assert_eq!(m.inbound_actionable_count, 1);
    }

    /// A push bridge whose pushes are failing (bridge unreachable) with backlog is deaf-eligible
    /// (the genuine dead-bridge / #62 case), and warns once past the threshold.
    #[tokio::test]
    async fn failing_push_bridge_becomes_deaf() {
        let state = test_state("push-failing");
        let store = store_key("push-failing");
        register_push_member(&state, &store, "s1", "addr:a").await;
        let id = insert_requires_disposition_to(&state, &store, "addr:a").await;
        state.on_deliver_record_attempt(
            &mk(&store, "s1", "addr:a"),
            id,
            Instant::now(),
            false,
            false,
            false,
            None,
            false,
        );

        let status = state.status().await;
        let m = member_status(&status, "addr:a");
        assert_eq!(m.push_delivery, PushDeliveryHealth::Failing);
        assert_eq!(m.station_health, StationHealth::UnattendedWithBacklog);
        assert!(
            m.deaf_since_ms.is_some(),
            "failing push sets the backlog timer"
        );

        // Backdate the backlog timer past the deaf threshold; still failing -> deaf_warn fires.
        {
            let mut members = state.members.lock().unwrap();
            let member = members.get_mut(&mk(&store, "s1", "addr:a")).unwrap();
            member.unattended_with_backlog_since_ms =
                Some(now_ms() - deaf_warn_threshold_ms() - 1_000);
        }
        let status2 = state.status().await;
        assert!(
            member_status(&status2, "addr:a").deaf_warn,
            "failing push past threshold is deaf"
        );
    }

    /// Bridge success is answerback: after a failing state, a subsequent accepted push clears the
    /// stale deaf/failing state and returns the station to attended-via-push.
    #[tokio::test]
    async fn accepted_push_clears_stale_failing_answerback() {
        let state = test_state("push-answerback");
        let store = store_key("push-answerback");
        register_push_member(&state, &store, "s1", "addr:a").await;
        let id = insert_requires_disposition_to(&state, &store, "addr:a").await;
        let key = mk(&store, "s1", "addr:a");
        state.on_deliver_record_attempt(&key, id, Instant::now(), false, false, false, None, false);
        // Backdate so it is deaf, then answerback.
        {
            let mut members = state.members.lock().unwrap();
            members
                .get_mut(&key)
                .unwrap()
                .unattended_with_backlog_since_ms =
                Some(now_ms() - deaf_warn_threshold_ms() - 1_000);
        }
        assert!(member_status(&state.status().await, "addr:a").deaf_warn);

        state.on_deliver_record_attempt(&key, id, Instant::now(), true, false, false, None, false);
        let status = state.status().await;
        let m = member_status(&status, "addr:a");
        assert_eq!(m.push_delivery, PushDeliveryHealth::Delivering);
        assert_eq!(m.station_health, StationHealth::AttendedPush);
        assert!(
            !m.deaf_warn,
            "an accepted push is answerback that clears stale deaf"
        );
        assert!(m.deaf_since_ms.is_none());
    }

    /// After a daemon restart the in-memory attempt map is empty, so a push station with backlog but
    /// no attempts yet reports `probing` (not confidently attended, not deaf) until the next sweep.
    #[tokio::test]
    async fn push_bridge_probing_when_no_attempts_recorded() {
        let state = test_state("push-probing");
        let store = store_key("push-probing");
        register_push_member(&state, &store, "s1", "addr:a").await;
        insert_requires_disposition_to(&state, &store, "addr:a").await;

        let status = state.status().await;
        let m = member_status(&status, "addr:a");
        assert_eq!(m.push_delivery, PushDeliveryHealth::Probing);
        assert_eq!(m.station_health, StationHealth::AttendedPush);
        assert!(
            !m.deaf_warn,
            "an un-probed push bridge must not be flagged deaf"
        );
        assert!(m.deaf_since_ms.is_none());
    }

    /// Regression: a pull station (no push handler) with backlog is unchanged — still
    /// `unattended_with_backlog`, and its push_delivery reports `not_registered`.
    #[tokio::test]
    async fn pull_station_backlog_unchanged() {
        let state = test_state("pull-backlog");
        let store = store_key("pull-backlog");
        assert!(matches!(
            request(state.clone(), register_req(&store, "s1", "addr:a")).await,
            Response::Registered { .. }
        ));
        insert_requires_disposition_to(&state, &store, "addr:a").await;

        let status = state.status().await;
        let m = member_status(&status, "addr:a");
        assert!(!m.push_registered);
        assert_eq!(m.push_delivery, PushDeliveryHealth::NotRegistered);
        assert_eq!(m.station_health, StationHealth::UnattendedWithBacklog);
        assert_eq!(m.pending_unconsumed_count, 1);
        assert_eq!(m.inbound_actionable_count, 1);
    }

    /// Status separates actionable inbound (requires this station's disposition) from raw pending,
    /// which also counts no-disposition notes.
    #[tokio::test]
    async fn status_distinguishes_actionable_inbound_from_pending() {
        let state = test_state("actionable-split");
        let store = store_key("actionable-split");
        register_push_member(&state, &store, "s1", "addr:a").await;
        insert_requires_disposition_to(&state, &store, "addr:a").await; // actionable + pending
        insert_to(&state, &store, "addr:a").await; // no-disposition note: pending only

        let status = state.status().await;
        let m = member_status(&status, "addr:a");
        assert_eq!(m.pending_unconsumed_count, 2);
        assert_eq!(m.inbound_actionable_count, 1);
    }

    /// A no-disposition (or CC) message is pushed once and never re-pushed once accepted, so
    /// informational traffic never enters an unbounded re-push pool.
    #[tokio::test]
    async fn no_disposition_message_skipped_forever_after_accept() {
        let state = test_state("no-disp-skip");
        let store = store_key("no-disp-skip");
        let key = mk(&store, "s1", "addr:a");
        let now = Instant::now();
        state.on_deliver_record_attempt(&key, 1, now, true, false, false, None, true);
        assert!(
            state.on_deliver_should_skip(
                &key,
                1,
                now + ON_DELIVER_ACCEPTED_BACKSTOP + Duration::from_secs(30)
            ),
            "an accepted no-disposition message is skipped forever, not re-pushed on the backstop"
        );
    }

    /// A still-unacked message is suppressed after the hard cap so it cannot be re-pushed forever;
    /// it is surfaced as a suppressed count, and a re-provision (forget) resets the budget.
    #[tokio::test]
    async fn repush_hard_cap_suppresses_and_resets_on_reprovision() {
        let state = test_state("repush-cap");
        let store = store_key("repush-cap");
        let key = mk(&store, "s1", "addr:a");
        let now = Instant::now();
        // One below the cap: still eligible once its backoff elapses.
        for _ in 0..(ON_DELIVER_MAX_REPUSH - 1) {
            state.on_deliver_record_attempt(&key, 5, now, false, false, false, None, false);
        }
        assert!(
            !state.on_deliver_should_skip(&key, 5, now + ON_DELIVER_ACCEPTED_BACKSTOP * 100),
            "one attempt below the hard cap should still be eligible after backoff"
        );
        assert_eq!(state.push_suppressed_count(&key), 0);
        // Reaching the cap suppresses further re-push.
        state.on_deliver_record_attempt(&key, 5, now, false, false, false, None, false);
        assert!(
            state.on_deliver_should_skip(&key, 5, now + ON_DELIVER_ACCEPTED_BACKSTOP * 100),
            "past the hard cap a message is suppressed regardless of elapsed time"
        );
        assert_eq!(state.push_suppressed_count(&key), 1);

        state.on_deliver_forget_member(&key);
        assert!(
            !state.on_deliver_should_skip(&key, 5, now),
            "a re-provision resets the attempt budget so the backlog re-delivers"
        );
        assert_eq!(state.push_suppressed_count(&key), 0);
    }

    /// Self-stop persistence: a deliberate detach (member present) writes the DURABLE tombstone the
    /// `telex copilot push` helper preflights, so self-stop survives a restart and is honored by a
    /// separate helper process — not just the in-memory definite-end.
    #[tokio::test]
    async fn deliberate_detach_writes_durable_tombstone() {
        let state = test_state("detach-durable-tombstone");
        let store = store_key("detach-durable-tombstone");
        register_push_member(&state, &store, "s1", "addr:a").await;
        let resp = request(
            state.clone(),
            Request::Detach {
                store_key: store.clone(),
                session_id: "s1".to_string(),
                address: "addr:a".to_string(),
            },
        )
        .await;
        assert!(
            matches!(resp, Response::Ack { .. }),
            "detach should ack, got {resp:?}"
        );
        let backend = state.backend_for(&store).await.unwrap();
        assert!(
            backend
                .detach_tombstone("s1", "addr:a")
                .await
                .unwrap()
                .is_some(),
            "a deliberate detach must durably tombstone the session/address for the push helper"
        );
    }

    /// Station stop reports whether the stopped station had a push bridge so the CLI can warn that
    /// the in-session bridge is still loaded (membership released != bridge unloaded).
    #[tokio::test]
    async fn station_stop_reports_push_registered_for_bridge_station() {
        let state = test_state("stop-warn-push");
        let store = store_key("stop-warn-push");
        register_push_member(&state, &store, "s1", "addr:a").await;
        let resp = request(
            state.clone(),
            Request::StationStop {
                store_key: store.clone(),
                session_id: "s1".to_string(),
                address: "addr:a".to_string(),
                wait_grace_ms: 100,
            },
        )
        .await;
        match resp {
            Response::StationStopped {
                detached,
                push_registered,
                ..
            } => {
                assert!(detached);
                assert!(
                    push_registered,
                    "station stop must report the bridge was registered"
                );
            }
            other => panic!("expected StationStopped, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn station_stop_reports_no_push_for_pull_station() {
        let state = test_state("stop-warn-pull");
        let store = store_key("stop-warn-pull");
        assert!(matches!(
            request(state.clone(), register_req(&store, "s1", "addr:a")).await,
            Response::Registered { .. }
        ));
        let resp = request(
            state.clone(),
            Request::StationStop {
                store_key: store.clone(),
                session_id: "s1".to_string(),
                address: "addr:a".to_string(),
                wait_grace_ms: 100,
            },
        )
        .await;
        match resp {
            Response::StationStopped {
                push_registered, ..
            } => {
                assert!(
                    !push_registered,
                    "a pull station has no push bridge to warn about"
                )
            }
            other => panic!("expected StationStopped, got {other:?}"),
        }
    }

    /// `push_delivery_health` classifies by the FRESHEST attempt across the member's messages, so a
    /// stale accept on one message cannot mask a fresh failure on another (deaf-detection latency).
    #[tokio::test]
    async fn push_delivery_health_uses_freshest_attempt() {
        let state = test_state("push-freshest");
        let store = store_key("push-freshest");
        let key = mk(&store, "s1", "addr:a");
        let base = Instant::now();
        // id 1 accepted at base; id 2 failed 1s later (fresher). Freshest is a failure -> Failing,
        // even though id 1's accept is still within its backstop.
        state.on_deliver_record_attempt(&key, 1, base, true, false, false, None, false);
        state.on_deliver_record_attempt(
            &key,
            2,
            base + Duration::from_secs(1),
            false,
            false,
            false,
            None,
            false,
        );
        assert_eq!(
            state.push_delivery_health(&key, 2, true, base + Duration::from_secs(2)),
            PushDeliveryHealth::Failing,
            "a fresh failure must not be masked by an older accept still inside its backstop"
        );
        // A newer accept on id 1 makes the freshest attempt an accept again -> Delivering.
        state.on_deliver_record_attempt(
            &key,
            1,
            base + Duration::from_secs(3),
            true,
            false,
            false,
            None,
            false,
        );
        assert_eq!(
            state.push_delivery_health(&key, 2, true, base + Duration::from_secs(4)),
            PushDeliveryHealth::Delivering
        );
    }

    /// Equal-timestamp attempts break the freshest tie toward a failure, so push health cannot flip
    /// nondeterministically from the unordered attempt map.
    #[tokio::test]
    async fn push_delivery_health_tie_breaks_toward_failure() {
        let state = test_state("push-tie");
        let store = store_key("push-tie");
        let key = mk(&store, "s1", "addr:a");
        let t = Instant::now();
        state.on_deliver_record_attempt(&key, 1, t, true, false, false, None, false);
        state.on_deliver_record_attempt(&key, 2, t, false, false, false, None, false);
        assert_eq!(
            state.push_delivery_health(&key, 2, true, t + Duration::from_secs(1)),
            PushDeliveryHealth::Failing,
            "a same-timestamp accept and failure must resolve to Failing deterministically"
        );
    }

    /// Push health ignores completed no-disposition/CC deliveries (accepted + skip_after_accept):
    /// a station whose only pending rows are such informational notes is not stale/probing, it has
    /// no outstanding push work.
    #[tokio::test]
    async fn push_delivery_health_ignores_completed_no_disposition() {
        let state = test_state("push-ignore-notes");
        let store = store_key("push-ignore-notes");
        let key = mk(&store, "s1", "addr:a");
        let base = Instant::now();
        // Accepted no-disposition note (skip_after_accept=true) is done work.
        state.on_deliver_record_attempt(&key, 1, base, true, false, false, None, true);
        // Even long after the backstop, it must NOT read as stale_accepted; there is no live work.
        assert_eq!(
            state.push_delivery_health(
                &key,
                1,
                true,
                base + ON_DELIVER_ACCEPTED_BACKSTOP + Duration::from_secs(30)
            ),
            PushDeliveryHealth::NoBacklog
        );
    }

    /// A re-provision bumps the member's push generation so a stale in-flight completion can be
    /// fenced (RD-2).
    #[tokio::test]
    async fn on_deliver_forget_member_bumps_generation() {
        let state = test_state("push-generation");
        let store = store_key("push-generation");
        let key = mk(&store, "s1", "addr:a");
        assert_eq!(state.on_deliver_generation(&key), 0);
        state.on_deliver_forget_member(&key);
        assert_eq!(state.on_deliver_generation(&key), 1);
        state.on_deliver_forget_member(&key);
        assert_eq!(state.on_deliver_generation(&key), 2);
    }

    /// End-to-end through `spawn_on_deliver`: a no-disposition note fired via the real on-deliver
    /// path is accepted once and then skipped forever (exercises the `skip_after_accept` computation
    /// in `spawn_on_deliver`, complementing the `on_deliver_record_attempt`-level unit test).
    #[tokio::test]
    async fn no_disposition_push_via_spawn_skips_forever() {
        let state = test_state("no-disp-spawn");
        let store = store_key("no-disp-spawn");
        let mut req = register_req(&store, "rcv", "addr:rcv");
        if let Request::Register { on_deliver, .. } = &mut req {
            *on_deliver = Some(exit_zero_argv());
        }
        assert!(matches!(
            request(state.clone(), req).await,
            Response::Registered { .. }
        ));
        // insert_to inserts a requires_disposition:false note.
        let id = insert_to(&state, &store, "addr:rcv").await;
        let row = state
            .backend_for(&store)
            .await
            .unwrap()
            .get_message(id)
            .await
            .unwrap()
            .unwrap();
        state.fire_on_deliver_on_commit(&store, &row);
        let member_key = mk(&store, "rcv", "addr:rcv");
        let mut accepted = false;
        for _ in 0..100 {
            if state.on_deliver_should_skip(&member_key, id, Instant::now()) {
                accepted = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert!(
            accepted,
            "the no-disposition note should be pushed and recorded accepted"
        );
        assert!(
            state.on_deliver_should_skip(
                &member_key,
                id,
                Instant::now() + Duration::from_secs(600)
            ),
            "an accepted no-disposition note is skipped forever, not re-pushed on the backstop"
        );
    }

    #[test]
    fn accepted_push_uses_long_backstop_failed_push_uses_fast_backoff() {
        let attempt = |accepted: bool, attempts: u32| PushAttempt {
            last: Instant::now(),
            attempts,
            accepted,
            deferred: false,
            notification_only: false,
            notification_lower_bound: None,
            skip_after_accept: false,
        };
        // A failed push retries on the fast, growing backoff so a dead bridge recovers quickly.
        assert_eq!(
            on_deliver_redelivery_delay(&attempt(false, 1)),
            ON_DELIVER_RETRY_BASE
        );
        assert_eq!(
            on_deliver_redelivery_delay(&attempt(false, 2)),
            ON_DELIVER_RETRY_BASE * 2
        );
        // An accepted push waits on the long backstop regardless of attempt count -- re-delivery
        // is otherwise re-provision-driven, not timer-driven.
        assert_eq!(
            on_deliver_redelivery_delay(&attempt(true, 1)),
            ON_DELIVER_ACCEPTED_BACKSTOP
        );
        assert_eq!(
            on_deliver_redelivery_delay(&attempt(true, 9)),
            ON_DELIVER_ACCEPTED_BACKSTOP
        );
        // The backstop is much longer than the fast failure backoff, so an accepted-but-unacked
        // message is not re-pushed on the fast churn cadence.
        assert!(ON_DELIVER_ACCEPTED_BACKSTOP > on_deliver_backoff(1));
    }

    #[tokio::test]
    async fn accepted_push_is_not_re_pushed_until_backstop_but_failed_push_is() {
        let state = test_state("on-deliver-accepted-backstop");
        let store = store_key("on-deliver-accepted-backstop");
        let member_key = MemberKey {
            store_key: store.clone(),
            session_id: "s1".to_string(),
            address: "addr:a".to_string(),
        };
        let now = Instant::now();
        // Accepted push: skipped for the whole backstop window; eligible only after it elapses.
        state.on_deliver_record_attempt(&member_key, 1, now, true, false, false, None, false);
        assert!(state.on_deliver_should_skip(&member_key, 1, now + ON_DELIVER_RETRY_BASE * 4));
        assert!(!state.on_deliver_should_skip(
            &member_key,
            1,
            now + ON_DELIVER_ACCEPTED_BACKSTOP + Duration::from_secs(1)
        ));
        // Failed push: eligible again as soon as the fast backoff elapses.
        state.on_deliver_record_attempt(&member_key, 2, now, false, false, false, None, false);
        assert!(state.on_deliver_should_skip(&member_key, 2, now));
        assert!(!state.on_deliver_should_skip(
            &member_key,
            2,
            now + ON_DELIVER_RETRY_BASE + Duration::from_secs(1)
        ));
    }

    #[tokio::test]
    async fn accepted_notification_only_push_is_not_replayed() {
        let state = test_state("on-deliver-cc-accepted-once");
        let store = store_key("on-deliver-cc-accepted-once");
        let member_key = MemberKey {
            store_key: store,
            session_id: "s1".to_string(),
            address: "addr:observer".to_string(),
        };
        let now = Instant::now();
        state.on_deliver_record_attempt(&member_key, 1, now, true, false, true, Some(1), true);
        assert!(state.on_deliver_should_skip(&member_key, 1, now));
        assert!(state.on_deliver_should_skip(
            &member_key,
            1,
            now + ON_DELIVER_ACCEPTED_BACKSTOP + Duration::from_secs(1)
        ));

        state.on_deliver_record_attempt(&member_key, 2, now, false, false, true, Some(2), true);
        assert!(state.on_deliver_should_skip(&member_key, 2, now));
        assert!(!state.on_deliver_should_skip(
            &member_key,
            2,
            now + ON_DELIVER_RETRY_BASE + Duration::from_secs(1)
        ));
    }

    #[tokio::test]
    async fn accepted_cc_push_does_not_advance_lower_bound_past_failed_cc() {
        let state = test_state("on-deliver-cc-failed-before-accepted");
        let store = store_key("on-deliver-cc-failed-before-accepted");
        let address = "addr:observer";
        let member_key = MemberKey {
            store_key: store.clone(),
            session_id: "s1".to_string(),
            address: address.to_string(),
        };
        let mut register = register_req(&store, "s1", address);
        if let Request::Register {
            on_deliver,
            on_deliver_wake_on_cc,
            ..
        } = &mut register
        {
            *on_deliver = Some(Vec::new());
            *on_deliver_wake_on_cc = true;
        }
        assert!(matches!(
            request(state.clone(), register).await,
            Response::Registered { .. }
        ));
        let initial_lower = state
            .get_member(&store, "s1", address)
            .unwrap()
            .on_deliver_cc_after_ms
            .unwrap();
        let first = insert_message_to(&state, &store, "addr:primary", Some(address)).await;
        let second = insert_message_to(&state, &store, "addr:primary", Some(address)).await;
        let backend = state.backend_for(&store).await.unwrap();
        let first_row = backend.get_message(first).await.unwrap().unwrap();
        let second_row = backend.get_message(second).await.unwrap().unwrap();
        assert!(first_row.created_at_ms > initial_lower);
        assert!(second_row.created_at_ms > first_row.created_at_ms);

        state.on_deliver_record_attempt(
            &member_key,
            first,
            Instant::now(),
            false,
            false,
            true,
            Some(first_row.created_at_ms),
            true,
        );
        state.on_deliver_record_attempt(
            &member_key,
            second,
            Instant::now(),
            true,
            false,
            true,
            Some(second_row.created_at_ms),
            true,
        );
        state.on_deliver_advance_cc_lower_bound(&member_key, second_row.created_at_ms);
        let member = state.get_member(&store, "s1", address).unwrap();
        assert!(
            member.on_deliver_cc_after_ms.unwrap() < first_row.created_at_ms,
            "lower bound must not advance past an outstanding failed notification"
        );
        let candidates = backend
            .fetch_wait_candidates(
                address,
                WaitFetchOptions {
                    wake_on_cc: true,
                    cc_after_ms: member.on_deliver_cc_after_ms.unwrap(),
                },
            )
            .await
            .unwrap();
        let candidate_ids: BTreeSet<i64> = candidates
            .iter()
            .map(|candidate| candidate.message.id)
            .collect();
        assert!(candidate_ids.contains(&first));
        assert!(candidate_ids.contains(&second));
        state.on_deliver_retain_pushed(&member_key, &candidate_ids);
        assert!(!state.on_deliver_should_skip(
            &member_key,
            first,
            Instant::now() + ON_DELIVER_RETRY_BASE + Duration::from_secs(1)
        ));
        assert!(state.on_deliver_should_skip(
            &member_key,
            second,
            Instant::now() + ON_DELIVER_ACCEPTED_BACKSTOP + Duration::from_secs(1)
        ));

        state.on_deliver_record_attempt(
            &member_key,
            first,
            Instant::now(),
            true,
            false,
            true,
            Some(first_row.created_at_ms),
            true,
        );
        state.on_deliver_advance_cc_lower_bound(&member_key, first_row.created_at_ms);
        let advanced = state.get_member(&store, "s1", address).unwrap();
        assert_eq!(
            advanced.on_deliver_cc_after_ms,
            Some(second_row.created_at_ms)
        );
        let remaining = backend
            .fetch_wait_candidates(
                address,
                WaitFetchOptions {
                    wake_on_cc: true,
                    cc_after_ms: advanced.on_deliver_cc_after_ms.unwrap(),
                },
            )
            .await
            .unwrap();
        assert!(
            remaining.is_empty(),
            "accepted notification-only CC rows should leave the sweep set once no failed earlier row blocks advancement"
        );
    }

    #[test]
    fn on_deliver_backoff_grows_and_caps() {
        assert_eq!(on_deliver_backoff(1), ON_DELIVER_RETRY_BASE);
        assert!(on_deliver_backoff(2) > on_deliver_backoff(1));
        assert!(on_deliver_backoff(3) > on_deliver_backoff(2));
        assert_eq!(on_deliver_backoff(100), ON_DELIVER_RETRY_MAX);
        assert!(on_deliver_backoff(6) <= ON_DELIVER_RETRY_MAX);
    }

    #[tokio::test]
    async fn on_deliver_permanent_exit_dead_letters_and_stops_retrying() {
        let state = test_state("on-deliver-deadletter");
        let store = store_key("on-deliver-deadletter");
        let mut req = register_req(&store, "rcv", "addr:rcv");
        if let Request::Register { on_deliver, .. } = &mut req {
            *on_deliver = Some(exit_three_argv());
        }
        assert!(matches!(
            request(state.clone(), req).await,
            Response::Registered { .. }
        ));
        let id = insert_to(&state, &store, "addr:rcv").await;
        let row = state
            .backend_for(&store)
            .await
            .unwrap()
            .get_message(id)
            .await
            .unwrap()
            .unwrap();
        state.fire_on_deliver_on_commit(&store, &row);
        let member_key = MemberKey {
            store_key: store.clone(),
            session_id: "rcv".to_string(),
            address: "addr:rcv".to_string(),
        };
        // Wait for the permanent-exit handler to run and dead-letter the message.
        let mut dead = false;
        for _ in 0..100 {
            if state.on_deliver_should_skip(&member_key, id, Instant::now()) {
                dead = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert!(
            dead,
            "a permanent-exit handler must dead-letter the message"
        );
        // Unlike a transient failure (which becomes retryable past its backoff), a dead-lettered
        // message stays skipped indefinitely -- no more futile retries (Namra push oversize).
        assert!(
            state.on_deliver_should_skip(
                &member_key,
                id,
                Instant::now() + Duration::from_secs(86400)
            ),
            "a dead-lettered message must stay skipped (not retried) indefinitely"
        );
    }

    // ---- issue #65: defer-until-idle daemon accounting + idle drain ----

    #[test]
    fn on_deliver_backstop_invariants() {
        // A deferred (busy) message is re-checked no faster than the heartbeat sweep (so a busy
        // bridge is not re-hit every tick) and sooner than a genuinely-queued accepted turn.
        assert!(ON_DELIVER_DEFERRED_BACKSTOP >= HEARTBEAT_INTERVAL);
        assert!(ON_DELIVER_DEFERRED_BACKSTOP < ON_DELIVER_ACCEPTED_BACKSTOP);
    }

    #[test]
    fn deferred_redelivery_delay_is_the_deferred_backstop() {
        let deferred = PushAttempt {
            last: Instant::now(),
            attempts: 0,
            accepted: false,
            deferred: true,
            notification_only: false,
            notification_lower_bound: None,
            skip_after_accept: false,
        };
        assert_eq!(
            on_deliver_redelivery_delay(&deferred),
            ON_DELIVER_DEFERRED_BACKSTOP
        );
    }

    #[tokio::test]
    async fn deferred_attempt_holds_at_backstop_and_stays_off_degraded_counter() {
        let state = test_state("on-deliver-deferred-acct");
        let store = store_key("on-deliver-deferred-acct");
        let member_key = MemberKey {
            store_key: store.clone(),
            session_id: "s1".to_string(),
            address: "addr:a".to_string(),
        };
        let now = Instant::now();
        // Re-deferring across a long busy turn must not accumulate the attempt counter (so the
        // degraded-status threshold never trips) -- deferring while a turn runs is normal.
        let mut attempts_seen = 0u32;
        for _ in 0..(ON_DELIVER_DEGRADED_AFTER + 3) {
            attempts_seen = state.on_deliver_record_attempt(
                &member_key,
                1,
                now,
                false,
                true,
                false,
                None,
                false,
            );
        }
        assert_eq!(
            attempts_seen, 0,
            "a deferred push must not increment the degraded-status attempt counter"
        );
        assert_eq!(state.on_deliver_deferred_count(&member_key), 1);
        // Held within the deferred backstop; eligible after it (bounded fallback if drain missed).
        assert!(state.on_deliver_should_skip(&member_key, 1, now + Duration::from_secs(5)));
        assert!(!state.on_deliver_should_skip(
            &member_key,
            1,
            now + ON_DELIVER_DEFERRED_BACKSTOP + Duration::from_secs(1)
        ));
    }

    #[tokio::test]
    async fn deferred_exit_records_deferred_outcome() {
        let state = test_state("on-deliver-deferred-exit");
        let store = store_key("on-deliver-deferred-exit");
        let mut req = register_req(&store, "rcv", "addr:rcv");
        if let Request::Register { on_deliver, .. } = &mut req {
            *on_deliver = Some(exit_four_argv());
        }
        assert!(matches!(
            request(state.clone(), req).await,
            Response::Registered { .. }
        ));
        let id = insert_to(&state, &store, "addr:rcv").await;
        let row = state
            .backend_for(&store)
            .await
            .unwrap()
            .get_message(id)
            .await
            .unwrap()
            .unwrap();
        state.fire_on_deliver_on_commit(&store, &row);
        let member_key = MemberKey {
            store_key: store.clone(),
            session_id: "rcv".to_string(),
            address: "addr:rcv".to_string(),
        };
        let mut deferred = false;
        for _ in 0..100 {
            if state.on_deliver_deferred_count(&member_key) == 1 {
                deferred = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert!(
            deferred,
            "an ON_DELIVER_DEFERRED_EXIT handler must record a deferred push attempt"
        );
        // Deferred is neither accepted (long backstop) nor a failure (fast backoff): it holds for
        // exactly the deferred backstop and is not treated as degraded.
        assert!(state.on_deliver_should_skip(&member_key, id, Instant::now()));
        assert!(!state.on_deliver_should_skip(
            &member_key,
            id,
            Instant::now() + ON_DELIVER_DEFERRED_BACKSTOP + Duration::from_secs(1)
        ));
    }

    #[tokio::test]
    async fn drain_clears_deferred_only_not_accepted() {
        let state = test_state("drain-clears-deferred");
        let store = store_key("drain-clears-deferred");
        let member_key = MemberKey {
            store_key: store.clone(),
            session_id: "s1".to_string(),
            address: "addr:a".to_string(),
        };
        let now = Instant::now();
        // id 1 deferred (bridge was busy), id 2 accepted (a genuinely queued turn).
        state.on_deliver_record_attempt(&member_key, 1, now, false, true, false, None, false);
        state.on_deliver_record_attempt(&member_key, 2, now, true, false, false, None, false);
        assert!(state.on_deliver_should_skip(&member_key, 1, now));
        assert!(state.on_deliver_should_skip(&member_key, 2, now));

        let cleared = state.on_deliver_clear_deferred(&member_key);
        assert_eq!(cleared, 1, "only the deferred attempt should be cleared");
        // The deferred message becomes eligible for immediate re-push; the accepted (queued) turn
        // is left untouched so the drain never re-injects a duplicate of a queued turn.
        assert!(
            !state.on_deliver_should_skip(&member_key, 1, now),
            "a cleared deferred message must be eligible for re-push"
        );
        assert!(
            state.on_deliver_should_skip(&member_key, 2, now),
            "an accepted queued turn must NOT be re-pushed by the drain"
        );
        assert_eq!(state.on_deliver_deferred_count(&member_key), 0);
    }

    // Repro for the discovered bug (issue #65 acceptance): a message deferred while busy, then
    // manually read + acked before the turn stops, must NOT be re-injected by the idle drain.
    // Deterministic: the drain's re-sweep re-derives the pushable set from durable state via
    // `fetch_wait_candidates`, so the guarantee is that after ack + drain the acked message is not a
    // candidate (cannot be pushed) while a still-unacked one is. This avoids racing the async
    // subprocess sweep (whose completion is covered end-to-end by the repushes-unacked test).
    #[tokio::test]
    async fn drain_deferred_skips_message_acked_before_idle() {
        let state = test_state("drain-skips-acked");
        let store = store_key("drain-skips-acked");

        let mut req = register_req(&store, "s1", "addr:a");
        if let Request::Register { on_deliver, .. } = &mut req {
            *on_deliver = Some(exit_zero_argv());
        }
        assert!(matches!(
            request(state.clone(), req).await,
            Response::Registered { .. }
        ));
        let member_key = MemberKey {
            store_key: store.clone(),
            session_id: "s1".to_string(),
            address: "addr:a".to_string(),
        };
        // Two messages arrive while busy and are deferred; `acked_id` is manually read + acked
        // before the turn stops, `live_id` stays unacked.
        let acked_id = insert_to(&state, &store, "addr:a").await;
        let live_id = insert_to(&state, &store, "addr:a").await;
        let now = Instant::now();
        state.on_deliver_record_attempt(
            &member_key,
            acked_id,
            now,
            false,
            true,
            false,
            None,
            false,
        );
        state.on_deliver_record_attempt(&member_key, live_id, now, false, true, false, None, false);
        let acked = request(state.clone(), ack_req(&store, "s1", "addr:a", acked_id)).await;
        assert!(
            matches!(
                acked,
                Response::Ack {
                    delivery_outcome: Some(DeliveryOutcome::Marked),
                    ..
                }
            ),
            "ack must durably consume the message, got {acked:?}"
        );
        // Turn stops -> idle drain: clears the deferred skip and queues the revalidating re-sweep.
        let drained = request(
            state.clone(),
            Request::DrainDeferred {
                store_key: store.clone(),
                session_id: "s1".to_string(),
                proof: Some(state.admin_cap.clone()),
            },
        )
        .await;
        assert!(matches!(drained, Response::Ack { .. }));
        assert_eq!(
            state.on_deliver_deferred_count(&member_key),
            0,
            "the drain must clear the deferred skip for both messages"
        );
        // The re-sweep's source of truth: the acked message is no longer a pushable candidate, so it
        // can never be re-injected as a stale turn; the still-unacked one remains eligible.
        let backend = state.backend_for(&store).await.unwrap();
        let candidates = backend
            .fetch_wait_candidates(
                "addr:a",
                WaitFetchOptions {
                    wake_on_cc: false,
                    cc_after_ms: 0,
                },
            )
            .await
            .unwrap();
        let candidate_ids: BTreeSet<i64> = candidates.iter().map(|c| c.message.id).collect();
        assert!(
            !candidate_ids.contains(&acked_id),
            "a message acked before turn-stop must not be a drain re-sweep candidate"
        );
        assert!(
            candidate_ids.contains(&live_id),
            "a still-unacked deferred message must remain a drain re-sweep candidate"
        );
    }

    // A message deferred while busy and NOT consumed is delivered after the turn stops (idle drain).
    #[tokio::test]
    async fn drain_deferred_repushes_unacked_after_turn_stop() {
        let state = test_state("drain-repushes-unacked");
        let store = store_key("drain-repushes-unacked");
        let root = std::env::current_dir()
            .unwrap()
            .join("target")
            .join("daemon-p3-tests")
            .join("drain-repushes-unacked-marker");
        std::fs::create_dir_all(&root).unwrap();
        let marker = root.join("pushed.json");
        let _ = std::fs::remove_file(&marker);

        let mut req = register_req(&store, "s1", "addr:a");
        if let Request::Register { on_deliver, .. } = &mut req {
            *on_deliver = Some(record_stdin_argv(&marker));
        }
        assert!(matches!(
            request(state.clone(), req).await,
            Response::Registered { .. }
        ));
        let id = insert_to(&state, &store, "addr:a").await;
        let member_key = MemberKey {
            store_key: store.clone(),
            session_id: "s1".to_string(),
            address: "addr:a".to_string(),
        };
        // Deferred while busy, never manually consumed.
        state.on_deliver_record_attempt(
            &member_key,
            id,
            Instant::now(),
            false,
            true,
            false,
            None,
            false,
        );
        assert!(
            state.on_deliver_should_skip(&member_key, id, Instant::now()),
            "a freshly-deferred message is held until the drain (or the deferred backstop)"
        );
        // Turn stops -> idle drain clears the deferred skip and re-sweeps; the bridge is idle now,
        // so the message is pushed.
        let drained = request(
            state.clone(),
            Request::DrainDeferred {
                store_key: store.clone(),
                session_id: "s1".to_string(),
                proof: Some(state.admin_cap.clone()),
            },
        )
        .await;
        assert!(matches!(drained, Response::Ack { .. }));
        // Async poll (yields to the runtime so the spawned sweep/child-process can progress; a
        // blocking wait would starve the current-thread executor).
        let mut pushed = false;
        for _ in 0..100 {
            if marker.exists() {
                pushed = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert!(
            pushed,
            "idle drain must re-push a still-unacked deferred message after the turn stops"
        );
    }

    #[tokio::test]
    async fn drain_deferred_requires_admin_cap() {
        let state = test_state("drain-cap");
        let store = store_key("drain-cap");
        let denied = request(
            state.clone(),
            Request::DrainDeferred {
                store_key: store.clone(),
                session_id: "s1".to_string(),
                proof: Some("wrong-cap".to_string()),
            },
        )
        .await;
        assert!(
            matches!(denied, Response::Error { .. }),
            "DrainDeferred must reject a bad admin cap"
        );
    }

    // A session attached with a named --backend/--db resolves a store the static drain hook does not
    // know; drain must still find its members by session id across stores (PAW review should-fix).
    #[tokio::test]
    async fn drain_deferred_matches_members_across_stores() {
        let state = test_state("drain-cross-store");
        let store = store_key("drain-cross-store");
        let mut req = register_req(&store, "s1", "addr:a");
        if let Request::Register { on_deliver, .. } = &mut req {
            *on_deliver = Some(exit_zero_argv());
        }
        assert!(matches!(
            request(state.clone(), req).await,
            Response::Registered { .. }
        ));
        let member_key = MemberKey {
            store_key: store.clone(),
            session_id: "s1".to_string(),
            address: "addr:a".to_string(),
        };
        let id = insert_to(&state, &store, "addr:a").await;
        state.on_deliver_record_attempt(
            &member_key,
            id,
            Instant::now(),
            false,
            true,
            false,
            None,
            false,
        );
        assert_eq!(state.on_deliver_deferred_count(&member_key), 1);

        // Drain with a DIFFERENT store_key (as the ambient default would be for a named-backend
        // session). The daemon matches by session id across stores, so the member is still drained.
        let drained = request(
            state.clone(),
            Request::DrainDeferred {
                store_key: "sqlite:/some/other/store.db".to_string(),
                session_id: "s1".to_string(),
                proof: Some(state.admin_cap.clone()),
            },
        )
        .await;
        assert!(matches!(drained, Response::Ack { .. }));
        assert_eq!(
            state.on_deliver_deferred_count(&member_key),
            0,
            "drain must clear the member's deferred skip even when resolved via a different store"
        );
    }

    #[tokio::test]
    async fn drain_deferred_bumps_generation_and_forget_clears_it() {
        let state = test_state("drain-gen");
        let store = store_key("drain-gen");
        let mut req = register_req(&store, "s1", "addr:a");
        if let Request::Register { on_deliver, .. } = &mut req {
            *on_deliver = Some(exit_zero_argv());
        }
        assert!(matches!(
            request(state.clone(), req).await,
            Response::Registered { .. }
        ));
        let member_key = MemberKey {
            store_key: store.clone(),
            session_id: "s1".to_string(),
            address: "addr:a".to_string(),
        };
        assert_eq!(state.on_deliver_drain_gen(&member_key), 0);
        // Each drain advances the generation so an inflight push can detect a drain it raced.
        for expected in 1..=2u64 {
            let _ = request(
                state.clone(),
                Request::DrainDeferred {
                    store_key: store.clone(),
                    session_id: "s1".to_string(),
                    proof: Some(state.admin_cap.clone()),
                },
            )
            .await;
            assert_eq!(state.on_deliver_drain_gen(&member_key), expected);
        }
        // Re-provision forgets the generation along with the rest of the member's push state.
        state.on_deliver_forget_member(&member_key);
        assert_eq!(state.on_deliver_drain_gen(&member_key), 0);
    }

    #[tokio::test]
    async fn register_refresh_with_none_preserves_push_handler() {
        let state = test_state("on-deliver-preserve");
        let store = store_key("on-deliver-preserve");
        // Provision a push handler.
        let mut req = register_req(&store, "rcv", "addr:rcv");
        if let Request::Register { on_deliver, .. } = &mut req {
            *on_deliver = Some(exit_zero_argv());
        }
        assert!(matches!(
            request(state.clone(), req).await,
            Response::Registered { .. }
        ));
        assert!(state
            .get_member(&store, "rcv", "addr:rcv")
            .unwrap()
            .on_deliver
            .is_some());
        // A generic refresh (recovery/pull re-attach) with on_deliver = None must NOT wipe it
        // (Namra #6). `register_req` defaults on_deliver to None.
        let refresh = register_req(&store, "rcv", "addr:rcv");
        assert!(matches!(
            request(state.clone(), refresh).await,
            Response::Registered { .. }
        ));
        assert!(
            state
                .get_member(&store, "rcv", "addr:rcv")
                .unwrap()
                .on_deliver
                .is_some(),
            "a refresh with on_deliver=None must preserve the existing bridge handler"
        );
    }

    fn wait_req(store: &str, session: &str, address: &str, timeout_ms: u64) -> Request {
        Request::Wait {
            store_key: store.to_string(),
            session_id: session.to_string(),
            address: address.to_string(),
            attention: None,
            min_attention: None,
            wake_on_cc: false,
            timeout_ms: Some(timeout_ms),
            waiter_pid: Some(std::process::id()),
            waiter_start_time: crate::session_watch::capture_process_start_time(std::process::id()),
        }
    }

    fn session_end_req(state: &DaemonState, store: &str, session: &str) -> Request {
        Request::SessionEnd {
            store_key: store.to_string(),
            session_id: session.to_string(),
            proof: Some(state.admin_cap.clone()),
        }
    }

    fn reset_req(state: &DaemonState, store: &str, address: &str) -> Request {
        Request::Reset {
            store_key: store.to_string(),
            address: address.to_string(),
            proof: Some(state.admin_cap.clone()),
        }
    }

    fn send_req(store: &str, session: &str, from_addr: Option<&str>) -> Request {
        Request::Send {
            store_key: store.to_string(),
            session_id: session.to_string(),
            from_addr: from_addr.map(str::to_string),
            to_addr: "dest".to_string(),
            cc: None,
            kind: "note".to_string(),
            attention: "background".to_string(),
            requires_disposition: false,
            subject: None,
            body: "body".to_string(),
            metadata: None,
        }
    }

    async fn request(state: Arc<DaemonState>, request: Request) -> Response {
        handle_request(state, request).await.0
    }

    #[tokio::test]
    async fn register_creates_membership_and_wait_unknown_needs_attach() {
        let state = test_state("register-wait");
        let store = store_key("register-wait");

        let unknown = request(state.clone(), wait_req(&store, "s1", "addr:a", 1)).await;
        assert!(matches!(
            unknown,
            Response::Error { ref code, .. } if code == proto::ERROR_NEEDS_ATTACH
        ));

        let registered = request(state.clone(), register_req(&store, "s1", "addr:a")).await;
        let epoch = match registered {
            Response::Registered {
                lease_epoch,
                owner_instance_id,
            } => {
                assert_eq!(owner_instance_id, state.instance_id);
                lease_epoch
            }
            other => panic!("expected Registered, got {other:?}"),
        };
        assert!(epoch > 0);

        let status = state.status().await;
        assert_eq!(status.members.len(), 1);
        assert_eq!(status.members[0].address, "addr:a");
        assert_eq!(status.members[0].lease_epoch, epoch);
        assert_eq!(status.stores.len(), 1);

        let timed_out = request(state, wait_req(&store, "s1", "addr:a", 1)).await;
        assert!(matches!(timed_out, Response::Timeout));
    }

    #[tokio::test]
    async fn detach_removes_membership_and_does_not_resurrect() {
        let state = test_state("detach");
        let store = store_key("detach");
        assert!(matches!(
            request(state.clone(), register_req(&store, "s1", "addr:a")).await,
            Response::Registered { .. }
        ));

        assert!(matches!(
            request(
                state.clone(),
                Request::Detach {
                    store_key: store.clone(),
                    session_id: "s1".to_string(),
                    address: "addr:a".to_string(),
                },
            )
            .await,
            Response::Ack { .. }
        ));
        assert!(state.status().await.members.is_empty());

        let wait_after_detach = request(state.clone(), wait_req(&store, "s1", "addr:a", 1)).await;
        assert!(matches!(
            wait_after_detach,
            Response::Error {
                ref code,
                needs_attach_reason: Some(NeedsAttachReason::DeliberatelyDetached),
                ..
            } if code == proto::ERROR_NEEDS_ATTACH
        ));

        let ack_after_detach = request(
            state.clone(),
            Request::Ack {
                store_key: store,
                session_id: "s1".to_string(),
                address: "addr:a".to_string(),
                message_id: 123,
            },
        )
        .await;
        assert!(matches!(
            ack_after_detach,
            Response::Error { ref code, .. } if code == proto::ERROR_NEEDS_ATTACH
        ));
        assert!(state.status().await.members.is_empty());
    }

    #[tokio::test]
    async fn ack_after_restart_lost_membership_can_reattach_and_mark() {
        // In-process state replacement models daemon restart deterministically; full IPC
        // multi-process restart coverage remains an integration harness axis.
        let store = store_key("ack-restart-lost");
        let message_id;
        {
            let state = test_state("ack-restart-lost-one");
            registered_epoch(state.clone(), &store, "s1", "addr:a").await;
            let backend = state.backend_for(&store).await.unwrap();
            message_id = insert_test_message(&backend, "addr:a", None).await;
            let (drain, action) = handle_request(
                state.clone(),
                Request::Drain {
                    proof: Some(state.admin_cap.clone()),
                },
            )
            .await;
            assert!(matches!(drain, Response::Ack { .. }));
            assert!(matches!(action, ClientAction::Drain));
        }

        let restarted = test_state("ack-restart-lost-two");
        let first_ack = request(
            restarted.clone(),
            Request::Ack {
                store_key: store.clone(),
                session_id: "s1".to_string(),
                address: "addr:a".to_string(),
                message_id,
            },
        )
        .await;
        assert!(matches!(
            first_ack,
            Response::Error {
                ref code,
                needs_attach_reason: Some(NeedsAttachReason::RestartLost),
                ..
            } if code == proto::ERROR_NEEDS_ATTACH
        ));

        registered_epoch(restarted.clone(), &store, "s1", "addr:a").await;
        let second_ack = request(restarted, ack_req(&store, "s1", "addr:a", message_id)).await;
        assert!(matches!(
            second_ack,
            Response::Ack {
                delivery_outcome: Some(DeliveryOutcome::Marked),
                ..
            }
        ));
    }

    #[tokio::test]
    async fn ack_after_durable_detach_tombstone_stays_terminal_across_restart() {
        // The durable tombstone is asserted across a fresh DaemonState; a real daemon process
        // restart exercises the same SQLite marker through the IPC harness.
        let store = store_key("ack-detach-tombstone");
        let message_id;
        {
            let state = test_state("ack-detach-tombstone-one");
            registered_epoch(state.clone(), &store, "s1", "addr:a").await;
            let backend = state.backend_for(&store).await.unwrap();
            message_id = insert_test_message(&backend, "addr:a", None).await;
            assert!(matches!(
                request(
                    state,
                    Request::Detach {
                        store_key: store.clone(),
                        session_id: "s1".to_string(),
                        address: "addr:a".to_string(),
                    },
                )
                .await,
                Response::Ack { .. }
            ));
        }

        let restarted = test_state("ack-detach-tombstone-two");
        let ack = request(
            restarted.clone(),
            ack_req(&store, "s1", "addr:a", message_id),
        )
        .await;
        assert!(matches!(
            ack,
            Response::Error {
                ref code,
                needs_attach_reason: Some(NeedsAttachReason::DeliberatelyDetached),
                ..
            } if code == proto::ERROR_NEEDS_ATTACH
        ));
        assert!(restarted.status().await.members.is_empty());

        registered_epoch(restarted.clone(), &store, "s1", "addr:a").await;
        let ack_after_explicit_register =
            request(restarted, ack_req(&store, "s1", "addr:a", message_id)).await;
        assert!(matches!(
            ack_after_explicit_register,
            Response::Ack {
                delivery_outcome: Some(DeliveryOutcome::Marked),
                ..
            }
        ));
    }

    #[tokio::test]
    async fn detach_after_restart_records_tombstone_and_wait_does_not_resurrect() {
        let store = store_key("detach-after-restart");
        {
            let state = test_state("detach-after-restart-one");
            registered_epoch(state.clone(), &store, "s1", "addr:a").await;
            let backend = state.backend_for(&store).await.unwrap();
            insert_test_message(&backend, "addr:a", None).await;
        }

        let restarted = test_state("detach-after-restart-two");
        let detach = request(
            restarted.clone(),
            Request::Detach {
                store_key: store.clone(),
                session_id: "s1".to_string(),
                address: "addr:a".to_string(),
            },
        )
        .await;
        assert!(matches!(detach, Response::Ack { .. }));

        let wait = request(restarted.clone(), wait_req(&store, "s1", "addr:a", 1)).await;
        assert!(matches!(
            wait,
            Response::Error {
                ref code,
                needs_attach_reason: Some(NeedsAttachReason::DeliberatelyDetached),
                ..
            } if code == proto::ERROR_NEEDS_ATTACH
        ));

        let ack = request(restarted, ack_req(&store, "s1", "addr:a", 1)).await;
        assert!(matches!(
            ack,
            Response::Error {
                ref code,
                needs_attach_reason: Some(NeedsAttachReason::DeliberatelyDetached),
                ..
            } if code == proto::ERROR_NEEDS_ATTACH
        ));
    }

    #[tokio::test]
    async fn recovery_register_refuses_tombstone_created_after_restart_lost() {
        let state = test_state("recovery-register-tombstone");
        let store = store_key("recovery-register-tombstone");
        {
            let first = test_state("recovery-register-tombstone-first");
            registered_epoch(first.clone(), &store, "s1", "addr:a").await;
            let backend = first.backend_for(&store).await.unwrap();
            insert_test_message(&backend, "addr:a", None).await;
        }

        let backend = state.backend_for(&store).await.unwrap();
        assert!(matches!(
            needs_attach_for_missing_member(&state, &backend, &store, "s1", "addr:a", "test").await,
            Response::Error {
                ref code,
                needs_attach_reason: Some(NeedsAttachReason::RestartLost),
                ..
            } if code == proto::ERROR_NEEDS_ATTACH
        ));
        backend
            .record_detach_tombstone("s1", "addr:a", "Detach")
            .await
            .unwrap();
        let mut recovery = register_req(&store, "s1", "addr:a");
        if let Request::Register { recovery, .. } = &mut recovery {
            *recovery = true;
        }
        let response = request(state.clone(), recovery).await;
        assert!(matches!(
            response,
            Response::Error {
                ref code,
                needs_attach_reason: Some(NeedsAttachReason::DeliberatelyDetached),
                ..
            } if code == proto::ERROR_NEEDS_ATTACH
        ));
        assert!(state.status().await.members.is_empty());
    }

    #[tokio::test]
    async fn explicit_register_clears_in_memory_definite_end() {
        let state = test_state("clear-definite-end");
        let store = store_key("clear-definite-end");
        registered_epoch(state.clone(), &store, "s1", "addr:a").await;
        assert!(matches!(
            request(
                state.clone(),
                Request::Detach {
                    store_key: store.clone(),
                    session_id: "s1".to_string(),
                    address: "addr:a".to_string(),
                },
            )
            .await,
            Response::Ack { .. }
        ));
        registered_epoch(state.clone(), &store, "s1", "addr:a").await;
        state.remove_member(&store, "s1", "addr:a");
        let backend = state.backend_for(&store).await.unwrap();
        let response =
            needs_attach_for_missing_member(&state, &backend, &store, "s1", "addr:a", "test").await;
        assert!(matches!(
            response,
            Response::Error {
                ref code,
                needs_attach_reason: Some(NeedsAttachReason::RestartLost),
                ..
            } if code == proto::ERROR_NEEDS_ATTACH
        ));
    }

    #[tokio::test]
    async fn same_session_id_in_two_store_keys_is_isolated() {
        let state = test_state("multi-store");
        let store_a = store_key("multi-a");
        let store_b = store_key("multi-b");

        assert!(matches!(
            request(
                state.clone(),
                register_req(&store_a, "same-session", "addr:a")
            )
            .await,
            Response::Registered { .. }
        ));
        let store_b_wait = request(
            state.clone(),
            wait_req(&store_b, "same-session", "addr:a", 1),
        )
        .await;
        assert!(matches!(
            store_b_wait,
            Response::Error { ref code, .. } if code == proto::ERROR_NEEDS_ATTACH
        ));

        assert!(matches!(
            request(
                state.clone(),
                register_req(&store_b, "same-session", "addr:b")
            )
            .await,
            Response::Registered { .. }
        ));
        assert_eq!(state.status().await.members.len(), 2);

        let detach_a = request(
            state.clone(),
            Request::Detach {
                store_key: store_a,
                session_id: "same-session".to_string(),
                address: "addr:a".to_string(),
            },
        )
        .await;
        assert!(matches!(detach_a, Response::Ack { .. }));
        assert_eq!(state.status().await.members.len(), 1);

        let wait_b = request(state, wait_req(&store_b, "same-session", "addr:b", 1)).await;
        assert!(matches!(wait_b, Response::Timeout));
    }

    #[tokio::test]
    async fn send_from_resolution_needs_attach_ambiguous_and_explicit_membership() {
        let state = test_state("send-resolution");
        let store = store_key("send-resolution");

        let no_members = request(state.clone(), send_req(&store, "s1", None)).await;
        assert!(matches!(
            no_members,
            Response::Error { ref code, .. } if code == proto::ERROR_NEEDS_ATTACH
        ));

        assert!(matches!(
            request(state.clone(), register_req(&store, "s1", "addr:a")).await,
            Response::Registered { .. }
        ));
        let explicit_missing = request(state.clone(), send_req(&store, "s1", Some("addr:b"))).await;
        assert!(matches!(
            explicit_missing,
            Response::Error { ref code, .. } if code == proto::ERROR_NEEDS_ATTACH
        ));

        let explicit_ok = request(state.clone(), send_req(&store, "s1", Some("addr:a"))).await;
        assert!(matches!(explicit_ok, Response::Sent { .. }));

        assert!(matches!(
            request(state.clone(), register_req(&store, "s1", "addr:b")).await,
            Response::Registered { .. }
        ));
        let ambiguous = request(state, send_req(&store, "s1", None)).await;
        assert!(matches!(
            ambiguous,
            Response::Error { ref code, .. } if code == proto::ERROR_AMBIGUOUS
        ));
    }

    #[tokio::test]
    async fn send_reply_explicit_from_respects_durable_detach_tombstone() {
        let state = test_state("send-reply-tombstone");
        let store = store_key("send-reply-tombstone");
        registered_epoch(state.clone(), &store, "s1", "addr:a").await;
        let backend = state.backend_for(&store).await.unwrap();
        let parent_id = insert_test_message(&backend, "addr:a", None).await;

        assert!(matches!(
            request(
                state.clone(),
                Request::Detach {
                    store_key: store.clone(),
                    session_id: "s1".to_string(),
                    address: "addr:a".to_string(),
                },
            )
            .await,
            Response::Ack { .. }
        ));

        let send = request(state.clone(), send_req(&store, "s1", Some("addr:a"))).await;
        assert!(matches!(
            send,
            Response::Error {
                ref code,
                needs_attach_reason: Some(NeedsAttachReason::DeliberatelyDetached),
                ..
            } if code == proto::ERROR_NEEDS_ATTACH
        ));

        let reply = request(
            state,
            Request::Reply {
                store_key: store,
                session_id: "s1".to_string(),
                from_addr: Some("addr:a".to_string()),
                message_id: parent_id,
                kind: "note".to_string(),
                attention: "background".to_string(),
                requires_disposition: false,
                subject: None,
                cc: None,
                body: "reply".to_string(),
            },
        )
        .await;
        assert!(matches!(
            reply,
            Response::Error {
                ref code,
                needs_attach_reason: Some(NeedsAttachReason::DeliberatelyDetached),
                ..
            } if code == proto::ERROR_NEEDS_ATTACH
        ));
    }

    #[tokio::test]
    async fn send_and_reply_reject_payloads_above_body_metadata_cap_before_insert() {
        let state = test_state("payload-cap");
        let store = store_key("payload-cap");
        registered_epoch(state.clone(), &store, "s1", "addr:a").await;
        registered_epoch(state.clone(), &store, "s2", "dest").await;
        let backend = state.backend_for(&store).await.unwrap();
        let before = backend.inbox("dest", true, 100).await.unwrap().len();
        let too_large = "x".repeat(proto::MAX_MESSAGE_BODY_METADATA_BYTES + 1);

        let send = request(
            state.clone(),
            Request::Send {
                store_key: store.clone(),
                session_id: "s1".to_string(),
                from_addr: Some("addr:a".to_string()),
                to_addr: "dest".to_string(),
                cc: None,
                kind: "note".to_string(),
                attention: "background".to_string(),
                requires_disposition: false,
                subject: None,
                body: too_large.clone(),
                metadata: None,
            },
        )
        .await;
        assert!(matches!(
            send,
            Response::Error { ref code, .. } if code == proto::ERROR_INCOMPATIBLE
        ));
        assert_eq!(
            backend.inbox("dest", true, 100).await.unwrap().len(),
            before
        );

        let parent_id = insert_test_message(&backend, "addr:a", None).await;
        let reply = request(
            state,
            Request::Reply {
                store_key: store,
                session_id: "s1".to_string(),
                from_addr: Some("addr:a".to_string()),
                message_id: parent_id,
                kind: "note".to_string(),
                attention: "background".to_string(),
                requires_disposition: false,
                subject: None,
                cc: None,
                body: too_large,
            },
        )
        .await;
        assert!(matches!(
            reply,
            Response::Error { ref code, .. } if code == proto::ERROR_INCOMPATIBLE
        ));
    }

    #[tokio::test]
    async fn ack_frame_names_address_and_rejects_unattended_address() {
        let state = test_state("ack");
        let store = store_key("ack");
        assert!(matches!(
            request(state.clone(), register_req(&store, "s1", "addr:a")).await,
            Response::Registered { .. }
        ));
        let backend = state.backend_for(&store).await.unwrap();
        let row = backend
            .insert_message(&NewMessage {
                parent_id: None,
                from_addr: Some("sender".to_string()),
                to_addr: "addr:a".to_string(),
                cc: None,
                kind: "note".to_string(),
                attention: Attention::Background,
                requires_disposition: false,
                subject: None,
                body: "hello".to_string(),
                metadata: None,
                sent_at_ms: now_ms(),
            })
            .await
            .unwrap();

        let ack = request(
            state.clone(),
            Request::Ack {
                store_key: store.clone(),
                session_id: "s1".to_string(),
                address: "addr:a".to_string(),
                message_id: row.id,
            },
        )
        .await;
        match ack {
            Response::Ack {
                delivery_outcome,
                address,
                message_id,
                ..
            } => {
                assert_eq!(delivery_outcome, Some(DeliveryOutcome::Marked));
                assert_eq!(address.as_deref(), Some("addr:a"));
                assert_eq!(message_id, Some(row.id));
            }
            other => panic!("expected ack response, got {other:?}"),
        }

        let wrong_address = request(
            state,
            Request::Ack {
                store_key: store,
                session_id: "s1".to_string(),
                address: "addr:b".to_string(),
                message_id: row.id,
            },
        )
        .await;
        assert!(matches!(
            wrong_address,
            Response::Error { ref code, .. } if code == proto::ERROR_NEEDS_ATTACH
        ));
    }

    #[tokio::test]
    async fn status_lists_only_in_memory_members_and_restart_does_not_rebuild() {
        let store = store_key("restart");
        {
            let state = test_state("restart-one");
            assert!(matches!(
                request(state.clone(), register_req(&store, "s1", "addr:a")).await,
                Response::Registered { .. }
            ));
            assert_eq!(state.status().await.members.len(), 1);
        }

        let restarted = test_state("restart-two");
        let status = restarted.status().await;
        assert!(status.members.is_empty());
        assert!(status.epoch_by_address.is_empty());

        let wait_after_restart = request(restarted, wait_req(&store, "s1", "addr:a", 1)).await;
        assert!(matches!(
            wait_after_restart,
            Response::Error { ref code, .. } if code == proto::ERROR_NEEDS_ATTACH
        ));
    }

    async fn registered_epoch(
        state: Arc<DaemonState>,
        store: &str,
        session: &str,
        address: &str,
    ) -> i64 {
        match request(state, register_req(store, session, address)).await {
            Response::Registered { lease_epoch, .. } => lease_epoch,
            other => panic!("expected Registered, got {other:?}"),
        }
    }

    async fn insert_test_message(backend: &Arc<dyn Backend>, to: &str, cc: Option<&str>) -> i64 {
        backend
            .insert_message(&NewMessage {
                parent_id: None,
                from_addr: Some("sender".to_string()),
                to_addr: to.to_string(),
                cc: cc.map(str::to_string),
                kind: "note".to_string(),
                attention: Attention::Background,
                requires_disposition: false,
                subject: None,
                body: "hello".to_string(),
                metadata: None,
                sent_at_ms: now_ms(),
            })
            .await
            .unwrap()
            .id
    }

    async fn rotate_owner(
        backend: &Arc<dyn Backend>,
        address: &str,
        predecessor: &str,
        predecessor_epoch: i64,
        successor: &str,
    ) -> i64 {
        assert!(
            backend
                .release_epoch_lease(address, predecessor, predecessor_epoch)
                .await
                .unwrap(),
            "predecessor should release current epoch before successor claim"
        );
        match backend
            .claim_epoch_lease(address, successor, 15)
            .await
            .unwrap()
        {
            EpochClaimResult::Claimed(claimed) => claimed.lease_epoch,
            other => panic!("expected successor claim, got {other:?}"),
        }
    }

    fn ack_req(store: &str, session: &str, address: &str, message_id: i64) -> Request {
        Request::Ack {
            store_key: store.to_string(),
            session_id: session.to_string(),
            address: address.to_string(),
            message_id,
        }
    }

    #[tokio::test]
    async fn wait_self_demotes_on_lost_epoch_before_emitting() {
        let state = test_state("wait-lost-owner");
        let store = store_key("wait-lost-owner");
        let epoch = registered_epoch(state.clone(), &store, "s1", "addr:a").await;
        let backend = state.backend_for(&store).await.unwrap();
        let message_id = insert_test_message(&backend, "addr:a", None).await;
        let successor_epoch =
            rotate_owner(&backend, "addr:a", &state.instance_id, epoch, "successor").await;
        assert_eq!(successor_epoch, epoch + 1);

        let wait = request(state.clone(), wait_req(&store, "s1", "addr:a", 1000)).await;
        assert!(matches!(
            wait,
            Response::Error { ref code, .. }
                if code == proto::ERROR_NEEDS_ATTACH || code == proto::ERROR_NOT_OWNER
        ));
        let status = state.status().await;
        assert!(status.members.is_empty());
        assert!(
            status.recent_errors.iter().any(|e| e.kind == "NotOwner"
                && e.message.contains("self-demoted")
                && e.message.contains("addr:a")),
            "status should expose self-demotion in recent_errors: {:?}",
            status.recent_errors
        );
        let undelivered = backend.fetch_undelivered("addr:a").await.unwrap();
        assert_eq!(
            undelivered.iter().map(|m| m.id).collect::<Vec<_>>(),
            vec![message_id]
        );
    }

    #[tokio::test]
    async fn ack_not_owner_self_demotes_and_future_wait_needs_attach() {
        let state = test_state("ack-not-owner");
        let store = store_key("ack-not-owner");
        let epoch = registered_epoch(state.clone(), &store, "s1", "addr:a").await;
        let backend = state.backend_for(&store).await.unwrap();
        let message_id = insert_test_message(&backend, "addr:a", None).await;
        rotate_owner(&backend, "addr:a", &state.instance_id, epoch, "successor").await;

        let ack = request(state.clone(), ack_req(&store, "s1", "addr:a", message_id)).await;
        match ack {
            Response::Ack {
                delivery_outcome,
                lease_epoch,
                ..
            } => {
                assert_eq!(delivery_outcome, Some(DeliveryOutcome::NotOwner));
                assert_eq!(lease_epoch, Some(epoch));
            }
            other => panic!("expected ack response, got {other:?}"),
        }
        assert!(state.status().await.members.is_empty());

        let wait = request(state, wait_req(&store, "s1", "addr:a", 1)).await;
        assert!(matches!(
            wait,
            Response::Error { ref code, .. } if code == proto::ERROR_NEEDS_ATTACH
        ));
    }

    #[tokio::test]
    async fn successor_consumed_mark_has_not_owner_precedence_for_predecessor_ack() {
        let state = test_state("ack-precedence");
        let store = store_key("ack-precedence");
        let epoch = registered_epoch(state.clone(), &store, "s1", "addr:a").await;
        let backend = state.backend_for(&store).await.unwrap();
        let message_id = insert_test_message(&backend, "addr:a", None).await;
        let successor_epoch =
            rotate_owner(&backend, "addr:a", &state.instance_id, epoch, "successor").await;

        let successor_mark = backend
            .mark_consumed_if_current_owner("addr:a", "successor", successor_epoch, message_id)
            .await
            .unwrap();
        assert_eq!(successor_mark, DeliveryOutcome::Marked);

        let predecessor_ack =
            request(state.clone(), ack_req(&store, "s1", "addr:a", message_id)).await;
        match predecessor_ack {
            Response::Ack {
                delivery_outcome, ..
            } => assert_eq!(delivery_outcome, Some(DeliveryOutcome::NotOwner)),
            other => panic!("expected ack response, got {other:?}"),
        }
        assert!(state.status().await.members.is_empty());
    }

    #[tokio::test]
    async fn drain_releases_epoch_rows_and_restart_claims_next_epoch() {
        let store = store_key("drain");
        let epoch;
        {
            let state = test_state("drain-one");
            epoch = registered_epoch(state.clone(), &store, "s1", "addr:a").await;
            let backend = state.backend_for(&store).await.unwrap();
            let (drain, action) = handle_request(
                state.clone(),
                Request::Drain {
                    proof: Some(state.admin_cap.clone()),
                },
            )
            .await;
            assert!(matches!(drain, Response::Ack { .. }));
            assert!(matches!(action, ClientAction::Drain));
            assert!(state.status().await.members.is_empty());
            let lease = backend.get_lease("addr:a").await.unwrap().unwrap();
            assert_eq!(lease.lease_epoch, Some(epoch));
            assert_eq!(lease.owner_instance_id, None);
        }

        let restarted = test_state("drain-two");
        let next_epoch = registered_epoch(restarted, &store, "s1", "addr:a").await;
        assert_eq!(next_epoch, epoch + 1);
    }

    #[tokio::test]
    async fn legacy_null_epoch_cutover_is_audited_in_status() {
        let state = test_state("legacy-cutover");
        let store = legacy_null_epoch_store_key("legacy-cutover");

        let registered = request(state.clone(), register_req(&store, "s1", "addr:legacy")).await;
        match registered {
            Response::Registered { lease_epoch, .. } => assert_eq!(lease_epoch, 1),
            other => panic!("expected Registered, got {other:?}"),
        }

        let status = state.status().await;
        assert!(status.recent_errors.iter().any(|e| {
            e.kind == "LegacyCutover"
                && e.message.contains("addr:legacy")
                && e.message.contains("epoch 1")
        }));
    }

    #[tokio::test]
    async fn wait_is_at_least_once_until_explicit_ack_consumes() {
        let state = test_state("at-least-once");
        let store = store_key("at-least-once");
        let epoch = registered_epoch(state.clone(), &store, "s1", "addr:a").await;
        let backend = state.backend_for(&store).await.unwrap();
        let message_id = insert_test_message(&backend, "addr:a", None).await;

        let wait = request(state.clone(), wait_req(&store, "s1", "addr:a", 1000)).await;
        match wait {
            Response::Message {
                id, lease_epoch, ..
            } => {
                assert_eq!(id, message_id);
                assert_eq!(lease_epoch, Some(epoch));
            }
            other => panic!("expected message response, got {other:?}"),
        }
        let undelivered = backend.fetch_undelivered("addr:a").await.unwrap();
        assert_eq!(
            undelivered.iter().map(|m| m.id).collect::<Vec<_>>(),
            vec![message_id]
        );

        let rearm_before_ack = request(state.clone(), wait_req(&store, "s1", "addr:a", 1000)).await;
        assert!(
            matches!(rearm_before_ack, Response::PresenceEnded),
            "same unacked message should not be handed to a freshly re-armed waiter"
        );

        let ack = request(state, ack_req(&store, "s1", "addr:a", message_id)).await;
        match ack {
            Response::Ack {
                delivery_outcome, ..
            } => assert_eq!(delivery_outcome, Some(DeliveryOutcome::Marked)),
            other => panic!("expected ack response, got {other:?}"),
        }
        assert!(backend
            .fetch_undelivered("addr:a")
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn wait_returns_small_error_for_oversized_historical_message_frame() {
        let state = test_state("oversized-frame");
        let store = store_key("oversized-frame");
        registered_epoch(state.clone(), &store, "s1", "addr:a").await;
        let backend = state.backend_for(&store).await.unwrap();
        let oversized = "x".repeat(proto::MAX_JSONL_FRAME_BYTES + 1);
        let message_id = backend
            .insert_message(&NewMessage {
                parent_id: None,
                from_addr: Some("sender".to_string()),
                to_addr: "addr:a".to_string(),
                cc: None,
                kind: "note".to_string(),
                attention: Attention::Background,
                requires_disposition: false,
                subject: None,
                body: oversized,
                metadata: None,
                sent_at_ms: now_ms(),
            })
            .await
            .unwrap()
            .id;

        let wait = request(state.clone(), wait_req(&store, "s1", "addr:a", 1_000)).await;
        match wait {
            Response::Error { code, message, .. } => {
                assert_eq!(code, proto::ERROR_INCOMPATIBLE);
                assert!(message.contains(&message_id.to_string()));
                assert!(message.contains("IPC frame"));
            }
            other => panic!("expected oversized-frame error, got {other:?}"),
        }
        let status = state.status().await;
        assert_eq!(status.members[0].last_waiter_outcome, None);
        assert_eq!(status.members[0].last_delivered_message_id, None);

        let second_wait = request(state, wait_req(&store, "s1", "addr:a", 1_000)).await;
        assert!(
            matches!(second_wait, Response::Error { ref code, .. } if code == proto::ERROR_INCOMPATIBLE),
            "oversized message should fail the same way on retry, not be blocked as delivered"
        );
    }

    #[tokio::test]
    async fn cc_fanout_is_visible_but_not_wait_deliverable() {
        let state = test_state("fanout");
        let store = store_key("fanout");
        registered_epoch(state.clone(), &store, "s1", "addr:a").await;
        registered_epoch(state.clone(), &store, "s1", "addr:b").await;
        let backend = state.backend_for(&store).await.unwrap();
        let message_id = insert_test_message(&backend, "addr:a", Some("addr:b")).await;

        let wait_a = request(state.clone(), wait_req(&store, "s1", "addr:a", 1000)).await;
        assert!(matches!(wait_a, Response::Message { id, .. } if id == message_id));
        let ack_a = request(state.clone(), ack_req(&store, "s1", "addr:a", message_id)).await;
        assert!(matches!(
            ack_a,
            Response::Ack {
                delivery_outcome: Some(DeliveryOutcome::Marked),
                ..
            }
        ));
        assert!(backend
            .fetch_undelivered("addr:a")
            .await
            .unwrap()
            .is_empty());
        assert!(backend
            .fetch_undelivered("addr:b")
            .await
            .unwrap()
            .is_empty());
        let inbox_b = backend.inbox("addr:b", true, 10).await.unwrap();
        assert!(inbox_b.iter().any(|item| {
            item.message.id == message_id && item.delivery_role == "cc" && !item.actionable
        }));

        let wait_b = request(state.clone(), wait_req(&store, "s1", "addr:b", 1)).await;
        assert!(matches!(wait_b, Response::Timeout));
        let ack_b = request(state, ack_req(&store, "s1", "addr:b", message_id)).await;
        assert!(matches!(
            ack_b,
            Response::Ack {
                delivery_outcome: Some(DeliveryOutcome::AlreadyConsumed),
                ..
            }
        ));
        assert!(backend
            .fetch_undelivered("addr:b")
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn wake_on_cc_delivers_live_cc_without_ack_requirement_or_replay() {
        let state = test_state("wake-on-cc");
        let store = store_key("wake-on-cc");
        registered_epoch(state.clone(), &store, "s1", "addr:primary").await;
        registered_epoch(state.clone(), &store, "s1", "addr:cc").await;
        let backend = state.backend_for(&store).await.unwrap();

        let waiter_state = state.clone();
        let waiter_store = store.clone();
        let waiter = tokio::spawn(async move {
            request(
                waiter_state,
                Request::Wait {
                    store_key: waiter_store,
                    session_id: "s1".to_string(),
                    address: "addr:cc".to_string(),
                    attention: None,
                    min_attention: None,
                    wake_on_cc: true,
                    timeout_ms: Some(1_000),
                    waiter_pid: Some(std::process::id()),
                    waiter_start_time: crate::session_watch::capture_process_start_time(
                        std::process::id(),
                    ),
                },
            )
            .await
        });
        tokio::time::sleep(Duration::from_millis(50)).await;
        let message_id = insert_test_message(&backend, "addr:primary", Some("addr:cc")).await;

        let delivered = waiter.await.expect("waiter");
        assert!(matches!(
            delivered,
            Response::Message {
                id,
                delivery_role,
                requires_disposition_for_current_recipient,
                ..
            } if id == message_id
                && delivery_role == "cc"
                && !requires_disposition_for_current_recipient
        ));
        let ack_cc = request(state.clone(), ack_req(&store, "s1", "addr:cc", message_id)).await;
        assert!(matches!(
            ack_cc,
            Response::Ack {
                delivery_outcome: Some(DeliveryOutcome::AlreadyConsumed),
                ..
            }
        ));

        let rearm = request(
            state,
            Request::Wait {
                store_key: store,
                session_id: "s1".to_string(),
                address: "addr:cc".to_string(),
                attention: None,
                min_attention: None,
                wake_on_cc: true,
                timeout_ms: Some(1),
                waiter_pid: Some(std::process::id()),
                waiter_start_time: crate::session_watch::capture_process_start_time(
                    std::process::id(),
                ),
            },
        )
        .await;
        assert!(matches!(rearm, Response::Timeout));
    }

    #[tokio::test]
    async fn wake_on_cc_does_not_weaken_primary_unacked_rearm_guard() {
        let state = test_state("wake-on-cc-primary-guard");
        let store = store_key("wake-on-cc-primary-guard");
        registered_epoch(state.clone(), &store, "s1", "addr:primary").await;
        registered_epoch(state.clone(), &store, "s1", "addr:cc").await;
        let backend = state.backend_for(&store).await.unwrap();
        let cc_message = insert_test_message(&backend, "addr:primary", Some("addr:cc")).await;

        let cc_wait = request(
            state.clone(),
            Request::Wait {
                store_key: store.clone(),
                session_id: "s1".to_string(),
                address: "addr:cc".to_string(),
                attention: None,
                min_attention: None,
                wake_on_cc: true,
                timeout_ms: Some(1),
                waiter_pid: Some(std::process::id()),
                waiter_start_time: crate::session_watch::capture_process_start_time(
                    std::process::id(),
                ),
            },
        )
        .await;
        assert!(
            matches!(cc_wait, Response::Timeout),
            "historical CC {cc_message} must not replay after the wait lower bound"
        );

        let primary_id = insert_test_message(&backend, "addr:cc", None).await;
        let primary_wait = request(state.clone(), wait_req(&store, "s1", "addr:cc", 1_000)).await;
        assert!(matches!(primary_wait, Response::Message { id, .. } if id == primary_id));
        let rearm_before_ack =
            request(state.clone(), wait_req(&store, "s1", "addr:cc", 1_000)).await;
        assert!(matches!(rearm_before_ack, Response::PresenceEnded));

        let ack = request(state.clone(), ack_req(&store, "s1", "addr:cc", primary_id)).await;
        assert!(matches!(
            ack,
            Response::Ack {
                delivery_outcome: Some(DeliveryOutcome::Marked),
                ..
            }
        ));
        let after_ack = request(state, wait_req(&store, "s1", "addr:cc", 1)).await;
        assert!(matches!(after_ack, Response::Timeout));
    }

    #[tokio::test]
    async fn wake_on_cc_composes_with_min_attention() {
        let state = test_state("wake-on-cc-min-attention");
        let store = store_key("wake-on-cc-min-attention");
        registered_epoch(state.clone(), &store, "s1", "addr:primary").await;
        registered_epoch(state.clone(), &store, "s1", "addr:cc").await;
        let backend = state.backend_for(&store).await.unwrap();

        let waiter_state = state.clone();
        let waiter_store = store.clone();
        let waiter = tokio::spawn(async move {
            request(
                waiter_state,
                Request::Wait {
                    store_key: waiter_store,
                    session_id: "s1".to_string(),
                    address: "addr:cc".to_string(),
                    attention: None,
                    min_attention: Some("interrupt".to_string()),
                    wake_on_cc: true,
                    timeout_ms: Some(1_000),
                    waiter_pid: Some(std::process::id()),
                    waiter_start_time: crate::session_watch::capture_process_start_time(
                        std::process::id(),
                    ),
                },
            )
            .await
        });
        tokio::time::sleep(Duration::from_millis(50)).await;
        let background = insert_test_message(&backend, "addr:primary", Some("addr:cc")).await;
        tokio::time::sleep(Duration::from_millis(50)).await;
        let interrupt = backend
            .insert_message(&NewMessage {
                parent_id: None,
                from_addr: Some("sender".to_string()),
                to_addr: "addr:primary".to_string(),
                cc: Some("addr:cc".to_string()),
                kind: "note".to_string(),
                attention: Attention::Interrupt,
                requires_disposition: false,
                subject: Some("interrupt cc".to_string()),
                body: "interrupt body".to_string(),
                metadata: None,
                sent_at_ms: now_ms(),
            })
            .await
            .unwrap()
            .id;

        let delivered = waiter.await.expect("waiter");
        assert!(
            matches!(delivered, Response::Message { id, delivery_role, .. } if id == interrupt && delivery_role == "cc"),
            "interrupt CC should wake, background CC {background} should be skipped by min-attention"
        );
    }

    #[tokio::test]
    async fn wake_on_cc_non_sqlite_store_is_typed_unsupported() {
        let state = test_state("wake-on-cc-non-sqlite");
        let response = request(
            state,
            Request::Wait {
                store_key: "postgres:unavailable-for-daemon-core".to_string(),
                session_id: "s1".to_string(),
                address: "addr:cc".to_string(),
                attention: None,
                min_attention: None,
                wake_on_cc: true,
                timeout_ms: Some(1),
                waiter_pid: Some(std::process::id()),
                waiter_start_time: crate::session_watch::capture_process_start_time(
                    std::process::id(),
                ),
            },
        )
        .await;
        assert!(matches!(
            response,
            Response::Error { code, .. } if code == proto::ERROR_UNSUPPORTED
        ));
    }

    #[tokio::test]
    async fn session_end_marks_idle_releases_waiter_and_rearm_receives_message() {
        let state = test_state("session-end");
        let store = store_key("session-end");
        registered_epoch(state.clone(), &store, "s1", "addr:a").await;

        let waiter_req = wait_req(&store, "s1", "addr:a", 5_000);
        let waiter_state = state.clone();
        let waiter = tokio::spawn(async move { request(waiter_state, waiter_req).await });
        tokio::time::sleep(Duration::from_millis(50)).await;

        let end = request(state.clone(), session_end_req(&state, &store, "s1")).await;
        assert!(matches!(end, Response::Ack { .. }));
        assert!(matches!(waiter.await.unwrap(), Response::PresenceEnded));

        let status = state.status().await;
        assert_eq!(status.members.len(), 1);
        assert!(status.members[0].idle);
        assert_eq!(status.members[0].waiters, 0);
        assert!(status.recent_errors.iter().any(|e| e.kind == "SessionEnd"));

        let backend = state.backend_for(&store).await.unwrap();
        let lease = backend.get_lease("addr:a").await.unwrap().unwrap();
        assert_eq!(lease.owner_instance_id, None);
        let message_id = insert_test_message(&backend, "addr:a", None).await;
        assert!(matches!(
            request(state.clone(), register_req(&store, "s1", "addr:a")).await,
            Response::Registered { .. }
        ));
        let wait = request(state, wait_req(&store, "s1", "addr:a", 1_000)).await;
        assert!(matches!(wait, Response::Message { id, .. } if id == message_id));
        assert!(backend.delivery_retention_count().await.unwrap() >= 1);
    }

    #[tokio::test]
    async fn station_stop_drains_live_waiter_and_status_lists_pid() {
        let state = test_state("station-stop");
        let store = store_key("station-stop");
        registered_epoch(state.clone(), &store, "s1", "addr:a").await;

        let waiter_req = wait_req(&store, "s1", "addr:a", 5_000);
        let waiter_state = state.clone();
        let waiter = tokio::spawn(async move { request(waiter_state, waiter_req).await });
        tokio::time::sleep(Duration::from_millis(75)).await;

        let status = state.status().await;
        assert_eq!(status.live_waiters.len(), 1);
        assert_eq!(status.members.len(), 1);
        assert_eq!(status.members[0].waiters, 1);
        assert_eq!(status.members[0].live_waiters.len(), 1);
        assert_eq!(status.members[0].live_waiters[0].address, "addr:a");
        assert!(status.members[0].live_waiters[0].pid > 0);

        let stopped = request(
            state.clone(),
            Request::StationStop {
                store_key: store.clone(),
                session_id: "s1".to_string(),
                address: "addr:a".to_string(),
                wait_grace_ms: 1_000,
            },
        )
        .await;
        match stopped {
            Response::StationStopped {
                detached,
                waiters_before,
                waiters_after,
                live_waiters,
                ..
            } => {
                assert!(detached);
                assert_eq!(waiters_before, 1);
                assert_eq!(waiters_after, 0);
                assert!(live_waiters.is_empty());
            }
            other => panic!("expected station stopped, got {other:?}"),
        }
        assert!(matches!(waiter.await.unwrap(), Response::PresenceEnded));
        let status = state.status().await;
        assert!(status.members.is_empty());
        assert!(status.live_waiters.is_empty());
    }

    #[tokio::test]
    async fn station_stop_drains_pidless_protocol_waiter() {
        let state = test_state("station-stop-pidless");
        let store = store_key("station-stop-pidless");
        registered_epoch(state.clone(), &store, "s1", "addr:a").await;

        let waiter_req = Request::Wait {
            store_key: store.clone(),
            session_id: "s1".to_string(),
            address: "addr:a".to_string(),
            attention: None,
            min_attention: None,
            wake_on_cc: false,
            timeout_ms: Some(5_000),
            waiter_pid: None,
            waiter_start_time: None,
        };
        let waiter_state = state.clone();
        let waiter = tokio::spawn(async move { request(waiter_state, waiter_req).await });
        tokio::time::sleep(Duration::from_millis(75)).await;

        let status = state.status().await;
        assert_eq!(status.live_waiters.len(), 1);
        assert_eq!(status.live_waiters[0].pid, 0);
        assert!(status.live_waiters[0].alive);

        let stopped = request(
            state.clone(),
            Request::StationStop {
                store_key: store.clone(),
                session_id: "s1".to_string(),
                address: "addr:a".to_string(),
                wait_grace_ms: 1_000,
            },
        )
        .await;
        assert!(matches!(
            stopped,
            Response::StationStopped {
                waiters_before: 1,
                waiters_after: 0,
                ..
            }
        ));
        assert!(matches!(waiter.await.unwrap(), Response::PresenceEnded));
    }

    #[tokio::test]
    async fn status_prunes_dead_pid_backed_waiter_records() {
        let state = test_state("dead-waiter-status");
        let store = store_key("dead-waiter-status");
        registered_epoch(state.clone(), &store, "s1", "addr:a").await;
        {
            let mut members = state.members.lock().unwrap();
            members
                .get_mut(&DaemonState::member_key(&store, "s1", "addr:a"))
                .unwrap()
                .waiters = 1;
        }
        state.waiters.lock().unwrap().insert(
            WaiterKey { waiter_id: 99 },
            WaiterRecord {
                waiter_id: 99,
                store_key: store.clone(),
                session_id: "s1".to_string(),
                address: "addr:a".to_string(),
                pid: 2_000_000_000,
                start_time: None,
                started_at_ms: now_ms(),
                attention: None,
                min_attention: None,
                wake_on_cc: false,
                cc_after_ms: None,
                timeout_ms: Some(5_000),
            },
        );

        let status = state.status().await;
        assert!(status.live_waiters.is_empty());
        assert_eq!(status.members.len(), 1);
        assert_eq!(status.members[0].waiters, 0);
        assert!(status.members[0].live_waiters.is_empty());
        assert_eq!(
            status.members[0].last_waiter_outcome,
            Some(WaiterOutcome::AbnormalExit)
        );
        assert_eq!(status.members[0].last_waiter_pid, Some(2_000_000_000));
    }

    #[tokio::test]
    async fn heartbeat_prunes_dead_waiter_into_abnormal_terminal_status() {
        let state = test_state("dead-waiter-heartbeat");
        let store = store_key("dead-waiter-heartbeat");
        let mut register = register_req(&store, "s1", "addr:a");
        if let Request::Register { watch_pids, .. } = &mut register {
            watch_pids.clear();
        }
        assert!(matches!(
            request(state.clone(), register).await,
            Response::Registered { .. }
        ));
        {
            let mut members = state.members.lock().unwrap();
            members
                .get_mut(&DaemonState::member_key(&store, "s1", "addr:a"))
                .unwrap()
                .waiters = 1;
        }
        state.waiters.lock().unwrap().insert(
            WaiterKey { waiter_id: 99 },
            WaiterRecord {
                waiter_id: 99,
                store_key: store.clone(),
                session_id: "s1".to_string(),
                address: "addr:a".to_string(),
                pid: 2_000_000_000,
                start_time: None,
                started_at_ms: now_ms(),
                attention: None,
                min_attention: None,
                wake_on_cc: false,
                cc_after_ms: None,
                timeout_ms: Some(5_000),
            },
        );

        heartbeat_members_once(state.clone()).await;

        let status = state.status().await;
        assert!(status.live_waiters.is_empty());
        assert_eq!(
            status.members[0].last_waiter_outcome,
            Some(WaiterOutcome::AbnormalExit)
        );
        assert_eq!(status.members[0].last_waiter_exit_code, None);
        assert_eq!(
            status.members[0].last_waiter_detail.as_deref(),
            Some("waiter process exited before daemon response")
        );
    }

    #[tokio::test]
    async fn status_reports_unattended_station_health_states() {
        let state = test_state("station-health");
        let store = store_key("station-health");
        registered_epoch(state.clone(), &store, "s1", "addr:a").await;

        let status = state.status().await;
        assert_eq!(status.members.len(), 1);
        assert_eq!(status.members[0].station_health, StationHealth::Unattended);
        assert_eq!(status.members[0].pending_unconsumed_count, 0);
        assert_eq!(status.members[0].live_waiters_count, 0);

        let backend = state.backend_for(&store).await.unwrap();
        let message_id = insert_test_message(&backend, "addr:a", None).await;
        let status = state.status().await;
        assert_eq!(
            status.members[0].station_health,
            StationHealth::UnattendedWithBacklog
        );
        assert_eq!(status.members[0].pending_unconsumed_count, 1);
        assert_eq!(status.members[0].live_waiters_count, 0);
        assert!(status.members[0]
            .health_detail
            .as_deref()
            .unwrap_or_default()
            .contains("pending unconsumed"));

        let wait = request(state.clone(), wait_req(&store, "s1", "addr:a", 1_000)).await;
        assert!(matches!(wait, Response::Message { id, .. } if id == message_id));
        let status = state.status().await;
        assert_eq!(
            status.members[0].station_health,
            StationHealth::RecentlyDelivered
        );
        assert_eq!(
            status.members[0].last_delivered_message_id,
            Some(message_id)
        );
        assert_eq!(
            status.members[0].last_waiter_outcome,
            Some(WaiterOutcome::Message)
        );
        assert_eq!(status.members[0].last_waiter_exit_code, Some(0));
    }

    #[tokio::test]
    async fn status_reports_thresholded_deaf_station_summary() {
        let state = test_state("deaf-summary");
        let store = store_key("deaf-summary");
        registered_epoch(state.clone(), &store, "s1", "addr:a").await;
        let backend = state.backend_for(&store).await.unwrap();
        insert_test_message(&backend, "addr:a", None).await;

        let status = state.status_with_thresholds(100_000, 100_000, 0).await;
        assert_eq!(
            status.members[0].station_health,
            StationHealth::UnattendedWithBacklog
        );
        assert!(status.members[0].deaf_warn);
        assert!(status.members[0].unattended_for_ms.unwrap_or_default() >= 0);
        assert_eq!(status.deaf_stations.count, 1);
        assert!(status.deaf_stations.warn);
        assert_eq!(status.deaf_stations.warn_threshold_ms, 0);
    }

    #[tokio::test]
    async fn deaf_warn_threshold_starts_when_backlog_appears() {
        let state = test_state("deaf-backlog-threshold");
        let store = store_key("deaf-backlog-threshold");
        registered_epoch(state.clone(), &store, "s1", "addr:a").await;
        {
            let mut members = state.members.lock().unwrap();
            members
                .get_mut(&DaemonState::member_key(&store, "s1", "addr:a"))
                .unwrap()
                .unattended_since_ms = Some(now_ms().saturating_sub(60_000));
        }
        let backend = state.backend_for(&store).await.unwrap();
        insert_test_message(&backend, "addr:a", None).await;

        let status = state.status_with_thresholds(100_000, 100_000, 30_000).await;

        assert_eq!(
            status.members[0].station_health,
            StationHealth::UnattendedWithBacklog
        );
        assert!(
            status.members[0].unattended_for_ms.unwrap_or_default() >= 60_000,
            "plain unattended age should preserve no-waiter duration"
        );
        assert!(
            status.members[0].deaf_since_ms.is_some(),
            "deaf threshold should have its own backlog start timestamp"
        );
        assert!(
            !status.members[0].deaf_warn,
            "deaf warning should not immediately inherit long no-backlog unattended age"
        );
    }

    #[tokio::test]
    async fn send_starts_deaf_clock_before_first_status_poll() {
        let state = test_state("deaf-send-clock");
        let store = store_key("deaf-send-clock");
        registered_epoch(state.clone(), &store, "receiver", "addr:receiver").await;
        registered_epoch(state.clone(), &store, "sender", "addr:sender").await;

        let sent = request(
            state.clone(),
            Request::Send {
                store_key: store.clone(),
                session_id: "sender".to_string(),
                from_addr: Some("addr:sender".to_string()),
                to_addr: "addr:receiver".to_string(),
                cc: None,
                kind: "note".to_string(),
                attention: "background".to_string(),
                requires_disposition: false,
                subject: None,
                body: "queued for deaf clock".to_string(),
                metadata: None,
            },
        )
        .await;
        assert!(matches!(sent, Response::Sent { .. }));

        tokio::time::sleep(Duration::from_millis(5)).await;
        let status = state.status_with_thresholds(100_000, 100_000, 1).await;

        assert_eq!(
            status.members[0].station_health,
            StationHealth::UnattendedWithBacklog
        );
        assert!(status.members[0].deaf_warn);
        assert!(status.members[0].deaf_for_ms.unwrap_or_default() >= 1);
    }

    #[tokio::test]
    async fn rearm_rejection_does_not_reset_unattended_clock() {
        let state = test_state("deaf-rearm-rejection");
        let store = store_key("deaf-rearm-rejection");
        registered_epoch(state.clone(), &store, "s1", "addr:a").await;
        let backend = state.backend_for(&store).await.unwrap();
        let message_id = insert_test_message(&backend, "addr:a", None).await;

        let first = request(state.clone(), wait_req(&store, "s1", "addr:a", 1_000)).await;
        assert!(matches!(first, Response::Message { id, .. } if id == message_id));
        let old_deaf_since = now_ms().saturating_sub(60_000);
        {
            let mut members = state.members.lock().unwrap();
            let member = members
                .get_mut(&DaemonState::member_key(&store, "s1", "addr:a"))
                .unwrap();
            member.unattended_since_ms = Some(old_deaf_since);
            member.unattended_with_backlog_since_ms = Some(old_deaf_since);
        }

        let rearm_before_ack =
            request(state.clone(), wait_req(&store, "s1", "addr:a", 1_000)).await;
        assert!(matches!(rearm_before_ack, Response::PresenceEnded));

        let status = state.status_with_thresholds(100_000, 100_000, 30_000).await;
        assert_eq!(status.members[0].unattended_since_ms, Some(old_deaf_since));
        assert!(status.members[0]
            .unattended_for_ms
            .is_some_and(|age| age >= 60_000));
    }

    #[tokio::test]
    async fn idle_marker_without_live_waiter_preserves_recent_message_terminal_outcome() {
        let state = test_state("idle-preserves-message");
        let store = store_key("idle-preserves-message");
        registered_epoch(state.clone(), &store, "s1", "addr:a").await;
        let backend = state.backend_for(&store).await.unwrap();
        let message_id = insert_test_message(&backend, "addr:a", None).await;

        let wait = request(state.clone(), wait_req(&store, "s1", "addr:a", 1_000)).await;
        assert!(matches!(wait, Response::Message { id, .. } if id == message_id));
        let end = request(state.clone(), session_end_req(&state, &store, "s1")).await;
        assert!(matches!(end, Response::Ack { .. }));

        let status = state.status().await;
        assert!(status.members[0].idle);
        assert_eq!(
            status.members[0].last_waiter_outcome,
            Some(WaiterOutcome::Message)
        );
        assert_eq!(
            status.members[0].last_delivered_message_id,
            Some(message_id)
        );
    }

    #[test]
    fn waiter_outcome_serializes_as_stable_kebab_case() {
        let values = [
            (WaiterOutcome::Message, "message"),
            (WaiterOutcome::IdleTimeout, "idle-timeout"),
            (WaiterOutcome::PresenceEnded, "presence-ended"),
            (WaiterOutcome::AbnormalExit, "abnormal-exit"),
        ];
        for (outcome, expected) in values {
            assert_eq!(serde_json::to_value(outcome).unwrap(), expected);
        }
    }

    #[tokio::test]
    async fn wait_timeout_records_daemon_authored_terminal_status() {
        let state = test_state("timeout-terminal-status");
        let store = store_key("timeout-terminal-status");
        registered_epoch(state.clone(), &store, "s1", "addr:a").await;

        let wait = request(state.clone(), wait_req(&store, "s1", "addr:a", 1)).await;
        assert!(matches!(wait, Response::Timeout));

        let status = state.status().await;
        assert_eq!(
            status.members[0].last_waiter_outcome,
            Some(WaiterOutcome::IdleTimeout)
        );
        assert_eq!(status.members[0].last_waiter_exit_code, Some(2));
        assert_eq!(status.members[0].last_waiter_pid, Some(std::process::id()));
    }

    #[tokio::test]
    async fn daemon_owned_wait_error_does_not_record_abnormal_exit() {
        let state = test_state("wait-error-not-abnormal");
        let store = store_key("wait-error-not-abnormal");
        registered_epoch(state.clone(), &store, "s1", "addr:a").await;

        let wait = request(
            state.clone(),
            Request::Wait {
                store_key: store.clone(),
                session_id: "s1".to_string(),
                address: "addr:a".to_string(),
                attention: None,
                min_attention: Some("not-an-attention".to_string()),
                wake_on_cc: false,
                timeout_ms: Some(1_000),
                waiter_pid: Some(std::process::id()),
                waiter_start_time: crate::session_watch::capture_process_start_time(
                    std::process::id(),
                ),
            },
        )
        .await;
        assert!(matches!(
            wait,
            Response::Error { ref code, .. } if code == proto::ERROR_INCOMPATIBLE
        ));

        let status = state.status().await;
        assert_eq!(status.members[0].last_waiter_outcome, None);
        assert_eq!(status.members[0].last_waiter_pid, None);
    }

    #[tokio::test]
    async fn session_end_records_presence_ended_detail() {
        let state = test_state("session-end-terminal-status");
        let store = store_key("session-end-terminal-status");
        registered_epoch(state.clone(), &store, "s1", "addr:a").await;

        let waiter_req = wait_req(&store, "s1", "addr:a", 5_000);
        let waiter_state = state.clone();
        let waiter = tokio::spawn(async move { request(waiter_state, waiter_req).await });
        tokio::time::sleep(Duration::from_millis(75)).await;

        let end = request(state.clone(), session_end_req(&state, &store, "s1")).await;
        assert!(matches!(end, Response::Ack { .. }));
        assert!(matches!(waiter.await.unwrap(), Response::PresenceEnded));

        let status = state.status().await;
        assert_eq!(
            status.members[0].last_waiter_outcome,
            Some(WaiterOutcome::PresenceEnded)
        );
        assert_eq!(status.members[0].last_waiter_exit_code, Some(5));
        assert_eq!(
            status.members[0].last_waiter_detail.as_deref(),
            Some("session-end")
        );
    }

    #[tokio::test]
    async fn status_reports_armed_station_health() {
        let state = test_state("station-health-armed");
        let store = store_key("station-health-armed");
        registered_epoch(state.clone(), &store, "s1", "addr:a").await;

        let waiter_req = wait_req(&store, "s1", "addr:a", 5_000);
        let waiter_state = state.clone();
        let waiter = tokio::spawn(async move { request(waiter_state, waiter_req).await });
        tokio::time::sleep(Duration::from_millis(75)).await;

        let status = state.status().await;
        assert_eq!(status.members[0].station_health, StationHealth::Armed);
        assert_eq!(status.members[0].live_waiters_count, 1);

        let stopped = request(
            state.clone(),
            Request::StationStop {
                store_key: store,
                session_id: "s1".to_string(),
                address: "addr:a".to_string(),
                wait_grace_ms: 1_000,
            },
        )
        .await;
        assert!(matches!(stopped, Response::StationStopped { .. }));
        assert!(matches!(waiter.await.unwrap(), Response::PresenceEnded));
    }

    #[tokio::test]
    async fn station_stop_prevents_orphan_waiter_from_consuming_next_message() {
        let state = test_state("station-stop-no-orphan");
        let store = store_key("station-stop-no-orphan");
        registered_epoch(state.clone(), &store, "s1", "addr:a").await;

        let waiter_req = wait_req(&store, "s1", "addr:a", 5_000);
        let waiter_state = state.clone();
        let waiter = tokio::spawn(async move { request(waiter_state, waiter_req).await });
        tokio::time::sleep(Duration::from_millis(75)).await;

        let stopped = request(
            state.clone(),
            Request::StationStop {
                store_key: store.clone(),
                session_id: "s1".to_string(),
                address: "addr:a".to_string(),
                wait_grace_ms: 1_000,
            },
        )
        .await;
        assert!(matches!(
            stopped,
            Response::StationStopped {
                waiters_after: 0,
                ..
            }
        ));
        assert!(matches!(waiter.await.unwrap(), Response::PresenceEnded));

        registered_epoch(state.clone(), &store, "sender", "addr:sender").await;
        let sent = request(
            state.clone(),
            Request::Send {
                store_key: store.clone(),
                session_id: "sender".to_string(),
                from_addr: Some("addr:sender".to_string()),
                to_addr: "addr:a".to_string(),
                cc: None,
                kind: "note".to_string(),
                attention: "background".to_string(),
                requires_disposition: false,
                subject: None,
                body: "after stop".to_string(),
                metadata: None,
            },
        )
        .await;
        assert!(matches!(
            sent,
            Response::Sent {
                receipt: SentReceipt {
                    receipt,
                    occupied: Some(false),
                    ..
                }
            } if receipt == "queued-unoccupied"
        ));

        registered_epoch(state.clone(), &store, "s2", "addr:a").await;
        let delivered = request(state, wait_req(&store, "s2", "addr:a", 1_000)).await;
        assert!(matches!(
            delivered,
            Response::Message { body, .. } if body == "after stop"
        ));
    }

    #[tokio::test]
    async fn concurrent_second_waiter_is_rejected_without_duplicate_delivery() {
        let state = test_state("concurrent-waiter-dedupe");
        let store = store_key("concurrent-waiter-dedupe");
        registered_epoch(state.clone(), &store, "s1", "addr:a").await;

        let first_req = wait_req(&store, "s1", "addr:a", 5_000);
        let first_state = state.clone();
        let first = tokio::spawn(async move { request(first_state, first_req).await });
        tokio::time::sleep(Duration::from_millis(75)).await;

        let status = state.status().await;
        assert_eq!(status.live_waiters.len(), 1);

        let second = request(state.clone(), wait_req(&store, "s1", "addr:a", 5_000)).await;
        assert!(matches!(second, Response::PresenceEnded));
        assert!(state
            .status()
            .await
            .recent_errors
            .iter()
            .any(|e| e.kind == "ConcurrentWaiter"));
        let after_rejection = state.status().await;
        assert_eq!(
            after_rejection.members[0].station_health,
            StationHealth::Armed
        );
        assert_eq!(after_rejection.members[0].last_waiter_outcome, None);

        registered_epoch(state.clone(), &store, "sender", "addr:sender").await;
        let sent = request(
            state.clone(),
            Request::Send {
                store_key: store.clone(),
                session_id: "sender".to_string(),
                from_addr: Some("addr:sender".to_string()),
                to_addr: "addr:a".to_string(),
                cc: None,
                kind: "note".to_string(),
                attention: "background".to_string(),
                requires_disposition: false,
                subject: Some("dedupe".to_string()),
                body: "only one delivery".to_string(),
                metadata: None,
            },
        )
        .await;
        assert!(matches!(
            sent,
            Response::Sent {
                receipt: SentReceipt { id: message_id, .. },
            } if message_id > 0
        ));
        let delivered = first.await.unwrap();
        let id = match delivered {
            Response::Message { id, body, .. } => {
                assert_eq!(body, "only one delivery");
                id
            }
            other => panic!("first waiter should receive the message, got {other:?}"),
        };
        let ack = request(state.clone(), ack_req(&store, "s1", "addr:a", id)).await;
        assert!(matches!(
            ack,
            Response::Ack {
                delivery_outcome: Some(DeliveryOutcome::Marked),
                ..
            }
        ));

        let after_ack = request(state, wait_req(&store, "s1", "addr:a", 1)).await;
        assert!(matches!(after_ack, Response::Timeout));
    }

    #[tokio::test]
    async fn sequential_rearm_before_ack_is_rejected_without_duplicate_delivery() {
        let state = test_state("sequential-rearm-before-ack");
        let store = store_key("sequential-rearm-before-ack");
        registered_epoch(state.clone(), &store, "s1", "addr:a").await;
        let backend = state.backend_for(&store).await.unwrap();
        let message_id = insert_test_message(&backend, "addr:a", None).await;

        let first = request(state.clone(), wait_req(&store, "s1", "addr:a", 1_000)).await;
        assert!(matches!(first, Response::Message { id, .. } if id == message_id));

        let rearm_before_ack =
            request(state.clone(), wait_req(&store, "s1", "addr:a", 1_000)).await;
        assert!(matches!(rearm_before_ack, Response::PresenceEnded));
        let after_rearm_rejection = state.status().await;
        assert_eq!(
            after_rearm_rejection.members[0].last_waiter_outcome,
            Some(WaiterOutcome::Message)
        );
        assert_eq!(
            after_rearm_rejection.members[0].last_delivered_message_id,
            Some(message_id)
        );
        assert!(state
            .status()
            .await
            .recent_errors
            .iter()
            .any(|e| e.kind == "UnackedDelivery" && e.message.contains(&message_id.to_string())));

        let ack = request(state.clone(), ack_req(&store, "s1", "addr:a", message_id)).await;
        assert!(matches!(
            ack,
            Response::Ack {
                delivery_outcome: Some(DeliveryOutcome::Marked),
                ..
            }
        ));
        let after_ack = request(state, wait_req(&store, "s1", "addr:a", 1)).await;
        assert!(matches!(after_ack, Response::Timeout));
    }

    #[tokio::test]
    async fn wait_min_attention_delivers_oldest_eligible_and_preserves_skipped_lower() {
        let state = test_state("wait-min-attention");
        let store = store_key("wait-min-attention");
        registered_epoch(state.clone(), &store, "s1", "addr:a").await;
        let backend = state.backend_for(&store).await.unwrap();
        let background = backend
            .insert_message(&NewMessage {
                parent_id: None,
                from_addr: Some("sender".to_string()),
                to_addr: "addr:a".to_string(),
                cc: None,
                kind: "note".to_string(),
                attention: Attention::Background,
                requires_disposition: false,
                subject: Some("background".to_string()),
                body: "background body".to_string(),
                metadata: None,
                sent_at_ms: now_ms(),
            })
            .await
            .unwrap()
            .id;
        let interrupt = backend
            .insert_message(&NewMessage {
                parent_id: None,
                from_addr: Some("sender".to_string()),
                to_addr: "addr:a".to_string(),
                cc: None,
                kind: "note".to_string(),
                attention: Attention::Interrupt,
                requires_disposition: false,
                subject: Some("interrupt".to_string()),
                body: "interrupt body".to_string(),
                metadata: None,
                sent_at_ms: now_ms(),
            })
            .await
            .unwrap()
            .id;

        let filtered = request(
            state.clone(),
            Request::Wait {
                store_key: store.clone(),
                session_id: "s1".to_string(),
                address: "addr:a".to_string(),
                attention: None,
                min_attention: Some("interrupt".to_string()),
                wake_on_cc: false,
                timeout_ms: Some(1_000),
                waiter_pid: Some(std::process::id()),
                waiter_start_time: crate::session_watch::capture_process_start_time(
                    std::process::id(),
                ),
            },
        )
        .await;
        assert!(matches!(filtered, Response::Message { id, .. } if id == interrupt));
        let ack = request(state.clone(), ack_req(&store, "s1", "addr:a", interrupt)).await;
        assert!(matches!(
            ack,
            Response::Ack {
                delivery_outcome: Some(DeliveryOutcome::Marked),
                ..
            }
        ));

        let bare = request(state, wait_req(&store, "s1", "addr:a", 1_000)).await;
        assert!(matches!(bare, Response::Message { id, .. } if id == background));
    }

    #[tokio::test]
    async fn wait_min_attention_times_out_when_only_lower_priority_exists() {
        let state = test_state("wait-min-attention-timeout");
        let store = store_key("wait-min-attention-timeout");
        registered_epoch(state.clone(), &store, "s1", "addr:a").await;
        let backend = state.backend_for(&store).await.unwrap();
        let background = insert_test_message(&backend, "addr:a", None).await;

        let filtered = request(
            state.clone(),
            Request::Wait {
                store_key: store.clone(),
                session_id: "s1".to_string(),
                address: "addr:a".to_string(),
                attention: None,
                min_attention: Some("interrupt".to_string()),
                wake_on_cc: false,
                timeout_ms: Some(1),
                waiter_pid: Some(std::process::id()),
                waiter_start_time: crate::session_watch::capture_process_start_time(
                    std::process::id(),
                ),
            },
        )
        .await;
        assert!(matches!(filtered, Response::Timeout));

        let bare = request(state, wait_req(&store, "s1", "addr:a", 1_000)).await;
        assert!(matches!(bare, Response::Message { id, .. } if id == background));
    }

    #[tokio::test]
    async fn detach_releases_waiter_without_consuming_later_message() {
        let state = test_state("detach-no-orphan");
        let store = store_key("detach-no-orphan");
        registered_epoch(state.clone(), &store, "s1", "addr:a").await;

        let waiter_req = wait_req(&store, "s1", "addr:a", 5_000);
        let waiter_state = state.clone();
        let waiter = tokio::spawn(async move { request(waiter_state, waiter_req).await });
        tokio::time::sleep(Duration::from_millis(75)).await;

        let detached = request(
            state.clone(),
            Request::Detach {
                store_key: store.clone(),
                session_id: "s1".to_string(),
                address: "addr:a".to_string(),
            },
        )
        .await;
        assert!(matches!(detached, Response::Ack { .. }));
        assert!(matches!(
            waiter.await.unwrap(),
            Response::Error {
                needs_attach_reason: Some(NeedsAttachReason::DeliberatelyDetached),
                ..
            }
        ));

        registered_epoch(state.clone(), &store, "sender", "addr:sender").await;
        let sent = request(
            state.clone(),
            Request::Send {
                store_key: store.clone(),
                session_id: "sender".to_string(),
                from_addr: Some("addr:sender".to_string()),
                to_addr: "addr:a".to_string(),
                cc: None,
                kind: "note".to_string(),
                attention: "background".to_string(),
                requires_disposition: false,
                subject: None,
                body: "after detach".to_string(),
                metadata: None,
            },
        )
        .await;
        assert!(matches!(
            sent,
            Response::Sent {
                receipt: SentReceipt {
                    receipt,
                    occupied: Some(false),
                    ..
                }
            } if receipt == "queued-unoccupied"
        ));
        registered_epoch(state.clone(), &store, "s2", "addr:a").await;
        let delivered = request(state, wait_req(&store, "s2", "addr:a", 1_000)).await;
        assert!(matches!(
            delivered,
            Response::Message { body, .. } if body == "after detach"
        ));
    }

    #[tokio::test]
    async fn reset_marks_idle_non_destructively_and_audits_prior_occupant() {
        let state = test_state("reset");
        let store = store_key("reset");
        let epoch = registered_epoch(state.clone(), &store, "s1", "addr:a").await;
        let backend = state.backend_for(&store).await.unwrap();

        let waiter_req = wait_req(&store, "s1", "addr:a", 5_000);
        let waiter_state = state.clone();
        let waiter = tokio::spawn(async move { request(waiter_state, waiter_req).await });
        tokio::time::sleep(Duration::from_millis(50)).await;

        let reset = request(state.clone(), reset_req(&state, &store, "addr:a")).await;
        assert!(matches!(reset, Response::Ack { .. }));
        assert!(matches!(waiter.await.unwrap(), Response::PresenceEnded));

        let status = state.status().await;
        assert_eq!(status.members.len(), 1);
        assert!(status.members[0].idle);
        assert!(status
            .recent_errors
            .iter()
            .any(|e| { e.kind == "Reset" && e.message.contains("prior_occupant=occupant-s1") }));
        let lease = backend.get_lease("addr:a").await.unwrap().unwrap();
        assert_eq!(lease.lease_epoch, Some(epoch));
        assert_eq!(lease.owner_instance_id, None);
    }

    #[tokio::test]
    async fn watch_pid_start_time_mismatch_marks_idle_and_releases_waiter() {
        let state = test_state("watch-mismatch");
        let store = store_key("watch-mismatch");
        let mut register = register_req(&store, "s1", "addr:a");
        if let Request::Register { watch_pids, .. } = &mut register {
            *watch_pids = vec![WatchPidSpec::anchor(std::process::id())];
        }
        assert!(matches!(
            request(state.clone(), register).await,
            Response::Registered { .. }
        ));

        let can_test = {
            let mut members = state.members.lock().unwrap();
            let member = members
                .get_mut(&DaemonState::member_key(&store, "s1", "addr:a"))
                .unwrap();
            if let Some(start_time) = member.watch_pids[0].start_time {
                member.watch_pids[0].start_time = Some(start_time.saturating_add(1));
                true
            } else {
                false
            }
        };
        if !can_test {
            return;
        }

        let waiter_req = wait_req(&store, "s1", "addr:a", 5_000);
        let waiter_state = state.clone();
        let waiter = tokio::spawn(async move { request(waiter_state, waiter_req).await });
        tokio::time::sleep(Duration::from_millis(50)).await;

        heartbeat_members_once(state.clone()).await;
        assert!(matches!(waiter.await.unwrap(), Response::PresenceEnded));
        let status = state.status().await;
        assert!(status.members[0].idle);
        assert!(status
            .recent_errors
            .iter()
            .any(|e| e.kind == "WatchPidDeath"));
        let backend = state.backend_for(&store).await.unwrap();
        let lease = backend.get_lease("addr:a").await.unwrap().unwrap();
        assert_eq!(lease.owner_instance_id, None);
        assert!(matches!(
            request(state.clone(), register_req(&store, "s2", "addr:a")).await,
            Response::Registered { .. }
        ));
    }

    #[tokio::test]
    async fn session_end_release_failure_keeps_member_active_for_retry() {
        let state = test_state("session-end-release-failure");
        let store = store_key("session-end-release-failure");
        registered_epoch(state.clone(), &store, "s1", "addr:a").await;

        let mut member = {
            let mut members = state.members.lock().unwrap();
            members
                .remove(&DaemonState::member_key(&store, "s1", "addr:a"))
                .unwrap()
        };
        member.store_key = "unsupported:release-failure".to_string();
        state.insert_member(member);

        let response = session_end(
            state.clone(),
            "unsupported:release-failure".to_string(),
            "s1".to_string(),
        )
        .await;
        assert!(matches!(
            response,
            Response::Error { code, .. } if code == proto::ERROR_UNSUPPORTED
        ));
        let status = state.status().await;
        let member = status
            .members
            .iter()
            .find(|m| m.store_key == "unsupported:release-failure")
            .expect("member retained after failed release");
        assert!(!member.idle);
    }

    #[tokio::test]
    async fn idle_ttl_returns_presence_ended_without_deleting_membership_or_deliveries() {
        let state = test_state("idle-ttl");
        let store = store_key("idle-ttl");
        registered_epoch(state.clone(), &store, "s1", "addr:a").await;

        let response = wait_for_message_with_idle_ttl(
            state.clone(),
            store.clone(),
            "s1".to_string(),
            "addr:a".to_string(),
            None,
            None,
            false,
            Some(5_000),
            Some(std::process::id()),
            crate::session_watch::capture_process_start_time(std::process::id()),
            Duration::from_millis(20),
        )
        .await;
        assert!(matches!(response, Response::PresenceEnded));
        let status = state.status().await;
        assert_eq!(status.members.len(), 1);
        assert!(status.members[0].idle);
        assert!(status.recent_errors.iter().any(|e| e.kind == "IdleTtlReap"));

        let backend = state.backend_for(&store).await.unwrap();
        let message_id = insert_test_message(&backend, "addr:a", None).await;
        assert!(backend.delivery_retention_count().await.unwrap() >= 1);
        let wait = request(state, wait_req(&store, "s1", "addr:a", 1_000)).await;
        assert!(matches!(wait, Response::Message { id, .. } if id == message_id));
    }

    #[tokio::test]
    async fn status_lists_p5_fields_retention_warnings_and_redacts_caps() {
        let state = test_state("status-p5");
        let store = store_key("status-p5");
        let mut register = register_req(&store, "s1", "addr:a");
        if let Request::Register { watch_pids, .. } = &mut register {
            *watch_pids = vec![
                WatchPidSpec::anchor(std::process::id()),
                WatchPidSpec {
                    pid: 2_000_000_000,
                    role: WatchPidRole::Anchor,
                },
            ];
        }
        assert!(matches!(
            request(state.clone(), register).await,
            Response::Registered { .. }
        ));
        let backend = state.backend_for(&store).await.unwrap();
        insert_test_message(&backend, "addr:a", None).await;
        assert!(matches!(
            request(state.clone(), reset_req(&state, &store, "addr:a")).await,
            Response::Ack { .. }
        ));

        let status = state.status_with_thresholds(0, 1, 0).await;
        assert_eq!(status.protocol_version, current_protocol_version());
        assert_eq!(status.instance_id, state.instance_id.as_str());
        assert!(status.stores.iter().any(|s| s.store_key == store));
        assert_eq!(status.members.len(), 1);
        assert_eq!(status.members[0].backend.as_str(), "sqlite");
        assert!(status.members[0].idle);
        assert_eq!(status.members[0].watch_pids.len(), 2);
        assert!(status.members[0].watch_pids.iter().any(|p| p.alive));
        assert!(status.members[0].watch_pids.iter().any(|p| !p.alive));
        assert_eq!(status.epoch_by_address.len(), 1);
        assert!(status.epoch_by_address[0].idle);
        assert_eq!(status.retention.len(), 1);
        assert!(status.retention[0].delivery_rows >= 1);
        assert!(status.retention[0].warn);
        assert_eq!(status.idle_stations.count, 1);
        assert!(status.idle_stations.warn);
        assert!(status.recent_errors.iter().any(|e| e.kind == "Reset"));
        let json = serde_json::to_string(&status).unwrap();
        assert!(!json.contains(&state.admin_cap));
        assert!(!json.contains("proof"));
    }

    #[tokio::test]
    async fn status_detail_requires_proof_and_minimal_status_is_unprivileged() {
        let state = test_state("status-detail");
        let store = store_key("status-detail");
        registered_epoch(state.clone(), &store, "s1", "addr:a").await;

        let minimal = request(
            state.clone(),
            Request::Status {
                store_key: Some(store.clone()),
                detail: false,
                proof: None,
            },
        )
        .await;
        match minimal {
            Response::StatusReport { status } => {
                assert!(status.members.is_empty());
                assert!(status.recent_errors.is_empty());
                assert!(status.backoff.iter().any(|b| b.contains("n/a")));
            }
            other => panic!("expected minimal status, got {other:?}"),
        }

        let denied = request(
            state.clone(),
            Request::Status {
                store_key: Some(store.clone()),
                detail: true,
                proof: None,
            },
        )
        .await;
        assert!(matches!(
            denied,
            Response::Error { ref code, .. } if code == proto::ERROR_UNAUTHORIZED
        ));

        let detailed = request(
            state.clone(),
            Request::Status {
                store_key: Some(store),
                detail: true,
                proof: Some(state.admin_cap.clone()),
            },
        )
        .await;
        match detailed {
            Response::StatusReport { status } => assert_eq!(status.members.len(), 1),
            other => panic!("expected detailed status, got {other:?}"),
        }
    }

    #[test]
    fn idle_ttl_env_values_are_clamped_outside_test_helpers() {
        assert_eq!(
            clamp_idle_ttl(Duration::from_millis(1), false),
            DEFAULT_IDLE_TTL
        );
        assert_eq!(
            clamp_idle_ttl(Duration::from_millis(1), true),
            Duration::from_millis(1)
        );
        assert_eq!(
            clamp_idle_ttl(DEFAULT_IDLE_TTL + Duration::from_millis(1), false),
            DEFAULT_IDLE_TTL + Duration::from_millis(1)
        );
    }

    #[tokio::test]
    async fn cross_store_session_end_only_reaps_matching_store_session() {
        let state = test_state("cross-store-session-end");
        let store_a = store_key("cross-store-a");
        let store_b = store_key("cross-store-b");
        registered_epoch(state.clone(), &store_a, "same-session", "addr:a").await;
        registered_epoch(state.clone(), &store_b, "same-session", "addr:b").await;

        let waiter_a_req = wait_req(&store_a, "same-session", "addr:a", 5_000);
        let waiter_b_req = wait_req(&store_b, "same-session", "addr:b", 5_000);
        let waiter_a_state = state.clone();
        let waiter_b_state = state.clone();
        let waiter_a = tokio::spawn(async move { request(waiter_a_state, waiter_a_req).await });
        let waiter_b = tokio::spawn(async move { request(waiter_b_state, waiter_b_req).await });
        tokio::time::sleep(Duration::from_millis(50)).await;

        let end = request(
            state.clone(),
            session_end_req(&state, &store_a, "same-session"),
        )
        .await;
        assert!(matches!(end, Response::Ack { .. }));
        assert!(matches!(waiter_a.await.unwrap(), Response::PresenceEnded));

        let backend_b = state.backend_for(&store_b).await.unwrap();
        let message_id = insert_test_message(&backend_b, "addr:b", None).await;
        assert!(
            matches!(waiter_b.await.unwrap(), Response::Message { id, .. } if id == message_id)
        );

        let status = state.status().await;
        let member_a = status
            .members
            .iter()
            .find(|m| m.store_key == store_a)
            .unwrap();
        let member_b = status
            .members
            .iter()
            .find(|m| m.store_key == store_b)
            .unwrap();
        assert!(member_a.idle);
        assert!(!member_b.idle);
    }

    #[tokio::test]
    async fn session_id_reuse_tripwire_emits_recent_error_warning() {
        let state = test_state("session-reuse");
        let store = store_key("session-reuse");
        registered_epoch(state.clone(), &store, "s1", "addr:a").await;
        assert!(matches!(
            request(state.clone(), session_end_req(&state, &store, "s1")).await,
            Response::Ack { .. }
        ));
        assert!(matches!(
            request(state.clone(), register_req(&store, "s1", "addr:b")).await,
            Response::Registered { .. }
        ));
        let status = state.status().await;
        assert!(status.recent_errors.iter().any(|e| {
            e.kind == "SessionIdReuse" && e.message.contains("SESSION_ID_REUSE_TRIPWIRE")
        }));
    }

    #[tokio::test]
    async fn session_id_reuse_tripwire_warns_on_same_shape_after_definite_end() {
        let state = test_state("session-reuse-same");
        let store = store_key("session-reuse-same");
        registered_epoch(state.clone(), &store, "s1", "addr:a").await;
        assert!(matches!(
            request(state.clone(), session_end_req(&state, &store, "s1")).await,
            Response::Ack { .. }
        ));
        assert!(matches!(
            request(state.clone(), register_req(&store, "s1", "addr:a")).await,
            Response::Registered { .. }
        ));
        let status = state.status().await;
        assert!(status.recent_errors.iter().any(|e| {
            e.kind == "SessionIdReuse"
                && e.message.contains("SESSION_ID_REUSE_TRIPWIRE")
                && e.message.contains("addr:a")
        }));
    }
}

#[cfg(feature = "sqlite")]
#[doc(hidden)]
pub mod test_support {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEST_SEQ: AtomicU64 = AtomicU64::new(1);

    #[derive(Clone)]
    pub struct TestDaemon {
        state: Arc<DaemonState>,
        root: PathBuf,
    }

    pub struct ListenerGuard {
        _inner: platform::Listener,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum TestClientAction {
        Continue,
        Drain,
    }

    impl From<ClientAction> for TestClientAction {
        fn from(value: ClientAction) -> Self {
            match value {
                ClientAction::Continue => Self::Continue,
                ClientAction::Drain => Self::Drain,
            }
        }
    }

    impl TestDaemon {
        pub fn new(label: &str) -> Self {
            Self::with_protocol(label, proto::PROTOCOL_MAJOR)
        }

        pub fn with_protocol(label: &str, protocol_major: u16) -> Self {
            let seq = TEST_SEQ.fetch_add(1, Ordering::SeqCst);
            let root = Self::test_root(label, seq);
            std::fs::create_dir_all(root.join("stores")).expect("create test root");
            let singleton =
                SingletonKey::from_parts("test-user", root.join("config"), protocol_major);
            let state = Arc::new(DaemonState {
                paths: DaemonPaths::for_key(singleton, root.join("run")),
                instance_id: format!("inst-{label}-{seq}"),
                admin_cap: format!("cap-{label}-{seq}"),
                stores: Mutex::new(HashMap::new()),
                store_open_guard: AsyncMutex::new(()),
                members: Mutex::new(BTreeMap::new()),
                waiters: Mutex::new(BTreeMap::new()),
                next_waiter_id: AtomicU64::new(1),
                recent_errors: Arc::new(Mutex::new(VecDeque::new())),
                ended_sessions: Mutex::new(BTreeMap::new()),
                draining: AtomicBool::new(false),
                on_deliver: OnDeliverState::default(),
            });
            Self { state, root }
        }

        pub fn root(&self) -> &Path {
            &self.root
        }

        fn test_root(_label: &str, seq: u64) -> PathBuf {
            #[cfg(unix)]
            {
                std::env::temp_dir().join(format!("td{}-{seq}", std::process::id()))
            }
            #[cfg(not(unix))]
            {
                std::env::current_dir()
                    .expect("current dir")
                    .join("target")
                    .join("daemon-core-sqlite-tests")
                    .join(format!("{}-{}-{seq}", _label, std::process::id()))
            }
        }

        pub fn paths(&self) -> &DaemonPaths {
            &self.state.paths
        }

        pub fn instance_id(&self) -> &str {
            &self.state.instance_id
        }

        pub fn admin_cap(&self) -> &str {
            &self.state.admin_cap
        }

        pub fn store_path(&self, label: &str) -> PathBuf {
            let seq = TEST_SEQ.fetch_add(1, Ordering::SeqCst);
            self.root
                .join("stores")
                .join(format!("{label}-{}-{seq}.db", std::process::id()))
        }

        pub fn store_key(&self, label: &str) -> String {
            store_key_for_path(self.store_path(label))
        }

        pub async fn request(&self, request: Request) -> Response {
            handle_request(self.state.clone(), request).await.0
        }

        pub async fn request_with_action(&self, request: Request) -> (Response, TestClientAction) {
            let (response, action) = handle_request(self.state.clone(), request).await;
            (response, action.into())
        }

        pub async fn register(&self, store_key: &str, session_id: &str, address: &str) -> Response {
            self.request(register_request(store_key, session_id, address))
                .await
        }

        pub async fn register_with_watch_pids(
            &self,
            store_key: &str,
            session_id: &str,
            address: &str,
            watch_pids: Vec<WatchPidSpec>,
        ) -> Response {
            self.request(register_request_with_watch_pids(
                store_key, session_id, address, watch_pids,
            ))
            .await
        }

        pub async fn wait(
            &self,
            store_key: &str,
            session_id: &str,
            address: &str,
            timeout_ms: u64,
        ) -> Response {
            self.request(wait_request(store_key, session_id, address, timeout_ms))
                .await
        }

        pub async fn wait_with_idle_ttl(
            &self,
            store_key: &str,
            session_id: &str,
            address: &str,
            timeout_ms: u64,
            idle_ttl: Duration,
        ) -> Response {
            wait_for_message_with_idle_ttl(
                self.state.clone(),
                store_key.to_string(),
                session_id.to_string(),
                address.to_string(),
                None,
                None,
                false,
                Some(timeout_ms),
                Some(std::process::id()),
                crate::session_watch::capture_process_start_time(std::process::id()),
                idle_ttl,
            )
            .await
        }

        pub async fn ack(
            &self,
            store_key: &str,
            session_id: &str,
            address: &str,
            message_id: i64,
        ) -> Response {
            self.request(ack_request(store_key, session_id, address, message_id))
                .await
        }

        pub async fn session_end(&self, store_key: &str, session_id: &str) -> Response {
            self.request(Request::SessionEnd {
                store_key: store_key.to_string(),
                session_id: session_id.to_string(),
                proof: Some(self.state.admin_cap.clone()),
            })
            .await
        }

        pub async fn reset(&self, store_key: &str, address: &str) -> Response {
            self.request(Request::Reset {
                store_key: store_key.to_string(),
                address: address.to_string(),
                proof: Some(self.state.admin_cap.clone()),
            })
            .await
        }

        pub async fn drain(&self) -> (Response, TestClientAction) {
            self.request_with_action(Request::Drain {
                proof: Some(self.state.admin_cap.clone()),
            })
            .await
        }

        pub async fn status(&self) -> DaemonStatus {
            self.state.status().await
        }

        pub async fn status_with_thresholds(
            &self,
            retention_warn_threshold: i64,
            idle_station_warn_threshold: usize,
            deaf_warn_threshold_ms: i64,
        ) -> DaemonStatus {
            self.state
                .status_with_thresholds(
                    retention_warn_threshold,
                    idle_station_warn_threshold,
                    deaf_warn_threshold_ms,
                )
                .await
        }

        pub async fn backend(
            &self,
            store_key: &str,
        ) -> std::result::Result<Arc<dyn Backend>, Response> {
            self.state.backend_for(store_key).await
        }

        pub async fn heartbeat_once(&self) {
            heartbeat_members_once(self.state.clone()).await;
        }

        pub fn rewind_on_deliver_attempt(
            &self,
            store_key: &str,
            session_id: &str,
            address: &str,
            message_id: i64,
            by: Duration,
        ) -> bool {
            let member_key = DaemonState::member_key(store_key, session_id, address);
            let mut pushed = self.state.on_deliver.pushed.lock().unwrap();
            let Some(attempt) = pushed
                .get_mut(&member_key)
                .and_then(|attempts| attempts.get_mut(&message_id))
            else {
                return false;
            };
            attempt.last = attempt.last.checked_sub(by).unwrap_or(attempt.last);
            true
        }

        pub fn skew_first_watch_pid_start_time(
            &self,
            store_key: &str,
            session_id: &str,
            address: &str,
        ) -> bool {
            let mut members = self.state.members.lock().unwrap();
            let Some(member) =
                members.get_mut(&DaemonState::member_key(store_key, session_id, address))
            else {
                return false;
            };
            let Some(first) = member.watch_pids.first_mut() else {
                return false;
            };
            if let Some(start_time) = first.start_time {
                first.start_time = Some(start_time.saturating_add(1));
                true
            } else {
                false
            }
        }
    }

    pub fn store_key_for_path(path: impl Into<PathBuf>) -> String {
        format!("sqlite:{}", path.into().to_string_lossy())
    }

    pub fn store_path_from_key(store_key: &str) -> Option<PathBuf> {
        store_key.strip_prefix("sqlite:").map(PathBuf::from)
    }

    pub fn register_request(store_key: &str, session_id: &str, address: &str) -> Request {
        register_request_with_watch_pids(
            store_key,
            session_id,
            address,
            vec![WatchPidSpec::anchor(std::process::id())],
        )
    }

    pub fn register_request_with_watch_pids(
        store_key: &str,
        session_id: &str,
        address: &str,
        watch_pids: Vec<WatchPidSpec>,
    ) -> Request {
        Request::Register {
            store_key: store_key.to_string(),
            address: address.to_string(),
            session_id: session_id.to_string(),
            occupant: format!("occupant-{session_id}"),
            description: Some("daemon-core sqlite test member".to_string()),
            scope: Some("scope:test".to_string()),
            tags: Some("section17".to_string()),
            watch_pids,
            recovery: false,
            on_deliver: None,
            on_deliver_wake_on_cc: false,
        }
    }

    pub fn wait_request(
        store_key: &str,
        session_id: &str,
        address: &str,
        timeout_ms: u64,
    ) -> Request {
        Request::Wait {
            store_key: store_key.to_string(),
            session_id: session_id.to_string(),
            address: address.to_string(),
            attention: None,
            min_attention: None,
            wake_on_cc: false,
            timeout_ms: Some(timeout_ms),
            waiter_pid: Some(std::process::id()),
            waiter_start_time: crate::session_watch::capture_process_start_time(std::process::id()),
        }
    }

    pub fn ack_request(
        store_key: &str,
        session_id: &str,
        address: &str,
        message_id: i64,
    ) -> Request {
        Request::Ack {
            store_key: store_key.to_string(),
            session_id: session_id.to_string(),
            address: address.to_string(),
            message_id,
        }
    }

    pub fn send_request(
        store_key: &str,
        session_id: &str,
        from_addr: Option<&str>,
        to_addr: &str,
        cc: Option<&str>,
        body: &str,
    ) -> Request {
        Request::Send {
            store_key: store_key.to_string(),
            session_id: session_id.to_string(),
            from_addr: from_addr.map(str::to_string),
            to_addr: to_addr.to_string(),
            cc: cc.map(str::to_string),
            kind: "note".to_string(),
            attention: "background".to_string(),
            requires_disposition: false,
            subject: None,
            body: body.to_string(),
            metadata: None,
        }
    }

    pub fn paths_for(
        user_identity: &str,
        config_root: impl Into<PathBuf>,
        run_dir: impl Into<PathBuf>,
        protocol_major: u16,
    ) -> DaemonPaths {
        DaemonPaths::for_key(
            SingletonKey::from_parts(user_identity, config_root.into(), protocol_major),
            run_dir,
        )
    }

    pub fn bind_listener(paths: &DaemonPaths) -> Result<ListenerGuard> {
        platform::Listener::bind(&paths.endpoint).map(|inner| ListenerGuard { _inner: inner })
    }

    pub async fn registered_epoch(
        daemon: &TestDaemon,
        store_key: &str,
        session_id: &str,
        address: &str,
    ) -> (i64, String) {
        match daemon.register(store_key, session_id, address).await {
            Response::Registered {
                lease_epoch,
                owner_instance_id,
            } => (lease_epoch, owner_instance_id),
            other => panic!("expected Registered, got {other:?}"),
        }
    }
}

fn random_token(prefix: &str) -> Result<String> {
    let mut bytes = [0u8; 32];
    getrandom::getrandom(&mut bytes).map_err(|e| DaemonError::Unsupported {
        capability: "secure random admin capability",
        message: e.to_string(),
    })?;
    Ok(format!("{prefix}-{}", hex_encode(&bytes)))
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}

fn canonical_current_exe() -> Result<PathBuf> {
    std::env::current_exe()
        .map_err(|e| io_err("resolving current executable", e))
        .and_then(|p| {
            std::fs::canonicalize(&p).map_err(|e| {
                io_err(
                    "canonicalizing current executable",
                    std::io::Error::new(e.kind(), format!("{}: {e}", p.display())),
                )
            })
        })
}

fn prepare_config_root() -> Result<PathBuf> {
    let home = crate::config::telex_home()
        .map_err(|e| DaemonError::Protocol(format!("resolving TELEX_HOME: {e:#}")))?;
    // config_root is singleton identity material only. Authority-bearing runtime artifacts
    // (cap files, locks, sockets) live under run_dir and keep the fail-closed owner-private check.
    std::fs::create_dir_all(&home).map_err(|e| io_err("creating daemon config root", e))?;
    std::fs::canonicalize(&home).map_err(|e| io_err("canonicalizing daemon config root", e))
}

pub fn resolved_runtime_dir() -> Result<PathBuf> {
    if let Ok(dir) = std::env::var("TELEX_RUN_DIR") {
        return Ok(PathBuf::from(dir));
    }
    crate::config::run_dir()
        .map_err(|e| DaemonError::Protocol(format!("resolving runtime directory: {e:#}")))
}

fn prepare_runtime_dir() -> Result<PathBuf> {
    let run_dir = resolved_runtime_dir()?;
    platform::ensure_owner_private_dir(&run_dir)
}

#[cfg(unix)]
mod platform {
    use super::*;
    use std::os::unix::fs::{
        DirBuilderExt, FileTypeExt, MetadataExt, OpenOptionsExt, PermissionsExt,
    };
    use std::os::unix::io::AsRawFd;
    use tokio::net::{UnixListener, UnixStream};

    pub type ClientConn = UnixStream;
    pub type ServerConn = UnixStream;

    pub struct Listener {
        inner: UnixListener,
        path: PathBuf,
        _lock: std::fs::File,
    }

    impl Listener {
        pub fn bind(endpoint: &Endpoint) -> Result<Self> {
            let path = match endpoint {
                Endpoint::UnixSocket(path) => path,
            };
            if let Some(parent) = path.parent() {
                ensure_owner_private_dir(parent)?;
            }
            let lock = acquire_endpoint_lock(path)?;
            if path.exists() {
                match std::os::unix::net::UnixStream::connect(path) {
                    Ok(_) => {
                        return Err(DaemonError::AlreadyRunning(format!(
                            "endpoint {} is live",
                            path.display()
                        )));
                    }
                    Err(_) => {
                        let meta = std::fs::symlink_metadata(path)
                            .map_err(|e| io_err("checking stale daemon socket", e))?;
                        if !meta.file_type().is_socket() {
                            return Err(DaemonError::AlreadyRunning(format!(
                                "endpoint path {} exists and is not a socket",
                                path.display()
                            )));
                        }
                        if stale_socket_owner_is_dead(path) {
                            std::fs::remove_file(path)
                                .map_err(|e| io_err("removing stale daemon socket", e))?;
                        } else {
                            return Err(DaemonError::AlreadyRunning(format!(
                                "endpoint socket {} exists and daemon liveness was not disproven",
                                path.display()
                            )));
                        }
                    }
                }
            }
            let inner = UnixListener::bind(path).map_err(|e| io_err("binding daemon socket", e))?;
            Ok(Self {
                inner,
                path: path.clone(),
                _lock: lock,
            })
        }

        pub async fn accept(&mut self) -> Result<ServerConn> {
            let (conn, _) = self
                .inner
                .accept()
                .await
                .map_err(|e| io_err("accepting daemon client", e))?;
            Ok(conn)
        }

        pub fn ready_for_next(&mut self) -> Result<()> {
            Ok(())
        }
    }

    impl Drop for Listener {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.path);
        }
    }

    fn acquire_endpoint_lock(endpoint: &Path) -> Result<std::fs::File> {
        let lock_path = endpoint.with_extension("lock");
        let lock = std::fs::OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .mode(0o600)
            .open(&lock_path)
            .map_err(|e| io_err("opening daemon endpoint lock", e))?;
        let rc = unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
        if rc == 0 {
            return Ok(lock);
        }
        let err = std::io::Error::last_os_error();
        if matches!(err.raw_os_error(), Some(e) if e == libc::EWOULDBLOCK || e == libc::EAGAIN) {
            return Err(DaemonError::AlreadyRunning(format!(
                "endpoint lock {} is already held",
                lock_path.display()
            )));
        }
        Err(io_err("locking daemon endpoint", err))
    }

    fn stale_socket_owner_is_dead(endpoint: &Path) -> bool {
        let Some(cap_path) = inferred_cap_path(endpoint) else {
            return false;
        };
        let Ok(cap) = read_cap_file(&cap_path) else {
            return false;
        };
        let Some(endpoint_hash) = endpoint_hash(endpoint) else {
            return false;
        };
        if cap.singleton_hash != endpoint_hash {
            return false;
        }
        let Ok((pid, start_time)) = cap_required_peer_identity(&cap) else {
            return false;
        };
        !crate::session_watch::process_alive_with_start_time(pid, Some(start_time))
    }

    fn inferred_cap_path(endpoint: &Path) -> Option<PathBuf> {
        let hash = endpoint_hash(endpoint)?;
        Some(endpoint.parent()?.join(format!("daemon-{hash}.cap")))
    }

    fn endpoint_hash(endpoint: &Path) -> Option<String> {
        let file_name = endpoint.file_name()?.to_str()?;
        let hash = file_name
            .strip_prefix("telex-daemon-")?
            .strip_suffix(".sock")?;
        Some(hash.to_string())
    }

    pub async fn connect(endpoint: &Endpoint) -> Result<ClientConn> {
        let path = match endpoint {
            Endpoint::UnixSocket(path) => path,
        };
        UnixStream::connect(path).await.map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                DaemonError::NotRunning(format!("endpoint {} does not exist", path.display()))
            } else {
                io_err("connecting to daemon socket", e)
            }
        })
    }

    pub fn current_user_identity() -> Result<String> {
        Ok(format!("uid:{}", unsafe { libc::geteuid() }))
    }

    pub fn ensure_owner_private_dir(path: &Path) -> Result<PathBuf> {
        if !path.exists() {
            let mut builder = std::fs::DirBuilder::new();
            builder.recursive(true).mode(0o700);
            builder
                .create(path)
                .map_err(|e| io_err("creating owner-private daemon directory", e))?;
        }
        let link_meta = std::fs::symlink_metadata(path)
            .map_err(|e| io_err("checking owner-private daemon directory", e))?;
        if link_meta.file_type().is_symlink() {
            return Err(DaemonError::Unsupported {
                capability: "owner-private daemon directory",
                message: format!("{} is a symlink", path.display()),
            });
        }
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
            .map_err(|e| io_err("setting owner-private daemon directory permissions", e))?;
        let meta = std::fs::metadata(path)
            .map_err(|e| io_err("checking owner-private daemon directory", e))?;
        let uid = unsafe { libc::geteuid() };
        if meta.uid() != uid {
            return Err(DaemonError::Unsupported {
                capability: "owner-private daemon directory",
                message: format!(
                    "{} is owned by uid {}, expected uid {}",
                    path.display(),
                    meta.uid(),
                    uid
                ),
            });
        }
        if meta.mode() & 0o077 != 0 {
            return Err(DaemonError::Unsupported {
                capability: "owner-private daemon directory",
                message: format!("{} is group/world accessible", path.display()),
            });
        }
        std::fs::canonicalize(path).map_err(|e| io_err("canonicalizing daemon directory", e))
    }

    pub fn write_owner_only_file(path: &Path, bytes: &[u8]) -> Result<()> {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)
            .map_err(|e| io_err("creating owner-only daemon capability file", e))?;
        use std::io::Write;
        file.write_all(bytes)
            .map_err(|e| io_err("writing daemon capability file", e))?;
        file.write_all(b"\n")
            .map_err(|e| io_err("writing daemon capability file", e))?;
        file.sync_all()
            .map_err(|e| io_err("syncing daemon capability file", e))?;
        Ok(())
    }

    pub fn verify_client_peer(conn: &ServerConn) -> Result<()> {
        let (_pid, uid) = peer_pid_uid(conn)?;
        let current = unsafe { libc::geteuid() };
        if uid != current {
            return Err(DaemonError::Unauthorized(format!(
                "client uid {uid} does not match daemon uid {current}"
            )));
        }
        Ok(())
    }

    pub fn verify_server_peer(
        conn: &ClientConn,
        expected_exe: &Path,
        expected_pid: Option<u32>,
        expected_start_time: Option<u64>,
    ) -> Result<()> {
        let (pid, uid) = peer_pid_uid(conn)?;
        let current = unsafe { libc::geteuid() };
        if uid != current {
            return Err(DaemonError::Unauthorized(format!(
                "server uid {uid} does not match client uid {current}"
            )));
        }
        let exe = std::fs::canonicalize(format!("/proc/{pid}/exe")).map_err(|e| {
            DaemonError::Unsupported {
                capability: "client-side server executable verification",
                message: format!("cannot verify /proc/{pid}/exe: {e}"),
            }
        })?;
        if !same_canonical_path(&exe, expected_exe) {
            return Err(DaemonError::Unauthorized(format!(
                "server executable {} does not match {}",
                exe.display(),
                expected_exe.display()
            )));
        }
        let start_time = linux_start_time_ticks(pid)?;
        let pid = u32::try_from(pid).map_err(|_| {
            DaemonError::Unauthorized(format!("server pid {pid} cannot be represented as u32"))
        })?;
        verify_expected_peer_identity(pid, Some(start_time), expected_pid, expected_start_time)?;
        Ok(())
    }

    #[cfg(target_os = "linux")]
    fn peer_pid_uid(conn: &UnixStream) -> Result<(libc::pid_t, libc::uid_t)> {
        let mut cred: libc::ucred = unsafe { std::mem::zeroed() };
        let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
        let rc = unsafe {
            libc::getsockopt(
                conn.as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_PEERCRED,
                &mut cred as *mut _ as *mut libc::c_void,
                &mut len,
            )
        };
        if rc != 0 {
            return Err(io_err(
                "reading unix peer credentials",
                std::io::Error::last_os_error(),
            ));
        }
        Ok((cred.pid, cred.uid))
    }

    #[cfg(not(target_os = "linux"))]
    fn peer_pid_uid(_conn: &UnixStream) -> Result<(libc::pid_t, libc::uid_t)> {
        Err(DaemonError::Unsupported {
            capability: "unix peer credential verification",
            message: "SO_PEERCRED implementation is only wired for Linux in P2".into(),
        })
    }

    #[cfg(target_os = "linux")]
    fn linux_start_time_ticks(pid: libc::pid_t) -> Result<u64> {
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).map_err(|e| {
            DaemonError::Unsupported {
                capability: "client-side server start-time verification",
                message: format!("cannot read /proc/{pid}/stat: {e}"),
            }
        })?;
        let after_name = stat
            .rsplit_once(") ")
            .ok_or_else(|| DaemonError::Unsupported {
                capability: "client-side server start-time verification",
                message: format!("cannot parse /proc/{pid}/stat"),
            })?;
        let fields: Vec<&str> = after_name.1.split_whitespace().collect();
        fields
            .get(19)
            .ok_or_else(|| DaemonError::Unsupported {
                capability: "client-side server start-time verification",
                message: format!("missing start-time field in /proc/{pid}/stat"),
            })?
            .parse::<u64>()
            .map_err(|e| DaemonError::Unsupported {
                capability: "client-side server start-time verification",
                message: format!("cannot parse start time for pid {pid}: {e}"),
            })
    }

    #[cfg(not(target_os = "linux"))]
    fn linux_start_time_ticks(_pid: libc::pid_t) -> Result<u64> {
        Err(DaemonError::Unsupported {
            capability: "client-side server start-time verification",
            message: "process start-time verification is only wired for Linux in P2".into(),
        })
    }
}

#[cfg(windows)]
mod platform {
    use super::*;
    use std::ffi::{c_void, OsStr, OsString};
    use std::io::Write;
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    use std::os::windows::io::{AsRawHandle, FromRawHandle};
    use tokio::net::windows::named_pipe::{ClientOptions, NamedPipeClient, NamedPipeServer};
    use windows_sys::Win32::Foundation::{
        CloseHandle, GetLastError, LocalFree, ERROR_ACCESS_DENIED, ERROR_ALREADY_EXISTS,
        ERROR_PIPE_BUSY, FILETIME, HANDLE, INVALID_HANDLE_VALUE, PSID,
    };
    use windows_sys::Win32::Security::Authorization::{
        ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
        ConvertStringSidToSidW, GetNamedSecurityInfoW, SetNamedSecurityInfoW, SDDL_REVISION_1,
        SE_FILE_OBJECT,
    };
    use windows_sys::Win32::Security::{
        AclSizeInformation, EqualSid, GetAce, GetAclInformation, GetSecurityDescriptorControl,
        GetSecurityDescriptorDacl, GetSecurityDescriptorOwner, GetTokenInformation, TokenUser,
        ACCESS_ALLOWED_ACE, ACCESS_DENIED_ACE, ACL, ACL_SIZE_INFORMATION,
        DACL_SECURITY_INFORMATION, OWNER_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION,
        SECURITY_ATTRIBUTES, SE_DACL_PRESENT, SE_DACL_PROTECTED, TOKEN_QUERY, TOKEN_USER,
    };
    use windows_sys::Win32::Storage::FileSystem::{
        CreateDirectoryW, CreateFileW, CREATE_NEW, FILE_ATTRIBUTE_NORMAL,
        FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_FLAG_OVERLAPPED,
        FILE_GENERIC_WRITE, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING, PIPE_ACCESS_DUPLEX,
    };
    use windows_sys::Win32::System::Pipes::{
        CreateNamedPipeW, GetNamedPipeClientProcessId, GetNamedPipeServerProcessId,
        PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_WAIT,
    };
    use windows_sys::Win32::System::Threading::{
        GetCurrentProcess, GetProcessTimes, OpenProcess, OpenProcessToken,
        QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION,
    };

    pub type ClientConn = NamedPipeClient;
    pub type ServerConn = NamedPipeServer;

    pub struct Listener {
        pipe_name: String,
        next: Option<NamedPipeServer>,
        first: bool,
    }

    impl Listener {
        pub fn bind(endpoint: &Endpoint) -> Result<Self> {
            let pipe_name = match endpoint {
                Endpoint::WindowsPipe(name) => name.clone(),
            };
            let next = Some(create_pipe(&pipe_name, true)?);
            Ok(Self {
                pipe_name,
                next,
                first: false,
            })
        }

        pub async fn accept(&mut self) -> Result<NamedPipeServer> {
            let server = self.next.take().ok_or_else(|| {
                DaemonError::Protocol("daemon pipe listener was not armed".into())
            })?;
            server
                .connect()
                .await
                .map_err(|e| io_err("accepting daemon named-pipe client", e))?;
            Ok(server)
        }

        pub fn ready_for_next(&mut self) -> Result<()> {
            if self.next.is_none() {
                self.next = Some(create_pipe(&self.pipe_name, self.first)?);
                self.first = false;
            }
            Ok(())
        }
    }

    pub async fn connect(endpoint: &Endpoint) -> Result<ClientConn> {
        let pipe_name = match endpoint {
            Endpoint::WindowsPipe(name) => name,
        };
        for _ in 0..20 {
            match ClientOptions::new().open(pipe_name) {
                Ok(client) => return Ok(client),
                Err(e) if e.raw_os_error() == Some(ERROR_PIPE_BUSY as i32) => {
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    return Err(DaemonError::NotRunning(format!(
                        "endpoint {pipe_name} does not exist"
                    )));
                }
                Err(e) if is_access_denied(&e) => {
                    return Err(access_denied_elevation_error(
                        format!("connecting to daemon named pipe {pipe_name}"),
                        e,
                    ));
                }
                Err(e) => return Err(io_err("connecting to daemon named pipe", e)),
            }
        }
        Err(DaemonError::Timeout(format!(
            "daemon pipe {pipe_name} stayed busy"
        )))
    }

    pub fn current_user_identity() -> Result<String> {
        let token = current_process_token()?;
        sid_string_from_token(token.0)
    }

    pub fn ensure_owner_private_dir(path: &Path) -> Result<PathBuf> {
        if !path.exists() {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| io_err("creating daemon directory parent", e))?;
            }
            create_owner_only_dir(path)?;
        }
        validate_owner_private_dir_shape(path)?;
        set_owner_only_dir_security(path)?;
        let canonical = std::fs::canonicalize(path)
            .map_err(|e| io_err("canonicalizing daemon directory", e))?;
        validate_owner_private_dir_shape(&canonical)?;
        set_owner_only_dir_security(&canonical)?;
        validate_owner_private_dir_security(&canonical)?;
        Ok(canonical)
    }

    pub fn write_owner_only_file(path: &Path, bytes: &[u8]) -> Result<()> {
        let mut sa = owner_only_security_attributes()?;
        let wide = wide_null(path.as_os_str());
        let handle = unsafe {
            CreateFileW(
                wide.as_ptr(),
                FILE_GENERIC_WRITE,
                0,
                &mut sa.attrs,
                CREATE_NEW,
                FILE_ATTRIBUTE_NORMAL,
                0,
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            return Err(io_err(
                "creating owner-only daemon capability file",
                std::io::Error::last_os_error(),
            ));
        }
        let mut file = unsafe { std::fs::File::from_raw_handle(handle as _) };
        file.write_all(bytes)
            .map_err(|e| io_err("writing daemon capability file", e))?;
        file.write_all(b"\n")
            .map_err(|e| io_err("writing daemon capability file", e))?;
        file.sync_all()
            .map_err(|e| io_err("syncing daemon capability file", e))?;
        Ok(())
    }

    pub fn verify_client_peer(conn: &NamedPipeServer) -> Result<()> {
        let mut pid = 0u32;
        let ok = unsafe { GetNamedPipeClientProcessId(conn.as_raw_handle() as HANDLE, &mut pid) };
        if ok == 0 {
            return Err(io_err(
                "reading named-pipe client pid",
                std::io::Error::last_os_error(),
            ));
        }
        verify_process_owner(pid)
    }

    pub fn verify_server_peer(
        conn: &ClientConn,
        expected_exe: &Path,
        expected_pid: Option<u32>,
        expected_start_time: Option<u64>,
    ) -> Result<()> {
        let mut pid = 0u32;
        let ok = unsafe { GetNamedPipeServerProcessId(conn.as_raw_handle() as HANDLE, &mut pid) };
        if ok == 0 {
            return Err(io_err(
                "reading named-pipe server pid",
                std::io::Error::last_os_error(),
            ));
        }
        let info = verify_process_owner_and_exe(pid, expected_exe)?;
        verify_expected_peer_identity(
            pid,
            Some(info.start_time_100ns),
            expected_pid,
            expected_start_time,
        )
    }

    fn create_pipe(pipe_name: &str, first: bool) -> Result<NamedPipeServer> {
        let mut sa = owner_only_security_attributes()?;
        let wide = wide_null(OsStr::new(pipe_name));
        let mut open_mode = PIPE_ACCESS_DUPLEX | FILE_FLAG_OVERLAPPED;
        if first {
            open_mode |= FILE_FLAG_FIRST_PIPE_INSTANCE;
        }
        let pipe_mode =
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS;
        let handle = unsafe {
            CreateNamedPipeW(
                wide.as_ptr(),
                open_mode,
                pipe_mode,
                255,
                8192,
                8192,
                0,
                &mut sa.attrs,
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            let err = std::io::Error::last_os_error();
            if err.raw_os_error() == Some(ERROR_ALREADY_EXISTS as i32)
                || (first && err.raw_os_error() == Some(ERROR_ACCESS_DENIED as i32))
            {
                return Err(DaemonError::AlreadyRunning(format!(
                    "named pipe {pipe_name} already has a first instance"
                )));
            }
            return Err(io_err("creating daemon named pipe", err));
        }
        unsafe { NamedPipeServer::from_raw_handle(handle as _) }
            .map_err(|e| io_err("wrapping daemon named pipe handle", e))
    }

    fn create_owner_only_dir(path: &Path) -> Result<()> {
        let mut sa = owner_only_security_attributes()?;
        let wide = wide_null(path.as_os_str());
        let ok = unsafe { CreateDirectoryW(wide.as_ptr(), &mut sa.attrs) };
        if ok == 0 {
            let err = unsafe { GetLastError() };
            if err == ERROR_ALREADY_EXISTS {
                return Ok(());
            }
            return Err(io_err(
                "creating owner-private daemon directory",
                std::io::Error::last_os_error(),
            ));
        }
        Ok(())
    }

    fn set_owner_only_dir_security(path: &Path) -> Result<()> {
        let sa = owner_only_security_attributes()?;
        let mut dacl_present = 0;
        let mut dacl_defaulted = 0;
        let mut dacl = std::ptr::null_mut();
        let ok = unsafe {
            GetSecurityDescriptorDacl(
                sa.descriptor,
                &mut dacl_present,
                &mut dacl,
                &mut dacl_defaulted,
            )
        };
        if ok == 0 || dacl_present == 0 || dacl.is_null() {
            return Err(io_err(
                "reading owner-private daemon directory DACL",
                std::io::Error::last_os_error(),
            ));
        }
        let mut owner_defaulted = 0;
        let mut owner = std::ptr::null_mut();
        let ok =
            unsafe { GetSecurityDescriptorOwner(sa.descriptor, &mut owner, &mut owner_defaulted) };
        if ok == 0 || owner.is_null() {
            return Err(io_err(
                "reading owner-private daemon directory owner",
                std::io::Error::last_os_error(),
            ));
        }
        let wide = wide_null(path.as_os_str());
        let rc = unsafe {
            SetNamedSecurityInfoW(
                wide.as_ptr(),
                SE_FILE_OBJECT,
                OWNER_SECURITY_INFORMATION
                    | DACL_SECURITY_INFORMATION
                    | PROTECTED_DACL_SECURITY_INFORMATION,
                owner,
                std::ptr::null_mut(),
                dacl,
                std::ptr::null_mut(),
            )
        };
        if rc != 0 {
            return Err(DaemonError::Unsupported {
                capability: "owner-private daemon directory",
                message: format!(
                    "setting DACL for {} failed: {}",
                    path.display(),
                    std::io::Error::from_raw_os_error(rc as i32)
                ),
            });
        }
        Ok(())
    }

    fn validate_owner_private_dir_shape(path: &Path) -> Result<()> {
        use std::os::windows::fs::MetadataExt;
        use std::path::{Component, Prefix};

        if path.components().any(|component| {
            matches!(
                component,
                Component::Prefix(prefix)
                    if matches!(prefix.kind(), Prefix::UNC(_, _) | Prefix::VerbatimUNC(_, _))
            )
        }) {
            return Err(DaemonError::Unsupported {
                capability: "owner-private daemon directory",
                message: format!("{} is not a local path", path.display()),
            });
        }
        let meta = std::fs::symlink_metadata(path)
            .map_err(|e| io_err("checking owner-private daemon directory", e))?;
        if !meta.is_dir() {
            return Err(DaemonError::Unsupported {
                capability: "owner-private daemon directory",
                message: format!("{} is not a directory", path.display()),
            });
        }
        if meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err(DaemonError::Unsupported {
                capability: "owner-private daemon directory",
                message: format!("{} is a reparse point", path.display()),
            });
        }
        Ok(())
    }

    fn validate_owner_private_dir_security(path: &Path) -> Result<()> {
        const ACCESS_ALLOWED_ACE_TYPE: u8 = 0;
        const ACCESS_DENIED_ACE_TYPE: u8 = 1;

        let mut sd: *mut c_void = std::ptr::null_mut();
        let mut owner: PSID = std::ptr::null_mut();
        let mut dacl: *mut ACL = std::ptr::null_mut();
        let wide = wide_null(path.as_os_str());
        let rc = unsafe {
            GetNamedSecurityInfoW(
                wide.as_ptr(),
                SE_FILE_OBJECT,
                OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
                &mut owner,
                std::ptr::null_mut(),
                &mut dacl,
                std::ptr::null_mut(),
                &mut sd,
            )
        };
        if rc != 0 {
            return Err(DaemonError::Unsupported {
                capability: "owner-private daemon directory",
                message: format!(
                    "cannot read security descriptor for {}: {}",
                    path.display(),
                    std::io::Error::from_raw_os_error(rc as i32)
                ),
            });
        }
        let _sd_guard = LocalAllocGuard(sd);

        let current_sid = sid_from_string(&current_user_identity()?)?;
        let system_sid = sid_from_string("S-1-5-18")?;
        let admins_sid = sid_from_string("S-1-5-32-544")?;

        if owner.is_null() || unsafe { EqualSid(owner, current_sid.0) } == 0 {
            return Err(DaemonError::Unsupported {
                capability: "owner-private daemon directory",
                message: format!("{} is not owned by the current SID", path.display()),
            });
        }

        let mut control = 0u16;
        let mut revision = 0u32;
        let ok = unsafe { GetSecurityDescriptorControl(sd, &mut control, &mut revision) };
        if ok == 0 || control & SE_DACL_PRESENT == 0 || control & SE_DACL_PROTECTED == 0 {
            return Err(DaemonError::Unsupported {
                capability: "owner-private daemon directory",
                message: format!("{} does not have a protected explicit DACL", path.display()),
            });
        }
        if dacl.is_null() {
            return Err(DaemonError::Unsupported {
                capability: "owner-private daemon directory",
                message: format!("{} is missing a DACL", path.display()),
            });
        }

        let mut info = ACL_SIZE_INFORMATION {
            AceCount: 0,
            AclBytesInUse: 0,
            AclBytesFree: 0,
        };
        let ok = unsafe {
            GetAclInformation(
                dacl,
                &mut info as *mut _ as *mut c_void,
                std::mem::size_of::<ACL_SIZE_INFORMATION>() as u32,
                AclSizeInformation,
            )
        };
        if ok == 0 || info.AceCount == 0 {
            return Err(io_err(
                "reading daemon directory ACL",
                std::io::Error::last_os_error(),
            ));
        }

        for idx in 0..info.AceCount {
            let mut ace_ptr: *mut c_void = std::ptr::null_mut();
            let ok = unsafe { GetAce(dacl, idx, &mut ace_ptr) };
            if ok == 0 || ace_ptr.is_null() {
                return Err(io_err(
                    "reading daemon directory ACE",
                    std::io::Error::last_os_error(),
                ));
            }

            let header = unsafe { &*(ace_ptr as *const windows_sys::Win32::Security::ACE_HEADER) };
            match header.AceType {
                ACCESS_ALLOWED_ACE_TYPE => {
                    let ace = unsafe { &*(ace_ptr as *const ACCESS_ALLOWED_ACE) };
                    let sid = (&ace.SidStart as *const u32).cast::<c_void>() as PSID;
                    let allowed = unsafe {
                        EqualSid(sid, current_sid.0) != 0
                            || EqualSid(sid, system_sid.0) != 0
                            || EqualSid(sid, admins_sid.0) != 0
                    };
                    if !allowed {
                        return Err(DaemonError::Unsupported {
                            capability: "owner-private daemon directory",
                            message: format!("{} grants access to a non-owner SID", path.display()),
                        });
                    }
                }
                ACCESS_DENIED_ACE_TYPE => {
                    let _ace = unsafe { &*(ace_ptr as *const ACCESS_DENIED_ACE) };
                    return Err(DaemonError::Unsupported {
                        capability: "owner-private daemon directory",
                        message: format!("{} contains a deny ACE", path.display()),
                    });
                }
                _ => {
                    return Err(DaemonError::Unsupported {
                        capability: "owner-private daemon directory",
                        message: format!(
                            "{} contains unsupported ACE type {}",
                            path.display(),
                            header.AceType
                        ),
                    });
                }
            }
        }

        Ok(())
    }

    #[cfg(test)]
    pub(super) fn owner_private_sddl_is_strict(sddl: &str, sid: &str) -> bool {
        if !sddl_section(sddl, "O:").is_some_and(|owner| {
            owner == sid || matches!(owner.as_str(), "OW" | "CO") || is_privileged_sid(&owner)
        }) {
            return false;
        }
        let Some(dacl) = sddl_section(sddl, "D:") else {
            return false;
        };
        let aces = parse_sddl_ace_sids(&dacl);
        if aces.is_empty() {
            return false;
        }
        let mut has_current_sid = false;
        for ace_sid in aces {
            if is_current_principal_sid(&ace_sid, sid) {
                has_current_sid = true;
                continue;
            }
            if !is_privileged_sid(&ace_sid) && !is_appcontainer_sid(&ace_sid) {
                return false;
            }
        }
        has_current_sid
    }

    #[cfg(test)]
    fn is_current_principal_sid(ace_sid: &str, current_sid: &str) -> bool {
        ace_sid == current_sid || matches!(ace_sid, "OW" | "CO") || ace_sid.starts_with("S-1-5-5-")
    }

    #[cfg(test)]
    fn is_privileged_sid(sid: &str) -> bool {
        matches!(sid, "SY" | "BA" | "S-1-5-18" | "S-1-5-32-544")
    }

    #[cfg(test)]
    fn is_appcontainer_sid(sid: &str) -> bool {
        matches!(sid, "AC") || sid.starts_with("S-1-15-2-") || sid.starts_with("S-1-15-3-")
    }

    #[cfg(test)]
    fn sddl_section(sddl: &str, marker: &str) -> Option<String> {
        let start = sddl.find(marker)? + marker.len();
        let end = ["O:", "G:", "D:", "S:"]
            .iter()
            .filter_map(|candidate| {
                sddl[start..]
                    .find(candidate)
                    .map(|offset| start + offset)
                    .filter(|idx| *idx > start)
            })
            .min()
            .unwrap_or(sddl.len());
        Some(sddl[start..end].to_string())
    }

    #[cfg(test)]
    fn parse_sddl_ace_sids(dacl: &str) -> Vec<String> {
        let mut sids = Vec::new();
        let mut rest = dacl;
        while let Some(start) = rest.find('(') {
            rest = &rest[start + 1..];
            let Some(end) = rest.find(')') else {
                return Vec::new();
            };
            let ace = &rest[..end];
            let fields: Vec<&str> = ace.split(';').collect();
            if fields.len() < 6 {
                return Vec::new();
            }
            sids.push(fields[5].to_string());
            rest = &rest[end + 1..];
        }
        sids
    }

    fn verify_process_owner(pid: u32) -> Result<()> {
        let info = process_identity(pid, None)?;
        let current = current_user_identity()?;
        if info.sid != current {
            return Err(DaemonError::Unauthorized(format!(
                "peer SID does not match current user SID"
            )));
        }
        Ok(())
    }

    fn verify_process_owner_and_exe(pid: u32, expected_exe: &Path) -> Result<ProcessIdentity> {
        let info = process_identity(pid, Some(expected_exe))?;
        let current = current_user_identity()?;
        if info.sid != current {
            return Err(DaemonError::Unauthorized(
                "server SID does not match current user SID".into(),
            ));
        }
        if let Some(exe) = &info.exe {
            if !same_canonical_path(exe, expected_exe) {
                return Err(DaemonError::Unauthorized(format!(
                    "server executable {} does not match {}",
                    exe.display(),
                    expected_exe.display()
                )));
            }
        }
        Ok(info)
    }

    #[derive(Debug)]
    struct ProcessIdentity {
        sid: String,
        exe: Option<PathBuf>,
        start_time_100ns: u64,
    }

    fn process_identity(pid: u32, expected_exe: Option<&Path>) -> Result<ProcessIdentity> {
        let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
        if process == 0 {
            let err = std::io::Error::last_os_error();
            if is_access_denied(&err) {
                return Err(access_denied_elevation_error(
                    format!("opening peer process {pid}"),
                    err,
                ));
            }
            return Err(io_err("opening peer process", err));
        }
        let process = Handle(process);
        let token = process_token(process.0)?;
        let sid = sid_string_from_token(token.0)?;
        let start_time_100ns = process_start_time(process.0)?;
        let exe = if expected_exe.is_some() {
            Some(process_exe(process.0)?)
        } else {
            None
        };
        Ok(ProcessIdentity {
            sid,
            exe,
            start_time_100ns,
        })
    }

    fn process_start_time(process: HANDLE) -> Result<u64> {
        let mut creation: FILETIME = unsafe { std::mem::zeroed() };
        let mut exit: FILETIME = unsafe { std::mem::zeroed() };
        let mut kernel: FILETIME = unsafe { std::mem::zeroed() };
        let mut user: FILETIME = unsafe { std::mem::zeroed() };
        let ok =
            unsafe { GetProcessTimes(process, &mut creation, &mut exit, &mut kernel, &mut user) };
        if ok == 0 {
            return Err(io_err(
                "reading peer process start time",
                std::io::Error::last_os_error(),
            ));
        }
        Ok(((creation.dwHighDateTime as u64) << 32) | creation.dwLowDateTime as u64)
    }

    fn process_exe(process: HANDLE) -> Result<PathBuf> {
        let mut buf = vec![0u16; 32768];
        let mut len = buf.len() as u32;
        let ok = unsafe { QueryFullProcessImageNameW(process, 0, buf.as_mut_ptr(), &mut len) };
        if ok == 0 {
            return Err(io_err(
                "reading peer process executable",
                std::io::Error::last_os_error(),
            ));
        }
        let raw = PathBuf::from(OsString::from_wide(&buf[..len as usize]));
        std::fs::canonicalize(&raw).map_err(|e| {
            io_err(
                "canonicalizing peer process executable",
                std::io::Error::new(e.kind(), format!("{}: {e}", raw.display())),
            )
        })
    }

    fn current_process_token() -> Result<Handle> {
        process_token(unsafe { GetCurrentProcess() })
    }

    fn process_token(process: HANDLE) -> Result<Handle> {
        let mut token = 0isize;
        let ok = unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut token) };
        if ok == 0 {
            let err = std::io::Error::last_os_error();
            if is_access_denied(&err) {
                return Err(access_denied_elevation_error(
                    "opening peer process token".to_string(),
                    err,
                ));
            }
            return Err(io_err("opening process token", err));
        }
        Ok(Handle(token))
    }

    fn is_access_denied(err: &std::io::Error) -> bool {
        err.raw_os_error() == Some(ERROR_ACCESS_DENIED as i32)
            || err.kind() == std::io::ErrorKind::PermissionDenied
    }

    fn access_denied_elevation_error(context: String, source: std::io::Error) -> DaemonError {
        DaemonError::Unauthorized(format!(
            "{context}: {source}. {WINDOWS_ELEVATION_MISMATCH_HINT}"
        ))
    }

    struct LocalAllocGuard(*mut c_void);

    impl Drop for LocalAllocGuard {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe {
                    LocalFree(self.0);
                }
            }
        }
    }

    struct OwnedSid(PSID);

    impl Drop for OwnedSid {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe {
                    LocalFree(self.0);
                }
            }
        }
    }

    fn sid_from_string(sid: &str) -> Result<OwnedSid> {
        let wide = wide_null(OsStr::new(sid));
        let mut raw: PSID = std::ptr::null_mut();
        let ok = unsafe { ConvertStringSidToSidW(wide.as_ptr(), &mut raw) };
        if ok == 0 || raw.is_null() {
            return Err(io_err(
                "converting SID string",
                std::io::Error::last_os_error(),
            ));
        }
        Ok(OwnedSid(raw))
    }

    fn sid_string_from_token(token: HANDLE) -> Result<String> {
        let mut needed = 0u32;
        unsafe {
            GetTokenInformation(token, TokenUser, std::ptr::null_mut(), 0, &mut needed);
        }
        if needed == 0 {
            return Err(io_err(
                "sizing token user information",
                std::io::Error::last_os_error(),
            ));
        }
        let mut buf = vec![0u8; needed as usize];
        let ok = unsafe {
            GetTokenInformation(
                token,
                TokenUser,
                buf.as_mut_ptr() as *mut c_void,
                needed,
                &mut needed,
            )
        };
        if ok == 0 {
            return Err(io_err(
                "reading token user information",
                std::io::Error::last_os_error(),
            ));
        }
        let token_user = unsafe { &*(buf.as_ptr() as *const TOKEN_USER) };
        let mut sid_ptr: *mut u16 = std::ptr::null_mut();
        let ok = unsafe { ConvertSidToStringSidW(token_user.User.Sid, &mut sid_ptr) };
        if ok == 0 {
            return Err(io_err(
                "converting SID to string",
                std::io::Error::last_os_error(),
            ));
        }
        let sid = unsafe { wide_ptr_to_string(sid_ptr) };
        unsafe {
            LocalFree(sid_ptr as *mut c_void);
        }
        Ok(sid)
    }

    struct OwnerOnlySecurityAttributes {
        attrs: SECURITY_ATTRIBUTES,
        descriptor: *mut c_void,
    }

    impl Drop for OwnerOnlySecurityAttributes {
        fn drop(&mut self) {
            if !self.descriptor.is_null() {
                unsafe {
                    LocalFree(self.descriptor);
                }
            }
        }
    }

    fn owner_only_security_attributes() -> Result<OwnerOnlySecurityAttributes> {
        let sid = current_user_identity()?;
        let sddl = format!("O:{sid}G:{sid}D:P(A;;GA;;;{sid})");
        let wide = wide_null(OsStr::new(&sddl));
        let mut descriptor: *mut c_void = std::ptr::null_mut();
        let ok = unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                wide.as_ptr(),
                SDDL_REVISION_1,
                &mut descriptor,
                std::ptr::null_mut(),
            )
        };
        if ok == 0 {
            return Err(DaemonError::Unsupported {
                capability: "owner-only Windows security descriptor",
                message: std::io::Error::last_os_error().to_string(),
            });
        }
        Ok(OwnerOnlySecurityAttributes {
            attrs: SECURITY_ATTRIBUTES {
                nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
                lpSecurityDescriptor: descriptor,
                bInheritHandle: 0,
            },
            descriptor,
        })
    }

    struct Handle(HANDLE);

    impl Drop for Handle {
        fn drop(&mut self) {
            if self.0 != 0 && self.0 != INVALID_HANDLE_VALUE {
                unsafe {
                    CloseHandle(self.0);
                }
            }
        }
    }

    fn wide_null(s: &OsStr) -> Vec<u16> {
        s.encode_wide().chain(std::iter::once(0)).collect()
    }

    unsafe fn wide_ptr_to_string(ptr: *const u16) -> String {
        let mut len = 0usize;
        while *ptr.add(len) != 0 {
            len += 1;
        }
        String::from_utf16_lossy(std::slice::from_raw_parts(ptr, len))
    }

    #[allow(dead_code)]
    fn _open_existing_for_probe(path: &Path) -> Result<Handle> {
        let wide = wide_null(path.as_os_str());
        let handle = unsafe {
            CreateFileW(
                wide.as_ptr(),
                FILE_GENERIC_WRITE,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                std::ptr::null_mut(),
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL,
                0,
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            return Err(io_err(
                "opening existing daemon artifact",
                std::io::Error::last_os_error(),
            ));
        }
        Ok(Handle(handle))
    }
}

#[cfg(not(any(unix, windows)))]
mod platform {
    use super::*;

    pub struct ClientConn;
    pub struct ServerConn;

    pub struct Listener;

    impl Listener {
        pub fn bind(_endpoint: &Endpoint) -> Result<Self> {
            Err(DaemonError::Unsupported {
                capability: "daemon IPC endpoint binding",
                message: "only Windows named pipes and Unix sockets are supported in P2".into(),
            })
        }

        pub async fn accept(&mut self) -> Result<ServerConn> {
            Err(DaemonError::Unsupported {
                capability: "daemon IPC endpoint accept",
                message: "only Windows named pipes and Unix sockets are supported in P2".into(),
            })
        }

        pub fn ready_for_next(&mut self) -> Result<()> {
            Ok(())
        }
    }

    pub async fn connect(_endpoint: &Endpoint) -> Result<ClientConn> {
        Err(DaemonError::Unsupported {
            capability: "daemon IPC endpoint connect",
            message: "only Windows named pipes and Unix sockets are supported in P2".into(),
        })
    }

    pub fn current_user_identity() -> Result<String> {
        Err(DaemonError::Unsupported {
            capability: "daemon singleton user identity",
            message: "no OS user identity implementation for this platform".into(),
        })
    }

    pub fn ensure_owner_private_dir(_path: &Path) -> Result<PathBuf> {
        Err(DaemonError::Unsupported {
            capability: "owner-private daemon directory",
            message: "no owner-only permission implementation for this platform".into(),
        })
    }

    pub fn write_owner_only_file(_path: &Path, _bytes: &[u8]) -> Result<()> {
        Err(DaemonError::Unsupported {
            capability: "owner-only daemon capability file",
            message: "no owner-only permission implementation for this platform".into(),
        })
    }

    pub fn verify_client_peer(_conn: &ServerConn) -> Result<()> {
        Err(DaemonError::Unsupported {
            capability: "server-side client peer verification",
            message: "no peer credential primitive for this platform".into(),
        })
    }

    pub fn verify_server_peer(
        _conn: &ClientConn,
        _expected_exe: &Path,
        _expected_pid: Option<u32>,
        _expected_start_time: Option<u64>,
    ) -> Result<()> {
        Err(DaemonError::Unsupported {
            capability: "client-side server-auth",
            message: "no peer credential primitive for this platform".into(),
        })
    }
}

fn same_canonical_path(a: &Path, b: &Path) -> bool {
    #[cfg(windows)]
    {
        normalize_windows_path(a).eq_ignore_ascii_case(&normalize_windows_path(b))
    }
    #[cfg(not(windows))]
    {
        a == b
    }
}

#[cfg(windows)]
fn normalize_windows_path(path: &Path) -> String {
    path.to_string_lossy()
        .trim_start_matches(r"\\?\")
        .replace('/', r"\")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon_ipc::{ERROR_UNAUTHORIZED, REDACTED_SECRET};

    fn repo_test_dir(label: &str) -> PathBuf {
        let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        path.push("target");
        path.push("daemon-p2-tests");
        path.push(format!(
            "{}-{}-{}",
            label,
            std::process::id(),
            monotonic_nonce()
        ));
        std::fs::create_dir_all(&path).expect("create repo-local test dir");
        path
    }

    #[test]
    fn singleton_hash_changes_with_protocol_major_and_config_root() {
        let root = PathBuf::from(r"C:\telex\root-a");
        let a = SingletonKey::from_parts("user-a", &root, proto::PROTOCOL_MAJOR);
        let b = SingletonKey::from_parts("user-a", &root, proto::PROTOCOL_MAJOR + 1);
        let c = SingletonKey::from_parts(
            "user-a",
            PathBuf::from(r"C:\telex\root-b"),
            proto::PROTOCOL_MAJOR,
        );
        assert_ne!(a.short_hash(), b.short_hash());
        assert_ne!(a.short_hash(), c.short_hash());
    }

    #[test]
    fn cap_paths_differ_for_protocol_major_parallel_daemons() {
        let run_dir = repo_test_dir("cap-paths");
        let n = DaemonPaths::for_key(
            SingletonKey::from_parts("user-a", PathBuf::from(r"C:\telex\root"), 1),
            &run_dir,
        );
        let n_plus_1 = DaemonPaths::for_key(
            SingletonKey::from_parts("user-a", PathBuf::from(r"C:\telex\root"), 2),
            &run_dir,
        );
        assert_ne!(n.singleton_hash, n_plus_1.singleton_hash);
        assert_ne!(n.cap_path, n_plus_1.cap_path);
        assert!(n.cap_path.to_string_lossy().contains("daemon-"));
    }

    #[test]
    fn admin_cap_proof_accepts_current_and_rejects_wrong_without_leak() {
        let expected = "cap-secret-value";
        verify_admin_proof(expected, Some(expected)).unwrap();

        let wrong = "wrong-secret-value";
        let response = verify_admin_proof(expected, Some(wrong)).unwrap_err();
        match response {
            Response::Error { code, message, .. } => {
                assert_eq!(code, ERROR_UNAUTHORIZED);
                assert!(!message.contains(expected));
                assert!(!message.contains(wrong));
                assert!(message.contains(REDACTED_SECRET));
            }
            other => panic!("expected error response, got {other:?}"),
        }
    }

    #[test]
    fn peer_identity_rejects_pid_or_start_time_mismatch() {
        verify_expected_peer_identity(10, Some(100), Some(10), Some(100)).unwrap();

        let pid_err = verify_expected_peer_identity(10, Some(100), Some(11), Some(100))
            .expect_err("pid mismatch should reject");
        assert!(matches!(pid_err, DaemonError::Unauthorized(_)));

        let start_err = verify_expected_peer_identity(10, Some(101), Some(10), Some(100))
            .expect_err("start-time mismatch should reject");
        assert!(matches!(start_err, DaemonError::Unauthorized(_)));

        let missing_start = verify_expected_peer_identity(10, None, Some(10), Some(100))
            .expect_err("missing start-time should fail closed when expected");
        assert!(matches!(missing_start, DaemonError::Unauthorized(_)));
    }

    #[test]
    fn cap_identity_requires_pid_and_start_time() {
        let missing_pid = CapFile {
            instance_id: "inst".to_string(),
            admin_cap: "cap".to_string(),
            singleton_hash: "hash".to_string(),
            protocol_major: proto::PROTOCOL_MAJOR,
            server_pid: None,
            server_start_time: Some(1),
        };
        assert!(matches!(
            cap_required_peer_identity(&missing_pid),
            Err(DaemonError::Unauthorized(_))
        ));

        let missing_start = CapFile {
            server_pid: Some(1),
            server_start_time: None,
            ..missing_pid
        };
        assert!(matches!(
            cap_required_peer_identity(&missing_start),
            Err(DaemonError::Unauthorized(_))
        ));
    }

    #[test]
    fn handshake_eof_message_names_handshake_and_windows_elevation() {
        let message = daemon_handshake_eof_message();
        assert!(message.contains("closed the connection during handshake"));
        #[cfg(windows)]
        {
            assert!(message.contains("different elevations"));
            assert!(message.contains("Administrator"));
        }
    }

    #[cfg(windows)]
    #[test]
    fn windows_owner_private_sddl_rejects_broad_aces() {
        let sid = platform::current_user_identity().expect("current SID");
        let private = format!("O:{sid}G:{sid}D:(A;;GA;;;SY)(A;;GA;;;BA)(A;;GA;;;{sid})");
        assert!(platform::owner_private_sddl_is_strict(&private, &sid));

        let private_full_well_known = format!(
            "O:S-1-5-32-544G:{sid}D:P(A;;GA;;;S-1-5-18)(A;;GA;;;S-1-5-32-544)(A;;GA;;;{sid})"
        );
        assert!(platform::owner_private_sddl_is_strict(
            &private_full_well_known,
            &sid
        ));

        let private_logon_and_packages = format!(
            "O:{sid}G:{sid}D:PAI(A;;GA;;;S-1-5-5-123-456)(A;;GR;;;S-1-15-2-2)(A;;GR;;;S-1-15-3-1024-1)"
        );
        assert!(platform::owner_private_sddl_is_strict(
            &private_logon_and_packages,
            &sid
        ));

        let broad = format!("O:{sid}G:{sid}D:(A;;GA;;;{sid})(A;;GR;;;WD)");
        assert!(!platform::owner_private_sddl_is_strict(&broad, &sid));

        let authenticated_users = format!("O:{sid}G:{sid}D:(A;;GA;;;S-1-5-11)");
        assert!(!platform::owner_private_sddl_is_strict(
            &authenticated_users,
            &sid
        ));
    }

    #[tokio::test]
    async fn endpoint_bind_exclusivity_rejects_second_listener() {
        let run_dir = repo_test_dir("bind-exclusive");
        let paths = DaemonPaths::for_key(
            SingletonKey::from_parts(
                format!("user-{}", std::process::id()),
                &run_dir,
                proto::PROTOCOL_MAJOR,
            ),
            &run_dir,
        );
        let _first = platform::Listener::bind(&paths.endpoint).expect("first listener binds");
        let second = platform::Listener::bind(&paths.endpoint);
        assert!(matches!(
            second,
            Err(DaemonError::AlreadyRunning(_)) | Err(DaemonError::Io { .. })
        ));
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn windows_pipe_listener_rearms_while_client_is_connected() {
        let run_dir = repo_test_dir("pipe-rearm");
        let paths = DaemonPaths::for_key(
            SingletonKey::from_parts(
                format!("user-{}", std::process::id()),
                &run_dir,
                proto::PROTOCOL_MAJOR,
            ),
            &run_dir,
        );
        let mut listener = platform::Listener::bind(&paths.endpoint).expect("bind listener");
        let endpoint = paths.endpoint.clone();
        let first_client = tokio::spawn(async move { platform::connect(&endpoint).await });
        let first_server = listener.accept().await.expect("accept first client");
        let first_client = first_client
            .await
            .expect("first connect")
            .expect("first client");

        listener
            .ready_for_next()
            .expect("rearm while first client is still connected");
        let endpoint = paths.endpoint.clone();
        let second_client = tokio::spawn(async move { platform::connect(&endpoint).await });
        let second_server = listener.accept().await.expect("accept second client");
        let second_client = second_client
            .await
            .expect("second connect")
            .expect("second client");
        drop((first_server, first_client, second_server, second_client));
    }
}
