//! Durable, host-local, owner-private **station intent** records (ADR 0050).
//!
//! An intent is the *desired* push registration for one `(store_key, session_id, address)` binding.
//! It is explicitly **not** membership and **not** attendance: in-memory `MemberRecord` plus the
//! backend epoch lease remain the only authority for who is attending an address and who may
//! deliver to it. An intent only says "if a compatible daemon is running and this producer proves
//! it is alive, the daemon should re-register this exact push handler."
//!
//! Consequences that fall out of that framing, and that the rest of the module enforces:
//!
//! * Intents are **host-local files**, so a shared Postgres store can never let one host restore
//!   another host's bridge. `host_id` + `boot_id` are additionally recorded so a synced or
//!   network-mounted home directory cannot defeat that either.
//! * Intents live under the daemon runtime directory, namespaced by the daemon singleton hash, so
//!   they inherit the hardened owner-private checks of `platform_fs` and stay isolated per config
//!   root and per protocol major.
//! * **No secret is ever stored in an intent.** The producer descriptor carries a constrained
//!   *pointer* to an owner-private credential file, resolved fresh at reconcile time, so a
//!   per-process rotating secret is a non-issue rather than a permanent fail-closed.
//! * Nothing here reads `.bindings.json`. An intent is never synthesized from legacy state,
//!   because a synthesized intent would be indistinguishable from an authentic one while carrying
//!   none of the store/mode/CC truth that makes restoration correct.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::daemon_ipc::IntentRecoveryState;
use crate::platform_fs;

/// On-disk schema version, plus the inclusive range this build accepts. Same shape as the wire
/// protocol's major/min-supported convention so version skew is a first-class, testable axis.
pub const STATION_INTENT_SCHEMA_VERSION: u32 = 1;
pub const STATION_INTENT_SCHEMA_MIN_SUPPORTED: u32 = 1;
pub const STATION_INTENT_SCHEMA_MAX_SUPPORTED: u32 = 1;

/// Per-scope write-time cap. A scope may legitimately *hold* more than this (an older build, a
/// manual copy, or a GC that has not run yet); see `ScanPage::over_cap`. Being over cap is never
/// a reason to delete anything.
pub const STATION_INTENT_MAX_COUNT: usize = 512;
/// Per-file byte cap, enforced on the open handle by `platform_fs::read_owner_only_file`.
pub const STATION_INTENT_MAX_BYTES: u64 = 16 * 1024;

/// A `pending` intent is one an attach wrote but never finalized. It is never reconciled, so a
/// crash mid-attach cannot leave a claimable record; this TTL removes the leftovers.
pub const STATION_INTENT_PENDING_TTL: Duration = Duration::from_secs(5 * 60);
/// Orphan TTL for intents that stayed unverifiable/insecure: long enough that a laptop closed for
/// a week still recovers, short enough that abandoned state does not accumulate forever.
pub const STATION_INTENT_UNVERIFIABLE_TTL: Duration = Duration::from_secs(7 * 24 * 60 * 60);

/// Ceiling for a credential descriptor's `max_age_ms`. A descriptor may only *lower* it: an
/// attacker-influenceable manifest must not be able to widen the window in which a stale secret
/// is still honored.
pub const CREDENTIAL_MAX_AGE_MS_DEFAULT: i64 = 24 * 60 * 60 * 1000;

/// Per-probe I/O ceiling. Deliberately much smaller than the per-intent budget so local validation
/// and the backend lease claim still fit inside it.
pub const BRIDGE_PROBE_TIMEOUT: Duration = Duration::from_secs(1);

/// The one producer-descriptor kind this build understands.
pub const PRODUCER_KIND_LOCAL_ENDPOINT_CHALLENGE_V1: &str = "local_endpoint_challenge_v1";
/// The one credential-descriptor kind this build understands.
pub const CREDENTIAL_KIND_OWNER_PRIVATE_JSON_FIELD_V1: &str = "owner_private_json_field_v1";

const INTENT_FILE_SUFFIX: &str = ".intent.json";
const SCAN_CURSOR_FILE: &str = "scan-cursor.json";

// ---------------------------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------------------------

#[derive(Debug)]
pub enum IntentError {
    /// The scope holds `STATION_INTENT_MAX_COUNT` intents already. Typed rather than "silently
    /// grow", so an unbounded producer is a visible error at the write, not a slow leak.
    CapExceeded {
        count: usize,
        cap: usize,
    },
    /// The manifest is larger than `STATION_INTENT_MAX_BYTES`.
    TooLarge {
        bytes: u64,
        cap: u64,
    },
    /// Schema version outside `[MIN_SUPPORTED, MAX_SUPPORTED]`.
    UnsupportedSchema {
        found: u32,
        min: u32,
        max: u32,
    },
    /// A structurally invalid descriptor (unknown kind, bad session id, injected argv parameter).
    Invalid(String),
    /// A security check failed. Distinct from `Invalid` because it maps to `Insecure`, not
    /// `Unverifiable`, and is never retried on the fast cadence.
    Insecure(String),
    Io(String),
    Json(String),
}

impl std::fmt::Display for IntentError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IntentError::CapExceeded { count, cap } => write!(
                f,
                "station-intent scope holds {count} intents, at or over the {cap} cap"
            ),
            IntentError::TooLarge { bytes, cap } => {
                write!(
                    f,
                    "station intent is {bytes} bytes, over the {cap} byte cap"
                )
            }
            IntentError::UnsupportedSchema { found, min, max } => write!(
                f,
                "station-intent schema version {found} is outside the supported range {min}..={max}"
            ),
            IntentError::Invalid(msg) => write!(f, "invalid station intent: {msg}"),
            IntentError::Insecure(msg) => write!(f, "insecure station intent: {msg}"),
            IntentError::Io(msg) => write!(f, "station-intent I/O failed: {msg}"),
            IntentError::Json(msg) => write!(f, "station-intent JSON failed: {msg}"),
        }
    }
}

impl std::error::Error for IntentError {}

impl From<platform_fs::FsError> for IntentError {
    fn from(value: platform_fs::FsError) -> Self {
        match value {
            platform_fs::FsError::Io { action, source } => {
                IntentError::Io(format!("{action}: {source}"))
            }
            // Every `Unsupported` from the owner-private primitives is a failed security or shape
            // check, so it must degrade to `Insecure`, never to a retryable failure.
            platform_fs::FsError::Unsupported {
                capability,
                message,
            } => IntentError::Insecure(format!("{capability}: {message}")),
        }
    }
}

pub type Result<T> = std::result::Result<T, IntentError>;

// ---------------------------------------------------------------------------------------------
// Identity
// ---------------------------------------------------------------------------------------------

/// A hashed, filesystem-safe intent identity.
///
/// `sha256(store_key | 0x1f | session_id | 0x1f | address)` truncated to 32 hex chars. Hashing
/// (rather than encoding) the tuple keeps address and store strings out of the filesystem path
/// entirely, so path traversal and filename-collision surface is closed by construction rather
/// than by escaping.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct IntentId(String);

impl IntentId {
    pub fn derive(store_key: &str, session_id: &str, address: &str) -> Self {
        let mut material = Vec::new();
        material.extend_from_slice(store_key.as_bytes());
        material.push(0x1f);
        material.extend_from_slice(session_id.as_bytes());
        material.push(0x1f);
        material.extend_from_slice(address.as_bytes());
        IntentId(platform_fs::sha256_hex(&material)[..32].to_string())
    }

    /// Parse an id out of a directory entry name, rejecting anything that is not exactly the
    /// 32-lowercase-hex shape this module writes.
    pub fn from_file_name(name: &str) -> Option<Self> {
        let stem = name.strip_suffix(INTENT_FILE_SUFFIX)?;
        if stem.len() != 32
            || !stem
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        {
            return None;
        }
        Some(IntentId(stem.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn file_name(&self) -> String {
        format!("{}{INTENT_FILE_SUFFIX}", self.0)
    }
}

impl std::fmt::Display for IntentId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Strict `session_id` validation, applied both at descriptor load and at argv build.
///
/// The session id is the only caller-supplied parameter that reaches a spawned handler's argv, so
/// it is constrained to a charset that cannot introduce a flag, a path, or a shell metacharacter.
pub fn validate_session_id(session_id: &str) -> Result<()> {
    if session_id.is_empty() || session_id.len() > 128 {
        return Err(IntentError::Invalid(format!(
            "session id must be 1..=128 characters, got {}",
            session_id.len()
        )));
    }
    // A leading `-` would make the value parse as a flag rather than a value if argv were ever
    // reordered or re-split, so it is refused outright rather than escaped.
    if session_id.starts_with('-') {
        return Err(IntentError::Invalid(
            "session id must not start with '-'".to_string(),
        ));
    }
    if !session_id
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err(IntentError::Invalid(
            "session id may only contain ASCII alphanumerics, '-' and '_'".to_string(),
        ));
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Descriptors
// ---------------------------------------------------------------------------------------------

/// How the daemon should rebuild the push handler. Deliberately carries **no executable path** and
/// **no `--backend`/`--db` strings**: argv is re-derived from the reconciling daemon's own
/// executable and store resolution, so an upgrade or rollback cannot restore a handler pointing at
/// a binary or store that no longer exists, and a tampered manifest cannot inject argv.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HandlerDescriptorV1 {
    pub kind: String,
    pub session_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProducerTransport {
    NamedPipe,
    UnixSocket,
    /// Forward-compat catch-all. An unknown transport is never connected to.
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProtocolRange {
    pub min: u32,
    pub max: u32,
}

/// A *pointer* to an owner-private credential, never the credential itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CredentialDescriptorV1 {
    pub kind: String,
    /// Names a producer root registered at composition time. The daemon core learns only that
    /// "root X is registered", never a Copilot path or filename.
    pub root_id: String,
    pub path: PathBuf,
    /// RFC 6901 JSON pointer into the credential file, e.g. `/secret`.
    pub pointer: String,
    /// Maximum age of the credential file, clamped to `CREDENTIAL_MAX_AGE_MS_DEFAULT` on use.
    pub max_age_ms: i64,
}

impl CredentialDescriptorV1 {
    /// Clamp `max_age_ms` into `1..=CREDENTIAL_MAX_AGE_MS_DEFAULT`. A descriptor may only lower
    /// the ceiling, never raise it.
    pub fn clamped_max_age_ms(&self) -> i64 {
        self.max_age_ms.clamp(1, CREDENTIAL_MAX_AGE_MS_DEFAULT)
    }

    fn validate(&self) -> Result<()> {
        if self.kind != CREDENTIAL_KIND_OWNER_PRIVATE_JSON_FIELD_V1 {
            return Err(IntentError::Invalid(format!(
                "unknown credential descriptor kind {:?}",
                self.kind
            )));
        }
        if self.root_id.is_empty() {
            return Err(IntentError::Invalid(
                "credential descriptor has an empty root_id".to_string(),
            ));
        }
        if !self.pointer.starts_with('/') {
            return Err(IntentError::Invalid(format!(
                "credential pointer {:?} is not an RFC 6901 pointer",
                self.pointer
            )));
        }
        if self.path.as_os_str().is_empty() {
            return Err(IntentError::Invalid(
                "credential descriptor has an empty path".to_string(),
            ));
        }
        Ok(())
    }
}

/// Everything the daemon needs to reach and *prove* the producer, expressed generically. The
/// daemon core never learns that the producer is Copilot; the descriptor kind is the boundary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProducerDescriptorV1 {
    pub kind: String,
    pub transport: ProducerTransport,
    pub endpoint_path: String,
    pub exe_path: PathBuf,
    pub pid: u32,
    /// Platform process start time. Required: an intent whose producer start time is unknown can
    /// never be verified, so it is refused at write time rather than fail-open at reconcile time.
    pub start_time: u64,
    pub host_id: String,
    pub boot_id: String,
    pub protocol: ProtocolRange,
    pub credential: CredentialDescriptorV1,
}

impl ProducerDescriptorV1 {
    /// Structural validation.
    ///
    /// `require_identity` is false only for a `Pending` intent: attach writes the record *before*
    /// `Register`, at a point where the bridge extension may not be loaded yet, so the concrete
    /// producer identity is genuinely not knowable. That is safe precisely because a `Pending`
    /// intent is never reconciled — the identity fields become mandatory at the moment the intent
    /// becomes `Live`, which is the only state that can ever claim a station.
    fn validate(&self, require_identity: bool) -> Result<()> {
        if self.kind != PRODUCER_KIND_LOCAL_ENDPOINT_CHALLENGE_V1 {
            return Err(IntentError::Invalid(format!(
                "unknown producer descriptor kind {:?}",
                self.kind
            )));
        }
        if matches!(self.transport, ProducerTransport::Unknown) {
            return Err(IntentError::Invalid(
                "producer descriptor has an unknown transport".to_string(),
            ));
        }
        if self.endpoint_path.is_empty() {
            return Err(IntentError::Invalid(
                "producer descriptor has an empty endpoint path".to_string(),
            ));
        }
        if require_identity {
            if self.pid == 0 || self.start_time == 0 {
                return Err(IntentError::Invalid(
                    "a live producer descriptor must carry a concrete pid and start time"
                        .to_string(),
                ));
            }
            if self.exe_path.as_os_str().is_empty() {
                return Err(IntentError::Invalid(
                    "a live producer descriptor must carry an executable path".to_string(),
                ));
            }
            if self.host_id.is_empty() || self.boot_id.is_empty() {
                return Err(IntentError::Invalid(
                    "a live producer descriptor must carry host and boot identity".to_string(),
                ));
            }
        }
        if self.protocol.min > self.protocol.max {
            return Err(IntentError::Invalid(
                "producer descriptor protocol range is inverted".to_string(),
            ));
        }
        self.credential.validate()
    }
}

/// The daemon protocol this intent was written against. Used for version-skew reporting, never to
/// gate the daemon's own behavior (a newer daemon reconciles an older intent).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DaemonCompat {
    pub protocol_major: u16,
    pub protocol_minor: u16,
}

/// Diagnostic evidence, matching the existing `*_since_ms` / `*_for_ms` / `*_count` idiom on
/// `MemberStatus`. Every field answers "why is this intent in this state right now", so an
/// operator never has to infer a cause from a bare state name.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IntentEvidence {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_attempt_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_success_ms: Option<i64>,
    #[serde(default)]
    pub attempts: u64,
    #[serde(default)]
    pub consecutive_failures: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub producer_verified_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_attempt_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recovery_latency_ms: Option<i64>,
}

// ---------------------------------------------------------------------------------------------
// The intent record
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StationIntentV1 {
    pub schema_version: u32,
    /// Monotonic per-intent generation. Reconciliation is generation-CAS guarded: a pass that
    /// observed generation N refuses to write back if the on-disk generation moved.
    pub generation: u64,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    /// Persisted lifecycle. Only `Pending`, `Live`, and `Revoked` are ever written; every other
    /// `IntentRecoveryState` is a *runtime projection* held in the daemon's in-memory index, so a
    /// transient failure never rewrites durable state.
    pub state: IntentRecoveryState,
    pub store_key: String,
    pub session_id: String,
    pub address: String,
    pub occupant: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tags: Option<String>,
    pub delivery_mode: String,
    #[serde(default)]
    pub wake_on_cc: bool,
    /// The member's `on_deliver_cc_after_ms` captured at finalize time. Passing this through
    /// instead of recomputing "now" at reconcile time is what keeps every CC message committed
    /// during a restart gap visible rather than permanently skipped.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cc_watermark_ms: Option<i64>,
    pub handler: HandlerDescriptorV1,
    pub producer: ProducerDescriptorV1,
    pub daemon_compat: DaemonCompat,
    pub singleton_hash: String,
    #[serde(default)]
    pub evidence: IntentEvidence,
    /// Unknown-field passthrough: a V1 daemon rewriting an intent written by a future build must
    /// not silently drop fields it does not understand.
    #[serde(flatten, default, skip_serializing_if = "BTreeMap::is_empty")]
    pub extra: BTreeMap<String, serde_json::Value>,
}

impl StationIntentV1 {
    pub fn id(&self) -> IntentId {
        IntentId::derive(&self.store_key, &self.session_id, &self.address)
    }

    /// Deterministic scan order: `(store_key, address, generation desc)`. Generation descending
    /// gives first-live-wins when two intents compete for one address.
    pub fn sort_key(&self) -> IntentSortKey {
        IntentSortKey {
            store_key: self.store_key.clone(),
            address: self.address.clone(),
            generation_desc: u64::MAX - self.generation,
            id: self.id(),
        }
    }

    /// Structural validation applied on every load and every write.
    pub fn validate(&self) -> Result<()> {
        if self.schema_version < STATION_INTENT_SCHEMA_MIN_SUPPORTED
            || self.schema_version > STATION_INTENT_SCHEMA_MAX_SUPPORTED
        {
            return Err(IntentError::UnsupportedSchema {
                found: self.schema_version,
                min: STATION_INTENT_SCHEMA_MIN_SUPPORTED,
                max: STATION_INTENT_SCHEMA_MAX_SUPPORTED,
            });
        }
        if self.store_key.is_empty() || self.address.is_empty() {
            return Err(IntentError::Invalid(
                "intent must carry a store key and an address".to_string(),
            ));
        }
        validate_session_id(&self.session_id)?;
        if self.handler.session_id != self.session_id {
            return Err(IntentError::Invalid(
                "handler descriptor session id does not match the intent session id".to_string(),
            ));
        }
        if self.delivery_mode != "push" {
            return Err(IntentError::Invalid(format!(
                "unsupported intent delivery mode {:?}",
                self.delivery_mode
            )));
        }
        // Producer identity is mandatory for every state that can ever claim a station, and
        // optional only for `Pending` — see `ProducerDescriptorV1::validate`.
        self.producer
            .validate(self.state != IntentRecoveryState::Pending)
    }

    /// Whether this intent is eligible for reconciliation at all. `Pending` never is: an attach
    /// that crashed before finalizing must not produce a claimable record.
    pub fn is_reconcilable(&self) -> bool {
        matches!(self.state, IntentRecoveryState::Live)
    }

    /// Whether this intent was written by this host and this boot.
    pub fn matches_local_identity(&self, host: &str, boot: &str) -> bool {
        self.producer.host_id == host && self.producer.boot_id == boot
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct IntentSortKey {
    pub store_key: String,
    pub address: String,
    pub generation_desc: u64,
    pub id: IntentId,
}

/// What a scan pass observed. `over_cap` is reported rather than acted on: the cap is a write-time
/// rule, and deleting an entry merely for being over cap would silently destroy a user's binding.
#[derive(Debug, Clone, Default)]
pub struct ScanPage {
    /// Manifests actually loaded this pass, in deterministic order.
    pub loaded: Vec<StationIntentV1>,
    /// Sort position of each entry in `loaded`, parallel by index.
    ///
    /// Exposed so the caller can advance the round-robin cursor to the last intent it actually
    /// *attempted*. `scan` cannot do that itself: it does not know how far a budget- or
    /// deadline-truncated pass got, and advancing past unattempted entries is precisely how a
    /// round-robin cursor starves the tail of a scope.
    pub loaded_positions: Vec<String>,
    /// Entries the pass observed but did not load because the budget was exhausted.
    pub skipped: Vec<IntentId>,
    /// Entries whose read failed a security or schema check, with the state they map to.
    pub rejected: Vec<(IntentId, IntentRecoveryState, String)>,
    pub observed_count: usize,
    pub over_cap: bool,
}

#[derive(Debug, Clone, Default)]
pub struct GcReport {
    pub removed: Vec<IntentId>,
    pub kept: usize,
    pub reasons: Vec<(IntentId, String)>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct ScanCursor {
    #[serde(default)]
    position: Option<String>,
}

// ---------------------------------------------------------------------------------------------
// The store
// ---------------------------------------------------------------------------------------------

/// The per-scope intent store, rooted at `<run_dir>/intents/<singleton_hash>`.
///
/// `run_dir` is the directory ADR 0025 designates as authority-bearing and the only one with a
/// real fail-closed owner-private check on both platforms. The `<singleton_hash>` component hashes
/// user identity, canonicalized config root, and protocol major, so a scope is isolated per config
/// root (destructive testing stays contained) *and* namespaced per protocol major (a protocol-major
/// change cannot make an old daemon and a new daemon fight over the same intents).
#[derive(Debug, Clone)]
pub struct IntentStore {
    root: PathBuf,
    singleton_hash: String,
}

impl IntentStore {
    /// Open (creating and repairing if needed) the intent scope for a daemon singleton.
    pub fn open(run_dir: &Path, singleton_hash: &str) -> Result<Self> {
        let root = run_dir.join("intents").join(singleton_hash);
        let root = platform_fs::ensure_owner_private_dir(&root)?;
        Ok(Self {
            root,
            singleton_hash: singleton_hash.to_string(),
        })
    }

    /// Open the scope **without** creating it, for read-only paths that must not have a side
    /// effect (e.g. a status projection on a host that never attached).
    pub fn open_existing(run_dir: &Path, singleton_hash: &str) -> Result<Option<Self>> {
        let root = run_dir.join("intents").join(singleton_hash);
        if !root.exists() {
            return Ok(None);
        }
        Self::open(run_dir, singleton_hash).map(Some)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn singleton_hash(&self) -> &str {
        &self.singleton_hash
    }

    pub fn path_for(&self, id: &IntentId) -> PathBuf {
        self.root.join(id.file_name())
    }

    /// Every intent id present in the scope, in filename order. Enumeration is unbounded (so
    /// `observed_count` and `over_cap` are honest); *loading* is what the pass budget bounds.
    pub fn list_ids(&self) -> Result<Vec<IntentId>> {
        let entries = match std::fs::read_dir(&self.root) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(IntentError::Io(format!("listing intent scope: {e}"))),
        };
        let mut ids = Vec::new();
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if let Some(id) = IntentId::from_file_name(&name) {
                ids.push(id);
            }
        }
        ids.sort();
        Ok(ids)
    }

    /// Load and validate one intent, fail-closed on every security and schema rule.
    pub fn load(&self, id: &IntentId) -> Result<StationIntentV1> {
        let path = self.path_for(id);
        let bytes = platform_fs::read_owner_only_file(&path, STATION_INTENT_MAX_BYTES)?;
        let intent: StationIntentV1 = serde_json::from_slice(&bytes)
            .map_err(|e| IntentError::Json(format!("parsing {}: {e}", path.display())))?;
        intent.validate()?;
        if intent.id() != *id {
            return Err(IntentError::Insecure(format!(
                "intent {} does not hash to its own filename",
                path.display()
            )));
        }
        if intent.singleton_hash != self.singleton_hash {
            return Err(IntentError::Insecure(format!(
                "intent {} belongs to singleton scope {}",
                path.display(),
                intent.singleton_hash
            )));
        }
        Ok(intent)
    }

    /// Load without failing the whole pass: map every rejection onto the recovery state it means.
    pub fn load_projected(
        &self,
        id: &IntentId,
    ) -> std::result::Result<StationIntentV1, (IntentRecoveryState, String)> {
        match self.load(id) {
            Ok(intent) => Ok(intent),
            Err(e @ IntentError::Insecure(_)) => {
                Err((IntentRecoveryState::Insecure, e.to_string()))
            }
            Err(e @ IntentError::UnsupportedSchema { .. }) => {
                Err((IntentRecoveryState::Incompatible, e.to_string()))
            }
            Err(e @ IntentError::Invalid(_)) => {
                Err((IntentRecoveryState::Incompatible, e.to_string()))
            }
            Err(e) => Err((IntentRecoveryState::Unverifiable, e.to_string())),
        }
    }

    /// Write an intent atomically. Enforces the per-scope count cap for *new* ids (an existing
    /// intent may always be rewritten, so an over-cap scope can still be revoked or GC'd out).
    pub fn write_atomic(&self, intent: &StationIntentV1) -> Result<()> {
        intent.validate()?;
        let id = intent.id();
        let path = self.path_for(&id);
        if !path.exists() {
            let count = self.list_ids()?.len();
            if count >= STATION_INTENT_MAX_COUNT {
                return Err(IntentError::CapExceeded {
                    count,
                    cap: STATION_INTENT_MAX_COUNT,
                });
            }
        }
        let bytes = serde_json::to_vec_pretty(intent)
            .map_err(|e| IntentError::Json(format!("serializing intent {id}: {e}")))?;
        if bytes.len() as u64 > STATION_INTENT_MAX_BYTES {
            return Err(IntentError::TooLarge {
                bytes: bytes.len() as u64,
                cap: STATION_INTENT_MAX_BYTES,
            });
        }
        platform_fs::write_owner_only_file_atomic(&path, &bytes)?;
        Ok(())
    }

    /// Compare-and-set on `generation`: refuse the write if the on-disk intent moved since it was
    /// read, so two passes (or a pass and an attach) cannot clobber each other.
    pub fn write_cas(&self, expected_generation: u64, intent: &StationIntentV1) -> Result<bool> {
        let id = intent.id();
        if !self.path_for(&id).exists() {
            if expected_generation == 0 {
                self.write_atomic(intent)?;
                return Ok(true);
            }
            return Ok(false);
        }
        let current = self.load(&id)?;
        if current.generation != expected_generation {
            return Ok(false);
        }
        self.write_atomic(intent)?;
        Ok(true)
    }

    /// Mark exactly one binding's intent revoked. Never whole-session, never cross-store: the id
    /// is derived from the full `(store_key, session_id, address)` tuple.
    pub fn revoke(
        &self,
        store_key: &str,
        session_id: &str,
        address: &str,
        now_ms: i64,
    ) -> Result<bool> {
        let id = IntentId::derive(store_key, session_id, address);
        if !self.path_for(&id).exists() {
            return Ok(false);
        }
        let mut intent = self.load(&id)?;
        if intent.state == IntentRecoveryState::Revoked {
            return Ok(false);
        }
        intent.state = IntentRecoveryState::Revoked;
        intent.generation = intent.generation.saturating_add(1);
        intent.updated_at_ms = now_ms;
        self.write_atomic(&intent)?;
        Ok(true)
    }

    /// Revoke every intent for a session in one store. Used by daemon-side session-end paths,
    /// which are session-scoped by nature (`sessionEnd`, watch-pid death).
    pub fn revoke_session(&self, store_key: &str, session_id: &str, now_ms: i64) -> Result<usize> {
        let mut revoked = 0;
        for id in self.list_ids()? {
            let Ok(intent) = self.load(&id) else { continue };
            if intent.store_key == store_key
                && intent.session_id == session_id
                && intent.state != IntentRecoveryState::Revoked
                && self.revoke(
                    &intent.store_key,
                    &intent.session_id,
                    &intent.address,
                    now_ms,
                )?
            {
                revoked += 1;
            }
        }
        Ok(revoked)
    }

    pub fn remove(&self, id: &IntentId) -> Result<bool> {
        match std::fs::remove_file(self.path_for(id)) {
            Ok(()) => Ok(true),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(IntentError::Io(format!("removing intent {id}: {e}"))),
        }
    }

    /// One bounded scan pass, resuming from the persisted round-robin cursor.
    ///
    /// The cursor stores a *sort position*, not an index, so entries added or removed between
    /// passes cannot starve another entry: the next pass resumes at the first position strictly
    /// greater than the last one processed, and wraps at the end.
    ///
    /// This function **reads** the cursor and never advances it. Only the caller knows how far a
    /// budget- or deadline-truncated pass actually got, and advancing past entries the pass never
    /// attempted is exactly how a round-robin cursor silently starves the tail of a scope. See
    /// [`IntentStore::advance_cursor`].
    pub fn scan(&self, budget: usize) -> Result<ScanPage> {
        let ids = self.list_ids()?;
        let observed_count = ids.len();
        let over_cap = observed_count > STATION_INTENT_MAX_COUNT;

        let cursor = self.read_cursor()?;
        let mut entries: Vec<(IntentSortKey, StationIntentV1)> = Vec::new();
        let mut rejected = Vec::new();
        let mut skipped = Vec::new();

        for id in &ids {
            match self.load_projected(id) {
                Ok(intent) => entries.push((intent.sort_key(), intent)),
                Err((state, detail)) => rejected.push((id.clone(), state, detail)),
            }
        }
        entries.sort_by(|a, b| a.0.cmp(&b.0));

        let total = entries.len();
        if total == 0 {
            return Ok(ScanPage {
                loaded: Vec::new(),
                loaded_positions: Vec::new(),
                skipped,
                rejected,
                observed_count,
                over_cap,
            });
        }

        let sort_positions: Vec<String> = entries.iter().map(|(k, _)| sort_position(k)).collect();
        // Resume at the first position strictly greater than the last processed one; wrap to the
        // start when the cursor sits at or past the maximum.
        let start = match cursor.position.as_deref() {
            Some(position) => sort_positions
                .iter()
                .position(|candidate| candidate.as_str() > position)
                .unwrap_or(0),
            None => 0,
        };

        let take = budget.min(total);
        let mut loaded = Vec::with_capacity(take);
        let mut loaded_positions = Vec::with_capacity(take);
        for offset in 0..total {
            let index = (start + offset) % total;
            if loaded.len() >= take {
                skipped.push(entries[index].1.id());
                continue;
            }
            loaded_positions.push(sort_positions[index].clone());
            loaded.push(entries[index].1.clone());
        }

        Ok(ScanPage {
            loaded,
            loaded_positions,
            skipped,
            rejected,
            observed_count,
            over_cap,
        })
    }

    /// Persist the round-robin cursor at the sort position of the last entry a pass processed.
    ///
    /// Called by the reconciler with the position of the last intent it actually attempted (or, if
    /// it attempted none, the last it considered), so a truncated pass resumes where it stopped
    /// instead of skipping everything it loaded but never reached.
    pub fn advance_cursor(&self, position: &str) -> Result<()> {
        self.write_cursor(position)
    }

    fn read_cursor(&self) -> Result<ScanCursor> {
        let path = self.root.join(SCAN_CURSOR_FILE);
        if !path.exists() {
            return Ok(ScanCursor::default());
        }
        // A corrupt or unreadable cursor is a scheduling hint, not authority: restart from the
        // beginning rather than failing the pass.
        match platform_fs::read_owner_only_file(&path, 4096) {
            Ok(bytes) => Ok(serde_json::from_slice(&bytes).unwrap_or_default()),
            Err(_) => Ok(ScanCursor::default()),
        }
    }

    fn write_cursor(&self, position: &str) -> Result<()> {
        let path = self.root.join(SCAN_CURSOR_FILE);
        let bytes = serde_json::to_vec(&ScanCursor {
            position: Some(position.to_string()),
        })
        .map_err(|e| IntentError::Json(format!("serializing scan cursor: {e}")))?;
        platform_fs::write_owner_only_file_atomic(&path, &bytes)?;
        Ok(())
    }

    /// Bounded garbage collection. The only mechanism that can bring an over-cap scope back under
    /// the cap, and the only place an intent file is ever deleted.
    ///
    /// Order matters: expired `Pending` first (crash-during-attach leftovers), then
    /// `Unverifiable`/`Insecure`/`Revoked` past their TTL, then intents whose credential file is
    /// gone, then intents whose identity belongs to another host or boot *and* whose producer is
    /// provably dead.
    pub fn gc(
        &self,
        now_ms: i64,
        local_host: Option<&str>,
        local_boot: Option<&str>,
    ) -> Result<GcReport> {
        let mut report = GcReport::default();
        for id in self.list_ids()? {
            let intent = match self.load(&id) {
                Ok(intent) => intent,
                Err(IntentError::Io(_)) => {
                    report.kept += 1;
                    continue;
                }
                Err(e) => {
                    // A manifest we cannot even parse securely is GC-eligible once it is older
                    // than the orphan TTL; until then it stays visible in status.
                    if file_age_ms(&self.path_for(&id), now_ms)
                        .is_some_and(|age| age > STATION_INTENT_UNVERIFIABLE_TTL.as_millis() as i64)
                    {
                        self.remove(&id)?;
                        report.removed.push(id.clone());
                        report
                            .reasons
                            .push((id, format!("unreadable past TTL: {e}")));
                    } else {
                        report.kept += 1;
                    }
                    continue;
                }
            };
            let age_ms = now_ms.saturating_sub(intent.updated_at_ms);
            let reason = if intent.state == IntentRecoveryState::Pending
                && age_ms > STATION_INTENT_PENDING_TTL.as_millis() as i64
            {
                Some("pending intent past its TTL (attach never finalized)".to_string())
            } else if matches!(
                intent.state,
                IntentRecoveryState::Unverifiable
                    | IntentRecoveryState::Insecure
                    | IntentRecoveryState::Revoked
            ) && age_ms > STATION_INTENT_UNVERIFIABLE_TTL.as_millis() as i64
            {
                Some("terminal intent past its TTL".to_string())
            } else if !intent.producer.credential.path.exists() {
                Some("credential file is gone".to_string())
            } else if let (Some(host), Some(boot)) = (local_host, local_boot) {
                if !intent.matches_local_identity(host, boot)
                    && !crate::session_watch::process_alive_with_start_time(
                        intent.producer.pid,
                        Some(intent.producer.start_time),
                    )
                {
                    Some("foreign host/boot identity with a dead producer".to_string())
                } else {
                    None
                }
            } else {
                None
            };
            match reason {
                Some(reason) => {
                    self.remove(&id)?;
                    report.removed.push(id.clone());
                    report.reasons.push((id, reason));
                }
                None => report.kept += 1,
            }
        }
        Ok(report)
    }
}

fn sort_position(key: &IntentSortKey) -> String {
    format!(
        "{}\u{1f}{}\u{1f}{:020}\u{1f}{}",
        key.store_key, key.address, key.generation_desc, key.id
    )
}

fn file_age_ms(path: &Path, now_ms: i64) -> Option<i64> {
    let modified = std::fs::metadata(path).ok()?.modified().ok()?;
    let modified_ms = modified
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_millis()).ok())?;
    Some(now_ms.saturating_sub(modified_ms))
}

/// Extract an RFC 6901 JSON pointer from a parsed document.
///
/// Deliberately minimal and total: an absent or non-string target is `None`, never a panic and
/// never a partial match, so a malformed credential file fails closed at the call site.
pub fn json_pointer_str<'a>(document: &'a serde_json::Value, pointer: &str) -> Option<&'a str> {
    if pointer.is_empty() {
        return document.as_str();
    }
    let mut current = document;
    for raw in pointer.trim_start_matches('/').split('/') {
        let token = raw.replace("~1", "/").replace("~0", "~");
        current = match current {
            serde_json::Value::Object(map) => map.get(&token)?,
            serde_json::Value::Array(items) => items.get(token.parse::<usize>().ok()?)?,
            _ => return None,
        };
    }
    current.as_str()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_run_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "telex-intent-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or_default()
        ));
        std::fs::create_dir_all(&dir).expect("run dir");
        dir
    }

    pub(crate) fn sample_intent(store_key: &str, session: &str, address: &str) -> StationIntentV1 {
        StationIntentV1 {
            schema_version: STATION_INTENT_SCHEMA_VERSION,
            generation: 1,
            created_at_ms: 1_000,
            updated_at_ms: 1_000,
            state: IntentRecoveryState::Live,
            store_key: store_key.to_string(),
            session_id: session.to_string(),
            address: address.to_string(),
            occupant: "occupant".to_string(),
            description: None,
            scope: None,
            tags: None,
            delivery_mode: "push".to_string(),
            wake_on_cc: false,
            cc_watermark_ms: Some(42),
            handler: HandlerDescriptorV1 {
                kind: "telex_copilot_push_v1".to_string(),
                session_id: session.to_string(),
            },
            producer: ProducerDescriptorV1 {
                kind: PRODUCER_KIND_LOCAL_ENDPOINT_CHALLENGE_V1.to_string(),
                transport: if cfg!(windows) {
                    ProducerTransport::NamedPipe
                } else {
                    ProducerTransport::UnixSocket
                },
                endpoint_path: "endpoint".to_string(),
                exe_path: PathBuf::from("exe"),
                pid: 4242,
                start_time: 99,
                host_id: "host".to_string(),
                boot_id: "boot".to_string(),
                protocol: ProtocolRange { min: 2, max: 2 },
                credential: CredentialDescriptorV1 {
                    kind: CREDENTIAL_KIND_OWNER_PRIVATE_JSON_FIELD_V1.to_string(),
                    root_id: "copilot_bridge_root".to_string(),
                    path: PathBuf::from("credential.json"),
                    pointer: "/secret".to_string(),
                    max_age_ms: 60_000,
                },
            },
            daemon_compat: DaemonCompat {
                protocol_major: 1,
                protocol_minor: 5,
            },
            singleton_hash: "hash".to_string(),
            evidence: IntentEvidence::default(),
            extra: BTreeMap::new(),
        }
    }

    #[test]
    fn intent_id_is_stable_and_collision_free_for_adversarial_inputs() {
        // The separator is a control byte, so no address/store combination can be crafted to
        // collide by moving the boundary between fields.
        let a = IntentId::derive("sqlite:/a", "s", "addr");
        let b = IntentId::derive("sqlite:/a", "s", "addr");
        assert_eq!(a, b);
        assert_ne!(
            IntentId::derive("sqlite:/a\u{1f}s", "", "addr"),
            IntentId::derive("sqlite:/a", "s", "addr")
        );
        assert_ne!(
            IntentId::derive("sqlite:/a", "s", "addr"),
            IntentId::derive("sqlite:/a", "s", "addr2")
        );
        // Traversal and separator characters never reach the filesystem path.
        let hostile = IntentId::derive("sqlite:/../../etc", "s", "../../../evil");
        assert_eq!(hostile.as_str().len(), 32);
        assert!(hostile.as_str().bytes().all(|b| b.is_ascii_hexdigit()));
        assert_eq!(
            IntentId::from_file_name(&hostile.file_name()).as_ref(),
            Some(&hostile)
        );
        assert!(IntentId::from_file_name("../evil.intent.json").is_none());
        assert!(IntentId::from_file_name("short.intent.json").is_none());
    }

    #[test]
    fn schema_round_trips_and_preserves_unknown_fields() {
        let mut intent = sample_intent("sqlite:/db", "sess", "addr");
        intent.extra.insert(
            "future_field".to_string(),
            serde_json::json!({"kept": true}),
        );
        let encoded = serde_json::to_vec(&intent).expect("encode");
        let decoded: StationIntentV1 = serde_json::from_slice(&encoded).expect("decode");
        assert_eq!(decoded, intent);
        assert_eq!(
            decoded.extra.get("future_field"),
            Some(&serde_json::json!({"kept": true})),
            "a V1 daemon must not drop a future build's field on rewrite"
        );
    }

    #[test]
    fn schema_version_outside_the_supported_range_is_rejected() {
        let mut low = sample_intent("sqlite:/db", "sess", "addr");
        low.schema_version = STATION_INTENT_SCHEMA_MIN_SUPPORTED - 1;
        assert!(matches!(
            low.validate(),
            Err(IntentError::UnsupportedSchema { .. })
        ));
        let mut high = sample_intent("sqlite:/db", "sess", "addr");
        high.schema_version = STATION_INTENT_SCHEMA_MAX_SUPPORTED + 1;
        assert!(matches!(
            high.validate(),
            Err(IntentError::UnsupportedSchema { .. })
        ));
    }

    #[test]
    fn descriptor_validation_rejects_injection_and_unknown_kinds() {
        let mut argv_injection = sample_intent("sqlite:/db", "sess", "addr");
        argv_injection.handler.session_id = "--backend evil".to_string();
        argv_injection.session_id = "--backend evil".to_string();
        assert!(matches!(
            argv_injection.validate(),
            Err(IntentError::Invalid(_))
        ));

        let mut mismatched = sample_intent("sqlite:/db", "sess", "addr");
        mismatched.handler.session_id = "other".to_string();
        assert!(matches!(
            mismatched.validate(),
            Err(IntentError::Invalid(_))
        ));

        let mut unknown_producer = sample_intent("sqlite:/db", "sess", "addr");
        unknown_producer.producer.kind = "shell_exec_v1".to_string();
        assert!(matches!(
            unknown_producer.validate(),
            Err(IntentError::Invalid(_))
        ));

        let mut unknown_credential = sample_intent("sqlite:/db", "sess", "addr");
        unknown_credential.producer.credential.kind = "plaintext".to_string();
        assert!(matches!(
            unknown_credential.validate(),
            Err(IntentError::Invalid(_))
        ));

        let mut no_start_time = sample_intent("sqlite:/db", "sess", "addr");
        no_start_time.producer.start_time = 0;
        assert!(matches!(
            no_start_time.validate(),
            Err(IntentError::Invalid(_))
        ));
    }

    #[test]
    fn credential_max_age_is_clamped_and_never_widened() {
        let mut intent = sample_intent("sqlite:/db", "sess", "addr");
        intent.producer.credential.max_age_ms = CREDENTIAL_MAX_AGE_MS_DEFAULT * 10;
        assert_eq!(
            intent.producer.credential.clamped_max_age_ms(),
            CREDENTIAL_MAX_AGE_MS_DEFAULT
        );
        intent.producer.credential.max_age_ms = 5_000;
        assert_eq!(intent.producer.credential.clamped_max_age_ms(), 5_000);
        intent.producer.credential.max_age_ms = -1;
        assert_eq!(intent.producer.credential.clamped_max_age_ms(), 1);
    }

    #[test]
    fn write_read_revoke_round_trip_is_exact_per_binding() {
        let run_dir = temp_run_dir("roundtrip");
        let store = IntentStore::open(&run_dir, "hash").expect("store");
        let a = sample_intent("sqlite:/a", "sess", "addr-1");
        let b = sample_intent("sqlite:/a", "sess", "addr-2");
        let c = sample_intent("sqlite:/b", "sess", "addr-1");
        for intent in [&a, &b, &c] {
            store.write_atomic(intent).expect("write");
        }
        assert_eq!(store.list_ids().expect("list").len(), 3);

        assert!(store
            .revoke("sqlite:/a", "sess", "addr-1", 2_000)
            .expect("revoke"));
        assert_eq!(
            store.load(&a.id()).expect("load a").state,
            IntentRecoveryState::Revoked
        );
        assert_eq!(
            store.load(&b.id()).expect("load b").state,
            IntentRecoveryState::Live,
            "revoking one address must not touch a sibling address"
        );
        assert_eq!(
            store.load(&c.id()).expect("load c").state,
            IntentRecoveryState::Live,
            "revoking one store must not touch the same address in another store"
        );
        // Revocation bumps the generation so a concurrent CAS write loses.
        assert_eq!(store.load(&a.id()).expect("load a").generation, 2);
        let _ = std::fs::remove_dir_all(&run_dir);
    }

    #[test]
    fn generation_cas_refuses_a_stale_write() {
        let run_dir = temp_run_dir("cas");
        let store = IntentStore::open(&run_dir, "hash").expect("store");
        let intent = sample_intent("sqlite:/a", "sess", "addr");
        store.write_atomic(&intent).expect("write");
        let mut stale = intent.clone();
        stale.occupant = "stale".to_string();
        let mut advanced = intent.clone();
        advanced.generation = 2;
        store.write_atomic(&advanced).expect("advance");
        assert!(
            !store.write_cas(1, &stale).expect("cas"),
            "stale CAS must lose"
        );
        assert_eq!(store.load(&intent.id()).expect("load").occupant, "occupant");
        let mut fresh = advanced.clone();
        fresh.occupant = "fresh".to_string();
        fresh.generation = 3;
        assert!(store.write_cas(2, &fresh).expect("cas"));
        assert_eq!(store.load(&intent.id()).expect("load").occupant, "fresh");
        let _ = std::fs::remove_dir_all(&run_dir);
    }

    #[test]
    fn atomic_write_leaves_no_partial_file() {
        let run_dir = temp_run_dir("atomic");
        let store = IntentStore::open(&run_dir, "hash").expect("store");
        let intent = sample_intent("sqlite:/a", "sess", "addr");
        store.write_atomic(&intent).expect("write");
        let temps = std::fs::read_dir(store.root())
            .expect("scan")
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
            .count();
        assert_eq!(temps, 0);
        for id in store.list_ids().expect("list") {
            store.load(&id).expect("every listed intent parses");
        }
        let _ = std::fs::remove_dir_all(&run_dir);
    }

    #[test]
    fn write_cap_produces_a_typed_error_and_never_deletes() {
        let run_dir = temp_run_dir("cap");
        let store = IntentStore::open(&run_dir, "hash").expect("store");
        for i in 0..STATION_INTENT_MAX_COUNT {
            let intent = sample_intent("sqlite:/a", "sess", &format!("addr-{i}"));
            store.write_atomic(&intent).expect("write under cap");
        }
        let overflow = sample_intent("sqlite:/a", "sess", "one-too-many");
        assert!(matches!(
            store.write_atomic(&overflow),
            Err(IntentError::CapExceeded { .. })
        ));
        assert_eq!(
            store.list_ids().expect("list").len(),
            STATION_INTENT_MAX_COUNT
        );
        // Rewriting an *existing* intent still works while at the cap, so an over-cap scope can
        // always be revoked and GC'd back down.
        let mut existing = sample_intent("sqlite:/a", "sess", "addr-0");
        existing.generation = 2;
        store.write_atomic(&existing).expect("rewrite at cap");
        let _ = std::fs::remove_dir_all(&run_dir);
    }

    #[test]
    fn over_cap_scope_is_reported_swept_completely_and_never_pruned() {
        let run_dir = temp_run_dir("overcap");
        let store = IntentStore::open(&run_dir, "hash").expect("store");
        // Seed past the cap by writing directly, the way an older build or a manual copy would.
        let total = STATION_INTENT_MAX_COUNT + 88;
        for i in 0..total {
            let intent = sample_intent("sqlite:/a", "sess", &format!("addr-{i:04}"));
            let bytes = serde_json::to_vec_pretty(&intent).expect("encode");
            platform_fs::write_owner_only_file_atomic(&store.path_for(&intent.id()), &bytes)
                .expect("seed");
        }
        assert_eq!(store.list_ids().expect("list").len(), total);

        let mut seen = std::collections::BTreeSet::new();
        let budget = 64;
        let passes = total.div_ceil(budget);
        for _ in 0..passes {
            let page = store.scan(budget).expect("scan");
            assert!(page.over_cap, "an over-cap scope must report over_cap");
            assert_eq!(page.observed_count, total);
            // The store never advances the cursor itself: only the caller knows how far a pass
            // actually got. A sweep that attempted everything it loaded advances to its last
            // position.
            if let Some(last) = page.loaded_positions.last() {
                store.advance_cursor(last).expect("advance cursor");
            }
            for intent in page.loaded {
                seen.insert(intent.id());
            }
        }
        assert_eq!(seen.len(), total, "the cursor must give complete coverage");
        assert_eq!(
            store.list_ids().expect("list").len(),
            total,
            "scanning an over-cap scope must never delete an entry"
        );
        let _ = std::fs::remove_dir_all(&run_dir);
    }

    #[test]
    fn gc_removes_expired_pending_and_keeps_live() {
        let run_dir = temp_run_dir("gc");
        let store = IntentStore::open(&run_dir, "hash").expect("store");
        let credential = run_dir.join("cred.json");
        std::fs::write(&credential, b"{\"secret\":\"x\"}").expect("credential");

        let mut pending = sample_intent("sqlite:/a", "sess", "pending");
        pending.state = IntentRecoveryState::Pending;
        pending.updated_at_ms = 0;
        pending.producer.credential.path = credential.clone();
        store.write_atomic(&pending).expect("write pending");

        let mut live = sample_intent("sqlite:/a", "sess", "live");
        live.producer.credential.path = credential.clone();
        store.write_atomic(&live).expect("write live");

        let mut fresh_pending = sample_intent("sqlite:/a", "sess", "fresh-pending");
        fresh_pending.state = IntentRecoveryState::Pending;
        fresh_pending.updated_at_ms = 1_000_000;
        fresh_pending.producer.credential.path = credential.clone();
        store
            .write_atomic(&fresh_pending)
            .expect("write fresh pending");

        let report = store.gc(1_000_000, None, None).expect("gc");
        assert!(report.removed.contains(&pending.id()));
        assert!(!report.removed.contains(&live.id()));
        assert!(
            !report.removed.contains(&fresh_pending.id()),
            "a pending intent inside its TTL is a live attach in flight"
        );

        // An intent whose credential file vanished is GC-eligible immediately: it can never be
        // verified again, so keeping it only wastes a scan slot.
        std::fs::remove_file(&credential).expect("remove credential");
        let report = store.gc(1_000_001, None, None).expect("gc again");
        assert!(report.removed.contains(&live.id()));
        let _ = std::fs::remove_dir_all(&run_dir);
    }

    #[test]
    fn json_pointer_extraction_is_total() {
        let doc = serde_json::json!({"secret": "abc", "nested": {"a/b": "slash"}, "list": ["x"]});
        assert_eq!(json_pointer_str(&doc, "/secret"), Some("abc"));
        assert_eq!(json_pointer_str(&doc, "/nested/a~1b"), Some("slash"));
        assert_eq!(json_pointer_str(&doc, "/list/0"), Some("x"));
        assert_eq!(json_pointer_str(&doc, "/missing"), None);
        assert_eq!(json_pointer_str(&doc, "/nested"), None);
        assert_eq!(json_pointer_str(&doc, "/list/9"), None);
    }

    #[test]
    fn an_intent_never_contains_secret_material() {
        let intent = sample_intent("sqlite:/a", "sess", "addr");
        let encoded = serde_json::to_string(&intent).expect("encode");
        assert!(!encoded.contains("\"secret\":"));
        // The credential is a pointer, not a value.
        assert!(encoded.contains("\"pointer\":\"/secret\""));
    }
}
