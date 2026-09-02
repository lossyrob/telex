//! Hidden daemon singleton foundation: singleton identity, endpoint naming, capability
//! file handling, connect-or-spawn, and a P2 JSONL server loop.
#![allow(clippy::result_large_err, clippy::too_many_arguments)]

#[cfg(feature = "postgres")]
use crate::backend::postgres::{
    make_tls as make_postgres_tls, notify_channel_for_schema, sanitize_ident,
};
#[cfg(feature = "sqlite")]
use crate::backend::sqlite::SqliteBackend;
use crate::backend::{Backend, WaitFetchOptions};
use crate::daemon_ipc::{
    self as proto, current_protocol_version, read_json_line, write_json_line, DaemonStatus,
    DeafStationStatus, DeliveryMode, EpochStatus, HandshakeError, HelloAck, IdleStationStatus,
    LiveWaiterStatus, MemberStatus, MembershipLossStatus, NeedsAttachReason, PushDeliveryHealth,
    RecentErrorStatus, Request, Response, RetentionStatus, SentReceipt, StationCapability,
    StationHealth, StoreStatus, WaiterOutcome, WatchPidRole, WatchPidSpec, WatchPidStatus,
    ON_DELIVER_DEFERRED_EXIT, ON_DELIVER_PERMANENT_EXIT,
};
use crate::model::{
    cc_recipients, delivery_role, now_ms, requires_disposition_for_recipient,
    ApplicationMessageOperation, Attention, DeliveryOutcome, Disposition, EpochClaimResult,
    MessageRow, NewMessage, STATUS_RETIRED,
};
#[cfg(test)]
use crate::model::{ApplicationOperationBegin, NewApplicationOperation};
use crate::station_intent;
#[cfg(feature = "postgres")]
use anyhow::Context;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::fmt;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
use std::sync::{Arc, Weak};
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

/// Daemon-owned station-intent reconciliation (issue #106 / ADR 0052).
///
/// Physically `src/daemon_reconcile.rs`. It is mounted as a child of `daemon` rather than as a
/// sibling crate module because it manipulates member records, admission guards, and epoch leases —
/// state that must stay private to the daemon. The crate root re-exports it as
/// `crate::daemon_reconcile`.
#[path = "daemon_reconcile.rs"]
pub mod reconcile;

#[cfg(windows)]
const WINDOWS_ELEVATION_MISMATCH_HINT: &str = "On Windows, this usually means the telex daemon and this process are running at different elevations (Administrator vs non-Administrator), so they cannot authenticate over the daemon named pipe. Stop the existing daemon from a matching-elevation terminal, or restart/attach from the same elevation as this session (for an elevated session, start telex from an Administrator terminal).";

fn daemon_handshake_eof_message() -> String {
    let message = "daemon closed the connection during handshake".to_string();
    #[cfg(windows)]
    let message = format!("{message}; {WINDOWS_ELEVATION_MISMATCH_HINT}");
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

/// Shared owner-private filesystem errors map onto the two `DaemonError` variants they were
/// raised as before the primitives moved into `crate::platform_fs`, so daemon-facing error text
/// is unchanged.
impl From<crate::platform_fs::FsError> for DaemonError {
    fn from(value: crate::platform_fs::FsError) -> Self {
        match value {
            crate::platform_fs::FsError::Io { action, source } => {
                DaemonError::Io { action, source }
            }
            crate::platform_fs::FsError::Unsupported {
                capability,
                message,
            } => DaemonError::Unsupported {
                capability,
                message,
            },
        }
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
    delivery_admissions: Mutex<HashMap<MemberKey, Weak<AsyncMutex<()>>>>,
    #[cfg(test)]
    delivery_admission_control: Mutex<Option<Arc<DeliveryAdmissionTestControl>>>,
    next_waiter_id: AtomicU64,
    recent_errors: Arc<Mutex<VecDeque<RecentErrorStatus>>>,
    ended_sessions: Mutex<BTreeMap<SessionKey, EndedSessionRecord>>,
    draining: AtomicBool,
    on_deliver: OnDeliverState,
    /// Station-intent reconciliation state: the cached index, the per-scope single-flight guard,
    /// and the trigger/report seam (issue #106 / ADR 0052).
    intents: reconcile::IntentRuntime,
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

#[derive(Clone, Copy)]
enum DeliveryAdmissionKind {
    Register,
    Wait,
}

#[cfg(test)]
struct DeliveryAdmissionTestLane {
    before_arrived: Semaphore,
    before_release: Semaphore,
    commit_arrived: Semaphore,
    commit_release: Semaphore,
}

#[cfg(test)]
impl DeliveryAdmissionTestLane {
    fn new() -> Self {
        Self {
            before_arrived: Semaphore::new(0),
            before_release: Semaphore::new(0),
            commit_arrived: Semaphore::new(0),
            commit_release: Semaphore::new(0),
        }
    }
}

#[cfg(test)]
struct DeliveryAdmissionTestControl {
    register: DeliveryAdmissionTestLane,
    wait: DeliveryAdmissionTestLane,
}

#[cfg(test)]
impl DeliveryAdmissionTestControl {
    fn new() -> Self {
        Self {
            register: DeliveryAdmissionTestLane::new(),
            wait: DeliveryAdmissionTestLane::new(),
        }
    }

    fn lane(&self, kind: DeliveryAdmissionKind) -> &DeliveryAdmissionTestLane {
        match kind {
            DeliveryAdmissionKind::Register => &self.register,
            DeliveryAdmissionKind::Wait => &self.wait,
        }
    }

    async fn before_lock(&self, kind: DeliveryAdmissionKind) {
        let lane = self.lane(kind);
        lane.before_arrived.add_permits(1);
        lane.before_release
            .acquire()
            .await
            .expect("admission before-lock release")
            .forget();
    }

    async fn before_commit(&self, kind: DeliveryAdmissionKind) {
        let lane = self.lane(kind);
        lane.commit_arrived.add_permits(1);
        lane.commit_release
            .acquire()
            .await
            .expect("admission commit release")
            .forget();
    }

    async fn wait_before_lock(&self, kind: DeliveryAdmissionKind) {
        self.lane(kind)
            .before_arrived
            .acquire()
            .await
            .expect("admission reached before-lock gate")
            .forget();
    }

    async fn wait_before_commit(&self, kind: DeliveryAdmissionKind) {
        self.lane(kind)
            .commit_arrived
            .acquire()
            .await
            .expect("admission reached commit gate")
            .forget();
    }

    fn release_before_lock(&self, kind: DeliveryAdmissionKind) {
        self.lane(kind).before_release.add_permits(1);
    }

    fn release_commit(&self, kind: DeliveryAdmissionKind) {
        self.lane(kind).commit_release.add_permits(1);
    }
}

#[derive(Clone, Debug)]
struct MemberRecord {
    address: String,
    capability: StationCapability,
    store_key: String,
    backend: String,
    session_id: String,
    application_responsibility: Option<String>,
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
            capabilities: proto::daemon_capabilities(),
            protocol_version: current_protocol_version(),
            daemon_version: proto::DAEMON_VERSION.to_string(),
            instance_id: self.instance_id.clone(),
            singleton_key: self.paths.singleton.redacted_material(),
            stores: Vec::new(),
            backoff: vec!["n/a: crashloop backoff is not persisted by the daemon".to_string()],
            recent_errors: Vec::new(),
            epoch_by_address: Vec::new(),
            members: Vec::new(),
            membership_losses: Vec::new(),
            live_waiters: Vec::new(),
            retention: Vec::new(),
            idle_stations: IdleStationStatus::default(),
            deaf_stations: DeafStationStatus::default(),
            // Intent rows are part of the authenticated projection only; the uncapped minimal
            // projection must not leak session ids or addresses.
            intents: Vec::new(),
            intent_index_as_of_ms: None,
            intent_over_cap: false,
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
        // Intent rows come from the cached index only, so building status never touches the intent
        // directory and never probes a producer.
        let intent_index = self.intent_index_snapshot();
        let intents = self.intent_statuses(None);
        DaemonStatus {
            capabilities: proto::daemon_capabilities(),
            protocol_version: current_protocol_version(),
            daemon_version: proto::DAEMON_VERSION.to_string(),
            instance_id: self.instance_id.clone(),
            singleton_key: self.paths.singleton.redacted_material(),
            stores,
            backoff: vec!["n/a: crashloop backoff is not persisted by the daemon".to_string()],
            recent_errors: self.recent_errors(),
            epoch_by_address,
            members,
            membership_losses: self.membership_losses(),
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
            intents,
            intent_index_as_of_ms: Some(intent_index.as_of_ms),
            intent_over_cap: intent_index.over_cap,
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

    async fn delivery_admission(
        &self,
        store_key: &str,
        session_id: &str,
        address: &str,
        kind: DeliveryAdmissionKind,
    ) -> Arc<AsyncMutex<()>> {
        let key = Self::member_key(store_key, session_id, address);
        let admission = {
            let mut admissions = self.delivery_admissions.lock().unwrap();
            admissions.retain(|_, weak| weak.strong_count() > 0);
            if let Some(existing) = admissions.get(&key).and_then(Weak::upgrade) {
                existing
            } else {
                let admission = Arc::new(AsyncMutex::new(()));
                admissions.insert(key, Arc::downgrade(&admission));
                admission
            }
        };
        let _ = kind;
        #[cfg(test)]
        {
            let control = self.delivery_admission_control.lock().unwrap().clone();
            if let Some(control) = control {
                control.before_lock(kind).await;
            }
        }
        admission
    }

    #[cfg(test)]
    async fn delivery_admission_before_commit(&self, kind: DeliveryAdmissionKind) {
        let control = self.delivery_admission_control.lock().unwrap().clone();
        if let Some(control) = control {
            control.before_commit(kind).await;
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

    /// Active members attending one address in one store, whatever session holds them.
    ///
    /// The member half of an operator reset's binding set; the other half is the durable scope,
    /// because a station with no member at all still has desired state to withdraw.
    fn address_members(&self, store_key: &str, address: &str) -> Vec<MemberRecord> {
        self.members
            .lock()
            .unwrap()
            .values()
            .filter(|m| m.store_key == store_key && m.address == address && !m.idle)
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
        self.members.lock().unwrap().values().any(|m| {
            m.store_key == store_key
                && m.address == address
                && !m.idle
                && m.capability == StationCapability::Bidirectional
        })
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

    fn membership_losses(&self) -> Vec<MembershipLossStatus> {
        self.ended_sessions
            .lock()
            .unwrap()
            .iter()
            .flat_map(|(session, ended)| {
                let reason = match ended.reason.as_str() {
                    "WatchPidDeath" => NeedsAttachReason::PredicateDeath,
                    "SessionEnd" | "Detach" | "ApplicationDetach" => {
                        NeedsAttachReason::DeliberatelyDetached
                    }
                    raw => NeedsAttachReason::Unknown(raw.to_string()),
                };
                ended
                    .addresses
                    .iter()
                    .map(move |address| MembershipLossStatus {
                        store_key: session.store_key.clone(),
                        session_id: session.session_id.clone(),
                        address: address.clone(),
                        reason: reason.clone(),
                        detail: ended.reason.clone(),
                        at_ms: ended.at_ms,
                    })
            })
            .collect()
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
        let delivery_mode = match (self.on_deliver.is_some(), live_waiters_count > 0) {
            (true, true) => DeliveryMode::Conflict,
            (true, false) => DeliveryMode::Push,
            (false, _) => DeliveryMode::Pull,
        };
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
            capability: self.capability,
            occupant: self.occupant.clone(),
            host: self.host.clone(),
            waiters: self.waiters,
            live_waiters_count,
            pending_unconsumed_count,
            inbound_actionable_count,
            station_health,
            delivery_mode,
            push_delivery,
            push_suppressed_count,
            health_detail,
            last_waiter_exit_at_ms: self.last_waiter_exit_at_ms,
            last_waiter_outcome: self.last_waiter_outcome,
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
        if self.on_deliver.is_some() && live_waiters_count > 0 {
            return (
                StationHealth::CoverageConflict,
                Some("push handler and pull waiter are active at the same time".to_string()),
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
        // unreachable). A delivering/deferred/probing/stale-accepted bridge is attended-via-push, never
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
                PushDeliveryHealth::Deferred => (
                    StationHealth::AttendedPush,
                    Some(format!(
                        "attended via push bridge (no waiter; expected in push mode); bridge reachable and delivery deferred until idle; {inbound_actionable_count} awaiting disposition"
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
            &startup,
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
    // Startup scan (trigger (a) of ADR 0052). Spawned, never awaited: the daemon accepts
    // connections immediately, so a large or corrupt intent scope cannot delay readiness.
    reconcile::spawn_startup_scan(state.clone());

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
        server_start_time,
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
        delivery_admissions: Mutex::new(HashMap::new()),
        #[cfg(test)]
        delivery_admission_control: Mutex::new(None),
        next_waiter_id: AtomicU64::new(1),
        recent_errors: Arc::new(Mutex::new(VecDeque::new())),
        ended_sessions: Mutex::new(BTreeMap::new()),
        draining: AtomicBool::new(false),
        on_deliver: OnDeliverState::default(),
        intents: reconcile::IntentRuntime::default(),
    })
}

fn current_process_start_time_for_cap() -> Result<Option<u64>> {
    let start_time = crate::session_watch::capture_process_start_time(std::process::id());
    if cfg!(any(target_os = "linux", target_os = "macos", windows)) && start_time.is_none() {
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
#[allow(clippy::large_enum_variant)]
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
        // deaf detection by up to a backstop. Ties on `last` are broken toward a real failure:
        // non-accepted sorts above accepted, then non-deferred sorts above a healthy busy deferral.
        // Equal-timestamp completions therefore cannot flip health nondeterministically.
        let freshest = relevant.max_by_key(|a| (a.last, !a.accepted, !a.deferred));
        match freshest {
            Some(attempt) if attempt.deferred => PushDeliveryHealth::Deferred,
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
                        if earliest_blocking.is_none_or(|blocking| ts < blocking) {
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
    // One loop, two responsibilities. Reconciliation rides the existing heartbeat tick rather
    // than adding a second timer, and it also wakes on an explicit trigger pulse (startup,
    // upgrade/rollback, `ReconcileIntents`, tests) so callers can drive a pass without waiting
    // out the interval. A pulse only *schedules* work: per-intent backoff, quarantine, and
    // deferred cadences are still honored inside the pass.
    //
    // The heartbeat deadline is tracked explicitly rather than by recreating a sleep each
    // iteration. Recreating it meant any trigger pulse restarted the elapsed heartbeat time, so a
    // caller pulsing faster than `HEARTBEAT_INTERVAL` starved `heartbeat_members_once` — the one
    // thing that renews every member's epoch lease — and let leases go stale inside
    // `liveness_window_secs()`, inviting a successor to claim them.
    let mut next_heartbeat = Instant::now() + HEARTBEAT_INTERVAL;
    loop {
        let tick = tokio::time::sleep_until(next_heartbeat.into());
        tokio::pin!(tick);
        tokio::select! {
            _ = &mut tick => {}
            _ = state.intents.trigger.notified() => {}
        }
        if Instant::now() >= next_heartbeat {
            heartbeat_members_once(state.clone()).await;
            // Anchored to the deadline, not to "now", so a slow heartbeat or a long pass cannot
            // make the effective interval drift outward tick after tick.
            let now = Instant::now();
            while next_heartbeat <= now {
                next_heartbeat += HEARTBEAT_INTERVAL;
            }
        }
        // The pass is bounded by `RECONCILE_PASS_DEADLINE < HEARTBEAT_INTERVAL`, so it cannot
        // overrun the tick that started it.
        reconcile::reconcile_once(state.clone(), None).await;
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
    // A `pending` station intent means a push attach for this binding is mid-flight, or crashed
    // before it finalized. The daemon never acts on a pending intent, so a generic
    // re-register-and-retry would race the attach and could create a pull-only member over a push
    // provisioning in progress. Report the specific reason instead, so the client stops and points
    // the user at the finalizing step rather than silently retrying.
    if state.pending_push_intent(store_key, session_id, address) {
        return proto::needs_attach_with_reason(
            format!(
                "session {session_id} has a push attach for {address} in {store_key} that has not finalized yet"
            ),
            NeedsAttachReason::PushIntentPending,
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

/// Bounded wait for in-flight `on_deliver` handlers before a graceful drain releases leases.
///
/// Epoch advancement fences the *daemon*, not helper processes a dying one already spawned. On the
/// graceful path we can close that overlap completely by simply waiting for the helpers to finish,
/// so a successor never races a predecessor's handler. It is bounded because a wedged helper must
/// not be able to hold a drain open indefinitely; the `--daemon-instance` fence covers whatever
/// escapes the bound (and the crash path, which has no drain at all).
const DRAIN_INFLIGHT_WAIT: Duration = Duration::from_secs(5);
const DRAIN_INFLIGHT_POLL: Duration = Duration::from_millis(50);

async fn drain_members(state: Arc<DaemonState>) -> std::result::Result<(), Response> {
    state.begin_draining();
    wait_for_inflight_handlers(&state, DRAIN_INFLIGHT_WAIT).await;
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

/// Wait, bounded, for every in-flight push helper to finish. `begin_draining` has already stopped
/// new pushes from being spawned, so this converges; the bound only guards against a helper that
/// never exits.
async fn wait_for_inflight_handlers(state: &Arc<DaemonState>, budget: Duration) {
    let deadline = Instant::now() + budget;
    loop {
        let inflight = state.on_deliver.inflight.lock().unwrap().len();
        if inflight == 0 {
            return;
        }
        if Instant::now() >= deadline {
            state.push_recent_error(
                "DrainInflight",
                format!(
                    "graceful drain proceeded with {inflight} in-flight push handler(s) after {}ms; \
                     the --daemon-instance fence stops them from injecting into a successor's session",
                    budget.as_millis()
                ),
            );
            return;
        }
        tokio::time::sleep(DRAIN_INFLIGHT_POLL).await;
    }
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

    let supports_delivery_quarantine = peer_supports_delivery_quarantine(&hello);
    let (response, action) =
        handle_request_with_capabilities(state, request, supports_delivery_quarantine).await;
    write_json_line(&mut write_half, &response).await?;
    Ok(action)
}

fn peer_supports_delivery_quarantine(hello: &proto::Hello) -> bool {
    hello
        .capabilities
        .iter()
        .any(|capability| capability == proto::CAP_DELIVERY_QUARANTINE_V1)
}

#[allow(dead_code)]
async fn handle_request(state: Arc<DaemonState>, request: Request) -> (Response, ClientAction) {
    handle_request_with_capabilities(state, request, true).await
}

async fn handle_request_with_capabilities(
    state: Arc<DaemonState>,
    request: Request,
    supports_delivery_quarantine: bool,
) -> (Response, ClientAction) {
    let response = match request {
        Request::Ping => Response::Pong {
            protocol_version: current_protocol_version(),
            daemon_version: proto::DAEMON_VERSION.to_string(),
            instance_id: state.instance_id.clone(),
            capabilities: proto::daemon_capabilities(),
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
            // Computed from in-memory members plus the cached intent index, *before* the
            // lease-release loop, so it describes what a successor will find and cannot push the
            // graceful drain past `--drain-timeout-ms`.
            let drain_intents = Some(state.drain_intent_report());
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
                    drain_intents,
                },
                ClientAction::Drain,
            );
        }
        Request::ReconcileIntents { proof, scope } => {
            // Admin-proofed exactly like `Drain`: reconciliation arms delivery and spawns handler
            // processes, so it must not be reachable from an unproofed request path.
            if let Err(response) = state.check_admin_cap(proof.as_deref()) {
                return (response, ClientAction::Continue);
            }
            // **One** deadline, originated here, shared with the pass.
            //
            // The previous shape was a spawned pass on its own four-second clock plus a
            // four-second `timeout` on the handler. Equal lengths, different origins: the handler's
            // clock started when the request arrived and the pass's when the task was scheduled, so
            // the handler could answer `admin_deadline` while the pass was still mid-wave — and the
            // pass then went on registering members, advancing cursors, and publishing a report
            // that belonged to a request already answered. Handing the pass this instant instead
            // makes "the pass is bounded" and "the request is bounded" the same statement, and
            // `RECONCILE_RESPONSE_RESERVE` is the slack between the pass's last phase and this
            // handler's answer.
            //
            // Spawned, then **joined** — not detached. Spawning is what keeps a client that hangs
            // up mid-request from tearing a pass in half at an arbitrary await point; joining is
            // what guarantees every member registration, cursor advance, and report the pass
            // performs has already happened when this response is written.
            let deadline = Instant::now() + reconcile::RECONCILE_PASS_WORK_BUDGET;
            let pass = tokio::spawn(reconcile::reconcile_once_until(
                state.clone(),
                scope,
                deadline,
                reconcile::PassOrigin::Request,
            ));
            match pass.await {
                Ok(report) => Response::Reconciled { report },
                // A panicked pass has no report to give. `ran: false` is the existing protocol for
                // "no result to report"; the CLI and the turn-boundary hook already retry on it
                // rather than reading a zeroed report as a successful pass that restored nothing.
                Err(_) => Response::Reconciled {
                    report: reconcile::abandoned_pass_report(&state, "pass_aborted"),
                },
            }
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
            replace_watch_pids,
            recovery,
            on_deliver,
            replace_on_deliver,
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
                replace_watch_pids,
                recovery,
                on_deliver,
                replace_on_deliver,
                on_deliver_wake_on_cc,
                None,
                None,
            )
            .await
        }
        Request::ApplicationRegister {
            store_key,
            address,
            session_id,
            application_responsibility,
            occupant,
            capability,
            description,
            scope,
            tags,
            watch_pids,
            recovery,
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
                false,
                recovery,
                None,
                false,
                false,
                Some(application_responsibility),
                Some(capability),
            )
            .await
        }
        Request::ApplicationDetach {
            store_key,
            session_id,
            application_responsibility,
            address,
            capability,
        } => {
            detach_application_member(
                state.clone(),
                store_key,
                session_id,
                application_responsibility,
                address,
                capability,
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
                supports_delivery_quarantine,
            )
            .await
        }
        Request::Ack {
            store_key,
            session_id,
            address,
            message_id,
        } => ack_message(state.clone(), store_key, session_id, address, message_id).await,
        Request::ApplicationAck {
            store_key,
            session_id,
            address,
            message_id,
            delivery_id,
        } => {
            ack_exact_delivery(
                state.clone(),
                store_key,
                session_id,
                address,
                message_id,
                delivery_id,
            )
            .await
        }
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
                None,
            )
            .await
        }
        Request::ApplicationSend {
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
            logical_store_id,
            application_responsibility,
            operation_id,
            payload_fingerprint,
        } => {
            send_message(
                state.clone(),
                store_key,
                session_id,
                Some(from_addr),
                to_addr,
                cc,
                kind,
                attention,
                requires_disposition,
                subject,
                body,
                metadata,
                Some((
                    logical_store_id,
                    application_responsibility,
                    operation_id,
                    payload_fingerprint,
                )),
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
                None,
                None,
            )
            .await
        }
        Request::ApplicationReply {
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
            metadata,
            logical_store_id,
            application_responsibility,
            operation_id,
            payload_fingerprint,
        } => {
            reply_message(
                state.clone(),
                store_key,
                session_id,
                Some(from_addr),
                message_id,
                kind,
                attention,
                requires_disposition,
                subject,
                cc,
                body,
                metadata,
                Some((
                    logical_store_id,
                    application_responsibility,
                    operation_id,
                    payload_fingerprint,
                )),
            )
            .await
        }
    };
    (response, ClientAction::Continue)
}

/// `Register`.
///
/// An **arming** register (`on_deliver.is_some()`) is a *durable* push registration whenever the
/// binding has a station-intent record: the durable armed proof is committed inside this function,
/// immediately before the member is installed, and a register that cannot persist the proof it owes
/// is aborted rather than reported as a success — see [`commit_armed_proof`].
#[allow(clippy::too_many_arguments)]
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
    replace_watch_pids: bool,
    recovery: bool,
    on_deliver: Option<Vec<String>>,
    replace_on_deliver: bool,
    on_deliver_wake_on_cc: bool,
    application_responsibility: Option<String>,
    capability: Option<StationCapability>,
) -> Response {
    if state.is_draining() {
        return proto::error_response(proto::ERROR_NOT_RUNNING, "daemon is draining");
    }
    // Admission is per station and outermost: never acquire it while holding another daemon
    // lock. Register may await backend work while holding it, but no long-lived waiter does.
    let delivery_admission = state
        .delivery_admission(
            &store_key,
            &session_id,
            &address,
            DeliveryAdmissionKind::Register,
        )
        .await;
    let _delivery_admission_guard = delivery_admission.lock().await;
    let watch_pids = capture_watch_pids(watch_pids);

    // Does this register *owe* a durable armed proof?
    //
    // Read once, here, under the per-station admission guard and before any commit path runs. The
    // question it answers cannot be answered at stamp time: "no record for this binding" is both
    // the ordinary pull or plain `--on-deliver` attach (nothing was ever written, so nothing is
    // owed) and a concurrent attach rollback that deleted the record this register was about to
    // stamp (everything is owed). Observing the record's existence up front separates the two, and
    // the two possible interleavings after this point are both safe: a record created between here
    // and the stamp is stamped anyway, and a record deleted between here and the stamp aborts the
    // register instead of returning an unbacked durable success.
    //
    // An unreadable scope fails closed for exactly the reason the anti-downgrade guard below does:
    // it is the `Insecure` condition the rest of this design refuses to guess about, and guessing
    // "no record" here is guessing in the direction that silently loses recovery.
    let owes_armed_proof = if on_deliver.is_some() {
        match state.durable_intent_present(&store_key, &session_id, &address) {
            Ok(present) => present,
            Err(detail) => {
                state.push_recent_error(
                    "StationIntent",
                    format!(
                        "refused push registration because the station-intent scope could not be read store={store_key} session={session_id} address={address}: {detail}"
                    ),
                );
                return proto::incompatible_with_reason(
                    format!(
                        "the station-intent scope could not be read ({detail}), so push for {address} cannot be registered durably; \
                         fix the scope permissions and re-run the attach"
                    ),
                    NeedsAttachReason::PushIntentUnrecoverable,
                );
            }
        }
    } else {
        false
    };

    if on_deliver.is_some() && state.has_live_waiter_for(&store_key, &session_id, &address) {
        state.push_recent_error(
            "DeliveryModeConflict",
            format!(
                "rejected push registration store={store_key} session={session_id} address={address}: a live pull waiter is armed"
            ),
        );
        return proto::error_response(
            proto::ERROR_INCOMPATIBLE,
            format!(
                "address {address} has a live pull waiter; stop the station before registering push"
            ),
        );
    }

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
                if let Some(capability) = capability {
                    if capability != existing.capability {
                        return proto::error_response(
                            proto::ERROR_CAPABILITY_CONFLICT,
                            format!(
                                "address {address} is already attached with {:?} capability; detach before changing capability",
                                existing.capability
                            ),
                        );
                    }
                    refreshed.capability = capability;
                }
                let preserving_on_deliver =
                    !replace_on_deliver && on_deliver.is_none() && existing.on_deliver.is_some();
                let removing_push = replace_on_deliver && on_deliver.is_none();
                let preserving_push_watch =
                    !replace_watch_pids && existing.on_deliver.is_some() && !removing_push;
                refreshed.watch_pids = if preserving_push_watch {
                    existing.watch_pids.clone()
                } else {
                    watch_pids
                };
                refreshed.idle = false;
                refreshed.idle_rearmable = false;
                // Preserve an already-registered push handler and, unless the refresh supplies an
                // explicit watch process, its liveness predicates when `on_deliver = None`.
                // Explicit bridge-lifetime refreshes may replace only the watch anchor without
                // re-provisioning push; generic re-attaches cannot silently disarm the bridge.
                refreshed.on_deliver = if replace_on_deliver {
                    on_deliver.clone()
                } else {
                    on_deliver.clone().or_else(|| existing.on_deliver.clone())
                };
                if on_deliver.is_some() {
                    refreshed.on_deliver_wake_on_cc = on_deliver_wake_on_cc;
                    refreshed.on_deliver_cc_after_ms =
                        match on_deliver_cc_lower_bound(&backend, &address, on_deliver_wake_on_cc)
                            .await
                        {
                            Ok(value) => value,
                            Err(response) => return response,
                        };
                } else if preserving_on_deliver {
                    refreshed.on_deliver_wake_on_cc = existing.on_deliver_wake_on_cc;
                    refreshed.on_deliver_cc_after_ms = existing.on_deliver_cc_after_ms;
                } else {
                    refreshed.on_deliver_wake_on_cc = false;
                    refreshed.on_deliver_cc_after_ms = None;
                }
                // A plain refresh can preserve an existing handler even when this request did not
                // carry `on_deliver`; validate the effective mode at the linearization point.
                if refreshed.on_deliver.is_some()
                    && state.has_live_waiter_for(&store_key, &session_id, &address)
                {
                    state.push_recent_error(
                        "DeliveryModeConflict",
                        format!(
                            "rejected push-preserving refresh store={store_key} session={session_id} address={address}: a live pull waiter is armed"
                        ),
                    );
                    return proto::error_response(
                        proto::ERROR_COLLISION,
                        format!(
                            "address {address} has a live pull waiter; stop the station before preserving push"
                        ),
                    );
                }
                state.check_session_id_reuse_tripwire(&refreshed);
                #[cfg(test)]
                state
                    .delivery_admission_before_commit(DeliveryAdmissionKind::Register)
                    .await;
                // Explicit push→pull downgrade: withdraw the durable intent as part of this same
                // admitted transition. See `withdraw_downgraded_intent`.
                if removing_push {
                    if let Err(response) =
                        withdraw_downgraded_intent(&state, &store_key, &session_id, &address)
                    {
                        return response;
                    }
                }
                // Commit the durable proof *before* the member, and before any other side effect
                // this branch performs. On failure the pre-existing member is left exactly as it
                // was: `existing` is still the installed record, its epoch lease is untouched, and
                // nothing this call built has been published — which is the whole reason the stamp
                // sits here rather than after the commit.
                if on_deliver.is_some() {
                    if let Err(response) = commit_armed_proof(
                        &state,
                        &store_key,
                        &session_id,
                        &address,
                        owes_armed_proof,
                    ) {
                        return response;
                    }
                }
                if !recovery {
                    state.clear_definite_session_end(&store_key, &session_id);
                }
                state.insert_member(refreshed.clone());
                // Reset the push retry state and re-scan backlog only on an explicit
                // (re-)provision; a plain refresh that merely preserved the handler keeps its
                // backoff intact (the per-heartbeat sweep still delivers any backlog).
                if on_deliver.is_some() || replace_on_deliver {
                    state.on_deliver_forget_member(&MemberKey {
                        store_key: refreshed.store_key.clone(),
                        session_id: refreshed.session_id.clone(),
                        address: refreshed.address.clone(),
                    });
                    if refreshed.on_deliver.is_some() {
                        spawn_on_deliver_backlog(state.clone(), refreshed.clone());
                    }
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
            proto::ERROR_COLLISION,
            format!(
                "address {} is already attended by session {} in this daemon",
                conflict.address, conflict.session_id
            ),
        );
    }

    // Anti-downgrade (issue #106 / ADR 0052 decision 10).
    //
    // We are about to create a **new** member for this key. If a live push intent exists for it,
    // creating a pull-only member here would silently downgrade a station the user provisioned for
    // push — the exact failure the issue calls out. The guard lives here, in `register_member`,
    // rather than in a Copilot-specific path, so it also covers older clients and plain
    // `telex attach`.
    //
    // It calls `reconcile_intent_locked`, the **guard-free inner** entry point: this function
    // already holds the per-`MemberKey` admission guard for this key, and that guard is documented
    // as outermost and non-reentrant, so calling the acquiring `reconcile_once` here would
    // self-deadlock the hottest register path.
    if on_deliver.is_none() && !replace_on_deliver {
        let intent_key = reconcile::IntentKey {
            store_key: store_key.clone(),
            session_id: session_id.clone(),
            address: address.clone(),
        };
        // Consult the **durable manifest**, not only the cached index. The index is populated by the
        // first reconcile pass, and `serve()` accepts connections before that pass runs — which is
        // exactly the daemon-replacement window this guard exists to protect.
        //
        // Three-way, deliberately: "the scope could not be opened" is the `Insecure` condition the
        // rest of the design fails closed on, and folding it into "no intent" made the guard fail
        // **open** in that same window — before the index is populated, an unopenable scope plus an
        // empty index meant a pull-only member was created over a live push intent on disk.
        let lookup = state.lookup_live_intent(&intent_key);
        let indexed_live = state.live_push_intent(&intent_key).is_some();
        match lookup {
            reconcile::LiveIntentLookup::Unavailable(detail) => {
                state.push_recent_error(
                    "PushIntentUnrecoverable",
                    format!(
                        "refused pull-only registration because the station-intent scope could not be read store={store_key} session={session_id} address={address}: {detail}"
                    ),
                );
                return proto::incompatible_with_reason(
                    format!(
                        "the station-intent scope could not be read ({detail}), so a live push intent for {address} cannot be ruled out; \
                         fix the scope permissions, or re-provision push with `telex --address {address} copilot resume` before attaching pull-only"
                    ),
                    NeedsAttachReason::PushIntentUnrecoverable,
                );
            }
            reconcile::LiveIntentLookup::Live(intent) => {
                let outcome = reconcile::reconcile_intent_locked(state.clone(), &intent).await;
                reconcile::apply_inline_success_projection(&state, &intent, &outcome);
                match outcome {
                    // Push was restored (or was already live): treat the incoming registration
                    // as a refresh of the now-push member rather than creating a pull-only one.
                    reconcile::IntentOutcome::Restored
                    | reconcile::IntentOutcome::RefreshedNoOp => {
                        if let Some(restored) = state.get_member(&store_key, &session_id, &address)
                        {
                            // Carry the incoming registration's process anchors onto the restored
                            // member: dropping them silently disabled `WatchPidDeath` reaping for
                            // a caller that registered with `--watch-pid`.
                            if !watch_pids.is_empty() {
                                let mut anchored = restored.clone();
                                anchored.watch_pids = watch_pids.clone();
                                state.insert_member(anchored);
                            }
                            return Response::Registered {
                                lease_epoch: restored.lease_epoch,
                                owner_instance_id: restored.owner_instance_id,
                            };
                        }
                    }
                    // A live armed pull waiter wins (decision 13): the anti-downgrade guarantee
                    // is explicitly scoped to the no-live-waiter case, so fall through and let
                    // the normal pull registration proceed.
                    reconcile::IntentOutcome::DeferredPullWaiter => {}
                    // Anything else means we could not prove the push path is recoverable, so
                    // we fail closed with a typed reason instead of creating a pull-only member
                    // over a live push intent.
                    other => {
                        let code = other.failure_code().unwrap_or("unrecoverable").to_string();
                        state.push_recent_error(
                            "PushIntentUnrecoverable",
                            format!(
                                "refused pull-only registration over a live push intent store={store_key} session={session_id} address={address}: {code}"
                            ),
                        );
                        return proto::incompatible_with_reason(
                            format!(
                                "address {address} has a live push intent for session {session_id} that could not be restored ({code}); \
                                 re-provision push with `telex --address {address} copilot resume`, or detach it with `telex --address {address} copilot detach` before attaching pull-only"
                            ),
                            NeedsAttachReason::PushIntentUnrecoverable,
                        );
                    }
                }
            }
            reconcile::LiveIntentLookup::Absent => {
                if indexed_live {
                    // The index says live but the manifest is gone or no longer live. Fail closed
                    // rather than guess.
                    return proto::incompatible_with_reason(
                        format!(
                            "address {address} has a live push intent for session {session_id} whose manifest could not be read; \
                             re-provision push with `telex --address {address} copilot resume`, or detach it before attaching pull-only"
                        ),
                        NeedsAttachReason::PushIntentUnrecoverable,
                    );
                }
            }
        }
    }

    let backend = match state.backend_for(&store_key).await {
        Ok(backend) => backend,
        Err(response) => return response,
    };
    if recovery {
        if let Some(responsibility) = application_responsibility.as_deref() {
            match backend
                .application_detach_intent(responsibility, &address)
                .await
            {
                Ok(Some(intent)) => {
                    return proto::needs_attach_with_reason(
                        format!(
                            "application responsibility {responsibility} deliberately detached from {address} in {store_key} by runtime {} at {}; explicit attach required",
                            intent.runtime_id, intent.at_ms
                        ),
                        NeedsAttachReason::DeliberatelyDetached,
                    )
                }
                Ok(None) => {}
                Err(e) => {
                    return proto::internal(format!(
                        "checking application detach intent before recovery register {responsibility}/{address}: {e:#}"
                    ))
                }
            }
        }
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
        if let Some(responsibility) = application_responsibility.as_deref() {
            match backend
                .application_detach_intent(responsibility, &address)
                .await
            {
                Ok(Some(intent)) => {
                    let _ = backend
                        .release_epoch_lease(
                            &address,
                            &claimed.owner_instance_id,
                            claimed.lease_epoch,
                        )
                        .await;
                    return proto::needs_attach_with_reason(
                        format!(
                            "application responsibility {responsibility} deliberately detached from {address} in {store_key} by runtime {} at {}; explicit attach required",
                            intent.runtime_id, intent.at_ms
                        ),
                        NeedsAttachReason::DeliberatelyDetached,
                    );
                }
                Ok(None) => {}
                Err(e) => {
                    let _ = backend
                        .release_epoch_lease(
                            &address,
                            &claimed.owner_instance_id,
                            claimed.lease_epoch,
                        )
                        .await;
                    return proto::internal(format!(
                        "checking application detach intent after recovery claim {responsibility}/{address}: {e:#}"
                    ));
                }
            }
        }
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
    } else {
        if let Err(e) = backend.clear_detach_tombstone(&session_id, &address).await {
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
        capability: capability.unwrap_or_default(),
        store_key: store_key.clone(),
        backend: backend.kind().to_string(),
        session_id: session_id.clone(),
        application_responsibility,
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
    let backlog = if record.on_deliver.is_some() {
        Some(record.clone())
    } else {
        None
    };
    #[cfg(test)]
    state
        .delivery_admission_before_commit(DeliveryAdmissionKind::Register)
        .await;
    if !recovery {
        if let Some(responsibility) = record.application_responsibility.as_deref() {
            if let Err(e) = backend
                .clear_application_detach_intent(responsibility, &address)
                .await
            {
                let _ = backend
                    .release_epoch_lease(&address, &claimed.owner_instance_id, claimed.lease_epoch)
                    .await;
                return proto::internal(format!(
                    "registering {address}: failed to clear durable application detach intent for {responsibility}: {e:#}"
                ));
            }
        }
    }
    // Commit the durable proof before publishing the member. If this fails, release the claimed
    // lease so reconciliation can retry without an in-memory member outrunning durable intent.
    if record.on_deliver.is_some() {
        if let Err(response) =
            commit_armed_proof(&state, &store_key, &session_id, &address, owes_armed_proof)
        {
            let _ = backend
                .release_epoch_lease(&address, &claimed.owner_instance_id, claimed.lease_epoch)
                .await;
            return response;
        }
    }
    // Explicit push→pull downgrade on a binding that had no in-memory member (the daemon-restart
    // shape: the manifest survived, the member did not). Same combined transition, same guard, same
    // rollback discipline as the armed proof above.
    if record.on_deliver.is_none() && replace_on_deliver {
        if let Err(response) = withdraw_downgraded_intent(&state, &store_key, &session_id, &address)
        {
            let _ = backend
                .release_epoch_lease(&address, &claimed.owner_instance_id, claimed.lease_epoch)
                .await;
            return response;
        }
    }
    if !recovery {
        state.clear_definite_session_end(&store_key, &session_id);
    }
    state.insert_member(record.clone());
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

/// Commit the durable **armed proof** for an arming push registration, or refuse the registration.
///
/// This is the transactional half of the arming register. It runs immediately before the member is
/// installed, so:
///
/// * There is no window in which a register has committed a member and not yet persisted its
///   proof. That window was real: a concurrent attach's rollback, deleting the `pending` record it
///   had just written, could land inside it — and the register still returned success for a push
///   station whose only durable trace had been destroyed.
/// * The per-intent write lock serializes this against `write_pending` and against the conditional
///   rollback delete, so a concurrent rollback either runs first (this reports `NoRecord`, and the
///   register is refused) or second (it finds an armed record and refuses to delete it). There is
///   no interleaving that yields "member armed, nothing durable".
///
/// `owes_proof` is the observation made at the top of the register, before any of this: it is what
/// separates "this binding has no intent record, so a proof is neither owed nor meaningful" — the
/// ordinary pull attach, or `telex attach --on-deliver` from a client that writes no intent — from
/// "the record that was here is gone".
///
/// The decision itself is the table in [`station_intent::armed_proof_admission`], so what this
/// function contributes is the wiring and the message, not a policy of its own. The part worth
/// stating here is the one that is *not* symmetric: a failure to open the scope refuses only a
/// register that owes a proof, because for a register that owes nothing the scope open is a
/// *create* of a directory it has nothing to put in — refusing push for every client that writes no
/// intent because that create failed is a denial with no durable state to protect. A record that is
/// present but unreadable is refused either way.
fn commit_armed_proof(
    state: &Arc<DaemonState>,
    store_key: &str,
    session_id: &str,
    address: &str,
    owes_proof: bool,
) -> std::result::Result<(), Response> {
    let stamped = state.stamp_intent_armed(store_key, session_id, address);
    let outcome = match &stamped {
        Ok(stamp) => Ok(*stamp),
        Err(refusal) => Err(refusal.failure),
    };
    if station_intent::armed_proof_admission(outcome, owes_proof)
        == station_intent::ArmedProofAdmission::Commit
    {
        return Ok(());
    }
    let detail = match &stamped {
        Ok(_) => "the station-intent record for this binding was removed while the registration was in flight".to_string(),
        Err(refusal) => refusal.detail.clone(),
    };
    state.push_recent_error(
        "StationIntent",
        format!(
            "refused push registration because the armed proof could not be persisted store={store_key} session={session_id} address={address}: {detail}"
        ),
    );
    Err(proto::incompatible_with_reason(
        format!(
            "push for {address} was not registered: the station-intent record that proves it is armed could not be written ({detail}); \
             re-run `telex --address {address} copilot resume` once the station-intent scope is writable"
        ),
        NeedsAttachReason::PushIntentUnrecoverable,
    ))
}

/// Withdraw the durable push intent for an explicit push→pull downgrade, or refuse the register.
///
/// `Register { on_deliver: None, replace_on_deliver: true }` is the one request shape that *means*
/// "give up push for this binding" — it is the only thing that clears an installed `on_deliver`,
/// and it is already exempt from the anti-downgrade guard for exactly that reason. The durable
/// desired state has to go with it, and it has to go under the **same** admission guard that
/// installs the pull-only member, because that is what makes the pair a single transition.
///
/// The fallback used to do this from the CLI, after the register returned. That is two transitions
/// with a gap: the daemon released this binding's admission at the end of the register, a reconcile
/// pass could take it and restore the push member from the still-live manifest, and the CLI's later
/// withdrawal then revoked the manifest while leaving the restored member armed next to the pull
/// waiter it was downgrading *to*. Doing it here closes the gap without a new protocol field —
/// there is no request shape to add, because the downgrade already had one.
///
/// Fallible and refusing: reporting `Registered` for a downgrade whose desired state still says
/// "restore push" hands back a member that the next pass is entitled to overwrite.
fn withdraw_downgraded_intent(
    state: &Arc<DaemonState>,
    store_key: &str,
    session_id: &str,
    address: &str,
) -> std::result::Result<(), Response> {
    match state.withdraw_intent_admitted(store_key, session_id, address, None) {
        Ok(_) => Ok(()),
        Err(detail) => {
            state.push_recent_error(
                "StationIntent",
                format!(
                    "refused push-to-pull downgrade because the station intent could not be withdrawn store={store_key} session={session_id} address={address}: {detail}"
                ),
            );
            Err(proto::incompatible_with_reason(
                format!(
                    "push for {address} was not downgraded to pull: its station-intent record could not be withdrawn ({detail}); \
                     a reconcile pass would restore push over the pull waiter, so the downgrade is refused"
                ),
                NeedsAttachReason::PushIntentUnrecoverable,
            ))
        }
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
        drain_intents: None,
    }
}

/// End a session's membership: release each binding's durable lease, mark it idle, and withdraw its
/// durable desired state — **each binding under its own admission guard, held across all three**.
///
/// Holding the guard across the whole per-binding transition is the correctness property, not an
/// optimization. Before it, the member snapshot, the lease releases and the idle marking all ran
/// outside the guard and only the withdrawal took it: a reconcile pass that held admission could
/// publish an armed push member in between, and the session end would then revoke the manifest
/// while leaving the member that manifest had authorized installed and delivering.
///
/// The binding set is the union of this session's live members and every binding the durable scope
/// names for it, because either one alone misses a case: a member with no manifest (nothing to
/// withdraw, but a member to end) and a manifest with no member (nothing to release, but the record
/// that would bring the session back).
///
/// Bounded by one total [`TEARDOWN_DEADLINE`] rather than a fresh budget per binding, and every
/// expiry is a failure of the whole request: an ended session whose desired state still says
/// "restore push" is exactly the state the next pass resurrects it from, so a partial teardown is
/// reported as a failed one.
async fn end_session_members(
    state: Arc<DaemonState>,
    store_key: String,
    session_id: String,
    kind: &str,
    reason: &str,
) -> Response {
    let deadline = station_intent::PassDeadline::at(Instant::now() + reconcile::TEARDOWN_DEADLINE);
    let durable = match state
        .session_teardown_bindings(&store_key, &session_id, deadline)
        .await
    {
        Ok(bindings) => bindings,
        Err(e) => {
            state.push_recent_error(kind, e.clone());
            return proto::internal(format!(
                "{kind} could not enumerate station intents for session {session_id} in {store_key}: {e}"
            ));
        }
    };
    let mut addresses: BTreeSet<String> = state
        .session_members(&store_key, &session_id)
        .into_iter()
        .map(|member| member.address)
        .collect();
    addresses.extend(durable.into_iter().map(|binding| binding.address));

    let mut affected = Vec::new();
    for address in addresses {
        let admission_budget = match teardown_budget(deadline) {
            Ok(budget) => budget,
            Err(e) => {
                state.push_recent_error(kind, e.clone());
                return proto::internal(format!(
                    "{kind} could not complete the teardown of session {session_id} in {store_key}: {e}"
                ));
            }
        };
        let admit = match state
            .admit_binding(&store_key, &session_id, &address, admission_budget)
            .await
        {
            Ok(admit) => admit,
            Err(e) => {
                state.push_recent_error(kind, e.clone());
                return proto::internal(format!(
                    "{kind} could not complete the teardown of session {session_id} in {store_key}: {e}"
                ));
            }
        };
        // Re-read under the guard: the member this teardown must end is whatever is installed
        // *now*, including one a reconcile pass published while this loop was waiting for
        // admission. Acting on the pre-admission snapshot is what let a freshly restored member
        // outlive the desired state that authorized it.
        if let Some(member) = state.get_member(&store_key, &session_id, &address) {
            if !member.idle {
                if let Some(response) =
                    release_definite_end_members(&state, std::slice::from_ref(&member), kind).await
                {
                    return response;
                }
            }
            if let Some(prior) =
                state.mark_member_idle(&store_key, &session_id, &address, kind, reason)
            {
                affected.push(prior);
            }
        }
        // An ended session must never be re-attended by a stale intent. This is daemon-owned and
        // harness-neutral: it covers `sessionEnd`, watch-pid death, and idle-TTL reaping alike,
        // because intents are generic records the daemon owns rather than Copilot state.
        //
        // Fallible, and it fails the request. A swallowed error here left desired state saying
        // "restore push" for a session that had ended, and the next reconcile pass — or the next
        // daemon — brought it back; the caller would have seen an `Ack` and had no reason to retry.
        if let Err(e) = state.withdraw_intent_admitted(&store_key, &session_id, &address, None) {
            state.push_recent_error(kind, e.clone());
            return proto::internal(format!(
                "{kind} could not withdraw station intents for session {session_id} in {store_key}: {e}"
            ));
        }
        // Still under the guard: nothing may have published a member here, and if anything did it
        // is a member the withdrawn record no longer authorizes.
        if let Some(stray) = state.mark_member_idle(&store_key, &session_id, &address, kind, reason)
        {
            affected.push(stray);
        }
        drop(admit);
    }

    if affected.is_empty() {
        state.push_recent_error(
            kind,
            format!("{kind} no-op store={store_key} session={session_id}: no active members"),
        );
    } else {
        state.record_definite_session_end(&store_key, &session_id, kind, &affected);
    }
    Response::Ack {
        message: Some(presence_ended_detail(kind)),
        delivery_outcome: None,
        address: None,
        message_id: None,
        lease_epoch: None,
        drain_intents: None,
    }
}

/// What is left of a teardown's one total deadline, or the explicit incomplete-teardown error.
///
/// Zero is refused rather than passed on: a zero-budget admission attempt is a teardown that
/// reports an answer it never waited for.
fn teardown_budget(
    deadline: station_intent::PassDeadline,
) -> std::result::Result<Duration, String> {
    match deadline.remaining() {
        Some(remaining) if remaining.is_zero() => Err(
            "the teardown deadline expired before every binding was withdrawn; \
             refusing to report a teardown that did not complete"
                .to_string(),
        ),
        Some(remaining) => Ok(remaining),
        None => Ok(reconcile::TEARDOWN_DEADLINE),
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

/// `Reset`: withdraw an address's attendance, in every session that holds it.
///
/// Each binding's idle marking and intent withdrawal run under **that binding's** admission guard,
/// held across both. Marking idle outside the guard and withdrawing inside it is not a
/// linearization: a reconcile pass holding admission could publish an armed push member between the
/// two, so the reset revoked the manifest and left behind precisely the armed member the operator
/// had just asked it to give up — with no durable marker anywhere saying so.
///
/// Scoped by *address*, not by the members this reset changed. Deriving the set from the affected
/// members missed exactly the cases that need it most: a station whose member was already idle
/// (marking it idle changes nothing, so nothing was withdrawn) and a station with no member at all,
/// where the manifest is the only thing left and is precisely what the next pass would restore from.
///
/// Reset is a deliberate operator withdrawal of attendance, so the *desired* state has to be
/// withdrawn with it. Without that the station intent stayed `live` and the next reconcile pass
/// re-registered the member within a tick — silently undoing the one operator action that has no
/// durable marker. Withdrawal is exact per binding and reversible with an explicit
/// `telex --address <address> copilot resume`.
async fn reset_station(state: Arc<DaemonState>, store_key: String, address: String) -> Response {
    let deadline = station_intent::PassDeadline::at(Instant::now() + reconcile::TEARDOWN_DEADLINE);
    let durable = match state
        .address_teardown_bindings(&store_key, &address, deadline)
        .await
    {
        Ok(bindings) => bindings,
        Err(e) => {
            state.push_recent_error("Reset", e.clone());
            return proto::internal(format!(
                "reset could not enumerate station intents for {address} in {store_key}: {e}"
            ));
        }
    };
    let mut sessions: BTreeSet<String> = state
        .address_members(&store_key, &address)
        .into_iter()
        .map(|member| member.session_id)
        .collect();
    sessions.extend(durable.into_iter().map(|binding| binding.session_id));

    let mut affected = Vec::new();
    for session_id in sessions {
        let admission_budget = match teardown_budget(deadline) {
            Ok(budget) => budget,
            Err(e) => {
                state.push_recent_error("Reset", e.clone());
                return proto::internal(format!(
                    "reset could not complete the teardown of {address} in {store_key}: {e}"
                ));
            }
        };
        let admit = match state
            .admit_binding(&store_key, &session_id, &address, admission_budget)
            .await
        {
            Ok(admit) => admit,
            Err(e) => {
                state.push_recent_error("Reset", e.clone());
                return proto::internal(format!(
                    "reset could not complete the teardown of {address} in {store_key}: {e}"
                ));
            }
        };
        // Re-read under the guard, so a member a reconcile pass published while this loop waited
        // for admission is idled by this reset rather than surviving it.
        if let Some(prior) = state.mark_member_idle(
            &store_key,
            &session_id,
            &address,
            "Reset",
            "operator reset requested",
        ) {
            affected.push(prior);
        }
        if let Err(e) = state.withdraw_intent_admitted(&store_key, &session_id, &address, None) {
            state.push_recent_error("Reset", e.clone());
            return proto::internal(format!(
                "reset could not withdraw station intents for {address} in {store_key}: {e}"
            ));
        }
        if let Some(stray) = state.mark_member_idle(
            &store_key,
            &session_id,
            &address,
            "Reset",
            "operator reset requested",
        ) {
            affected.push(stray);
        }
        drop(admit);
    }

    // Resetting the epoch is the durable fence. Do it only after every binding we could enumerate
    // has been made non-deliverable under its own admission guard. If enumeration or withdrawal
    // fails, leaving the existing epoch in place is safer than releasing it while a local member
    // can still deliver.
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
        lease_epoch: durable_epoch.or_else(|| affected.first().map(|m| m.lease_epoch)),
        drain_intents: None,
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
    // Admission is taken *before* the member is read, and held across the whole detach: the
    // backend release, the local member removal, and the intent withdrawal. Reading the member
    // outside the guard and withdrawing inside it is not a linearization — a reconcile pass
    // holding admission can publish an armed push member between the two, so the detach removed
    // the member it saw, revoked the manifest, and left the *new* member installed and armed.
    //
    // Admission is an in-memory async lock, so spanning the backend awaits is safe (it is the
    // outermost lock in the ordering and never held while acquiring another admission). What must
    // not span a backend await is the per-intent *filesystem* lock, and it does not:
    // `withdraw_intent_admitted` is a self-contained synchronous store operation.
    let admit = match state
        .admit_binding(
            &store_key,
            &session_id,
            &address,
            reconcile::TEARDOWN_DEADLINE,
        )
        .await
    {
        Ok(admit) => admit,
        Err(e) => {
            state.push_recent_error("Detach", e.clone());
            return proto::internal(format!(
                "detach could not take delivery admission for {address} in {store_key}: {e}"
            ));
        }
    };
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
                    std::slice::from_ref(&member),
                );
                // Do NOT record the durable tombstone again here: `release_epoch_lease_for_detach`
                // above already wrote it atomically inside the lease-release transaction (see the
                // backend contract). A second, non-atomic write can race a concurrent explicit
                // re-attach's tombstone clear and recreate a stale tombstone for a freshly-live
                // station, which `telex copilot push` would then refuse permanently.
                //
                // Intent withdrawal happens *after* the durable tombstone, deliberately: a crash
                // between the two leaves tombstone-wins, which the reconciler already honors, so
                // the station still cannot auto-return. The reverse order could leave a live
                // intent with no tombstone.
                //
                // The withdrawal itself is a self-contained synchronous store operation, and the
                // admission guard taken at the top of this function is still held, so no
                // filesystem lock is held across the backend awaits above or below it. It is
                // fallible and it fails the detach: reporting "detached" while the desired state
                // still says "restore push" is the exact shape of the bug this ordering exists to
                // prevent — with a tombstone present the station would not return, but the
                // operator would have no signal that the local record disagreed.
                if let Err(e) =
                    state.withdraw_intent_admitted(&store_key, &session_id, &address, None)
                {
                    state.push_recent_error("Detach", e.clone());
                    return proto::internal(format!(
                        "detached {address} durably but could not withdraw its station intent: {e}"
                    ));
                }
                // Nothing can have published a member since admission was taken, but sweep anyway:
                // this is the one place that can still prove the guard held, and removing an
                // already-absent member is free.
                state.remove_member(&store_key, &session_id, &address);
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
        drop(admit);
        Response::Ack {
            message: Some("detached".to_string()),
            delivery_outcome: None,
            address: Some(address),
            message_id: None,
            lease_epoch: Some(member.lease_epoch),
            drain_intents: None,
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
        // Same ordering as the attached branch: durable tombstone first, local intent second, both
        // under the admission guard taken at the top.
        //
        // This is the branch that most needs to be fallible. "No in-memory member" is exactly the
        // state a daemon restart leaves behind, so the manifest here is very often the *only*
        // remaining record of the binding — and the only thing that could bring it back.
        if let Err(e) = state.withdraw_intent_admitted(&store_key, &session_id, &address, None) {
            state.push_recent_error("Detach", e.clone());
            return proto::internal(format!(
                "recorded a durable detach tombstone for {address} but could not withdraw its \
                 station intent: {e}"
            ));
        }
        state.remove_member(&store_key, &session_id, &address);
        drop(admit);
        Response::Ack {
            message: Some("not-attached".to_string()),
            delivery_outcome: None,
            address: Some(address),
            message_id: None,
            lease_epoch: None,
            drain_intents: None,
        }
    }
}

async fn detach_application_member(
    state: Arc<DaemonState>,
    store_key: String,
    session_id: String,
    application_responsibility: String,
    address: String,
    capability: StationCapability,
) -> Response {
    // Application detach and register must share the binding admission boundary. In particular,
    // the durable detach intent must remain ordered with local-member publication: a registration
    // that clears it cannot publish a member between this lookup and its removal.
    let _admit = match state
        .admit_binding(
            &store_key,
            &session_id,
            &address,
            reconcile::TEARDOWN_DEADLINE,
        )
        .await
    {
        Ok(admit) => admit,
        Err(e) => {
            state.push_recent_error("ApplicationDetach", e.clone());
            return proto::internal(format!(
                "application detach could not take delivery admission for {address} in {store_key}: {e}"
            ));
        }
    };
    let backend = match state.backend_for(&store_key).await {
        Ok(backend) => backend,
        Err(response) => return response,
    };
    let capability_name = match capability {
        StationCapability::SendOnly => "send-only",
        StationCapability::Bidirectional => "bidirectional",
    };
    if let Some(member) = state.get_member(&store_key, &session_id, &address) {
        if member.application_responsibility.as_deref() != Some(application_responsibility.as_str())
        {
            return proto::error_response(
                proto::ERROR_NOT_OWNER,
                format!(
                    "session {session_id} does not own application responsibility {application_responsibility} at {address}"
                ),
            );
        }
        match backend
            .release_epoch_lease_for_application_detach(
                &address,
                &member.owner_instance_id,
                member.lease_epoch,
                &application_responsibility,
                &session_id,
                capability_name,
                "ApplicationDetach",
            )
            .await
        {
            Ok(true) => {
                state.remove_member(&store_key, &session_id, &address);
                state.record_definite_session_end(
                    &store_key,
                    &session_id,
                    "ApplicationDetach",
                    std::slice::from_ref(&member),
                );
                Response::Ack {
                    message: Some("detached".to_string()),
                    delivery_outcome: None,
                    address: Some(address),
                    message_id: None,
                    lease_epoch: Some(member.lease_epoch),
                    drain_intents: None,
                }
            }
            Ok(false) => {
                self_demote_member(
                    &state,
                    &member,
                    "application detach release_epoch_lease returned 0 rows",
                );
                proto::error_response(
                    proto::ERROR_NOT_OWNER,
                    format!("session {session_id} no longer owns {address} in {store_key}"),
                )
            }
            Err(e) => proto::internal(format!(
                "detaching application responsibility {application_responsibility} from {address}: {e:#}"
            )),
        }
    } else {
        if let Err(e) = backend
            .record_application_detach_intent(
                &application_responsibility,
                &address,
                &session_id,
                capability_name,
                "ApplicationDetach",
            )
            .await
        {
            return proto::internal(format!(
                "recording application detach intent for {application_responsibility}/{address}: {e:#}"
            ));
        }
        Response::Ack {
            message: Some("not-attached".to_string()),
            delivery_outcome: None,
            address: Some(address),
            message_id: None,
            lease_epoch: None,
            drain_intents: None,
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
    supports_delivery_quarantine: bool,
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
        supports_delivery_quarantine,
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
    supports_delivery_quarantine: bool,
    idle_ttl: Duration,
) -> Response {
    if state.is_draining() {
        return proto::error_response(proto::ERROR_NOT_RUNNING, "daemon is draining");
    }

    match state.get_member(&store_key, &session_id, &address) {
        Some(member) if member.capability == StationCapability::SendOnly => {
            return proto::unsupported(format!(
                "address {address} is attached with send-only capability"
            ));
        }
        Some(member) if member.on_deliver.is_some() => {
            state.push_recent_error(
                "DeliveryModeConflict",
                format!(
                    "rejected pull wait store={store_key} session={session_id} address={address}: push delivery is registered"
                ),
            );
            return Response::PresenceEnded;
        }
        Some(_) => {}
        None => {
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
    let delivery_admission = state
        .delivery_admission(
            &store_key,
            &session_id,
            &address,
            DeliveryAdmissionKind::Wait,
        )
        .await;
    let delivery_admission_guard = delivery_admission.lock().await;
    // Repeat the member/mode checks after all async preflight work. The opposite-mode recheck and
    // waiter installation below share the same per-station admission guard.
    match state.get_member(&store_key, &session_id, &address) {
        Some(member) if member.capability == StationCapability::SendOnly => {
            return proto::unsupported(format!(
                "address {address} is attached with send-only capability"
            ));
        }
        Some(member) if member.on_deliver.is_some() => {
            state.push_recent_error(
                "DeliveryModeConflict",
                format!(
                    "rejected pull wait at admission store={store_key} session={session_id} address={address}: push delivery is registered"
                ),
            );
            return Response::PresenceEnded;
        }
        Some(_) => {}
        None => {
            return needs_attach_for_missing_member(
                &state,
                &backend,
                &store_key,
                &session_id,
                &address,
                "wait-admission",
            )
            .await;
        }
    }
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
    #[cfg(test)]
    state
        .delivery_admission_before_commit(DeliveryAdmissionKind::Wait)
        .await;
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
    drop(delivery_admission_guard);
    drop(delivery_admission);
    let parsed_min_attention = match min_attention.as_deref().map(Attention::parse).transpose() {
        Ok(value) => value,
        Err(e) => {
            waiter_guard.suppress_abnormal_on_drop();
            return proto::error_response(proto::ERROR_INCOMPATIBLE, e.to_string());
        }
    };
    let mut skipped_oversized_cc = BTreeSet::new();
    loop {
        if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
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
        for candidate in candidates.into_iter().filter(|candidate| {
            wait_attention_matches(
                candidate.message.attention.as_str(),
                attention.as_deref(),
                parsed_min_attention,
            )
        }) {
            let notification_only = candidate.notification_only;
            let Some(delivery_id) = candidate.delivery_id else {
                waiter_guard.suppress_abnormal_on_drop();
                return proto::internal(format!(
                    "wait candidate for message {} recipient {address} has no durable delivery identity",
                    candidate.message.id
                ));
            };
            if notification_only && skipped_oversized_cc.contains(&delivery_id) {
                continue;
            }
            let snapshot_version = candidate.snapshot_version;
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
                metadata: row.metadata,
                sent_at_ms: row.sent_at_ms,
                buffered_at_ms: now_ms(),
                delivery_id: Some(delivery_id),
                snapshot_version: Some(snapshot_version),
                lease_epoch: Some(current.lease_epoch),
            };
            match proto::json_line_frame_len(&response) {
                Ok(len) if len <= proto::MAX_JSONL_FRAME_BYTES => {
                    state.record_waiter_message_exit(
                        &store_key,
                        &session_id,
                        &address,
                        row.id,
                        waiter_pid,
                    );
                    return response;
                }
                Ok(len) => {
                    if notification_only {
                        if skipped_oversized_cc.insert(delivery_id) {
                            state.push_recent_error(
                                "OversizedCcNotificationFrame",
                                format!(
                                    "skipped oversized notification-only CC frame store={store_key} address={address} message_id={} delivery_id={delivery_id}: serialized frame is {len} bytes, limit is {}",
                                    row.id,
                                    proto::MAX_JSONL_FRAME_BYTES
                                ),
                            );
                        }
                        continue;
                    }
                    let note = format!(
                        "daemon rejected delivery frame: serialized_bytes={len}; max_bytes={}",
                        proto::MAX_JSONL_FRAME_BYTES
                    );
                    let outcome = backend
                        .application_disposition_with_ack(
                            &address,
                            &current.owner_instance_id,
                            current.lease_epoch,
                            row.id,
                            delivery_id,
                            Disposition::Rejected.as_str(),
                            Some(&note),
                            Some("daemon"),
                            Some("daemon-quarantine"),
                            None,
                        )
                        .await;
                    match outcome {
                        Ok((
                            Some(_),
                            DeliveryOutcome::Marked | DeliveryOutcome::AlreadyConsumed,
                        )) => {
                            state.push_recent_error(
                                "OversizedDeliveryFrame",
                                format!(
                                    "durably rejected undeliverable historical message store={store_key} address={address} message_id={} delivery_id={delivery_id}: serialized frame is {len} bytes, limit is {}",
                                    row.id,
                                    proto::MAX_JSONL_FRAME_BYTES
                                ),
                            );
                        }
                        Ok((_, DeliveryOutcome::NotOwner)) => {
                            self_demote_member(
                                &state,
                                &current,
                                "oversized delivery cleanup returned NotOwner",
                            );
                            waiter_guard.suppress_abnormal_on_drop();
                            return needs_attach_for_missing_member(
                                &state,
                                &backend,
                                &store_key,
                                &session_id,
                                &address,
                                "oversized delivery cleanup",
                            )
                            .await;
                        }
                        Ok((disposition, other)) => {
                            waiter_guard.suppress_abnormal_on_drop();
                            return proto::internal(format!(
                                "rejecting oversized historical message {} delivery {delivery_id} returned {other:?} with disposition={}",
                                row.id,
                                disposition.is_some()
                            ));
                        }
                        Err(e) => {
                            waiter_guard.suppress_abnormal_on_drop();
                            return proto::internal(format!(
                                "consuming oversized historical message {} delivery {delivery_id}: {e:#}",
                                row.id
                            ));
                        }
                    }
                    waiter_guard.suppress_abnormal_on_drop();
                    state.record_waiter_exit(
                        &store_key,
                        &session_id,
                        &address,
                        WaiterOutcome::DeliveryQuarantined,
                        Some(6),
                        Some(format!(
                            "message_id={}; delivery_id={delivery_id}; serialized_bytes={len}; max_bytes={}",
                            row.id,
                            proto::MAX_JSONL_FRAME_BYTES
                        )),
                        waiter_pid_for_status,
                    );
                    return if supports_delivery_quarantine {
                        Response::DeliveryQuarantined {
                            message_id: row.id,
                            recipient: address.clone(),
                            serialized_bytes: len,
                            max_bytes: proto::MAX_JSONL_FRAME_BYTES,
                            may_continue: true,
                        }
                    } else {
                        proto::error_response(
                            proto::ERROR_INCOMPATIBLE,
                            format!(
                                "message {} exceeds the peer response limit; its unchanged delivery was quarantined",
                                row.id
                            ),
                        )
                    };
                }
                Err(e) => {
                    waiter_guard.suppress_abnormal_on_drop();
                    return proto::internal(format!("sizing message {} IPC frame: {e}", row.id));
                }
            }
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
    ack_message_inner(state, store_key, session_id, address, message_id, None).await
}

async fn ack_exact_delivery(
    state: Arc<DaemonState>,
    store_key: String,
    session_id: String,
    address: String,
    message_id: i64,
    delivery_id: i64,
) -> Response {
    ack_message_inner(
        state,
        store_key,
        session_id,
        address,
        message_id,
        Some(delivery_id),
    )
    .await
}

async fn ack_message_inner(
    state: Arc<DaemonState>,
    store_key: String,
    session_id: String,
    address: String,
    message_id: i64,
    delivery_id: Option<i64>,
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
    if member.capability == StationCapability::SendOnly {
        return proto::unsupported("acknowledgment requires bidirectional membership");
    }
    let outcome = match delivery_id {
        Some(delivery_id) => {
            backend
                .mark_delivery_consumed_if_current_owner(
                    &address,
                    &member.owner_instance_id,
                    member.lease_epoch,
                    message_id,
                    delivery_id,
                )
                .await
        }
        None => {
            backend
                .mark_consumed_if_current_owner(
                    &address,
                    &member.owner_instance_id,
                    member.lease_epoch,
                    message_id,
                )
                .await
        }
    };
    match outcome {
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
                drain_intents: None,
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

fn validate_message_delivery_frame_size(
    message: &NewMessage,
    thread_id: i64,
    cc: &[String],
) -> std::result::Result<(), Response> {
    let response = conservative_delivery_frame(message, thread_id, cc);
    let recipient = match &response {
        Response::Message { delivered_to, .. } => delivered_to.as_str(),
        _ => unreachable!("conservative delivery frame is always a message"),
    };
    match proto::json_line_frame_len(&response) {
        Ok(len) if len <= proto::MAX_JSONL_FRAME_BYTES => Ok(()),
        Ok(len) => Err(proto::error_response(
            proto::ERROR_INCOMPATIBLE,
            format!(
                "message delivery frame for {recipient} serializes to {len} bytes; limit is {} bytes",
                proto::MAX_JSONL_FRAME_BYTES
            ),
        )),
        Err(e) => Err(proto::internal(format!(
            "sizing message delivery frame for {recipient}: {e}"
        ))),
    }
}

fn conservative_delivery_frame(message: &NewMessage, thread_id: i64, cc: &[String]) -> Response {
    let recipient = std::iter::once(message.to_addr.as_str())
        .chain(cc.iter().map(String::as_str))
        .max_by_key(|recipient| {
            serde_json::to_string(recipient)
                .map(|encoded| encoded.len())
                .unwrap_or(usize::MAX)
        })
        .unwrap_or(message.to_addr.as_str());
    Response::Message {
        id: i64::MAX,
        thread_id,
        parent_id: message.parent_id,
        from_addr: message.from_addr.clone(),
        to_addr: message.to_addr.clone(),
        delivered_to: recipient.to_string(),
        primary_to: message.to_addr.clone(),
        cc: cc.to_vec(),
        // Longer than either actual role and therefore conservative.
        delivery_role: "unknown".to_string(),
        kind: message.kind.clone(),
        attention: message.attention.as_str().to_string(),
        requires_disposition: message.requires_disposition,
        // `false` serializes one byte longer than `true`.
        requires_disposition_for_current_recipient: false,
        subject: message.subject.clone(),
        body: message.body.clone(),
        metadata: message.metadata.clone(),
        sent_at_ms: i64::MAX,
        buffered_at_ms: i64::MAX,
        delivery_id: Some(i64::MAX),
        snapshot_version: Some(i64::MAX),
        lease_epoch: Some(i64::MAX),
    }
}

fn normalize_message_recipients(
    primary: &str,
    cc: Option<&str>,
) -> std::result::Result<(Option<String>, Vec<String>), Response> {
    let mut recipients = BTreeSet::from([primary.to_string()]);
    let mut recipient_entries = 1usize;
    for recipient in cc
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|recipient| !recipient.is_empty())
    {
        recipient_entries += 1;
        if recipient_entries > proto::MAX_MESSAGE_RECIPIENTS {
            return Err(proto::error_response(
                proto::ERROR_INCOMPATIBLE,
                format!(
                    "message has more than {} recipient entries",
                    proto::MAX_MESSAGE_RECIPIENTS
                ),
            ));
        }
        recipients.insert(recipient.to_string());
    }
    let cc: Vec<String> = recipients
        .into_iter()
        .filter(|recipient| recipient != primary)
        .collect();
    Ok(((!cc.is_empty()).then(|| cc.join(",")), cc))
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
    application_operation: Option<(String, String, String, String)>,
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
    let (cc, cc_recipients) = match normalize_message_recipients(&to_addr, cc.as_deref()) {
        Ok(recipients) => recipients,
        Err(response) => return response,
    };
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
    if let Err(response) = validate_message_delivery_frame_size(&new, i64::MAX, &cc_recipients) {
        return response;
    }
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
    let row = match application_operation {
        Some((logical_store_id, application_responsibility, operation_id, payload_fingerprint)) => {
            backend
                .insert_application_message(
                    &new,
                    &ApplicationMessageOperation {
                        logical_store_id,
                        application_responsibility,
                        operation_id,
                        payload_fingerprint,
                    },
                )
                .await
        }
        None => backend.insert_message(&new).await,
    };
    let row = match row {
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
    metadata: Option<String>,
    application_operation: Option<(String, String, String, String)>,
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
    let (cc, cc_recipients) = match normalize_message_recipients(&to, cc.as_deref()) {
        Ok(recipients) => recipients,
        Err(response) => return response,
    };
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
        metadata,
        sent_at_ms: now_ms(),
    };
    if let Err(response) =
        validate_message_delivery_frame_size(&new, parent.thread_id, &cc_recipients)
    {
        return response;
    }
    if let Err(e) = backend.ensure_address(&to, None, None, None).await {
        return proto::internal(format!("ensuring reply destination {to}: {e:#}"));
    }
    let row = match application_operation {
        Some((logical_store_id, application_responsibility, operation_id, payload_fingerprint)) => {
            backend
                .insert_application_message(
                    &new,
                    &ApplicationMessageOperation {
                        logical_store_id,
                        application_responsibility,
                        operation_id,
                        payload_fingerprint,
                    },
                )
                .await
        }
        None => backend.insert_message(&new).await,
    };
    let row = match row {
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
        test_state_at(label, seq, root)
    }

    fn test_state_at(label: &str, seq: u64, root: PathBuf) -> Arc<DaemonState> {
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
            delivery_admissions: Mutex::new(HashMap::new()),
            #[cfg(test)]
            delivery_admission_control: Mutex::new(None),
            next_waiter_id: AtomicU64::new(1),
            recent_errors: Arc::new(Mutex::new(VecDeque::new())),
            ended_sessions: Mutex::new(BTreeMap::new()),
            draining: AtomicBool::new(false),
            on_deliver: OnDeliverState::default(),
            intents: reconcile::IntentRuntime::default(),
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

    fn close_test_stores(state: &DaemonState, backend: Arc<dyn Backend>) {
        // A real daemon restart closes store locks before its successor starts. These unit
        // tests reuse one process, so release the old state's stores explicitly rather than
        // depending on Arc drop timing under the parallel test harness.
        state.stores.lock().unwrap().clear();
        assert_eq!(
            Arc::strong_count(&backend),
            1,
            "unexpected backend owner retained across simulated restart"
        );
        drop(backend);
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
            replace_watch_pids: false,
            recovery: false,
            on_deliver: None,
            replace_on_deliver: false,
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

    #[tokio::test]
    async fn explicit_replace_clears_push_while_plain_refresh_preserves_it() {
        let state = test_state("on-deliver-replace");
        let store = store_key("on-deliver-replace");
        let mut push = register_req(&store, "s1", "addr:a");
        if let Request::Register { on_deliver, .. } = &mut push {
            *on_deliver = Some(vec!["handler".to_string()]);
        }
        assert!(matches!(
            request(state.clone(), push).await,
            Response::Registered { .. }
        ));

        assert!(matches!(
            request(state.clone(), register_req(&store, "s1", "addr:a")).await,
            Response::Registered { .. }
        ));
        assert!(
            state
                .get_member(&store, "s1", "addr:a")
                .unwrap()
                .on_deliver
                .is_some(),
            "ordinary refresh must preserve push"
        );

        let mut pull = register_req(&store, "s1", "addr:a");
        if let Request::Register {
            replace_on_deliver, ..
        } = &mut pull
        {
            *replace_on_deliver = true;
        }
        assert!(matches!(
            request(state.clone(), pull).await,
            Response::Registered { .. }
        ));
        let status = state.status().await;
        assert_eq!(status.members[0].delivery_mode, DeliveryMode::Pull);
        assert!(!status.members[0].push_registered);
        assert!(state.on_deliver_candidates(&store, "addr:a").is_empty());
    }

    #[tokio::test]
    async fn wait_rejects_member_with_registered_push() {
        let state = test_state("push-rejects-wait");
        let store = store_key("push-rejects-wait");
        let mut push = register_req(&store, "s1", "addr:a");
        if let Request::Register { on_deliver, .. } = &mut push {
            *on_deliver = Some(vec!["handler".to_string()]);
        }
        assert!(matches!(
            request(state.clone(), push).await,
            Response::Registered { .. }
        ));

        let wait = request(state.clone(), wait_req(&store, "s1", "addr:a", 100)).await;
        assert!(matches!(wait, Response::PresenceEnded));
        let status = state.status().await;
        assert_eq!(status.members[0].delivery_mode, DeliveryMode::Push);
        assert_eq!(status.members[0].live_waiters_count, 0);
        assert!(status
            .recent_errors
            .iter()
            .any(|error| error.kind == "DeliveryModeConflict"));
    }

    #[tokio::test]
    async fn push_registration_rejects_live_waiter_without_stopping_it() {
        let state = test_state("wait-rejects-push");
        let store = store_key("wait-rejects-push");
        registered_epoch(state.clone(), &store, "s1", "addr:a").await;

        let waiter_state = state.clone();
        let waiter_req = wait_req(&store, "s1", "addr:a", 5_000);
        let waiter = tokio::spawn(async move { request(waiter_state, waiter_req).await });
        tokio::time::sleep(Duration::from_millis(75)).await;

        let mut push = register_req(&store, "s1", "addr:a");
        if let Request::Register { on_deliver, .. } = &mut push {
            *on_deliver = Some(vec!["handler".to_string()]);
        }
        let rejected = request(state.clone(), push).await;
        assert!(matches!(
            rejected,
            Response::Error { code, .. } if code == proto::ERROR_INCOMPATIBLE
        ));
        let status = state.status().await;
        assert_eq!(status.members[0].delivery_mode, DeliveryMode::Pull);
        assert_eq!(status.members[0].live_waiters_count, 1);
        assert!(!status.members[0].push_registered);

        {
            let mut members = state.members.lock().unwrap();
            members
                .get_mut(&DaemonState::member_key(&store, "s1", "addr:a"))
                .unwrap()
                .on_deliver = Some(vec!["legacy-handler".to_string()]);
        }
        let conflict = state.status().await;
        assert_eq!(conflict.members[0].delivery_mode, DeliveryMode::Conflict);
        assert_eq!(
            conflict.members[0].station_health,
            StationHealth::CoverageConflict
        );
        state
            .members
            .lock()
            .unwrap()
            .get_mut(&DaemonState::member_key(&store, "s1", "addr:a"))
            .unwrap()
            .on_deliver = None;

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

    async fn run_concurrent_delivery_admission_case(register_wins: bool) {
        let label = if register_wins {
            "concurrent-delivery-admission-register"
        } else {
            "concurrent-delivery-admission-wait"
        };
        let state = test_state(label);
        let store = store_key(label);
        registered_epoch(state.clone(), &store, "s1", "addr:a").await;

        let control = Arc::new(DeliveryAdmissionTestControl::new());
        *state.delivery_admission_control.lock().unwrap() = Some(control.clone());

        let mut push = register_req(&store, "s1", "addr:a");
        if let Request::Register { on_deliver, .. } = &mut push {
            *on_deliver = Some(vec!["handler".to_string()]);
        }
        let push_state = state.clone();
        let push_task = tokio::spawn(async move { request(push_state, push).await });

        let wait_state = state.clone();
        let wait_request = wait_req(&store, "s1", "addr:a", 5_000);
        let mut wait_task = Some(tokio::spawn(async move {
            request(wait_state, wait_request).await
        }));

        tokio::time::timeout(
            Duration::from_secs(2),
            control.wait_before_lock(DeliveryAdmissionKind::Register),
        )
        .await
        .expect("register reached admission boundary");
        tokio::time::timeout(
            Duration::from_secs(2),
            control.wait_before_lock(DeliveryAdmissionKind::Wait),
        )
        .await
        .expect("wait reached admission boundary");

        let winner = if register_wins {
            DeliveryAdmissionKind::Register
        } else {
            DeliveryAdmissionKind::Wait
        };
        let loser = if register_wins {
            DeliveryAdmissionKind::Wait
        } else {
            DeliveryAdmissionKind::Register
        };
        control.release_before_lock(winner);
        tokio::time::timeout(Duration::from_secs(2), control.wait_before_commit(winner))
            .await
            .expect("winner reached commit boundary");

        // The loser is now released while the winner is paused after its final recheck. With the
        // admission mutex removed, it would also reach the commit boundary and this assertion
        // would fail; with the mutex, it cannot pass the winner until after the winner commits.
        control.release_before_lock(loser);
        assert!(
            tokio::time::timeout(
                Duration::from_millis(100),
                control.wait_before_commit(loser)
            )
            .await
            .is_err(),
            "losing mode crossed the commit boundary before the winner installed"
        );
        control.release_commit(winner);

        let push_response = tokio::time::timeout(Duration::from_secs(2), push_task)
            .await
            .expect("concurrent push admission completed")
            .expect("push task joined");
        *state.delivery_admission_control.lock().unwrap() = None;

        let push_admitted = matches!(push_response, Response::Registered { .. });
        assert_eq!(push_admitted, register_wins);
        if !push_admitted {
            assert!(matches!(
                push_response,
                Response::Error { code, .. } if code == proto::ERROR_INCOMPATIBLE
            ));
        }

        if push_admitted {
            let wait_response =
                tokio::time::timeout(Duration::from_secs(2), wait_task.take().expect("wait task"))
                    .await
                    .expect("losing wait admission completed")
                    .expect("wait task joined");
            assert!(matches!(wait_response, Response::PresenceEnded));
        }

        if !register_wins {
            let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
            while !state.has_live_waiter_for(&store, "s1", "addr:a") {
                assert!(
                    tokio::time::Instant::now() < deadline,
                    "winning waiter did not install"
                );
                tokio::task::yield_now().await;
            }
        }

        let status = state.status().await;
        let member = status
            .members
            .iter()
            .find(|member| member.address == "addr:a")
            .expect("member status");
        let waiter_admitted = member.live_waiters_count == 1;
        assert_ne!(
            push_admitted, waiter_admitted,
            "exactly one delivery mode must be admitted: {member:?}"
        );
        assert_ne!(member.delivery_mode, DeliveryMode::Conflict);
        assert_ne!(member.station_health, StationHealth::CoverageConflict);

        if waiter_admitted {
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
            let wait_response =
                tokio::time::timeout(Duration::from_secs(2), wait_task.take().expect("wait task"))
                    .await
                    .expect("winning waiter stopped")
                    .expect("wait task joined");
            assert!(matches!(wait_response, Response::PresenceEnded));
        }
    }

    #[tokio::test]
    async fn concurrent_push_and_pull_admission_is_linearizable_in_both_orders() {
        run_concurrent_delivery_admission_case(true).await;
        run_concurrent_delivery_admission_case(false).await;
    }

    // -----------------------------------------------------------------------------------------
    // The arming-proof transaction (issue #106 / ADR 0052).
    //
    // A push register that owes a durable armed proof commits that proof *before* the member, and
    // fails the whole registration when it cannot. These tests drive each way the proof can fail
    // deterministically, through the same admission commit gate the linearizability test above
    // uses, so the interleaving is scheduled rather than raced.
    // -----------------------------------------------------------------------------------------

    /// Open the daemon's own intent scope, creating the run dir the way the real startup path does.
    fn intent_scope(state: &Arc<DaemonState>) -> crate::station_intent::IntentStore {
        std::fs::create_dir_all(&state.paths.run_dir).expect("create test run dir");
        crate::station_intent::IntentStore::open(&state.paths.run_dir, &state.paths.singleton_hash)
            .expect("open intent scope")
    }

    /// Seed the `Pending` record a first attach writes, and return its id.
    fn seed_pending_intent(
        state: &Arc<DaemonState>,
        store: &str,
        session: &str,
        address: &str,
    ) -> crate::station_intent::IntentId {
        let intent = crate::intent_test_support::pending_intent(
            store,
            session,
            address,
            &state.paths.singleton_hash,
        );
        let id = intent.id();
        intent_scope(state).write_pending(&intent).expect("seed");
        id
    }

    fn push_register_req(store: &str, session: &str, address: &str) -> Request {
        let mut req = register_req(store, session, address);
        if let Request::Register {
            on_deliver,
            replace_on_deliver,
            ..
        } = &mut req
        {
            // Empty argv: `push_registered` is true and the daemon never execs a handler.
            *on_deliver = Some(Vec::new());
            *replace_on_deliver = true;
        }
        req
    }

    /// Drive one arming register up to its commit gate, run `interleave` while it is parked there,
    /// then let it finish. The register is paused *after* every fallible step and *before* the
    /// proof commit, which is exactly the window a concurrent attach rollback lands in.
    ///
    /// Whatever `interleave` returns is held alive until the register has completed, so a scoped
    /// guard (an RAII filesystem fault, say) covers the proof commit rather than only the closure.
    async fn register_push_with_interleave<F, T>(
        state: &Arc<DaemonState>,
        store: &str,
        session: &str,
        address: &str,
        interleave: F,
    ) -> Response
    where
        F: FnOnce() -> T,
    {
        let control = Arc::new(DeliveryAdmissionTestControl::new());
        *state.delivery_admission_control.lock().unwrap() = Some(control.clone());
        control.release_before_lock(DeliveryAdmissionKind::Register);

        let request_state = state.clone();
        let req = push_register_req(store, session, address);
        let task = tokio::spawn(async move { request(request_state, req).await });

        tokio::time::timeout(
            Duration::from_secs(5),
            control.wait_before_commit(DeliveryAdmissionKind::Register),
        )
        .await
        .expect("arming register reached its commit gate");
        let held = interleave();
        control.release_commit(DeliveryAdmissionKind::Register);

        let response = tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .expect("arming register completed")
            .expect("register task joined");
        drop(held);
        *state.delivery_admission_control.lock().unwrap() = None;
        response
    }

    fn assert_refused_for_unrecoverable_proof(response: &Response) {
        match response {
            Response::Error {
                code,
                needs_attach_reason,
                ..
            } => {
                assert_eq!(code, proto::ERROR_INCOMPATIBLE, "got {response:?}");
                assert_eq!(
                    needs_attach_reason.as_ref(),
                    Some(&NeedsAttachReason::PushIntentUnrecoverable),
                    "the refusal must carry the typed reason the client acts on: {response:?}"
                );
            }
            other => panic!("expected a typed refusal, got {other:?}"),
        }
    }

    /// The exact concurrent-attach race the finding names, scheduled rather than hoped for.
    ///
    /// Attach A writes its `pending` record and registers. Concurrent attach B, for the same
    /// binding, replaces that record with its own generation, then fails downstream and rolls its
    /// write back — deleting the file. Before the fix, A's proof was stamped *after* A had already
    /// committed its member and A's response had been decided, so the delete landed in between: the
    /// stamp found nothing, the miss was swallowed as "the ordinary pull-attach case", and A
    /// returned a successful push registration whose only durable trace had been destroyed. The
    /// station delivered until the next daemon replacement and then silently stopped.
    ///
    /// Now the proof is committed before the member, so this interleaving cannot produce an armed
    /// member with no record: it produces a refusal, with the lease released and no member.
    #[tokio::test]
    async fn a_concurrent_pending_rollback_before_the_proof_refuses_the_register() {
        let state = test_state("arming-proof-missing-manifest");
        let store = store_key("arming-proof-missing-manifest");
        let id = seed_pending_intent(&state, &store, "s1", "addr:a");

        let scope = intent_scope(&state);
        let racing_scope = scope.clone();
        let racing_id = id.clone();
        let racing_store = store.clone();
        let racing_hash = state.paths.singleton_hash.clone();
        let response = register_push_with_interleave(&state, &store, "s1", "addr:a", move || {
            // B's pending write: the same binding, a new generation, no armed proof yet.
            let replacement = crate::intent_test_support::pending_intent(
                &racing_store,
                "s1",
                "addr:a",
                &racing_hash,
            );
            let crate::station_intent::PendingWrite::Created { generation } = racing_scope
                .write_pending(&replacement)
                .expect("concurrent pending write")
            else {
                panic!("the concurrent attach must have created its own generation");
            };
            // B fails and rolls back exactly what it wrote. Both of the rollback's own gates still
            // hold at this instant — the record is `Pending` and unarmed — so the delete happens.
            assert!(
                racing_scope
                    .remove_if_unchanged(&racing_id, generation, |c| c.state
                        == crate::daemon_ipc::IntentRecoveryState::Pending
                        && !c.is_armed())
                    .expect("rollback delete"),
                "precondition: the concurrent rollback really removed the record"
            );
        })
        .await;

        assert_refused_for_unrecoverable_proof(&response);
        assert!(
            state.get_member(&store, "s1", "addr:a").is_none(),
            "a refused register must not leave an armed member with no durable proof behind"
        );
        assert!(
            scope.load(&id).is_err(),
            "and it must not resurrect the record the rollback deleted"
        );
        // The epoch lease is released, so the address is claimable again rather than wedged by a
        // registration that was refused.
        let backend = state.backend_for(&store).await.expect("backend");
        let claimed = backend
            .claim_epoch_lease("addr:a", "another-instance", 30)
            .await
            .expect("claim");
        assert!(
            matches!(claimed, EpochClaimResult::Claimed(_)),
            "the refused register must have released its lease, got {claimed:?}"
        );
    }

    /// The narrower shape of the same failure: the manifest is simply **missing** at the moment the
    /// proof would be written, with no concurrent writer involved (an operator wipe, an external
    /// cleaner, a scope on a volume that went away). A register that observed a record when it
    /// started still owes a proof, so this is a refusal, not a silent downgrade to "nothing to do".
    #[tokio::test]
    async fn a_register_whose_manifest_is_missing_at_the_proof_is_refused() {
        let state = test_state("arming-proof-manifest-gone");
        let store = store_key("arming-proof-manifest-gone");
        let id = seed_pending_intent(&state, &store, "s1", "addr:a");
        let scope = intent_scope(&state);

        let gone = scope.clone();
        let gone_id = id.clone();
        let response = register_push_with_interleave(&state, &store, "s1", "addr:a", move || {
            std::fs::remove_file(gone.path_for(&gone_id)).expect("remove the manifest");
        })
        .await;

        assert_refused_for_unrecoverable_proof(&response);
        assert!(state.get_member(&store, "s1", "addr:a").is_none());
    }

    /// A record that exists but cannot be *written* is the same refusal.
    ///
    /// The stamp is the durability guarantee, so "the manifest is corrupt / the scope is broken"
    /// must fail the register rather than being logged and shrugged off.
    #[tokio::test]
    async fn a_register_whose_proof_cannot_be_written_is_refused_and_leaves_no_member() {
        let state = test_state("arming-proof-write-failure");
        let store = store_key("arming-proof-write-failure");
        let id = seed_pending_intent(&state, &store, "s1", "addr:a");
        let scope = intent_scope(&state);

        let corrupt = scope.clone();
        let corrupt_id = id.clone();
        let response = register_push_with_interleave(&state, &store, "s1", "addr:a", move || {
            std::fs::write(corrupt.path_for(&corrupt_id), b"{ truncated")
                .expect("corrupt the manifest under the register");
        })
        .await;

        assert_refused_for_unrecoverable_proof(&response);
        assert!(
            state.get_member(&store, "s1", "addr:a").is_none(),
            "a register that could not persist its proof must not leave a member behind"
        );
    }

    /// A **committed** register's record cannot be removed by a concurrent rollback before the
    /// proof lands, because the proof lands first.
    ///
    /// The rollback's own conditional delete is the second gate: once the record carries the proof,
    /// `rollback_removable` no longer holds and the generation has moved, so the delete is refused
    /// twice over. The register is idempotent across a repeat, which is what a re-attach does.
    #[tokio::test]
    async fn a_concurrent_rollback_cannot_delete_the_record_of_a_committed_register() {
        let state = test_state("arming-proof-rollback-race");
        let store = store_key("arming-proof-rollback-race");
        let id = seed_pending_intent(&state, &store, "s1", "addr:a");
        let scope = intent_scope(&state);
        let before = scope.load(&id).expect("seeded record");

        let response = request(state.clone(), push_register_req(&store, "s1", "addr:a")).await;
        assert!(
            matches!(response, Response::Registered { .. }),
            "got {response:?}"
        );
        let armed = scope.load(&id).expect("the record must still be here");
        assert!(
            armed.is_armed(),
            "a successful arming register is only successful once its proof is durable"
        );
        assert!(armed.generation > before.generation);

        // The losing half of the race: a concurrent attach's rollback, deciding from the
        // generation it wrote, now tries to delete this record.
        assert!(
            !scope
                .remove_if_unchanged(&id, before.generation, |c| c.state
                    == crate::daemon_ipc::IntentRecoveryState::Pending
                    && !c.is_armed())
                .expect("stale-generation rollback"),
            "a rollback holding the pre-register generation must never delete the proof"
        );
        assert!(
            !scope
                .remove_if_unchanged(&id, armed.generation, |c| c.state
                    == crate::daemon_ipc::IntentRecoveryState::Pending
                    && !c.is_armed())
                .expect("current-generation rollback"),
            "and even at the current generation the armed record is not the rollback's to delete"
        );
        assert!(scope.load(&id).is_ok(), "the proof survives both attempts");

        // Idempotency: a re-register (a re-attach, a resume) proves the same thing without
        // churning the generation, so concurrent CAS holders stay valid.
        let response = request(state.clone(), push_register_req(&store, "s1", "addr:a")).await;
        assert!(
            matches!(response, Response::Registered { .. }),
            "got {response:?}"
        );
        let again = scope.load(&id).expect("reload");
        assert_eq!(
            again.generation, armed.generation,
            "an idempotent re-register must not move the generation"
        );
        assert_eq!(again.armed, armed.armed);
    }

    /// A binding with **no** durable record is a supported mode, and it must stay one.
    ///
    /// A plain `telex attach --on-deliver` (and every pull attach) writes no intent, so there is
    /// nothing to prove and nothing to fail. Refusing those would break push for every client that
    /// is not the Copilot bridge — which is why the register observes whether a record exists
    /// *before* it runs, rather than inferring it from a missing record at stamp time.
    #[tokio::test]
    async fn a_push_register_for_a_binding_with_no_intent_record_still_succeeds() {
        let state = test_state("arming-proof-no-record");
        let store = store_key("arming-proof-no-record");
        let scope = intent_scope(&state);

        let response = request(state.clone(), push_register_req(&store, "s1", "addr:a")).await;
        assert!(
            matches!(response, Response::Registered { .. }),
            "a push attach that never wrote an intent must not be refused, got {response:?}"
        );
        assert!(state
            .get_member(&store, "s1", "addr:a")
            .is_some_and(|member| member.on_deliver.is_some()));
        assert!(
            scope
                .load(&crate::station_intent::IntentId::derive(
                    &store, "s1", "addr:a"
                ))
                .is_err(),
            "and it must not invent a record either"
        );
    }

    /// The whole point of the previous test, held apart from the failure this one pins: **absence**
    /// is what commits, and absence has to be *proven*.
    ///
    /// `Path::exists()` answers `false` for a record it could not stat — an ACL, an untraversable
    /// parent, a volume that went away — so an existing record that had become invisible read as
    /// "this binding is new": the up-front observation said no proof was owed, the stamp said
    /// `NoRecord` for the same reason, and the admission table then *committed* an armed member
    /// over a durable record it had never proven anything about. The register reported a durable
    /// push registration, and the record on disk — possibly a `live` one mid-reconcile — was
    /// neither armed nor consulted.
    ///
    /// Scheduled at the exact interleaving that makes it worst: nothing exists when the register
    /// makes its `owes` observation (so `owes_armed_proof == false`, the permissive column of the
    /// admission table), and the record appears and becomes unstatable while the register is parked
    /// at its commit gate. `RecordUnusable` is the one outcome that refuses in *both* columns, so
    /// this must be a refusal.
    #[tokio::test]
    async fn an_unstatable_record_refuses_an_arming_register_that_owed_no_proof() {
        let state = test_state("arming-proof-unstatable-record");
        let store = store_key("arming-proof-unstatable-record");
        let scope = intent_scope(&state);
        let id = crate::station_intent::IntentId::derive(&store, "s1", "addr:a");
        assert!(
            !scope.path_for(&id).exists(),
            "precondition: the register must observe no record, so it owes no proof"
        );

        let racing_scope = scope.clone();
        let racing_store = store.clone();
        let racing_hash = state.paths.singleton_hash.clone();
        let faulted_path = scope.path_for(&id);
        let response = register_push_with_interleave(&state, &store, "s1", "addr:a", move || {
            // A record for this binding appears after the observation — a concurrent attach, a
            // resume, an operator restore — and its metadata then becomes unreadable.
            let appeared = crate::intent_test_support::pending_intent(
                &racing_store,
                "s1",
                "addr:a",
                &racing_hash,
            );
            racing_scope
                .write_pending(&appeared)
                .expect("the record appears after the owes observation");
            crate::platform_fs::stat_faults::Unstatable::new(faulted_path)
        })
        .await;

        assert_refused_for_unrecoverable_proof(&response);
        assert!(
            state.get_member(&store, "s1", "addr:a").is_none(),
            "an unprovable record must not leave an armed member behind"
        );
        let record = scope
            .load(&id)
            .expect("the refused register must leave the record it could not read alone");
        assert!(
            !record.is_armed(),
            "and it must not have claimed a proof it never wrote"
        );

        // The control, on the same state and the same binding: with the fault gone the record is
        // visible again, so the identical register now succeeds *and* stamps its proof. The refusal
        // above was caused by the unreadable record and by nothing else about this scenario.
        let response = request(state.clone(), push_register_req(&store, "s1", "addr:a")).await;
        assert!(
            matches!(response, Response::Registered { .. }),
            "a readable record must still register, got {response:?}"
        );
        assert!(
            scope.load(&id).expect("reload").is_armed(),
            "and the register that succeeded must have left its durable proof"
        );
    }

    /// The same collapse one step earlier, in the observation itself.
    ///
    /// `durable_intent_present` answers "does this register owe a proof?". An existing record it
    /// could not stat used to answer `false` — no proof owed — which is the permissive column of
    /// the admission table for every subsequent outcome. Undecidable existence is now an error, and
    /// the register fails closed exactly as it does for an unreadable scope.
    #[tokio::test]
    async fn an_unstatable_record_makes_an_arming_register_fail_closed_up_front() {
        let state = test_state("arming-proof-unstatable-observation");
        let store = store_key("arming-proof-unstatable-observation");
        let id = seed_pending_intent(&state, &store, "s1", "addr:a");
        let scope = intent_scope(&state);

        let _fault = crate::platform_fs::stat_faults::Unstatable::new(scope.path_for(&id));
        assert!(
            state
                .durable_intent_present(&store, "s1", "addr:a")
                .is_err(),
            "an undecidable record must not be reported as 'this binding has no record'"
        );

        let response = request(state.clone(), push_register_req(&store, "s1", "addr:a")).await;
        assert_refused_for_unrecoverable_proof(&response);
        assert!(
            state.get_member(&store, "s1", "addr:a").is_none(),
            "a register that could not tell whether it owes a proof must commit nothing"
        );
    }

    /// A scope whose *root* cannot be stat'd is not an empty scope.
    ///
    /// `open_existing` returns `Ok(None)` for "this host never attached", which every read path
    /// treats as "no binding here has a durable record". Handing that answer to a scope full of
    /// records the daemon merely could not see is the same fail-open one directory higher up.
    #[tokio::test]
    async fn an_unstatable_scope_root_is_not_an_empty_scope() {
        let state = test_state("arming-proof-unstatable-scope");
        let store = store_key("arming-proof-unstatable-scope");
        seed_pending_intent(&state, &store, "s1", "addr:a");

        let _fault = crate::platform_fs::stat_faults::Unstatable::new(
            state
                .paths
                .run_dir
                .join("intents")
                .join(&state.paths.singleton_hash),
        );
        assert!(
            state
                .durable_intent_present(&store, "s1", "addr:a")
                .is_err(),
            "an unreadable scope root must fail closed, not report an empty scope"
        );

        let response = request(state.clone(), push_register_req(&store, "s1", "addr:a")).await;
        assert_refused_for_unrecoverable_proof(&response);
        assert!(state.get_member(&store, "s1", "addr:a").is_none());
    }

    /// The anti-downgrade guard is the other consumer of the same question, and it fails open in
    /// the same way.
    ///
    /// A pull-only registration over a binding that has a durable push intent must be refused. The
    /// guard re-reads the manifest precisely because the cached index is empty in the
    /// daemon-replacement window it protects — and then `Path::exists()` handed it `Absent` for a
    /// record it could not stat, which is the one answer that lets the downgrade through.
    #[tokio::test]
    async fn an_unstatable_record_refuses_a_pull_only_downgrade_rather_than_allowing_it() {
        let state = test_state("anti-downgrade-unstatable-record");
        let store = store_key("anti-downgrade-unstatable-record");
        let id = seed_pending_intent(&state, &store, "s1", "addr:a");
        let scope = intent_scope(&state);

        let key = reconcile::IntentKey {
            store_key: store.clone(),
            session_id: "s1".to_string(),
            address: "addr:a".to_string(),
        };
        let fault = crate::platform_fs::stat_faults::Unstatable::new(scope.path_for(&id));
        assert!(
            matches!(
                state.lookup_live_intent(&key),
                reconcile::LiveIntentLookup::Unavailable(_)
            ),
            "a record the guard could not stat must be 'unavailable', never 'absent'"
        );

        let response = request(state.clone(), register_req(&store, "s1", "addr:a")).await;
        assert_refused_for_unrecoverable_proof(&response);
        assert!(
            state.get_member(&store, "s1", "addr:a").is_none(),
            "a pull-only member must not be created over a record the guard could not read"
        );

        // And a scope the guard *can* read still admits the pull-only registration: absence is
        // proven here, not assumed.
        drop(fault);
        assert!(
            matches!(
                state.lookup_live_intent(&key),
                reconcile::LiveIntentLookup::Absent
            ),
            "a readable non-live record is genuinely absent for the guard's purposes"
        );
        let response = request(state.clone(), register_req(&store, "s1", "addr:a")).await;
        assert!(
            matches!(response, Response::Registered { .. }),
            "got {response:?}"
        );
    }

    /// A store file whose metadata could not be read is **transient**, not terminal.
    ///
    /// `store_missing` parks the intent on the hour-long quarantine cadence, on the reasoning that
    /// a store which does not exist will not start existing. `Path::exists()` handed that verdict
    /// to a store file behind a lock, an ACL, or a mount that came back a second later — replacing
    /// the published recovery bounds with an hour of silence for a condition that self-heals.
    #[tokio::test]
    async fn an_unreadable_sqlite_store_file_is_transient_not_terminal() {
        let state = test_state("store-open-unstatable");
        let store = store_key("store-open-unstatable");
        let path = crate::daemon::test_support::store_path_from_key(&store).expect("sqlite path");

        // Provable absence keeps its terminal verdict, which is what makes the other half a real
        // distinction rather than a blanket downgrade.
        assert!(!path.exists(), "precondition: nothing has opened the store");
        let absent = reconcile::backend_open_existing_only(&state, &store).await;
        assert!(
            matches!(
                absent.as_ref().err().map(String::as_str),
                Some("store_missing")
            ),
            "an absent store keeps its terminal verdict, got {:?}",
            absent.as_ref().err()
        );

        std::fs::write(&path, b"").expect("create the store file");
        let _fault = crate::platform_fs::stat_faults::Unstatable::new(&path);
        let unreadable = reconcile::backend_open_existing_only(&state, &store).await;
        let detail = unreadable
            .as_ref()
            .err()
            .cloned()
            .unwrap_or_else(|| "opened a store it could not stat".to_string());
        assert!(
            detail.starts_with("store_unreadable"),
            "an undecidable store file must take the retry ladder, got {detail:?}"
        );
    }

    /// A refused proof on a **refresh** must not disturb the member that was already there.
    ///
    /// The refresh path adopts an existing member and its epoch lease. Rolling the refresh back by
    /// removing the member (or releasing that lease) would turn a failed *diagnostic* write into
    /// the loss of a working station, so the proof is committed before the refreshed record is
    /// installed and a failure simply leaves the incumbent alone.
    #[tokio::test]
    async fn a_failed_proof_on_a_refresh_leaves_the_pre_existing_member_untouched() {
        let state = test_state("arming-proof-refresh-rollback");
        let store = store_key("arming-proof-refresh-rollback");
        let id = seed_pending_intent(&state, &store, "s1", "addr:a");
        let scope = intent_scope(&state);

        // First register: succeeds and stamps, so a member and a lease now exist.
        let first = request(state.clone(), push_register_req(&store, "s1", "addr:a")).await;
        let Response::Registered {
            lease_epoch,
            owner_instance_id,
        } = first
        else {
            panic!("expected the first register to succeed, got {first:?}");
        };
        let adopted = state
            .get_member(&store, "s1", "addr:a")
            .expect("member installed");

        // Now break the record and re-register. The refresh path must refuse, and the incumbent
        // member must be exactly as it was.
        std::fs::write(scope.path_for(&id), b"{ truncated").expect("corrupt the manifest");
        let response = request(state.clone(), push_register_req(&store, "s1", "addr:a")).await;
        assert_refused_for_unrecoverable_proof(&response);

        let after = state
            .get_member(&store, "s1", "addr:a")
            .expect("the pre-existing member must survive a refused refresh");
        assert_eq!(after.lease_epoch, lease_epoch);
        assert_eq!(after.owner_instance_id, owner_instance_id);
        assert_eq!(after.on_deliver, adopted.on_deliver);
        assert!(
            !after.idle,
            "the incumbent must not be demoted by a refused refresh"
        );
    }

    /// A register that owes **no** proof must not be refused because the scope could not be
    /// *created*.
    ///
    /// The proof commit opened the scope through the creating path, so a run directory in which the
    /// intent scope cannot be made — here a plain file where the `intents` directory belongs, but
    /// equally a read-only volume, a full disk, or leftover debris — turned every push registration
    /// into `Incompatible` / `PushIntentUnrecoverable`, including the ones for clients that write no
    /// intent at all and therefore have no durable state to lose. That is a denial with nothing to
    /// protect: the register is refused to guard a record that provably does not exist.
    ///
    /// The stamp now opens the scope through the read path, so a scope with no directory is
    /// `NoRecord` — "provably nothing to prove" — rather than a failure, and it creates nothing as a
    /// side effect of a registration that has nothing to write.
    #[tokio::test]
    async fn a_push_register_owing_no_proof_survives_a_scope_that_cannot_be_created() {
        let state = test_state("arming-proof-uncreatable-scope");
        let store = store_key("arming-proof-uncreatable-scope");
        std::fs::create_dir_all(&state.paths.run_dir).expect("run dir");
        // A file where the scope's parent directory belongs: the scope cannot be created, and it
        // does not exist, so nothing about this binding is durable.
        std::fs::write(state.paths.run_dir.join("intents"), b"not a directory")
            .expect("block the scope");

        assert!(
            matches!(
                state.stamp_intent_armed(&store, "s1", "addr:a"),
                Ok(crate::station_intent::ArmedProofStamp::NoRecord)
            ),
            "precondition: a scope that does not exist holds no records"
        );

        let response = request(state.clone(), push_register_req(&store, "s1", "addr:a")).await;
        assert!(
            matches!(response, Response::Registered { .. }),
            "a push register with no durable record must not be refused by a scope it never used, got {response:?}"
        );
        assert!(state
            .get_member(&store, "s1", "addr:a")
            .is_some_and(|member| member.on_deliver.is_some()));
        assert!(
            !state.paths.run_dir.join("intents").is_dir(),
            "and the proof commit must not have created a scope it had nothing to put in"
        );
    }

    /// The proof-commit gate, driven directly across both values of the obligation.
    ///
    /// `commit_armed_proof` is the whole difference between "a push registration that is durably
    /// recoverable" and "one that says it is", so its two directions are pinned here rather than
    /// only through the register paths above: a *scope-level* failure refuses only a register that
    /// owes a proof, while a record that is present and unreadable refuses **either way** — that is
    /// durable state about this binding that could not be verified, and it fails closed exactly as
    /// the anti-downgrade guard does.
    #[test]
    fn the_proof_commit_gate_refuses_an_unowed_register_only_for_a_broken_record() {
        let state = test_state("arming-proof-gate");
        let store = store_key("arming-proof-gate");

        // (a) No record and no scope: nothing is owed, nothing is provable, and a register that
        // owes nothing commits. A register that *did* observe a record fails closed.
        assert!(commit_armed_proof(&state, &store, "s1", "addr:a", false).is_ok());
        let refused = commit_armed_proof(&state, &store, "s1", "addr:a", true)
            .expect_err("an owed proof with no record must refuse");
        assert_refused_for_unrecoverable_proof(&refused);

        // (b) A healthy record is stamped whether or not the up-front observation saw it. This is
        // the benign half of the observation race: a record created between the observation and the
        // stamp is proven anyway.
        let id = seed_pending_intent(&state, &store, "s1", "addr:b");
        let scope = intent_scope(&state);
        assert!(commit_armed_proof(&state, &store, "s1", "addr:b", false).is_ok());
        assert!(
            scope.load(&id).expect("reload").is_armed(),
            "an unowed commit still stamps a record that is there"
        );

        // (c) A record that is present and unreadable is a refusal in both directions.
        let broken = seed_pending_intent(&state, &store, "s1", "addr:c");
        std::fs::write(scope.path_for(&broken), b"{ truncated").expect("corrupt the manifest");
        assert!(
            matches!(
                state.stamp_intent_armed(&store, "s1", "addr:c"),
                Err(reconcile::ArmedProofRefusal {
                    failure: crate::station_intent::ArmedProofFailure::RecordUnusable,
                    ..
                })
            ),
            "precondition: a corrupt record is classified as the record's failure, not the scope's"
        );
        let refused = commit_armed_proof(&state, &store, "s1", "addr:c", false)
            .expect_err("a broken record refuses even an unowed register");
        assert_refused_for_unrecoverable_proof(&refused);
        let refused = commit_armed_proof(&state, &store, "s1", "addr:c", true)
            .expect_err("and certainly an owed one");
        assert_refused_for_unrecoverable_proof(&refused);
    }

    /// **Post-combination invariant.** Both durable commits inside `register_member` still happen
    /// *before* the member is published, after the withdrawal work reordered the paths around them.
    ///
    /// The two orderings are the register-side counterpart of the withdrawal rules: the armed proof
    /// must be durable before an armed member exists (or a crash leaves push delivery working with
    /// no record that can recover it), and an Application Client's durable detach intent must be
    /// cleared before its member exists (or the member is observable while durable state still says
    /// the responsibility was detached). Both are structural — the failure they prevent is a crash
    /// in a window, which no behavioral test can schedule — so they are asserted against the source
    /// the same way the reconciler's tombstone-clearing claim is.
    #[test]
    fn register_member_commits_durable_state_before_it_publishes_the_member() {
        let source = include_str!("daemon.rs");
        let start = source
            .find("async fn register_member(")
            .expect("register_member must exist");
        let body = &source[start..];
        let end = body
            .find("\nasync fn commit_armed_proof")
            .or_else(|| body.find("\nfn commit_armed_proof"))
            .unwrap_or(body.len());
        let body = &body[..end];

        let insert = body
            .find("state.insert_member(record.clone());")
            .expect("register_member must publish the member");
        let proof = body
            .find("commit_armed_proof(")
            .expect("register_member must commit the armed proof");
        let detach_intent = body
            .find("clear_application_detach_intent(")
            .expect("register_member must clear the application detach intent");

        assert!(
            proof < insert,
            "the armed proof must be durable before an armed push member exists"
        );
        assert!(
            detach_intent < insert,
            "an Application Client's durable detach intent must be cleared before its member exists"
        );
    }

    /// **Post-combination invariant.** Every explicit teardown routes through the one linearized
    /// withdrawal operation, and none of them reaches a raw revoke.
    ///
    /// Asserted structurally because the failure it prevents is a *new* teardown path being added
    /// later with its own best-effort revocation — exactly how the paths this work consolidated
    /// drifted apart in the first place.
    #[test]
    fn no_teardown_path_bypasses_the_linearized_withdrawal() {
        for (name, source) in [
            ("daemon.rs", include_str!("daemon.rs")),
            ("daemon_reconcile.rs", include_str!("daemon_reconcile.rs")),
            ("commands/copilot.rs", include_str!("commands/copilot.rs")),
        ] {
            let production = match source.find("mod tests {") {
                Some(index) => &source[..index],
                None => source,
            };
            for (index, line) in production.lines().enumerate() {
                let trimmed = line.trim_start();
                if trimmed.starts_with("//") || trimmed.starts_with("///") {
                    continue;
                }
                assert!(
                    !line.contains("revoke_intent_for_binding")
                        && !line.contains("revoke_intents_for_session")
                        && !line.contains(".revoke("),
                    "{name}:{} reaches a raw revoke instead of withdraw_binding",
                    index + 1
                );
            }
        }
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

    /// How long a test may wait for an out-of-process on-deliver handler to be observed.
    ///
    /// Every assertion below that asks "was this pushed?" is really waiting on a **child process
    /// launch**: the daemon records the attempt only once the handler has exited, so the wait
    /// covers spawn + interpreter startup + exit. On Windows the recording handler is
    /// `powershell.exe`, whose cold start is seconds — not milliseconds — on a loaded four-core CI
    /// runner, and none of that is bounded by anything telex controls.
    ///
    /// This is a ceiling, not a measurement. A correct implementation satisfies these waits in
    /// milliseconds, so the suite stays fast; a broken one still fails, just at the ceiling instead
    /// of at an arbitrary 2.5s that a busy runner can blow through on its own. The two tests that
    /// failed on `windows-latest` while every sibling with identical logic passed
    /// (`on_deliver_wake_on_cc_pushes_live_cc_without_replay`,
    /// `drain_deferred_repushes_unacked_after_turn_stop`) were exactly the two whose end-to-end
    /// budget had to cover a PowerShell start.
    const HANDLER_OBSERVATION_BUDGET: Duration = Duration::from_secs(60);

    /// Poll `observed` until it holds or [`HANDLER_OBSERVATION_BUDGET`] runs out.
    ///
    /// Async on purpose: these run on a current-thread runtime, and the push being waited on is a
    /// task on that same runtime. A blocking sleep would starve the thing under observation.
    async fn wait_for(mut observed: impl FnMut() -> bool) -> bool {
        let deadline = Instant::now() + HANDLER_OBSERVATION_BUDGET;
        loop {
            if observed() {
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }

    async fn wait_for_file(path: &std::path::Path) -> bool {
        wait_for(|| path.exists()).await
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
        let pushed =
            wait_for(|| state.on_deliver_should_skip(&member_key, id, Instant::now())).await;
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
        let attempted =
            wait_for(|| state.on_deliver_should_skip(&member_key, live, Instant::now())).await;
        assert!(attempted, "live CC should record an on-deliver attempt");
        assert!(
            wait_for_file(&descriptor_path).await,
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
        let recorded =
            wait_for(|| state.on_deliver_should_skip(&member_key, id, Instant::now())).await;
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
        let resp = request(state.clone(), req).await;
        assert!(
            matches!(resp, Response::Registered { .. }),
            "push member should register; got: {resp:?}"
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
        state.on_deliver_record_attempt(&key, 3, t, false, true, false, None, false);
        assert_eq!(
            state.push_delivery_health(&key, 3, true, t + Duration::from_secs(1)),
            PushDeliveryHealth::Failing,
            "a same-timestamp real failure must win over a healthy busy deferral"
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
        let accepted =
            wait_for(|| state.on_deliver_should_skip(&member_key, id, Instant::now())).await;
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
        let resp = request(state.clone(), register).await;
        assert!(
            matches!(resp, Response::Registered { .. }),
            "expected Registered, got: {resp:?}"
        );
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
        let dead = wait_for(|| state.on_deliver_should_skip(&member_key, id, Instant::now())).await;
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
        assert_eq!(
            state.push_delivery_health(&member_key, 1, true, now),
            PushDeliveryHealth::Deferred,
            "a busy deferral proves the bridge is reachable"
        );
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
        let deferred = wait_for(|| state.on_deliver_deferred_count(&member_key) == 1).await;
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
        assert!(
            wait_for_file(&marker).await,
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
    async fn send_only_application_membership_is_not_inbound_attendance() {
        let state = test_state("application-send-only");
        let store = store_key("application-send-only");
        let address = "addr:sender-only";
        let registered = request(
            state.clone(),
            Request::ApplicationRegister {
                store_key: store.clone(),
                address: address.to_string(),
                session_id: "application-runtime".to_string(),
                application_responsibility: "application".to_string(),
                occupant: "application".to_string(),
                capability: StationCapability::SendOnly,
                description: None,
                scope: None,
                tags: None,
                watch_pids: Vec::new(),
                recovery: false,
            },
        )
        .await;
        assert!(matches!(registered, Response::Registered { .. }));

        let sent = request(
            state.clone(),
            Request::Send {
                store_key: store.clone(),
                session_id: "application-runtime".to_string(),
                from_addr: Some(address.to_string()),
                to_addr: address.to_string(),
                cc: None,
                kind: "note".to_string(),
                attention: "background".to_string(),
                requires_disposition: false,
                subject: None,
                body: "must remain queued".to_string(),
                metadata: None,
            },
        )
        .await;
        assert!(matches!(
            &sent,
            Response::Sent {
                receipt: SentReceipt {
                    occupied: Some(false),
                    receipt,
                    ..
                }
            } if receipt == "queued-unoccupied"
        ));
        let message_id = match sent {
            Response::Sent { receipt } => receipt.id,
            _ => unreachable!(),
        };
        let backend = state.backend_for(&store).await.unwrap();
        let delivery_id = backend
            .delivery_for_recipient(message_id, address)
            .await
            .unwrap()
            .unwrap()
            .id;
        assert!(matches!(
            request(
                state.clone(),
                Request::ApplicationAck {
                    store_key: store.clone(),
                    session_id: "application-runtime".to_string(),
                    address: address.to_string(),
                    message_id,
                    delivery_id,
                }
            )
            .await,
            Response::Error { ref code, .. } if code == proto::ERROR_UNSUPPORTED
        ));

        let wait = request(
            state,
            Request::Wait {
                store_key: store,
                session_id: "application-runtime".to_string(),
                address: address.to_string(),
                attention: None,
                min_attention: None,
                wake_on_cc: false,
                timeout_ms: Some(1),
                waiter_pid: Some(std::process::id()),
                waiter_start_time: None,
            },
        )
        .await;
        assert!(matches!(
            wait,
            Response::Error { ref code, .. } if code == proto::ERROR_UNSUPPORTED
        ));
    }

    #[tokio::test]
    async fn application_membership_capability_change_requires_detach() {
        let state = test_state("application-capability-change");
        let store = store_key("application-capability-change");
        let register = |capability| Request::ApplicationRegister {
            store_key: store.clone(),
            address: "addr:app".to_string(),
            session_id: "application-runtime".to_string(),
            application_responsibility: "application".to_string(),
            occupant: "application".to_string(),
            capability,
            description: None,
            scope: None,
            tags: None,
            watch_pids: Vec::new(),
            recovery: false,
        };
        assert!(matches!(
            request(state.clone(), register(StationCapability::Bidirectional)).await,
            Response::Registered { .. }
        ));
        assert!(matches!(
            request(state.clone(), register(StationCapability::SendOnly)).await,
            Response::Error { ref code, .. } if code == proto::ERROR_CAPABILITY_CONFLICT
        ));
        assert_eq!(
            state
                .get_member(&store, "application-runtime", "addr:app")
                .unwrap()
                .capability,
            StationCapability::Bidirectional
        );
    }

    #[tokio::test]
    async fn application_detach_blocks_replacement_runtime_bounded_repair() {
        let state = test_state("application-stable-detach");
        let store = store_key("application-stable-detach");
        let address = "addr:stable-detach";
        let responsibility = "stable-application";
        let register = |session: &str, recovery: bool| Request::ApplicationRegister {
            store_key: store.clone(),
            address: address.to_string(),
            session_id: session.to_string(),
            application_responsibility: responsibility.to_string(),
            occupant: responsibility.to_string(),
            capability: StationCapability::Bidirectional,
            description: None,
            scope: None,
            tags: None,
            watch_pids: Vec::new(),
            recovery,
        };

        assert!(matches!(
            request(state.clone(), register("runtime-one", false)).await,
            Response::Registered { .. }
        ));
        assert!(matches!(
            request(
                state.clone(),
                Request::ApplicationDetach {
                    store_key: store.clone(),
                    session_id: "runtime-one".to_string(),
                    application_responsibility: responsibility.to_string(),
                    address: address.to_string(),
                    capability: StationCapability::Bidirectional,
                }
            )
            .await,
            Response::Ack { .. }
        ));
        assert!(matches!(
            request(state.clone(), register("runtime-two", true)).await,
            Response::Error {
                ref code,
                needs_attach_reason: Some(NeedsAttachReason::DeliberatelyDetached),
                ..
            } if code == proto::ERROR_NEEDS_ATTACH
        ));
        let backend = state.backend_for(&store).await.unwrap();
        let intent = backend
            .application_detach_intent(responsibility, address)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(intent.runtime_id, "runtime-one");

        let control = Arc::new(DeliveryAdmissionTestControl::new());
        *state.delivery_admission_control.lock().unwrap() = Some(control.clone());
        let explicit_attach = register("runtime-two", false);
        let attach_state = state.clone();
        let attach = tokio::spawn(async move { request(attach_state, explicit_attach).await });
        control
            .wait_before_lock(DeliveryAdmissionKind::Register)
            .await;
        control.release_before_lock(DeliveryAdmissionKind::Register);
        control
            .wait_before_commit(DeliveryAdmissionKind::Register)
            .await;
        assert!(backend
            .application_detach_intent(responsibility, address)
            .await
            .unwrap()
            .is_some());
        control.release_commit(DeliveryAdmissionKind::Register);
        assert!(matches!(attach.await.unwrap(), Response::Registered { .. }));
        assert!(backend
            .application_detach_intent(responsibility, address)
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn failing_detach_intent_clear_keeps_membership_unobservable() {
        let state = test_state("application-failing-detach-intent-clear");
        let store = store_key("application-failing-detach-intent-clear");
        let address = "addr:failing-detach-intent-clear";
        let responsibility = "stable-application";
        let register = |session: &str, recovery: bool| Request::ApplicationRegister {
            store_key: store.clone(),
            address: address.to_string(),
            session_id: session.to_string(),
            application_responsibility: responsibility.to_string(),
            occupant: responsibility.to_string(),
            capability: StationCapability::Bidirectional,
            description: None,
            scope: None,
            tags: None,
            watch_pids: Vec::new(),
            recovery,
        };

        assert!(matches!(
            request(state.clone(), register("runtime-one", false)).await,
            Response::Registered { .. }
        ));
        assert!(matches!(
            request(
                state.clone(),
                Request::ApplicationDetach {
                    store_key: store.clone(),
                    session_id: "runtime-one".to_string(),
                    application_responsibility: responsibility.to_string(),
                    address: address.to_string(),
                    capability: StationCapability::Bidirectional,
                }
            )
            .await,
            Response::Ack { .. }
        ));

        let db_path = store
            .strip_prefix("sqlite:")
            .expect("SQLite test store key");
        rusqlite::Connection::open(db_path)
            .unwrap()
            .execute_batch(
                "CREATE TRIGGER fail_application_detach_intent_clear
                 BEFORE DELETE ON application_detach_intents
                 BEGIN
                     SELECT RAISE(FAIL, 'injected detach-intent clear failure');
                 END;",
            )
            .unwrap();

        let control = Arc::new(DeliveryAdmissionTestControl::new());
        *state.delivery_admission_control.lock().unwrap() = Some(control.clone());
        let attach_state = state.clone();
        let attach_request = register("runtime-two", false);
        let attach = tokio::spawn(async move { request(attach_state, attach_request).await });
        control
            .wait_before_lock(DeliveryAdmissionKind::Register)
            .await;
        control.release_before_lock(DeliveryAdmissionKind::Register);
        control
            .wait_before_commit(DeliveryAdmissionKind::Register)
            .await;

        assert!(state.get_member(&store, "runtime-two", address).is_none());
        assert!(matches!(
            request(
                state.clone(),
                Request::Send {
                    store_key: store.clone(),
                    session_id: "runtime-two".to_string(),
                    from_addr: Some(address.to_string()),
                    to_addr: "addr:destination".to_string(),
                    cc: None,
                    kind: "note".to_string(),
                    attention: "background".to_string(),
                    requires_disposition: false,
                    subject: None,
                    body: "must not send before detach intent clears".to_string(),
                    metadata: None,
                }
            )
            .await,
            Response::Error { ref code, .. } if code == proto::ERROR_NEEDS_ATTACH
        ));

        let recovery_state = state.clone();
        let recovery_request = register("runtime-two", true);
        let mut recovery =
            tokio::spawn(async move { request(recovery_state, recovery_request).await });
        control
            .wait_before_lock(DeliveryAdmissionKind::Register)
            .await;
        control.release_before_lock(DeliveryAdmissionKind::Register);
        assert!(
            tokio::time::timeout(Duration::from_millis(20), &mut recovery)
                .await
                .is_err(),
            "recovery register must remain serialized behind the pending attach"
        );

        control.release_commit(DeliveryAdmissionKind::Register);
        assert!(matches!(
            attach.await.unwrap(),
            Response::Error { ref code, ref message, .. }
                if code == proto::ERROR_INTERNAL
                    && message.contains("injected detach-intent clear failure")
        ));
        assert!(matches!(
            recovery.await.unwrap(),
            Response::Error {
                ref code,
                needs_attach_reason: Some(NeedsAttachReason::DeliberatelyDetached),
                ..
            } if code == proto::ERROR_NEEDS_ATTACH
        ));
        assert!(state.get_member(&store, "runtime-two", address).is_none());
        let backend = state.backend_for(&store).await.unwrap();
        assert!(backend
            .application_detach_intent(responsibility, address)
            .await
            .unwrap()
            .is_some());
    }

    #[tokio::test]
    async fn application_reply_preserves_opaque_metadata_through_thread_and_receive() {
        let state = test_state("application-reply-metadata");
        let store = store_key("application-reply-metadata");
        for (session, address) in [
            ("sender-runtime", "addr:sender"),
            ("target-runtime", "addr:target"),
        ] {
            assert!(matches!(
                request(
                    state.clone(),
                    Request::ApplicationRegister {
                        store_key: store.clone(),
                        address: address.to_string(),
                        session_id: session.to_string(),
                        application_responsibility: session.to_string(),
                        occupant: session.to_string(),
                        capability: StationCapability::Bidirectional,
                        description: None,
                        scope: None,
                        tags: None,
                        watch_pids: Vec::new(),
                        recovery: false,
                    }
                )
                .await,
                Response::Registered { .. }
            ));
        }
        let backend = state.backend_for(&store).await.unwrap();
        let parent = backend
            .insert_message(&NewMessage {
                from_addr: Some("addr:target".into()),
                to_addr: "addr:sender".into(),
                kind: "request".into(),
                attention: Attention::Background,
                body: "parent".into(),
                sent_at_ms: now_ms(),
                ..Default::default()
            })
            .await
            .unwrap();
        let logical_store_id = backend.logical_store_id().await.unwrap();
        let operation = NewApplicationOperation {
            logical_store_id: logical_store_id.clone(),
            application_responsibility: "metadata-test".into(),
            operation_id: "reply-operation".into(),
            operation_kind: "reply".into(),
            sender: "addr:sender".into(),
            recipients_json: format!("[{},[]]", parent.id),
            payload_fingerprint: "opaque-fingerprint".into(),
            retry_budget: 0,
            created_at_ms: now_ms(),
        };
        assert!(matches!(
            backend
                .begin_application_operation(&operation)
                .await
                .unwrap(),
            ApplicationOperationBegin::Started(_)
        ));
        let metadata = r#"{"urn:test:opaque":{"nested":[1,true,"value"]}}"#;
        let sent = request(
            state.clone(),
            Request::ApplicationReply {
                store_key: store.clone(),
                session_id: "sender-runtime".into(),
                from_addr: "addr:sender".into(),
                message_id: parent.id,
                kind: "reply".into(),
                attention: "background".into(),
                requires_disposition: false,
                subject: None,
                cc: None,
                body: "reply".into(),
                metadata: Some(metadata.into()),
                logical_store_id,
                application_responsibility: "metadata-test".into(),
                operation_id: "reply-operation".into(),
                payload_fingerprint: "opaque-fingerprint".into(),
            },
        )
        .await;
        let reply_id = match sent {
            Response::Sent { receipt } => receipt.id,
            other => panic!("expected sent reply, got {other:?}"),
        };
        assert_eq!(
            backend
                .get_message(reply_id)
                .await
                .unwrap()
                .unwrap()
                .metadata
                .as_deref(),
            Some(metadata)
        );
        assert!(
            backend
                .thread_messages(parent.thread_id)
                .await
                .unwrap()
                .iter()
                .any(|message| message.id == reply_id
                    && message.metadata.as_deref() == Some(metadata))
        );
        assert!(matches!(
            request(
                state,
                Request::Wait {
                    store_key: store,
                    session_id: "target-runtime".into(),
                    address: "addr:target".into(),
                    attention: None,
                    min_attention: None,
                    wake_on_cc: false,
                    timeout_ms: Some(100),
                    waiter_pid: Some(std::process::id()),
                    waiter_start_time: None,
                }
            )
            .await,
            Response::Message {
                id,
                metadata: Some(actual),
                ..
            } if id == reply_id && actual == metadata
        ));
    }

    #[tokio::test]
    async fn bidirectional_application_membership_is_occupied_and_exact_acknowledgeable() {
        let state = test_state("application-bidirectional");
        let store = store_key("application-bidirectional");
        let address = "addr:application";
        assert!(matches!(
            request(
                state.clone(),
                Request::ApplicationRegister {
                    store_key: store.clone(),
                    address: address.to_string(),
                    session_id: "application-runtime".to_string(),
                    application_responsibility: "application".to_string(),
                    occupant: "application".to_string(),
                    capability: StationCapability::Bidirectional,
                    description: None,
                    scope: None,
                    tags: None,
                    watch_pids: Vec::new(),
                    recovery: false,
                }
            )
            .await,
            Response::Registered { .. }
        ));
        let sent = request(
            state.clone(),
            Request::Send {
                store_key: store.clone(),
                session_id: "application-runtime".to_string(),
                from_addr: Some(address.to_string()),
                to_addr: address.to_string(),
                cc: None,
                kind: "note".to_string(),
                attention: "background".to_string(),
                requires_disposition: false,
                subject: None,
                body: "deliver".to_string(),
                metadata: None,
            },
        )
        .await;
        assert!(matches!(
            sent,
            Response::Sent {
                receipt: SentReceipt {
                    occupied: Some(true),
                    ..
                }
            }
        ));
        let delivered = request(
            state.clone(),
            Request::Wait {
                store_key: store.clone(),
                session_id: "application-runtime".to_string(),
                address: address.to_string(),
                attention: None,
                min_attention: None,
                wake_on_cc: false,
                timeout_ms: Some(100),
                waiter_pid: Some(std::process::id()),
                waiter_start_time: None,
            },
        )
        .await;
        let (message_id, delivery_id) = match delivered {
            Response::Message {
                id,
                delivery_id: Some(delivery_id),
                ..
            } => (id, delivery_id),
            other => panic!("expected exact delivery, got {other:?}"),
        };
        assert!(matches!(
            request(
                state,
                Request::ApplicationAck {
                    store_key: store,
                    session_id: "application-runtime".to_string(),
                    address: address.to_string(),
                    message_id,
                    delivery_id,
                }
            )
            .await,
            Response::Ack {
                delivery_outcome: Some(DeliveryOutcome::Marked),
                ..
            }
        ));
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
            close_test_stores(&state, backend);
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
    async fn send_and_reply_reject_oversized_delivery_frames_before_insert() {
        let state = test_state("payload-cap");
        let store = store_key("payload-cap");
        registered_epoch(state.clone(), &store, "s1", "addr:a").await;
        registered_epoch(state.clone(), &store, "s2", "dest").await;
        let backend = state.backend_for(&store).await.unwrap();
        let before = backend.inbox("dest", true, 100).await.unwrap().len();
        let deliveries_before = backend.delivery_retention_count().await.unwrap();
        let too_large = "x".repeat(proto::MAX_MESSAGE_BODY_METADATA_BYTES + 1);
        let escape_heavy_metadata =
            "\"".repeat(proto::MAX_MESSAGE_BODY_METADATA_BYTES.saturating_mul(3) / 5);

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
        assert_eq!(
            backend.delivery_retention_count().await.unwrap(),
            deliveries_before
        );

        let escaped_send = request(
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
                body: String::new(),
                metadata: Some(escape_heavy_metadata.clone()),
            },
        )
        .await;
        assert!(matches!(
            escaped_send,
            Response::Error { ref code, ref message, .. }
                if code == proto::ERROR_INCOMPATIBLE
                    && message.contains("delivery frame")
        ));
        assert_eq!(
            backend.inbox("dest", true, 100).await.unwrap().len(),
            before
        );
        assert_eq!(
            backend.delivery_retention_count().await.unwrap(),
            deliveries_before
        );

        let too_many_cc = (0..proto::MAX_MESSAGE_RECIPIENTS)
            .map(|index| format!("cc:{index}"))
            .collect::<Vec<_>>()
            .join(",");
        assert!(backend.get_address("cap-dest").await.unwrap().is_none());
        let capped = request(
            state.clone(),
            Request::Send {
                store_key: store.clone(),
                session_id: "s1".to_string(),
                from_addr: Some("addr:a".to_string()),
                to_addr: "cap-dest".to_string(),
                cc: Some(too_many_cc),
                kind: "note".to_string(),
                attention: "background".to_string(),
                requires_disposition: false,
                subject: None,
                body: "bounded".to_string(),
                metadata: None,
            },
        )
        .await;
        assert!(matches!(
            capped,
            Response::Error { ref code, ref message, .. }
                if code == proto::ERROR_INCOMPATIBLE
                    && message.contains("recipient entries")
        ));
        assert!(backend.get_address("cap-dest").await.unwrap().is_none());
        assert!(backend
            .inbox("cap-dest", true, 100)
            .await
            .unwrap()
            .is_empty());
        assert_eq!(
            backend.delivery_retention_count().await.unwrap(),
            deliveries_before
        );

        let parent_id = insert_test_message(&backend, "addr:a", None).await;
        let reply_deliveries_before = backend.delivery_retention_count().await.unwrap();
        let reply = request(
            state.clone(),
            Request::Reply {
                store_key: store.clone(),
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
        assert_eq!(backend.inbox("addr:a", true, 100).await.unwrap().len(), 1);

        let escaped_reply = request(
            state,
            Request::ApplicationReply {
                store_key: store,
                session_id: "s1".to_string(),
                from_addr: "addr:a".to_string(),
                message_id: parent_id,
                kind: "note".to_string(),
                attention: "background".to_string(),
                requires_disposition: false,
                subject: None,
                cc: None,
                body: String::new(),
                metadata: Some(escape_heavy_metadata),
                logical_store_id: backend.logical_store_id().await.unwrap(),
                application_responsibility: "reply-test".to_string(),
                operation_id: "escape-heavy-reply".to_string(),
                payload_fingerprint: "e".repeat(64),
            },
        )
        .await;
        assert!(matches!(
            escaped_reply,
            Response::Error { ref code, ref message, .. }
                if code == proto::ERROR_INCOMPATIBLE
                    && message.contains("delivery frame")
        ));
        assert_eq!(
            backend.delivery_retention_count().await.unwrap(),
            reply_deliveries_before
        );
    }

    #[tokio::test]
    async fn escape_heavy_metadata_is_received_unchanged_when_delivery_frame_fits() {
        let state = test_state("escape-heavy-metadata");
        let store = store_key("escape-heavy-metadata");
        registered_epoch(state.clone(), &store, "sender-session", "sender").await;
        registered_epoch(state.clone(), &store, "recipient-session", "recipient").await;
        let metadata = serde_json::json!({
            "quotes": "\"".repeat(8_192),
            "controls": "\u{0000}\u{0001}\n\r\t".repeat(2_048),
        })
        .to_string();
        let worst_recipient = "\\\"\u{0001}".repeat(2_048);
        let normalized_cc = vec!["short".to_string(), worst_recipient.clone()];
        let mut near_limit = NewMessage {
            from_addr: Some("sender".into()),
            to_addr: "recipient".into(),
            cc: Some(normalized_cc.join(",")),
            kind: "note".into(),
            attention: Attention::Background,
            body: String::new(),
            sent_at_ms: now_ms(),
            ..Default::default()
        };
        let expected_empty = Response::Message {
            id: i64::MAX,
            thread_id: i64::MAX,
            parent_id: None,
            from_addr: Some("sender".into()),
            to_addr: "recipient".into(),
            delivered_to: worst_recipient,
            primary_to: "recipient".into(),
            cc: normalized_cc.clone(),
            delivery_role: "unknown".into(),
            kind: "note".into(),
            attention: "background".into(),
            requires_disposition: false,
            requires_disposition_for_current_recipient: false,
            subject: None,
            body: String::new(),
            metadata: None,
            sent_at_ms: i64::MAX,
            buffered_at_ms: i64::MAX,
            delivery_id: Some(i64::MAX),
            snapshot_version: Some(i64::MAX),
            lease_epoch: Some(i64::MAX),
        };
        let empty_len = proto::json_line_frame_len(&expected_empty).unwrap();
        near_limit.body = "x".repeat(proto::MAX_JSONL_FRAME_BYTES - empty_len);
        let expected = Response::Message {
            id: i64::MAX,
            thread_id: i64::MAX,
            parent_id: None,
            from_addr: Some("sender".into()),
            to_addr: "recipient".into(),
            delivered_to: "\\\"\u{0001}".repeat(2_048),
            primary_to: "recipient".into(),
            cc: normalized_cc.clone(),
            delivery_role: "unknown".into(),
            kind: "note".into(),
            attention: "background".into(),
            requires_disposition: false,
            requires_disposition_for_current_recipient: false,
            subject: None,
            body: near_limit.body.clone(),
            metadata: None,
            sent_at_ms: i64::MAX,
            buffered_at_ms: i64::MAX,
            delivery_id: Some(i64::MAX),
            snapshot_version: Some(i64::MAX),
            lease_epoch: Some(i64::MAX),
        };
        assert_eq!(
            proto::json_line_frame_len(&expected).unwrap(),
            proto::MAX_JSONL_FRAME_BYTES
        );
        assert!(
            validate_message_delivery_frame_size(&near_limit, i64::MAX, &normalized_cc).is_ok()
        );
        near_limit.body.push('x');
        assert!(
            validate_message_delivery_frame_size(&near_limit, i64::MAX, &normalized_cc).is_err()
        );

        let send = request(
            state.clone(),
            Request::Send {
                store_key: store.clone(),
                session_id: "sender-session".to_string(),
                from_addr: Some("sender".to_string()),
                to_addr: "recipient".to_string(),
                cc: None,
                kind: "note".to_string(),
                attention: "background".to_string(),
                requires_disposition: false,
                subject: Some("\"quoted\"\nsubject".to_string()),
                body: "\"body\"\nwith\tcontrols".to_string(),
                metadata: Some(metadata.clone()),
            },
        )
        .await;
        assert!(matches!(send, Response::Sent { .. }));

        let received = request(
            state,
            wait_req(&store, "recipient-session", "recipient", 1_000),
        )
        .await;
        assert!(proto::json_line_frame_len(&received).unwrap() <= proto::MAX_JSONL_FRAME_BYTES);
        assert!(matches!(
            received,
            Response::Message {
                metadata: Some(received),
                ..
            } if received == metadata
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
            close_test_stores(&state, backend);
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
    async fn oversized_historical_message_is_consumed_without_blocking_next_delivery() {
        let seq = TEST_SEQ.fetch_add(1, Ordering::SeqCst);
        let root = std::env::current_dir()
            .unwrap()
            .join("target")
            .join("daemon-p3-tests")
            .join(format!("oversized-frame-restart-{seq}"));
        let state = test_state_at("oversized-frame", seq, root.clone());
        let store = store_key("oversized-frame");
        registered_epoch(state.clone(), &store, "s1", "addr:a").await;
        let backend = state.backend_for(&store).await.unwrap();
        let prior_id = insert_test_message(&backend, "addr:a", None).await;
        assert!(matches!(
            request(state.clone(), wait_req(&store, "s1", "addr:a", 1_000)).await,
            Response::Message { id, .. } if id == prior_id
        ));
        assert!(matches!(
            request(state.clone(), ack_req(&store, "s1", "addr:a", prior_id)).await,
            Response::Ack {
                delivery_outcome: Some(DeliveryOutcome::Marked),
                ..
            }
        ));
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
        let following_id = backend
            .insert_message(&NewMessage {
                parent_id: None,
                from_addr: Some("sender".to_string()),
                to_addr: "addr:a".to_string(),
                cc: None,
                kind: "note".to_string(),
                attention: Attention::Background,
                requires_disposition: false,
                subject: None,
                body: "following".to_string(),
                metadata: None,
                sent_at_ms: now_ms(),
            })
            .await
            .unwrap()
            .id;

        let wait = request(state.clone(), wait_req(&store, "s1", "addr:a", 1_000)).await;
        match wait {
            Response::DeliveryQuarantined {
                message_id: quarantined_id,
                ref recipient,
                serialized_bytes,
                max_bytes,
                may_continue: true,
            } => {
                assert_eq!(quarantined_id, message_id);
                assert_eq!(recipient, "addr:a");
                assert!(serialized_bytes > max_bytes);
            }
            other => panic!("expected oversized-frame error, got {other:?}"),
        }
        let status = state.status().await;
        assert_eq!(
            status.members[0].last_waiter_outcome,
            Some(WaiterOutcome::DeliveryQuarantined)
        );
        assert_eq!(status.members[0].last_waiter_exit_code, Some(6));
        assert_eq!(status.members[0].last_delivered_message_id, Some(prior_id));
        let dispositions = backend.dispositions_for(message_id).await.unwrap();
        assert!(dispositions.iter().any(|disposition| {
            disposition.recipient == "addr:a"
                && disposition.state == Disposition::Rejected.as_str()
                && disposition.note.as_deref().is_some_and(|note| {
                    note.contains("serialized_bytes=") && note.contains("max_bytes=")
                })
                && disposition.by_principal.as_deref() == Some("daemon")
        }));
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
        drop(backend);
        drop(state);

        let restarted = test_state_at("oversized-frame", seq + 1, root);
        registered_epoch(restarted.clone(), &store, "s1", "addr:a").await;
        let backend = restarted.backend_for(&store).await.unwrap();
        let persisted_dispositions = backend.dispositions_for(message_id).await.unwrap();
        assert!(persisted_dispositions.iter().any(|disposition| {
            disposition.recipient == "addr:a"
                && disposition.state == Disposition::Rejected.as_str()
                && disposition.by_principal.as_deref() == Some("daemon")
        }));
        let second_wait = request(restarted, wait_req(&store, "s1", "addr:a", 1_000)).await;
        assert!(
            matches!(second_wait, Response::Message { id, .. } if id == following_id),
            "the consumed poison row must not block the following delivery"
        );
        assert_eq!(
            backend
                .fetch_undelivered("addr:a")
                .await
                .unwrap()
                .iter()
                .map(|message| message.id)
                .collect::<Vec<_>>(),
            vec![following_id]
        );
    }

    #[tokio::test]
    async fn quarantine_response_is_capability_fenced() {
        assert!(proto::daemon_capabilities()
            .iter()
            .any(|capability| capability == proto::CAP_DELIVERY_QUARANTINE_V1));
        assert!(!proto::REQUIRED_CAPABILITIES.contains(&proto::CAP_DELIVERY_QUARANTINE_V1));
        assert!(proto::client_hello("test")
            .capabilities
            .iter()
            .any(|capability| capability == proto::CAP_DELIVERY_QUARANTINE_V1));
        let daemon = crate::daemon::test_support::TestDaemon::new("quarantine-capability");
        let store = daemon.store_key("quarantine-capability");
        assert!(matches!(
            daemon
                .register(&store, "old-session", "old-recipient")
                .await,
            Response::Registered { .. }
        ));
        assert!(matches!(
            daemon
                .register(&store, "new-session", "new-recipient")
                .await,
            Response::Registered { .. }
        ));
        let backend = daemon.backend(&store).await.unwrap();
        for recipient in ["old-recipient", "new-recipient"] {
            backend
                .insert_message(&NewMessage {
                    from_addr: Some("sender".into()),
                    to_addr: recipient.into(),
                    kind: "note".into(),
                    attention: Attention::Background,
                    body: "x".repeat(proto::MAX_JSONL_FRAME_BYTES + 1),
                    sent_at_ms: now_ms(),
                    ..Default::default()
                })
                .await
                .unwrap();
        }
        let old_following = backend
            .insert_message(&NewMessage {
                from_addr: Some("sender".into()),
                to_addr: "old-recipient".into(),
                kind: "note".into(),
                attention: Attention::Background,
                body: "following".into(),
                sent_at_ms: now_ms(),
                ..Default::default()
            })
            .await
            .unwrap()
            .id;
        let old = daemon
            .request_without_delivery_quarantine(wait_req(
                &store,
                "old-session",
                "old-recipient",
                1_000,
            ))
            .await;
        assert!(matches!(
            old,
            Response::Error { ref code, .. } if code == proto::ERROR_INCOMPATIBLE
        ));
        assert!(matches!(
            daemon
                .request_without_delivery_quarantine(wait_req(
                    &store,
                    "old-session",
                    "old-recipient",
                    1_000,
                ))
                .await,
            Response::Message { id, .. } if id == old_following
        ));
        let new = daemon
            .wait(&store, "new-session", "new-recipient", 1_000)
            .await;
        assert!(matches!(
            new,
            Response::DeliveryQuarantined {
                ref recipient,
                may_continue: true,
                ..
            } if recipient == "new-recipient"
        ));
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
    async fn oversized_notification_only_cc_is_skipped_without_workflow_disposition() {
        let state = test_state("oversized-cc-notification");
        let store = store_key("oversized-cc-notification");
        registered_epoch(state.clone(), &store, "s1", "addr:a").await;
        let backend = state.backend_for(&store).await.unwrap();
        let waiter_state = state.clone();
        let waiter_store = store.clone();
        let waiter = tokio::spawn(async move {
            request(
                waiter_state,
                Request::Wait {
                    store_key: waiter_store,
                    session_id: "s1".into(),
                    address: "addr:a".into(),
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
        let oversized_id = backend
            .insert_message(&NewMessage {
                from_addr: Some("sender".into()),
                to_addr: "primary".into(),
                cc: Some("addr:a".into()),
                kind: "note".into(),
                attention: Attention::Background,
                body: "x".repeat(proto::MAX_JSONL_FRAME_BYTES + 1),
                sent_at_ms: now_ms(),
                ..Default::default()
            })
            .await
            .unwrap()
            .id;
        let following_id = backend
            .insert_message(&NewMessage {
                from_addr: Some("sender".into()),
                to_addr: "primary".into(),
                cc: Some("addr:a".into()),
                kind: "note".into(),
                attention: Attention::Background,
                body: "following".into(),
                sent_at_ms: now_ms(),
                ..Default::default()
            })
            .await
            .unwrap()
            .id;

        let wait = waiter.await.unwrap();
        match wait {
            Response::Message {
                id, delivery_role, ..
            } => {
                assert_eq!(id, following_id);
                assert_eq!(delivery_role, "cc");
            }
            other => panic!("expected following CC notification, got {other:?}"),
        }
        assert!(backend
            .dispositions_for(oversized_id)
            .await
            .unwrap()
            .is_empty());
        assert!(state
            .status()
            .await
            .recent_errors
            .iter()
            .any(|error| error.kind == "OversizedCcNotificationFrame"));
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
            (WaiterOutcome::DeliveryQuarantined, "delivery-quarantined"),
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
        assert!(status.membership_losses.iter().any(|loss| {
            loss.store_key == store
                && loss.session_id == "s1"
                && loss.address == "addr:a"
                && loss.reason == NeedsAttachReason::PredicateDeath
        }));
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
            true,
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

    /// A live accept loop on a [`TestDaemon`]'s **real** IPC endpoint.
    ///
    /// `TestDaemon` normally short-circuits the transport and calls `handle_request` directly, which
    /// is right for behavioral tests and wrong for the one property that is *about* the transport:
    /// the published four-second `ReconcileIntents` bound is end-to-end, so the only way to check it
    /// is to make a real client, over a real socket, against the real `handle_client` path — hello
    /// frame, peer verification, request, response — and time it. Aborted on drop, so a test that
    /// fails still tears its endpoint down.
    pub struct TestIpcServer {
        task: tokio::task::JoinHandle<()>,
    }

    impl Drop for TestIpcServer {
        fn drop(&mut self) {
            self.task.abort();
        }
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

    /// A `Send`-able handle onto a running `TestDaemon`, for tests that need a second concurrent
    /// request (e.g. holding a live pull waiter open while a reconcile pass runs).
    #[derive(Clone)]
    pub struct TestDaemonHandle {
        state: Arc<DaemonState>,
    }

    impl TestDaemonHandle {
        pub async fn request(&self, request: Request) -> Response {
            handle_request(self.state.clone(), request).await.0
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
                delivery_admissions: Mutex::new(HashMap::new()),
                #[cfg(test)]
                delivery_admission_control: Mutex::new(None),
                next_waiter_id: AtomicU64::new(1),
                recent_errors: Arc::new(Mutex::new(VecDeque::new())),
                ended_sessions: Mutex::new(BTreeMap::new()),
                draining: AtomicBool::new(false),
                on_deliver: OnDeliverState::default(),
                intents: reconcile::IntentRuntime::default(),
            });
            Self { state, root }
        }

        /// A `TestDaemon` that has exercised the **real** startup path: the run dir is created and
        /// owner-private-checked, the intent scope is opened, and the startup scan (GC + first
        /// reconcile pass) has completed.
        ///
        /// The plain constructor deliberately skips this so existing tests stay fast and hermetic;
        /// startup reconciliation needs the real path, so it gets its own constructor rather than a
        /// flag that silently changes what every other test exercises.
        pub async fn with_startup_scan(label: &str) -> Self {
            let daemon = Self::new(label);
            std::fs::create_dir_all(daemon.state.paths.run_dir.clone())
                .expect("create test run dir");
            let mut reports = daemon.state.reconcile_reports();
            let before = reports.borrow_and_update().pass_seq;
            reconcile::spawn_startup_scan(daemon.state.clone());
            let _ = reconcile::await_next_report(reports, before, Duration::from_secs(30)).await;
            daemon
        }

        /// Open (creating if needed) this daemon's station-intent scope, so a test can seed intents
        /// exactly where the daemon will look for them.
        pub fn intent_store(&self) -> crate::station_intent::IntentStore {
            std::fs::create_dir_all(&self.state.paths.run_dir).expect("create test run dir");
            crate::station_intent::IntentStore::open(
                &self.state.paths.run_dir,
                &self.state.paths.singleton_hash,
            )
            .expect("open test intent scope")
        }

        pub fn singleton_hash(&self) -> &str {
            &self.state.paths.singleton_hash
        }

        /// Drive exactly one reconciliation pass and return its report. Deterministic: no wall-clock
        /// sleep, no polling.
        pub async fn reconcile_once(&self) -> crate::daemon_ipc::ReconcileReport {
            reconcile::reconcile_once(self.state.clone(), None).await
        }

        /// Drive one pass against a **caller-supplied absolute deadline**, exactly as the admin
        /// request handler does.
        ///
        /// This is the seam a deadline-edge test needs: passing an instant that has already elapsed
        /// puts every phase on the far side of its budget deterministically, with no sleep, no
        /// filesystem fault injection, and no dependence on how fast the runner is.
        pub async fn reconcile_once_until(
            &self,
            deadline: std::time::Instant,
        ) -> crate::daemon_ipc::ReconcileReport {
            reconcile::reconcile_once_until(
                self.state.clone(),
                None,
                deadline,
                reconcile::PassOrigin::Request,
            )
            .await
        }

        /// The bytes of this scope's reconcile event log, or `None` when nothing has been appended.
        pub fn reconcile_event_log_bytes(&self) -> Option<Vec<u8>> {
            std::fs::read(
                self.intent_store()
                    .root()
                    .join(reconcile::RECONCILE_EVENT_LOG_FILE),
            )
            .ok()
        }

        /// Every `(store_key, session_id, address)` this daemon currently holds a member for.
        ///
        /// The membership table itself rather than a status projection: "no member was published
        /// after the response" has to be checked against what the daemon actually holds, not against
        /// a view that could filter one out.
        pub fn member_keys(&self) -> Vec<String> {
            self.state
                .members
                .lock()
                .unwrap()
                .keys()
                .map(|key| format!("{key:?}"))
                .collect()
        }

        /// Run the trigger half of the production `heartbeat_loop`: wake on a trigger pulse and
        /// drive a pass. Nothing else from that loop, so a test still controls every tick.
        ///
        /// This exists so the trigger seam can be asserted for real. `TestDaemon` does not run
        /// `serve()`, so before this the only consumer of `IntentRuntime::trigger` in a test
        /// process was nothing at all: a pulse-and-wait always timed out and fell back to calling
        /// `reconcile_once` directly, which means the seam could have been completely unwired
        /// with the "seam works" test still green (and taking the full timeout to say so).
        pub fn spawn_trigger_consumer(&self) -> tokio::task::JoinHandle<()> {
            let state = self.state.clone();
            tokio::spawn(async move {
                loop {
                    state.intents.trigger.notified().await;
                    reconcile::reconcile_once(state.clone(), None).await;
                }
            })
        }

        /// Pulse the reconcile trigger and await the report of the pass it drives.
        ///
        /// Requires a trigger consumer to be running — see [`TestDaemon::spawn_trigger_consumer`].
        /// Without one the pulse has nowhere to go and this necessarily times out, which is
        /// exactly how a "the seam is wired" test can pass while the seam is entirely unwired.
        pub async fn pulse_reconcile_and_wait(
            &self,
            timeout: Duration,
        ) -> Option<crate::daemon_ipc::ReconcileReport> {
            let mut reports = self.state.reconcile_reports();
            let before = reports.borrow_and_update().pass_seq;
            self.state.pulse_reconcile();
            reconcile::await_next_report(reports, before, timeout).await
        }

        pub fn reconcile_reports(
            &self,
        ) -> tokio::sync::watch::Receiver<crate::daemon_ipc::ReconcileReport> {
            self.state.reconcile_reports()
        }

        pub fn intent_index(&self) -> reconcile::IntentIndexSnapshot {
            self.state.intent_index_snapshot()
        }

        pub fn drain_intent_report(&self) -> crate::daemon_ipc::DrainIntentReport {
            self.state.drain_intent_report()
        }

        /// Model the daemon's in-memory advance of a push member's CC watermark, which normally
        /// happens as CC notifications are accepted.
        pub fn set_member_cc_after_ms(
            &self,
            store_key: &str,
            session_id: &str,
            address: &str,
            value: Option<i64>,
        ) {
            let key = DaemonState::member_key(store_key, session_id, address);
            let mut members = self.state.members.lock().unwrap();
            if let Some(member) = members.get_mut(&key) {
                member.on_deliver_cc_after_ms = value;
            }
        }

        pub fn member_cc_after_ms(
            &self,
            store_key: &str,
            session_id: &str,
            address: &str,
        ) -> Option<Option<i64>> {
            self.state
                .get_member(store_key, session_id, address)
                .map(|member| member.on_deliver_cc_after_ms)
        }

        /// Model a daemon replacement for the cached index only: the durable scope survives, the
        /// in-memory projection does not.
        pub fn clear_intent_index(&self) {
            let mut index = self.state.intents.index_for_test().lock().unwrap();
            index.entries.clear();
            index.as_of_ms = crate::model::now_ms();
        }

        pub fn set_draining_for_test(&self, draining: bool) {
            self.state.draining.store(draining, Ordering::SeqCst);
        }

        /// The intent-row projection the authenticated status surface exposes.
        pub fn intent_statuses(&self) -> Vec<crate::daemon_ipc::IntentStatus> {
            self.state.intent_statuses(None)
        }

        /// Reconcile one intent while *holding* its admission guard, exactly as the inline
        /// anti-downgrade path inside `register_member` does. A deadlock here is a real deadlock in
        /// the hottest register path, so this is exercised directly.
        pub async fn reconcile_intent_under_admission_guard(
            &self,
            intent: &crate::station_intent::StationIntentV1,
        ) -> String {
            let admission = self
                .state
                .delivery_admission(
                    &intent.store_key,
                    &intent.session_id,
                    &intent.address,
                    DeliveryAdmissionKind::Register,
                )
                .await;
            let _guard = admission.lock().await;
            let outcome = reconcile::reconcile_intent_locked(self.state.clone(), intent).await;
            format!("{outcome:?}")
        }

        /// Hold a binding's delivery-admission guard, exactly as a concurrent register, detach, or
        /// reset does while it works.
        ///
        /// Returned owned so a test can keep it across awaits and observe what a *contended* guard
        /// does to a bounded operation. Contention here is not exotic: the guard is the outermost
        /// lock on the two writers of a station's membership, so every published bound on the
        /// reconcile path has to hold while it is held by someone else.
        pub async fn hold_delivery_admission(
            &self,
            store_key: &str,
            session_id: &str,
            address: &str,
        ) -> tokio::sync::OwnedMutexGuard<()> {
            self.state
                .delivery_admission(
                    store_key,
                    session_id,
                    address,
                    DeliveryAdmissionKind::Register,
                )
                .await
                .lock_owned()
                .await
        }

        /// Withdraw one binding with an explicit admission budget, as `apply_outcome` does with
        /// whatever is left of the pass deadline.
        pub async fn withdraw_within(
            &self,
            store_key: &str,
            session_id: &str,
            address: &str,
            admission_budget: Duration,
        ) -> std::result::Result<String, String> {
            self.state
                .withdraw_intent_at_generation_within(
                    store_key,
                    session_id,
                    address,
                    None,
                    admission_budget,
                )
                .await
                .map(|outcome| format!("{outcome:?}"))
        }

        /// Publish an armed push member for a binding **without** taking its admission guard, as a
        /// reconcile pass does at the end of a restore it is already admitted for.
        ///
        /// This is what makes the reverse-order race deterministic. A test holds the binding's
        /// admission guard, starts a teardown (which must now block on that guard), and then calls
        /// this — modelling the pass that publishes a member *after* the teardown began but
        /// *before* it could act. A teardown that reads membership outside the guard leaves this
        /// member installed; one that re-reads under the guard does not.
        ///
        /// It goes through the real backend (`ensure_address` + `claim_epoch_lease`) rather than
        /// only inserting a record, so the published member has a genuine owner and epoch and a
        /// detach's `release_epoch_lease_for_detach` sees an owner it can actually release.
        pub async fn publish_push_member_unadmitted(
            &self,
            store_key: &str,
            session_id: &str,
            address: &str,
        ) {
            let backend = match self.state.backend_for(store_key).await {
                Ok(backend) => backend,
                Err(response) => panic!("test backend for {store_key}: {response:?}"),
            };
            backend
                .ensure_address(address, None, None, None)
                .await
                .expect("ensure address for published push member");
            let claimed = match backend
                .claim_epoch_lease(address, &self.state.instance_id, liveness_window_secs())
                .await
                .expect("claim epoch lease for published push member")
            {
                EpochClaimResult::Claimed(claimed) => claimed,
                other => panic!("expected a fresh epoch claim, got {other:?}"),
            };
            self.state.insert_member(MemberRecord {
                address: address.to_string(),
                capability: StationCapability::default(),
                store_key: store_key.to_string(),
                backend: backend.kind().to_string(),
                session_id: session_id.to_string(),
                application_responsibility: None,
                occupant: "reconciler".to_string(),
                host: crate::config::hostname(),
                waiters: 0,
                watch_pids: Vec::new(),
                description: None,
                scope: None,
                tags: None,
                lease_epoch: claimed.lease_epoch,
                owner_instance_id: claimed.owner_instance_id,
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
                on_deliver: Some(vec!["cmd".to_string(), "--armed".to_string()]),
                on_deliver_wake_on_cc: false,
                on_deliver_cc_after_ms: None,
            });
        }

        /// Whether an **active** (non-idle) push-armed member is installed for this binding.
        ///
        /// Idleness matters: `reset` and session end mark members idle rather than removing them,
        /// and a member's `on_deliver` survives that. The question a teardown race asks is whether
        /// live push coverage remains, which is "armed *and* not idle".
        pub fn has_active_push_member(
            &self,
            store_key: &str,
            session_id: &str,
            address: &str,
        ) -> bool {
            self.state
                .get_member(store_key, session_id, address)
                .is_some_and(|member| !member.idle && member.on_deliver.is_some())
        }

        /// Whether any member record at all is installed for this binding.
        pub fn has_member(&self, store_key: &str, session_id: &str, address: &str) -> bool {
            self.state
                .get_member(store_key, session_id, address)
                .is_some()
        }

        /// The daemon's run/state paths.
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
            let hello = proto::client_hello("test");
            handle_request_with_capabilities(
                self.state.clone(),
                request,
                peer_supports_delivery_quarantine(&hello),
            )
            .await
            .0
        }

        pub async fn request_without_delivery_quarantine(&self, request: Request) -> Response {
            let mut hello = proto::client_hello("test");
            hello
                .capabilities
                .retain(|capability| capability != proto::CAP_DELIVERY_QUARANTINE_V1);
            handle_request_with_capabilities(
                self.state.clone(),
                request,
                peer_supports_delivery_quarantine(&hello),
            )
            .await
            .0
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
                true,
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

        /// Drop a member from the in-memory table **without** tombstoning it, modelling exactly
        /// what a daemon replacement leaves behind: the durable intent survives, the member does
        /// not, and no explicit detach was ever performed.
        pub fn forget_member(&self, store_key: &str, session_id: &str, address: &str) {
            self.state.remove_member(store_key, session_id, address);
        }

        /// The opened backend for a store, so a test can assert on durable state (tombstones,
        /// leases) that the daemon's own response does not expose.
        pub async fn open_backend(&self, store_key: &str) -> Arc<dyn Backend> {
            self.state
                .backend_for(store_key)
                .await
                .expect("open test backend")
        }

        /// A cheap handle a spawned task can use to issue requests against this daemon, so a test
        /// can hold a live `Wait` open while driving another operation.
        pub fn handle(&self) -> TestDaemonHandle {
            TestDaemonHandle {
                state: self.state.clone(),
            }
        }

        /// Start serving this daemon's real IPC endpoint. See [`TestIpcServer`].
        pub fn serve_ipc(&self) -> TestIpcServer {
            std::fs::create_dir_all(&self.state.paths.run_dir).expect("create test run dir");
            let mut listener = platform::Listener::bind(&self.state.paths.endpoint)
                .expect("bind the test daemon endpoint");
            let state = self.state.clone();
            let task = tokio::spawn(async move {
                loop {
                    let Ok(conn) = listener.accept().await else {
                        return;
                    };
                    if listener.ready_for_next().is_err() {
                        return;
                    }
                    let state = state.clone();
                    tokio::spawn(async move {
                        let _ = handle_client(conn, state).await;
                    });
                }
            });
            TestIpcServer { task }
        }

        /// Connect a real client to this daemon's endpoint through the production handshake.
        ///
        /// Requires [`TestDaemon::serve_ipc`] to be running.
        pub async fn connect_ipc(&self, store_key: &str) -> DaemonClient {
            let conn = platform::connect(&self.state.paths.endpoint)
                .await
                .expect("connect to the test daemon endpoint");
            handshake_connected(conn, self.state.paths.clone(), store_key)
                .await
                .expect("handshake with the test daemon")
        }

        /// The bytes of this scope's round-robin cursor file, or `None` when no pass has written
        /// one yet.
        ///
        /// Byte-exact rather than parsed on purpose: a test asserting "nothing was published after
        /// the response" has to fail on *any* difference, including one a future field would hide
        /// from a structural comparison.
        pub fn scan_cursor_bytes(&self) -> Option<Vec<u8>> {
            std::fs::read(
                self.intent_store()
                    .root()
                    .join(crate::station_intent::SCAN_CURSOR_FILE),
            )
            .ok()
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
            replace_watch_pids: false,
            recovery: false,
            on_deliver: None,
            replace_on_deliver: false,
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
    // Exactly the two extension traits this module still calls through: `FileTypeExt` for
    // `is_socket()` on the stale-endpoint check, and `OpenOptionsExt` for `.mode(0o600)` on the
    // endpoint lock. The mode/uid/permission work that needed `DirBuilderExt`, `MetadataExt`, and
    // `PermissionsExt` moved to `platform_fs::imp` when the owner-private primitives were promoted,
    // so importing them here is an unused import — and CI builds with `-D warnings`.
    use std::os::unix::fs::{FileTypeExt, OpenOptionsExt};
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
            let Endpoint::UnixSocket(path) = endpoint;
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
            .truncate(false)
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
        let Endpoint::UnixSocket(path) = endpoint;
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

    // Owner-private filesystem and process-identity primitives live in `crate::platform_fs` so the
    // daemon and the station-intent store share one hardened implementation (ADR 0052). These
    // wrappers only adapt the shared error type; the daemon's cap-file, socket, and
    // peer-verification behavior is byte-for-byte unchanged.
    pub fn ensure_owner_private_dir(path: &Path) -> Result<PathBuf> {
        crate::platform_fs::ensure_owner_private_dir(path).map_err(Into::into)
    }

    pub fn write_owner_only_file(path: &Path, bytes: &[u8]) -> Result<()> {
        crate::platform_fs::write_owner_only_file(path, bytes).map_err(Into::into)
    }

    pub fn process_exe_path(pid: u32) -> Result<PathBuf> {
        crate::platform_fs::process_exe_path(pid).map_err(Into::into)
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
        let pid = u32::try_from(pid).map_err(|_| {
            DaemonError::Unauthorized(format!("server pid {pid} cannot be represented as u32"))
        })?;
        let current = unsafe { libc::geteuid() };
        if uid != current {
            return Err(DaemonError::Unauthorized(format!(
                "server uid {uid} does not match client uid {current}"
            )));
        }
        let exe = server_executable(pid)?;
        if !same_canonical_path(&exe, expected_exe) {
            return Err(DaemonError::Unauthorized(format!(
                "server executable {} does not match {}",
                exe.display(),
                expected_exe.display()
            )));
        }
        let start_time = server_process_start_time(pid)?;
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

    #[cfg(target_os = "macos")]
    fn peer_pid_uid(conn: &UnixStream) -> Result<(libc::pid_t, libc::uid_t)> {
        let mut pid: libc::pid_t = 0;
        let mut pid_len = std::mem::size_of::<libc::pid_t>() as libc::socklen_t;
        let pid_rc = unsafe {
            libc::getsockopt(
                conn.as_raw_fd(),
                libc::SOL_LOCAL,
                libc::LOCAL_PEERPID,
                &mut pid as *mut _ as *mut libc::c_void,
                &mut pid_len,
            )
        };
        if pid_rc != 0 {
            return Err(io_err(
                "reading unix peer pid",
                std::io::Error::last_os_error(),
            ));
        }

        let mut uid: libc::uid_t = 0;
        let mut gid: libc::gid_t = 0;
        let uid_rc = unsafe { libc::getpeereid(conn.as_raw_fd(), &mut uid, &mut gid) };
        if uid_rc != 0 {
            return Err(io_err(
                "reading unix peer credentials",
                std::io::Error::last_os_error(),
            ));
        }
        Ok((pid, uid))
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    fn peer_pid_uid(_conn: &UnixStream) -> Result<(libc::pid_t, libc::uid_t)> {
        Err(DaemonError::Unsupported {
            capability: "unix peer credential verification",
            message: "peer credential verification is only wired for Linux and macOS".into(),
        })
    }

    #[cfg(target_os = "linux")]
    fn server_executable(pid: u32) -> Result<PathBuf> {
        process_exe_path(pid).map_err(|e| DaemonError::Unsupported {
            capability: "client-side server executable verification",
            message: e.to_string(),
        })
    }

    #[cfg(target_os = "macos")]
    fn server_executable(pid: u32) -> Result<PathBuf> {
        process_exe_path(pid).map_err(|e| DaemonError::Unsupported {
            capability: "client-side server executable verification",
            message: e.to_string(),
        })
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    fn server_executable(_pid: u32) -> Result<PathBuf> {
        Err(DaemonError::Unsupported {
            capability: "client-side server executable verification",
            message: "server executable verification is only wired for Linux and macOS".into(),
        })
    }

    #[cfg(target_os = "linux")]
    fn server_process_start_time(pid: u32) -> Result<u64> {
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

    #[cfg(target_os = "macos")]
    fn server_process_start_time(pid: u32) -> Result<u64> {
        crate::session_watch::capture_process_start_time(pid).ok_or_else(|| {
            DaemonError::Unsupported {
                capability: "client-side server start-time verification",
                message: format!("cannot capture process start time for pid {pid}"),
            }
        })
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    fn server_process_start_time(_pid: u32) -> Result<u64> {
        Err(DaemonError::Unsupported {
            capability: "client-side server start-time verification",
            message: "process start-time verification is only wired for Linux and macOS".into(),
        })
    }
}

#[cfg(windows)]
mod platform {
    use super::*;
    use std::ffi::{c_void, OsStr};
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::io::AsRawHandle;
    use tokio::net::windows::named_pipe::{ClientOptions, NamedPipeClient, NamedPipeServer};
    use windows_sys::Win32::Foundation::{
        CloseHandle, LocalFree, ERROR_ACCESS_DENIED, ERROR_ALREADY_EXISTS, ERROR_PIPE_BUSY,
        FILETIME, HANDLE, INVALID_HANDLE_VALUE,
    };
    use windows_sys::Win32::Security::Authorization::{
        ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
        SDDL_REVISION_1,
    };
    use windows_sys::Win32::Security::{
        GetTokenInformation, TokenUser, SECURITY_ATTRIBUTES, TOKEN_INFORMATION_CLASS, TOKEN_QUERY,
        TOKEN_USER,
    };
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_FLAG_OVERLAPPED,
        FILE_GENERIC_WRITE, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING, PIPE_ACCESS_DUPLEX,
    };
    use windows_sys::Win32::System::Pipes::{
        CreateNamedPipeW, GetNamedPipeClientProcessId, GetNamedPipeServerProcessId,
        PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_WAIT,
    };
    use windows_sys::Win32::System::Threading::{
        GetCurrentProcess, GetProcessTimes, OpenProcess, OpenProcessToken,
        PROCESS_QUERY_LIMITED_INFORMATION,
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
        let Endpoint::WindowsPipe(pipe_name) = endpoint;
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

    // Owner-private filesystem and process-identity primitives live in `crate::platform_fs` so the
    // daemon and the station-intent store share one hardened implementation (ADR 0052). These
    // wrappers only adapt the shared error type; the daemon's cap-file, pipe, and
    // peer-verification behavior is byte-for-byte unchanged.
    pub fn ensure_owner_private_dir(path: &Path) -> Result<PathBuf> {
        crate::platform_fs::ensure_owner_private_dir(path).map_err(Into::into)
    }

    pub fn write_owner_only_file(path: &Path, bytes: &[u8]) -> Result<()> {
        crate::platform_fs::write_owner_only_file(path, bytes).map_err(Into::into)
    }

    pub fn process_exe_path(pid: u32) -> Result<PathBuf> {
        crate::platform_fs::process_exe_path(pid).map_err(Into::into)
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
        let sa = owner_only_security_attributes()?;
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
                &sa.attrs,
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
            return Err(DaemonError::Unauthorized(
                "peer SID does not match current user SID".to_string(),
            ));
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
        let token = process_token(process.0, format!("opening peer process token for {pid}"))?;
        let sid = sid_string_from_token(token.0)?;
        let start_time_100ns = process_start_time(process.0)?;
        let exe = if expected_exe.is_some() {
            // One implementation of executable resolution, shared with the intent producer-identity
            // path (`platform_fs::process_exe_path`), so the two can never disagree about what
            // "this pid's executable" means.
            Some(process_exe_path(pid).map_err(|e| DaemonError::Unsupported {
                capability: "peer process executable verification",
                message: e.to_string(),
            })?)
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

    fn current_process_token() -> Result<Handle> {
        process_token(
            unsafe { GetCurrentProcess() },
            "opening current process token".to_string(),
        )
    }

    fn process_token(process: HANDLE, access_denied_context: String) -> Result<Handle> {
        let mut token = 0isize;
        let ok = unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut token) };
        if ok == 0 {
            let err = std::io::Error::last_os_error();
            if is_access_denied(&err) {
                return Err(access_denied_elevation_error(access_denied_context, err));
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

    fn sid_string_from_token(token: HANDLE) -> Result<String> {
        let buf = token_information(token, TokenUser, "reading token user information")?;
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

    /// `GetTokenInformation` with storage aligned for every token structure read from it.
    fn token_information(
        token: HANDLE,
        class: TOKEN_INFORMATION_CLASS,
        action: &'static str,
    ) -> Result<Vec<u64>> {
        let mut needed = 0u32;
        unsafe {
            GetTokenInformation(token, class, std::ptr::null_mut(), 0, &mut needed);
        }
        if needed == 0 {
            return Err(io_err(action, std::io::Error::last_os_error()));
        }
        // A byte vector does not guarantee the alignment required to dereference TOKEN_USER.
        let mut buf = vec![0u64; needed as usize / std::mem::size_of::<u64>() + 1];
        let ok = unsafe {
            GetTokenInformation(
                token,
                class,
                buf.as_mut_ptr() as *mut c_void,
                needed,
                &mut needed,
            )
        };
        if ok == 0 {
            return Err(io_err(action, std::io::Error::last_os_error()));
        }
        Ok(buf)
    }

    #[cfg(test)]
    pub(super) fn token_user_information_is_aligned() -> Result<bool> {
        let token = current_process_token()?;
        let buf = token_information(token.0, TokenUser, "reading token user information")?;
        Ok((buf.as_ptr() as usize) % std::mem::align_of::<TOKEN_USER>() == 0)
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

/// One authenticated line exchange with a **local** endpoint whose owner is already known.
///
/// This is the client half of the daemon's peer rule, factored out of the reconciler's producer
/// probe so that every path that hands a secret to a local endpoint gets the same order of
/// operations rather than a re-implementation of it:
///
/// 1. connect,
/// 2. `platform::verify_server_peer` — same user, the recorded executable, and the recorded
///    `(pid, start_time)` — **before a single byte is written**,
/// 3. write exactly one request line,
/// 4. read exactly one response line, hard-capped at a frame of `max_response_bytes` (newline
///    included) — one byte more is refused rather than returned truncated.
///
/// The order is the property. A request that carries a credential is unrecoverable once written:
/// endpoint names are predictable (they are derived from a session id), so anything that can bind
/// the name first receives whatever the client sends before it ever inspects an answer. Proving
/// the peer *after* the write, or inferring identity from the reply, protects nothing. A platform
/// that cannot resolve the peer fails closed for the same reason.
///
/// Visibility is deliberately `pub(crate)`: this exposes the daemon's private `platform` peer
/// primitives to the rest of the crate and nothing wider.
pub(crate) mod verified_peer {
    use super::*;
    use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt};

    /// Windows named-pipe busy retry interval while a prior client holds the instance.
    #[cfg(windows)]
    const PIPE_BUSY_RETRY: Duration = Duration::from_millis(50);

    /// The identity the endpoint's server process must prove before anything is sent to it.
    #[derive(Debug, Clone, Copy)]
    pub(crate) struct ExpectedPeer<'a> {
        pub exe_path: &'a Path,
        pub pid: u32,
        pub start_time: u64,
    }

    /// What the exchange does, and the ceilings it is held to.
    #[derive(Debug, Clone, Copy)]
    pub(crate) struct LineExchange<'a> {
        /// The request, without a trailing newline — the exchange frames it.
        pub request_line: &'a str,
        /// Hard ceiling on the answer, **newline included**: a frame of exactly this many bytes is
        /// answered, one byte more is refused. The peer is authenticated but never *trusted*: an
        /// unbounded read lets a buggy or hostile server stream until the timeout and hand an
        /// arbitrarily large string to the caller.
        pub max_response_bytes: u64,
        pub connect_timeout: Duration,
        pub exchange_timeout: Duration,
    }

    /// Why an exchange did not produce an answer. Split finely enough that every caller can keep
    /// the failure classification it already published.
    #[derive(Debug)]
    pub(crate) enum ExchangeError {
        /// The endpoint could not be reached at all.
        Connect(DaemonError),
        /// Connecting did not complete inside `connect_timeout`.
        ConnectTimeout,
        /// The peer is not the expected producer — **nothing was sent**.
        PeerUnverified(DaemonError),
        /// Transport failure during the exchange itself.
        Io(std::io::Error),
        /// The exchange did not complete inside `exchange_timeout`.
        ExchangeTimeout,
        /// The answer's frame exceeded `max_response_bytes`.
        ResponseTooLarge,
    }

    impl std::fmt::Display for ExchangeError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            match self {
                ExchangeError::Connect(e) => write!(f, "connecting to the endpoint: {e}"),
                ExchangeError::ConnectTimeout => write!(f, "the endpoint did not accept in time"),
                ExchangeError::PeerUnverified(e) => write!(
                    f,
                    "the process serving the endpoint is not the expected peer, so nothing was sent: {e}"
                ),
                ExchangeError::Io(e) => write!(f, "the endpoint exchange failed: {e}"),
                ExchangeError::ExchangeTimeout => write!(f, "the endpoint did not answer in time"),
                ExchangeError::ResponseTooLarge => {
                    write!(f, "the endpoint's answer exceeded the response cap")
                }
            }
        }
    }

    /// A local endpoint at `path`, in the transport this platform uses for one.
    pub(crate) fn local_endpoint(path: &str) -> Endpoint {
        #[cfg(windows)]
        {
            Endpoint::WindowsPipe(path.to_string())
        }
        #[cfg(unix)]
        {
            Endpoint::UnixSocket(PathBuf::from(path))
        }
        #[cfg(not(any(unix, windows)))]
        {
            let _ = path;
            unreachable!("no local endpoint transport on this platform")
        }
    }

    /// Connect, retrying only the "another client holds the single pipe instance" condition, which
    /// is transient by construction. Every other error returns immediately, so an absent endpoint
    /// is still reported as absent rather than waited on.
    async fn connect_when_free(endpoint: &Endpoint) -> Result<platform::ClientConn> {
        #[cfg(windows)]
        {
            loop {
                match platform::connect(endpoint).await {
                    Ok(conn) => return Ok(conn),
                    Err(DaemonError::Timeout(_)) => {
                        tokio::time::sleep(PIPE_BUSY_RETRY).await;
                    }
                    Err(e) => return Err(e),
                }
            }
        }
        #[cfg(not(windows))]
        {
            platform::connect(endpoint).await
        }
    }

    /// Connect to `endpoint`, prove the peer, then exchange one line. See the module docs for why
    /// the order is the whole of the guarantee.
    pub(crate) async fn exchange(
        endpoint: &Endpoint,
        peer: ExpectedPeer<'_>,
        exchange: LineExchange<'_>,
    ) -> std::result::Result<String, ExchangeError> {
        let conn = match tokio::time::timeout(exchange.connect_timeout, connect_when_free(endpoint))
            .await
        {
            Ok(Ok(conn)) => conn,
            Ok(Err(e)) => return Err(ExchangeError::Connect(e)),
            Err(_) => return Err(ExchangeError::ConnectTimeout),
        };
        // Same-user ownership, executable match, and pid+start-time identity, all in one call,
        // before a single byte leaves this process.
        if let Err(e) = platform::verify_server_peer(
            &conn,
            peer.exe_path,
            Some(peer.pid),
            Some(peer.start_time),
        ) {
            return Err(ExchangeError::PeerUnverified(e));
        }

        let (read_half, mut write_half) = tokio::io::split(conn);
        // Read one byte *past* the cap rather than exactly up to it. `max_response_bytes` is the
        // largest frame that is allowed through, newline included, so a reader limited to exactly
        // the cap cannot tell "a legal frame that happens to be cap bytes" from "a frame that was
        // cut off at the cap": both come back cap bytes long, and the only way to stay safe is to
        // reject the legal one too. The extra byte makes the two distinguishable — an over-cap
        // answer is the only thing that can produce more than `max_response_bytes` — so the
        // boundary is exact: a `cap`-byte frame is answered, a `cap + 1`-byte one is refused, and
        // nothing larger is ever buffered.
        let read_budget = exchange.max_response_bytes.saturating_add(1);
        let mut reader = BufReader::new(read_half.take(read_budget));
        let request = exchange.request_line;
        let io = async {
            write_half.write_all(request.as_bytes()).await?;
            write_half.write_all(b"\n").await?;
            write_half.flush().await?;
            let mut response = String::new();
            reader.read_line(&mut response).await?;
            Ok::<String, std::io::Error>(response)
        };
        let response = match tokio::time::timeout(exchange.exchange_timeout, io).await {
            Ok(Ok(response)) => response,
            Ok(Err(e)) => return Err(ExchangeError::Io(e)),
            Err(_) => return Err(ExchangeError::ExchangeTimeout),
        };
        if response.len() as u64 > exchange.max_response_bytes {
            return Err(ExchangeError::ResponseTooLarge);
        }
        Ok(response)
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
    use crate::ipc::test_support::short_runtime_root;

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
        let run_dir = short_runtime_root();
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

    #[cfg(windows)]
    #[test]
    fn windows_peer_token_information_is_pointer_aligned() {
        assert!(
            platform::token_user_information_is_aligned().expect("read current token"),
            "TOKEN_USER must never be dereferenced through byte-aligned storage"
        );
    }

    #[tokio::test]
    async fn endpoint_bind_exclusivity_rejects_second_listener() {
        let run_dir = short_runtime_root();
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
        let run_dir = short_runtime_root();
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
