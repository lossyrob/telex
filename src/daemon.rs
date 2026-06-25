//! Hidden daemon singleton foundation: singleton identity, endpoint naming, capability
//! file handling, connect-or-spawn, and a P2 JSONL server loop.

#[cfg(feature = "sqlite")]
use crate::backend::sqlite::SqliteBackend;
use crate::backend::Backend;
use crate::daemon_ipc::{
    self as proto, current_protocol_version, read_json_line, write_json_line, DaemonStatus,
    EpochStatus, HandshakeError, HelloAck, IdleStationStatus, LiveWaiterStatus, MemberStatus,
    NeedsAttachReason, RecentErrorStatus, Request, Response, RetentionStatus, SentReceipt,
    StoreStatus, WatchPidRole, WatchPidSpec, WatchPidStatus,
};
use crate::model::{
    now_ms, Attention, DeliveryOutcome, EpochClaimResult, NewMessage, STATUS_RETIRED,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::fmt;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tokio::io::BufReader;
use tokio::sync::Mutex as AsyncMutex;

pub const READINESS_TIMEOUT: Duration = Duration::from_secs(5);
pub const CONNECT_ATTEMPT_TIMEOUT: Duration = Duration::from_millis(500);
pub const BACKOFF_INITIAL: Duration = Duration::from_millis(50);
pub const BACKOFF_MAX: Duration = Duration::from_millis(500);
pub const CRASHLOOP_MAX: usize = 3;
pub const CRASHLOOP_WINDOW: Duration = Duration::from_secs(10);
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(5);
const RECENT_ERROR_LIMIT: usize = 32;
const DEFAULT_IDLE_TTL: Duration = Duration::from_secs(24 * 60 * 60);
const DEFAULT_RETENTION_WARN_ROWS: i64 = 100_000;
const DEFAULT_IDLE_STATION_WARN: usize = 1_000;

pub type Result<T> = std::result::Result<T, DaemonError>;

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
            HandshakeError::Eof => {
                DaemonError::Protocol("daemon closed the connection".to_string())
            }
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
    recent_errors: Mutex<VecDeque<RecentErrorStatus>>,
    ended_sessions: Mutex<BTreeMap<SessionKey, EndedSessionRecord>>,
    draining: AtomicBool,
}

#[derive(Clone)]
struct StoreEntry {
    kind: String,
    backend: Arc<dyn Backend>,
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
    store_key: String,
    session_id: String,
    address: String,
    pid: u32,
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
}

#[derive(Clone, Debug)]
struct WatchPidRecord {
    pid: u32,
    start_time: Option<u64>,
    role: WatchPidRole,
}

#[derive(Clone, Debug)]
struct WaiterRecord {
    store_key: String,
    session_id: String,
    address: String,
    pid: u32,
    start_time: Option<u64>,
    started_at_ms: i64,
    attention: Option<String>,
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
        self.status_with_thresholds(retention_warn_threshold(), idle_station_warn_threshold())
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
        }
    }

    async fn status_with_thresholds(
        &self,
        retention_warn_threshold: i64,
        idle_station_warn_threshold: usize,
    ) -> DaemonStatus {
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

        let member_records: Vec<MemberRecord> =
            self.members.lock().unwrap().values().cloned().collect();
        let live_waiters = self.live_waiter_statuses();
        let members: Vec<MemberStatus> = member_records
            .iter()
            .map(|member| member.status(&live_waiters))
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

        let backend = open_store_backend(store_key).await?;
        self.stores.lock().unwrap().insert(
            store_key.to_string(),
            StoreEntry {
                kind: backend.kind().to_string(),
                backend: backend.clone(),
            },
        );
        Ok(backend)
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

    fn waiter_key(store_key: &str, session_id: &str, address: &str, pid: u32) -> WaiterKey {
        WaiterKey {
            store_key: store_key.to_string(),
            session_id: session_id.to_string(),
            address: address.to_string(),
            pid,
        }
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

    fn has_address_member(&self, store_key: &str, address: &str) -> bool {
        self.members
            .lock()
            .unwrap()
            .values()
            .any(|m| m.store_key == store_key && m.address == address && !m.idle)
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
            return Some(member.clone());
        }
        if !member.idle_rearmable {
            return None;
        }
        member.idle = false;
        member.idle_rearmable = false;
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
        let mut members = self.members.lock().unwrap();
        let should_remove = members.get(&key).is_some_and(|current| {
            current.lease_epoch == record.lease_epoch
                && current.owner_instance_id == record.owner_instance_id
        });
        if should_remove {
            members.remove(&key);
        }
        should_remove
    }

    fn members_snapshot(&self) -> Vec<MemberRecord> {
        self.members.lock().unwrap().values().cloned().collect()
    }

    fn clear_members(&self) {
        self.members.lock().unwrap().clear();
        self.waiters.lock().unwrap().clear();
    }

    fn push_recent_error(&self, kind: impl Into<String>, message: impl Into<String>) {
        let mut errors = self.recent_errors.lock().unwrap();
        let message = proto::redact_secrets(message.into(), &[&self.admin_cap]);
        errors.push_back(RecentErrorStatus {
            at_ms: now_ms(),
            kind: kind.into(),
            message,
        });
        while errors.len() > RECENT_ERROR_LIMIT {
            errors.pop_front();
        }
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

    fn add_waiter(&self, waiter: WaiterRecord) {
        let store_key = waiter.store_key.clone();
        let session_id = waiter.session_id.clone();
        let address = waiter.address.clone();
        if waiter.pid != 0 {
            self.waiters.lock().unwrap().insert(
                Self::waiter_key(&store_key, &session_id, &address, waiter.pid),
                waiter,
            );
        }
        if let Some(member) = self.members.lock().unwrap().get_mut(&Self::member_key(
            &store_key,
            &session_id,
            &address,
        )) {
            member.waiters = member.waiters.saturating_add(1);
        }
    }

    fn remove_waiter(&self, store_key: &str, session_id: &str, address: &str, pid: u32) {
        if pid != 0 {
            self.waiters
                .lock()
                .unwrap()
                .remove(&Self::waiter_key(store_key, session_id, address, pid));
        }
        if let Some(member) = self
            .members
            .lock()
            .unwrap()
            .get_mut(&Self::member_key(store_key, session_id, address))
        {
            member.waiters = member.waiters.saturating_sub(1);
        }
    }
}

impl MemberRecord {
    fn status(&self, live_waiters: &[LiveWaiterStatus]) -> MemberStatus {
        MemberStatus {
            store_key: self.store_key.clone(),
            backend: self.backend.clone(),
            session_id: self.session_id.clone(),
            address: self.address.clone(),
            occupant: self.occupant.clone(),
            host: self.host.clone(),
            waiters: self.waiters,
            live_waiters: live_waiters
                .iter()
                .filter(|waiter| {
                    waiter.store_key == self.store_key
                        && waiter.session_id == self.session_id
                        && waiter.address == self.address
                })
                .cloned()
                .collect(),
            watch_pids: self.watch_pids.iter().map(WatchPidRecord::status).collect(),
            description: self.description.clone(),
            scope: self.scope.clone(),
            tags: self.tags.clone(),
            lease_epoch: self.lease_epoch,
            owner_instance_id: self.owner_instance_id.clone(),
            idle: self.idle,
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
            store_key: self.store_key.clone(),
            session_id: self.session_id.clone(),
            address: self.address.clone(),
            pid: self.pid,
            alive: crate::session_watch::process_alive_with_start_time(self.pid, self.start_time),
            started_at_ms: self.started_at_ms,
            start_time: self.start_time,
            attention: self.attention.clone(),
            timeout_ms: self.timeout_ms,
        }
    }
}

struct WaiterGuard {
    state: Arc<DaemonState>,
    store_key: String,
    session_id: String,
    address: String,
    pid: u32,
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
        timeout_ms: Option<u64>,
    ) -> Self {
        let pid = pid.unwrap_or(0);
        state.add_waiter(WaiterRecord {
            store_key: store_key.to_string(),
            session_id: session_id.to_string(),
            address: address.to_string(),
            pid,
            start_time,
            started_at_ms: now_ms(),
            attention,
            timeout_ms,
        });
        Self {
            state,
            store_key: store_key.to_string(),
            session_id: session_id.to_string(),
            address: address.to_string(),
            pid,
        }
    }
}

impl Drop for WaiterGuard {
    fn drop(&mut self) {
        self.state
            .remove_waiter(&self.store_key, &self.session_id, &self.address, self.pid);
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
    match connect_existing(store_key).await {
        Ok(client) => return Ok(client),
        Err(e @ (DaemonError::Unauthorized(_) | DaemonError::Incompatible(_))) => return Err(e),
        Err(_) => {}
    }

    let deadline = Instant::now() + READINESS_TIMEOUT;
    let mut launches: Vec<Instant> = Vec::new();
    let mut backoff = BACKOFF_INITIAL;
    let mut last_err: Option<DaemonError> = None;

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
    let mut command = tokio::process::Command::new(exe);
    command
        .arg("daemon")
        .arg("serve")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(false);
    command
        .spawn()
        .map(|_| ())
        .map_err(|e| io_err("spawning daemon", e))
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
        recent_errors: Mutex::new(VecDeque::new()),
        ended_sessions: Mutex::new(BTreeMap::new()),
        draining: AtomicBool::new(false),
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

async fn open_store_backend(store_key: &str) -> std::result::Result<Arc<dyn Backend>, Response> {
    let Some(path) = store_key.strip_prefix("sqlite:") else {
        return Err(proto::unsupported(format!(
            "daemon-core P3 serves sqlite:<absolute-path> store keys only, got {store_key}"
        )));
    };
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
        Ok(Arc::new(backend))
    }
    #[cfg(not(feature = "sqlite"))]
    {
        let _ = path;
        Err(proto::unsupported(
            "this telex build does not include the sqlite backend",
        ))
    }
}

enum ClientAction {
    Continue,
    Drain,
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
    let members = state.members_snapshot();
    for member in members {
        if state
            .get_member(&member.store_key, &member.session_id, &member.address)
            .is_none()
        {
            continue;
        }
        if let Some(reason) = watch_pid_reap_reason(&member.watch_pids) {
            state.mark_session_idle(
                &member.store_key,
                &member.session_id,
                "WatchPidDeath",
                &reason,
                true,
            );
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
            Ok(true) => {}
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
                refreshed.watch_pids = watch_pids;
                refreshed.idle = false;
                refreshed.idle_rearmable = false;
                state.check_session_id_reuse_tripwire(&refreshed);
                if !recovery {
                    state.clear_definite_session_end(&store_key, &session_id);
                }
                state.insert_member(refreshed.clone());
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
        .find(|m| m.store_key == store_key && m.address == address)
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
    };
    state.check_session_id_reuse_tripwire(&record);
    if !recovery {
        state.clear_definite_session_end(&store_key, &session_id);
    }
    state.insert_member(record);
    Response::Registered {
        lease_epoch: claimed.lease_epoch,
        owner_instance_id: claimed.owner_instance_id,
    }
}

async fn session_end(state: Arc<DaemonState>, store_key: String, session_id: String) -> Response {
    let affected = state.mark_session_idle(
        &store_key,
        &session_id,
        "SessionEnd",
        "authoritative sessionEnd hook",
        true,
    );
    if affected.is_empty() {
        state.push_recent_error(
            "SessionEnd",
            format!("SessionEnd no-op store={store_key} session={session_id}: no active members"),
        );
    }
    Response::Ack {
        message: Some("session-ended".to_string()),
        delivery_outcome: None,
        address: None,
        message_id: None,
        lease_epoch: None,
    }
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
    let deadline = timeout_ms.map(|ms| Instant::now() + Duration::from_millis(ms));
    let idle_deadline = Instant::now() + idle_ttl;
    let _waiter = WaiterGuard::new(
        state.clone(),
        &store_key,
        &session_id,
        &address,
        waiter_pid,
        waiter_start_time,
        attention.clone(),
        timeout_ms,
    );
    loop {
        if state.is_draining() {
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
        let rows = match backend.fetch_undelivered(&address).await {
            Ok(rows) => rows,
            Err(e) => {
                return proto::internal(format!("fetching undelivered for {address}: {e:#}"));
            }
        };
        if let Some(row) = rows.into_iter().find(|row| {
            attention
                .as_deref()
                .map_or(true, |want| row.attention == want)
        }) {
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
                return response;
            }
            let response = Response::Message {
                id: row.id,
                thread_id: row.thread_id,
                parent_id: row.parent_id,
                from_addr: row.from_addr,
                to_addr: row.to_addr,
                kind: row.kind,
                attention: row.attention,
                requires_disposition: row.requires_disposition,
                subject: row.subject,
                body: row.body,
                sent_at_ms: row.sent_at_ms,
                buffered_at_ms: now_ms(),
                lease_epoch: Some(current.lease_epoch),
            };
            return match proto::json_line_frame_len(&response) {
                Ok(len) if len <= proto::MAX_JSONL_FRAME_BYTES => response,
                Ok(len) => proto::error_response(
                    proto::ERROR_INCOMPATIBLE,
                    format!(
                        "message {} serializes to {len} bytes, exceeding IPC frame limit {}",
                        row.id,
                        proto::MAX_JSONL_FRAME_BYTES
                    ),
                ),
                Err(e) => proto::internal(format!("sizing message {} IPC frame: {e}", row.id)),
            };
        }
        if let Some(deadline) = deadline {
            let now = Instant::now();
            if now >= deadline {
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
                return Response::PresenceEnded;
            }
            let remaining = deadline.saturating_duration_since(now);
            let ttl_remaining = idle_deadline.saturating_duration_since(now);
            tokio::time::sleep(remaining.min(ttl_remaining).min(Duration::from_millis(100))).await;
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
                return Response::PresenceEnded;
            }
            tokio::time::sleep(
                idle_deadline
                    .saturating_duration_since(now)
                    .min(Duration::from_millis(250)),
            )
            .await;
        }
    }
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
    let _ = backend.notify_new(&to_addr, row.id, row.sent_at_ms).await;
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
        cc: None,
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
    let _ = backend.notify_new(&to, row.id, row.sent_at_ms).await;
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
            recent_errors: Mutex::new(VecDeque::new()),
            ended_sessions: Mutex::new(BTreeMap::new()),
            draining: AtomicBool::new(false),
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
        }
    }

    fn wait_req(store: &str, session: &str, address: &str, timeout_ms: u64) -> Request {
        Request::Wait {
            store_key: store.to_string(),
            session_id: session.to_string(),
            address: address.to_string(),
            attention: None,
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

        for _ in 0..2 {
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
        }

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

        let wait = request(state, wait_req(&store, "s1", "addr:a", 1_000)).await;
        match wait {
            Response::Error { code, message, .. } => {
                assert_eq!(code, proto::ERROR_INCOMPATIBLE);
                assert!(message.contains(&message_id.to_string()));
                assert!(message.contains("IPC frame"));
            }
            other => panic!("expected oversized-frame error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn acking_one_fanout_recipient_does_not_consume_the_other() {
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
        assert_eq!(
            backend
                .fetch_undelivered("addr:b")
                .await
                .unwrap()
                .iter()
                .map(|m| m.id)
                .collect::<Vec<_>>(),
            vec![message_id]
        );

        let wait_b = request(state.clone(), wait_req(&store, "s1", "addr:b", 1000)).await;
        assert!(matches!(wait_b, Response::Message { id, .. } if id == message_id));
        let ack_b = request(state, ack_req(&store, "s1", "addr:b", message_id)).await;
        assert!(matches!(
            ack_b,
            Response::Ack {
                delivery_outcome: Some(DeliveryOutcome::Marked),
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

        let status = state.status_with_thresholds(0, 1).await;
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
                recent_errors: Mutex::new(VecDeque::new()),
                ended_sessions: Mutex::new(BTreeMap::new()),
                draining: AtomicBool::new(false),
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
        ) -> DaemonStatus {
            self.state
                .status_with_thresholds(retention_warn_threshold, idle_station_warn_threshold)
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
            return Err(io_err(
                "opening peer process",
                std::io::Error::last_os_error(),
            ));
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
            return Err(io_err(
                "opening process token",
                std::io::Error::last_os_error(),
            ));
        }
        Ok(Handle(token))
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
