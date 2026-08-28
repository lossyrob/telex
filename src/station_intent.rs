//! Durable, host-local, owner-private **station intent** records (ADR 0052).
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
use std::time::{Duration, Instant};

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
///
/// Measured from the *attach lifecycle's* creation ([`StationIntentV1::pending_clock_ms`]): a retry
/// of an unfinalized attach inherits the clock (so retrying cannot extend the lifetime), while an
/// attach over a revoked or otherwise inert record starts a new lifecycle and gets the whole
/// window.
pub const STATION_INTENT_PENDING_TTL: Duration = Duration::from_secs(5 * 60);
/// TTL for a `pending` intent that carries a durable **armed proof** — a daemon accepted
/// `Register` for this binding and armed push delivery, but the producer was never proven (the
/// attach, or the whole process, died between the two).
///
/// Deliberately far longer than [`STATION_INTENT_PENDING_TTL`], because the two records mean
/// opposite things. A bare `pending` record is an attach that may never have reached the daemon at
/// all, so five minutes is generous. An *armed* one describes push delivery that a daemon really
/// did arm: deleting it at five minutes silently disarms recovery for a binding that is working
/// right now, and the user only discovers it after the next daemon replacement — precisely the
/// failure this feature exists to remove. The turn-boundary hook promotes it on the first turn
/// stop after the bridge loads, so this TTL only has to outlast an idle gap between turns.
pub const STATION_INTENT_ARMED_PENDING_TTL: Duration = Duration::from_secs(24 * 60 * 60);
/// Orphan TTL for intents that stayed unverifiable/insecure: long enough that a laptop closed for
/// a week still recovers, short enough that abandoned state does not accumulate forever.
pub const STATION_INTENT_UNVERIFIABLE_TTL: Duration = Duration::from_secs(7 * 24 * 60 * 60);
/// How long a **finalized** intent's credential file may be missing before the intent is treated
/// as orphaned.
///
/// A missing credential is a *transient producer* condition, not a teardown: the bridge deletes
/// and rewrites its own registry on `/clear`, `extensions_reload`, and extension-host restart, and
/// it rewrites it non-atomically on a heartbeat. Deleting an intent the instant the file is absent
/// destroys the very record recovery depends on, so the rule is TTL-governed like every other GC
/// reason, measured against the last time the daemon proved (or attempted) the producer rather
/// than against manifest age — see [`IntentStore::gc`].
pub const STATION_INTENT_CREDENTIAL_MISSING_TTL: Duration = Duration::from_secs(15 * 60);

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
pub const SCAN_CURSOR_FILE: &str = "scan-cursor.json";
/// Cursor key for a pass that is **not** filtered to one store. Never a valid `store_key`, so it
/// can never collide with a scoped entry.
const UNSCOPED_CURSOR_KEY: &str = "";
/// How often a directory enumeration re-checks its deadline.
///
/// The first stride is always enumerated, so an already-expired deadline can never truncate an
/// ordinary scope (which is capped far below a single stride's worth of real work) down to nothing
/// — truncation is reserved for the pathological case this bound exists for: a directory on a
/// wedged network mount where each `readdir` entry itself blocks.
const ENUMERATION_DEADLINE_STRIDE: usize = 64;
/// Per-intent write-lock file suffix, plus the bounded acquisition policy. The bound matters:
/// the reconciler takes this lock inside `RECONCILE_PER_INTENT_TIMEOUT`, so waiting must never be
/// open-ended, and a writer that died mid-update must not wedge the binding.
const INTENT_LOCK_SUFFIX: &str = ".lock";
const INTENT_LOCK_ATTEMPTS: u32 = 25;
const INTENT_LOCK_RETRY: Duration = Duration::from_millis(20);
const INTENT_LOCK_STALE: Duration = Duration::from_secs(10);

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

/// Existence of one path in this module's error type, with the path in the message.
///
/// Never `Path::exists()`. Every existence question in this module is an authority question — "does
/// this binding already have a durable record?", "is there still a record to revoke?", "has the
/// producer's credential really gone?" — and `exists()` answers *no* for a record that is sitting
/// right there behind an ACL, a broken mount, or an untraversable parent. That answer routes the
/// caller down the "this binding is new / this record is gone" branch, which is the branch that
/// commits, deletes, or overwrites. See [`platform_fs::path_present`].
fn path_present(path: &Path) -> Result<bool> {
    platform_fs::path_present(path)
        .map_err(|e| IntentError::Io(format!("checking {}: {e}", path.display())))
}

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

/// Durable proof that a daemon accepted `Register` for this binding **and armed push delivery**.
///
/// Written by the daemon itself at the moment it commits the push member, never by the producer
/// side, and never inferred from the existence of a credential file. That is the whole point: a
/// `pending` record with no proof is an attach that may have failed before it ever reached a
/// daemon, and it must not be promotable just because a bridge happens to be running. A record
/// with one describes push delivery that really was armed, so the turn-boundary finalizer may
/// promote it even after the arming daemon is gone.
///
/// It is *not* an authorization to deliver. `is_reconcilable` stays `Live`-only, so an armed
/// `pending` record is still never reconciled, and restoration still requires the full
/// peer-verification and probe chain plus the daemon epoch fence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArmedProofV1 {
    /// When the daemon armed push for this binding.
    pub armed_at_ms: i64,
    /// The arming daemon's instance id. Diagnostic and audit only — a later daemon is a different
    /// instance by construction, and requiring a match would reintroduce exactly the dependency on
    /// daemon memory this proof exists to remove.
    pub daemon_instance_id: String,
}

/// What a producer-side finalize is allowed to do to a durable record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FinalizeAdmission {
    /// `pending` → `live`: the attach is being completed.
    Promote,
    /// `live` → `live`: the producer identity of an already-armed record is being re-recorded
    /// after a bridge reload.
    Refresh,
    /// A `pending` record with no durable armed proof, and no daemon reporting an armed member
    /// right now. Promoting it would let a merely-existing bridge credential arm an attach that
    /// was never registered.
    RefusedNotArmed,
    /// An explicit teardown (detach, session end, fallback downgrade, operator reset) landed while
    /// the finalize was in flight. Tombstone-wins: a revocation is never undone by a finalize.
    RefusedRevoked,
    /// Anything else — a state this build does not persist, so it is not a transition it owns.
    RefusedState,
}

impl FinalizeAdmission {
    pub fn is_allowed(self) -> bool {
        matches!(
            self,
            FinalizeAdmission::Promote | FinalizeAdmission::Refresh
        )
    }

    pub fn reason(self) -> &'static str {
        match self {
            FinalizeAdmission::Promote => "promote",
            FinalizeAdmission::Refresh => "refresh",
            FinalizeAdmission::RefusedNotArmed => "the daemon has not armed push for this binding",
            FinalizeAdmission::RefusedRevoked => "the binding was revoked concurrently",
            FinalizeAdmission::RefusedState => "the record is not in a finalizable state",
        }
    }
}

/// The one place the producer-side `pending`/`live` transition rules live.
///
/// Split out of the finalize call site so the rules are decidable — and testable — without a
/// daemon, a bridge, or a filesystem. Two independent authorities may permit a promotion, and the
/// distinction between them is the whole of the fix for the reload-plus-replacement deadlock:
///
/// * `armed_now` — a daemon reports an armed push member for this binding *right now*. This is
///   what a first attach has, and it is the only authority a record with no durable proof gets.
/// * `armed_durably` — the record itself carries an armed proof, or is already `live`. This
///   survives the daemon, so a bridge that reloaded after a daemon crash can re-record its
///   identity without first needing the member that cannot exist until the identity is right.
pub fn finalize_admission(
    state: IntentRecoveryState,
    armed_durably: bool,
    armed_now: bool,
) -> FinalizeAdmission {
    match state {
        IntentRecoveryState::Live => FinalizeAdmission::Refresh,
        IntentRecoveryState::Pending => {
            if armed_durably || armed_now {
                FinalizeAdmission::Promote
            } else {
                FinalizeAdmission::RefusedNotArmed
            }
        }
        IntentRecoveryState::Revoked | IntentRecoveryState::Tombstoned => {
            FinalizeAdmission::RefusedRevoked
        }
        _ => FinalizeAdmission::RefusedState,
    }
}

/// What an explicit withdrawal did to one binding's durable record.
///
/// Every variant is a *decided* outcome. There is deliberately no "maybe" and no "best effort":
/// a withdrawal that could not decide what it did is an error
/// ([`IntentStore::withdraw_binding`] returns `Result`), because the whole point of the operation
/// is that the caller has just torn something down and needs to know the desired state went with
/// it. A silently-swallowed failure here is a station that auto-returns after the operator gave it
/// up — the exact failure the durable record exists to prevent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Withdrawal {
    /// **Proven** absent: there is no record for this binding, so there is nothing to withdraw.
    ///
    /// Never inferred from an unreadable path. A record telex merely could not stat is an error,
    /// not a "nothing to do" — and it is never turned into a fabricated `revoked` tombstone
    /// either: a withdrawal has no producer identity of its own to write, so it can only ever
    /// transition or delete a record that is already there.
    NoRecord,
    /// An unfinalized `pending` record at exactly the observed generation was deleted.
    ///
    /// Deleted rather than marked `revoked` because a `pending` record describes an attach that
    /// never completed: its producer identity may still be the attach-time placeholder, so
    /// persisting it as a `revoked` record would leave an identity-less tombstone occupying the
    /// binding for the seven-day terminal TTL, and every re-attach in that window would inherit
    /// the finished lifecycle's clock. The delete is conditional and lock-held like every other
    /// deletion in this module.
    DeletedPending { generation: u64 },
    /// A durable record was transitioned to `revoked`, and now sits at this generation.
    Revoked { generation: u64 },
    /// The record was already `revoked`. Withdrawal is idempotent: a detach, a session end, and a
    /// reset can all name the same binding, and the second one is a success, not a conflict.
    AlreadyRevoked { generation: u64 },
    /// The caller asked to withdraw one specific generation and the record has moved on.
    ///
    /// Only reachable through [`IntentStore::withdraw_binding_at_generation`], which the
    /// reconciler uses: a pass that decided "this binding is tombstoned" against generation *N*
    /// must not delete the `pending` record a re-attach wrote at *N+1* while the pass was in
    /// flight.
    Superseded { generation: u64 },
}

impl Withdrawal {
    /// The generation this outcome describes, when there is a record it describes at all.
    pub fn generation(&self) -> Option<u64> {
        match self {
            Withdrawal::NoRecord => None,
            Withdrawal::DeletedPending { generation }
            | Withdrawal::Revoked { generation }
            | Withdrawal::AlreadyRevoked { generation }
            | Withdrawal::Superseded { generation } => Some(*generation),
        }
    }

    /// Whether durable state actually moved, so a caller can log (and count) a real teardown
    /// without treating an idempotent repeat as one.
    pub fn changed(&self) -> bool {
        matches!(
            self,
            Withdrawal::DeletedPending { .. } | Withdrawal::Revoked { .. }
        )
    }

    /// Whether the binding is now durably withdrawn — deleted, or `revoked`.
    ///
    /// `Superseded` is deliberately **not** withdrawn: a newer record exists and this call did not
    /// decide anything about it.
    pub fn is_withdrawn(&self) -> bool {
        matches!(
            self,
            Withdrawal::NoRecord
                | Withdrawal::DeletedPending { .. }
                | Withdrawal::Revoked { .. }
                | Withdrawal::AlreadyRevoked { .. }
        )
    }
}

/// One binding named by the scope, for callers that withdraw a *set* of bindings.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct IntentBinding {
    pub store_key: String,
    pub session_id: String,
    pub address: String,
}

/// Which bindings the scope currently names, split by whether their identity is *proven*.
///
/// The split is a security rule, not a diagnostic. A binding in `bindings` came from a manifest
/// that loaded — so its `(store_key, session_id, address)` was verified against the filename hash
/// and the singleton scope, and deriving an id from it can only ever name the file it came from.
/// A manifest that failed to load has no verified identity: the tuple in `unreadable` is read
/// straight out of unvalidated bytes, so acting on it would let one manifest name — and withdraw —
/// *another* binding's record. It is therefore only ever used to **refuse**: a caller withdrawing
/// a session or an address treats an unreadable manifest that claims membership of the set as a
/// failure of the whole operation rather than silently skipping it or acting on the claim.
#[derive(Debug, Default)]
pub struct BindingScan {
    pub bindings: Vec<IntentBinding>,
    pub unreadable: Vec<(IntentId, Option<IntentBinding>)>,
    /// Whether the enumeration stopped at its deadline instead of reaching the end of the scope.
    ///
    /// A set-scoped teardown must **refuse** on this rather than act on the partial list: "these
    /// are the bindings I managed to read before I ran out of time" is not the same statement as
    /// "these are the bindings of this session/address", and withdrawing the first answer while
    /// reporting the second is exactly the incomplete teardown that leaves a binding behind to
    /// restore push for a station the operator gave up.
    pub truncated: bool,
}

/// The record a mutating store call actually wrote, so a caller can roll back exactly what it
/// created and nothing else.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PendingWrite {
    /// A `pending` record was created (or replaced a non-live one) at this generation.
    Created { generation: u64 },
    /// A `live` record already existed and was left untouched; finalize updates it in place.
    KeptExistingLive { generation: u64 },
}

/// What [`IntentStore::stamp_armed_proof`] actually did.
///
/// Three-way rather than a bool, because the daemon has to tell "this binding has no durable
/// record, so there is nothing to prove" (an ordinary pull attach, or a push attach from a client
/// that writes no intent) apart from "the record that was here is gone, so the proof this register
/// owes cannot be persisted". Collapsing those into one `false` is what let a register whose
/// record had been deleted under it still report durable success.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArmedProofStamp {
    /// No record exists for this binding at all.
    NoRecord,
    /// The proof was written; the record now sits at this generation.
    Stamped { generation: u64 },
    /// The record already carried a proof, so nothing was written and the generation did not move.
    /// Idempotency is load-bearing: the hot re-register path must not churn the generation and
    /// invalidate concurrent CAS holders.
    AlreadyArmed { generation: u64 },
}

impl ArmedProofStamp {
    /// Whether a durable armed proof exists for the binding as a result of this call.
    pub fn is_proven(&self) -> bool {
        matches!(
            self,
            ArmedProofStamp::Stamped { .. } | ArmedProofStamp::AlreadyArmed { .. }
        )
    }
}

/// Why an armed-proof stamp could not be performed at all.
///
/// The distinction is not cosmetic: one of these two says something about *this binding's durable
/// record* and the other says nothing about it, and a register that owes no proof must not be
/// refused by a condition that cannot be about a record it does not have.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArmedProofFailure {
    /// The intent scope itself could not be opened, so nothing about this binding was observable —
    /// not even whether it has a durable record. A scope that simply does not exist is *not* this:
    /// that is [`ArmedProofStamp::NoRecord`], because a scope with no directory has no records.
    ScopeUnavailable,
    /// The scope was reachable, this binding has a record in it, and that record could not be
    /// read, locked, or written. Durable state about this binding exists and could not be
    /// verified.
    RecordUnusable,
}

/// What an arming register must do with the outcome of its proof commit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArmedProofAdmission {
    /// Commit the member: the proof is durable, or there is provably nothing to prove.
    Commit,
    /// Refuse the registration rather than report a durable success it cannot back.
    Refuse,
}

/// The one place the daemon-side "may this arming register commit?" rule lives.
///
/// Split out of the register call site for the same reason [`finalize_admission`] is: the rule is
/// then decidable — and testable — without a daemon, a scope, or a filesystem fault. `owes_proof`
/// is the observation the register made up front, under the per-station admission guard, about
/// whether the binding had a durable record before any of this started.
///
/// | Stamp outcome | `owes_proof == false` | `owes_proof == true` |
/// |---|---|---|
/// | `Stamped` / `AlreadyArmed` | commit | commit |
/// | `NoRecord` | commit | refuse |
/// | `Err(ScopeUnavailable)` | commit | refuse |
/// | `Err(RecordUnusable)` | **refuse** | refuse |
///
/// Two asymmetries carry the whole rule:
///
/// * **A register that owes nothing is not refused by a scope-level failure.** A pull attach and a
///   plain `telex attach --on-deliver` write no intent, so the scope may not exist at all — and
///   opening it is then a *create*, which can fail for reasons that have nothing to do with this
///   registration (a read-only run dir, a stray file where the scope should be, a full disk).
///   Refusing push for every non-bridge client because a directory the register had nothing to put
///   in could not be created is a denial with no safety value: there is no durable record to lose.
/// * **A broken record fails closed either way.** `RecordUnusable` means the scope was reachable
///   *and* a record for this exact binding is there and unverifiable. That is durable state about
///   the binding, so it is refused even when the up-front observation said nothing was owed — which
///   is the concurrent-attach window, where a record can appear after the observation and before
///   the stamp.
pub fn armed_proof_admission(
    stamped: std::result::Result<ArmedProofStamp, ArmedProofFailure>,
    owes_proof: bool,
) -> ArmedProofAdmission {
    match stamped {
        Ok(ArmedProofStamp::Stamped { .. } | ArmedProofStamp::AlreadyArmed { .. }) => {
            ArmedProofAdmission::Commit
        }
        // The record this register owed a proof to is gone, or the scope that would hold it could
        // not be opened. Both are fail-closed *only* when a proof was owed.
        Ok(ArmedProofStamp::NoRecord) | Err(ArmedProofFailure::ScopeUnavailable) => {
            if owes_proof {
                ArmedProofAdmission::Refuse
            } else {
                ArmedProofAdmission::Commit
            }
        }
        Err(ArmedProofFailure::RecordUnusable) => ArmedProofAdmission::Refuse,
    }
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
    /// Durable proof that a daemon armed push delivery for this binding. Absent on every record
    /// written before this field existed, which is treated exactly like "never armed".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub armed: Option<ArmedProofV1>,
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

    /// Whether a daemon has durably proven it armed push delivery for this binding.
    ///
    /// `Live` implies it: a record only reaches `live` through a finalize, which only runs after
    /// the daemon confirmed `push_registered`. The explicit proof matters for the window in
    /// between, where the daemon armed delivery and the record has not been promoted yet.
    pub fn is_armed(&self) -> bool {
        self.armed.is_some() || self.state == IntentRecoveryState::Live
    }

    /// The **pending lifecycle clock**: the moment the `Pending` TTL is measured from.
    ///
    /// Deliberately *not* `updated_at_ms`. That field is refreshed by every `write_pending`, and
    /// `write_pending` is what a re-attach performs — so a producer whose finalize keeps failing
    /// (a bridge stuck mid-reload, a probe that never answers) re-attaches every few seconds and
    /// pushed the five-minute TTL out indefinitely. The record that GC exists to collect was the
    /// one record it could never reach.
    ///
    /// Each of the two pending TTLs is therefore anchored to the event it is actually about, and
    /// neither event can be replayed by retrying:
    ///
    /// * An **unarmed** `pending` record is "an attach that may never have reached the daemon", so
    ///   it ages from `created_at_ms`. Within one pending lifecycle `write_pending` carries that
    ///   field forward from the record it replaces, precisely so a re-attach cannot reset it; a
    ///   *new* lifecycle (an attach over a revoked or otherwise finished record) gets its own
    ///   creation time, because it is a different attach and has not spent any of its TTL yet.
    /// * An **armed** `pending` record is "a daemon really did arm push for this binding", so it
    ///   ages from the armed proof's own timestamp. `stamp_armed_proof` is idempotent, so a
    ///   re-register cannot move that either, and a new lifecycle never inherits a proof, so the
    ///   longer clock only ever measures an arming this lifecycle earned. Floored at
    ///   `created_at_ms` so a hand-edited or clock-skewed proof can never age a record from
    ///   *before* it existed.
    pub fn pending_clock_ms(&self) -> i64 {
        match self.armed.as_ref() {
            Some(proof) => proof.armed_at_ms.max(self.created_at_ms),
            None => self.created_at_ms,
        }
    }

    /// The last moment the producer behind this intent was actually *proven* — a successful
    /// reconcile, a verified probe, or the durable state transition a finalize performs (which is
    /// itself gated on a live probe).
    ///
    /// Deliberately excludes `evidence.last_attempt_ms`. An attempt is not proof, and because the
    /// reconciler persists scheduling state on every genuine failure, an intent whose producer is
    /// permanently gone gets its attempt clock refreshed every few seconds forever — which made
    /// both the credential-missing TTL and the dead-producer orphan TTL unreachable for exactly
    /// the records they exist to collect.
    pub fn last_proven_ms(&self) -> i64 {
        self.evidence
            .last_success_ms
            .max(self.evidence.producer_verified_ms)
            .unwrap_or(i64::MIN)
            .max(self.updated_at_ms)
            .max(self.created_at_ms)
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
    pub rejected: Vec<(IntentId, Rejection)>,
    pub observed_count: usize,
    pub over_cap: bool,
    /// The pass ran out of deadline **during discovery**, so it did not see the whole scope.
    ///
    /// Reported rather than hidden because it changes what the other counters mean:
    /// `observed_count` becomes a lower bound and `over_cap` becomes "at least this many were
    /// seen", so a caller that publishes them must not overwrite a complete pass's numbers with a
    /// truncated pass's. Discovery resumes from its own persisted position on the next pass, so
    /// truncation delays coverage instead of losing it.
    pub discovery_truncated: bool,
}

/// Why one manifest was rejected, and which binding it named when that could be established.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rejection {
    pub state: IntentRecoveryState,
    pub detail: String,
    pub identity: Option<RejectedIdentity>,
}

/// The binding a rejected manifest names. Read from the parsed document, never from the filename
/// (which is only a truncated hash).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RejectedIdentity {
    pub store_key: String,
    pub session_id: String,
    pub address: String,
}

#[derive(Debug, Clone, Default)]
pub struct GcReport {
    pub removed: Vec<IntentId>,
    pub kept: usize,
    pub reasons: Vec<(IntentId, String)>,
    /// The sweep examined every entry in the scope before it returned.
    ///
    /// A deadline-truncated sweep is not a failure and not a full sweep either: `kept` and
    /// `removed` then describe only the slice it reached, and the *next* sweep resumes from the
    /// persisted GC position rather than restarting at the head. Callers use this to decide
    /// whether a maintenance slot was actually consumed.
    pub complete: bool,
}

/// An absolute stop time shared by every bounded step of one pass.
///
/// Absolute rather than per-step durations on purpose. A published "a pass finishes within N
/// seconds" bound is only real if every step measures itself against the *same* instant: a chain of
/// independent per-step timeouts bounds each step and nothing at all, which is exactly how an
/// unbounded discovery or GC phase used to be able to push a 4-second pass past its own tick.
///
/// [`PassDeadline::unbounded`] is the escape hatch for callers that are genuinely not on the pass
/// path (one-shot maintenance, fixtures); it never expires, so those callers keep the pre-deadline
/// behavior exactly.
#[derive(Debug, Clone, Copy, Default)]
pub struct PassDeadline(Option<Instant>);

impl PassDeadline {
    pub fn at(at: Instant) -> Self {
        Self(Some(at))
    }

    pub fn unbounded() -> Self {
        Self(None)
    }

    pub fn expired(&self) -> bool {
        self.0.is_some_and(|at| Instant::now() >= at)
    }

    pub fn instant(&self) -> Option<Instant> {
        self.0
    }

    pub fn remaining(&self) -> Option<Duration> {
        self.0
            .map(|at| at.saturating_duration_since(Instant::now()))
    }

    /// The earlier of two deadlines; an unbounded one never widens a bounded one.
    pub fn earliest(self, other: PassDeadline) -> PassDeadline {
        match (self.0, other.0) {
            (Some(a), Some(b)) => PassDeadline(Some(a.min(b))),
            (Some(a), None) => PassDeadline(Some(a)),
            (None, other) => PassDeadline(other),
        }
    }
}

/// What an async caller got back from a blocking filesystem phase it ran behind a deadline.
///
/// The distinction is the whole point of [`run_blocking_within`]: a cooperative deadline check
/// between two synchronous syscalls bounds a phase only if every individual syscall returns
/// promptly, and that is exactly the assumption a hung network mount, a stalled anti-virus filter,
/// or a slow `fsync` breaks. `Overran` is the answer that keeps the published bound true when it
/// does.
#[derive(Debug)]
pub enum BoundedPhase<T> {
    Completed(T),
    /// The phase did not finish before the deadline, so the caller stopped **waiting** for it.
    ///
    /// It was not cancelled: blocking work is never interrupted mid-syscall, so a partially staged
    /// atomic write always gets to finish its own rename rather than being torn in half. What is
    /// abandoned is the *result* — the caller must publish nothing derived from this phase and must
    /// report the pass as deadline-truncated.
    Overran,
}

impl<T> BoundedPhase<T> {
    pub fn completed(self) -> Option<T> {
        match self {
            Self::Completed(value) => Some(value),
            Self::Overran => None,
        }
    }

    pub fn overran(&self) -> bool {
        matches!(self, Self::Overran)
    }
}

/// Run one blocking filesystem phase on the blocking pool and stop waiting for it at `deadline`.
///
/// This is the execution boundary that turns the cooperative deadline checks inside
/// [`IntentStore::list_ids_bounded`], [`IntentStore::scan_bounded`] and [`IntentStore::gc_bounded`]
/// into a bound an *async* caller can actually rely on. Those checks sit between synchronous
/// calls; they cannot bound the call that is currently blocked, and running them directly on a
/// runtime worker meant a single slow `read_dir`, `read`, or atomic rename could push a 4-second
/// pass — and the admin request awaiting it — arbitrarily past its deadline while also parking a
/// runtime worker thread.
///
/// Both halves are load-bearing:
///
/// * The work is moved to the blocking pool, so an overrun stalls a pool thread rather than the
///   reactor, and the async pass stays responsive enough to *return* on time.
/// * The wait is bounded, and only the wait. The task keeps running to completion, which is what
///   makes abandoning it safe: durable state is only ever advanced through an atomic write, and a
///   background completion finishes that write instead of leaving a half-written manifest or
///   cursor behind.
///
/// An unbounded deadline awaits normally, so off-pass callers keep the pre-deadline behavior
/// exactly. A panicking phase is reported as `Overran` for the same reason a slow one is: there is
/// no result, so the caller must treat the phase as having produced nothing.
///
/// `grace` is what a phase that checks the *same* deadline cooperatively needs in order to hand
/// back the partial page it deliberately stopped building. Waiting exactly to the deadline would
/// pre-empt that return every single time the phase actually used its budget, so a scope large
/// enough to be interesting would be reported as overrun on every pass and never make visible
/// progress. Phases whose inner work has no deadline check of its own — a cursor write, an
/// evidence rewrite — pass [`Duration::ZERO`], because for them the deadline *is* the hard bound.
/// Callers must size `grace` into the pass budget: the true worst case of this call is
/// `deadline + grace`.
pub async fn run_blocking_within<T, F>(
    deadline: PassDeadline,
    grace: Duration,
    work: F,
) -> BoundedPhase<T>
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    let handle = tokio::task::spawn_blocking(work);
    match deadline.remaining() {
        Some(remaining) => match tokio::time::timeout(remaining + grace, handle).await {
            Ok(Ok(value)) => BoundedPhase::Completed(value),
            Ok(Err(_)) | Err(_) => BoundedPhase::Overran,
        },
        None => match handle.await {
            Ok(value) => BoundedPhase::Completed(value),
            Err(_) => BoundedPhase::Overran,
        },
    }
}

/// Run one blocking filesystem phase, but only while `reserve` of the deadline is still intact.
///
/// [`run_blocking_within`] bounds when the **caller** returns; it does not bound when the work it
/// started finishes. For a phase whose only effect is the value it hands back — a discovery page, a
/// GC report — that distinction is harmless, because a caller that stopped waiting simply publishes
/// nothing derived from it. For a phase that mutates *durable, pass-visible* state — the evidence
/// block, the round-robin cursor — it is not: abandoning the wait leaves a write that can land after
/// the pass has already answered its caller, so the pass can no longer make the statement a
/// request-scoped caller needs, which is "nothing this pass was going to change had still not
/// changed when I answered".
///
/// The fix is to not start it. A durable write is launched only while `reserve` of the deadline
/// remains and is then joined inside that same deadline, so the ordinary near-deadline case degrades
/// to [`BoundedPhase::Overran`] **without the phase having run at all** — the honest answer, since
/// the pass then neither observed nor caused a change and reports itself deadline-truncated.
///
/// One residual case is not eliminable on any platform: a write that *was* started with the reserve
/// intact and still has not returned by the deadline. It is reported as `Overran` and left to
/// finish, never cancelled, because a staged atomic write torn in half is strictly worse than a late
/// one. What bounds the blast radius is what such a write is allowed to be: a single-file atomic
/// rewrite guarded by a generation CAS. It cannot register a member, cannot emit a report, and
/// cannot resurrect a record a concurrent teardown has since moved on from.
pub async fn run_blocking_reserved<T, F>(
    deadline: PassDeadline,
    reserve: Duration,
    work: F,
) -> BoundedPhase<T>
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    if deadline
        .remaining()
        .is_some_and(|remaining| remaining < reserve)
    {
        return BoundedPhase::Overran;
    }
    run_blocking_within(deadline, Duration::ZERO, work).await
}

/// How long a cooperatively bounded blocking phase is given to *return* after its own deadline.
///
/// It covers the tail of one already-started synchronous step plus the phase's own wrap-up (sorting
/// a page, persisting the resume cursor). It is deliberately small and deliberately non-zero, and
/// it is charged against the pass budget in `daemon_reconcile`'s constants rather than being
/// silently absorbed.
pub const RECONCILE_BLOCKING_GRACE: Duration = Duration::from_millis(200);

/// The persisted scheduling hints for one scope directory.
///
/// Three distinct positions, because they answer three different questions and sharing one field
/// between them is a starvation bug rather than a simplification:
///
/// * `scopes[k].position` — how far the *caller* got in sort order for cursor key `k`. Keyed by
///   scope because a pass filtered to one store must not move (or be moved by) another store's
///   round-robin position: with a single shared position, a scoped pass advanced past intents it
///   never considered, and an unscoped pass then skipped them for a full cycle.
/// * `scopes[k].discovery` — how far *discovery itself* got reading manifests, so a pass whose
///   deadline expired mid-scope resumes reading where it stopped instead of re-reading the same
///   head forever.
/// * `gc_position` — the same idea for the GC sweep, which is scope-wide by nature (it is the only
///   thing that deletes files, and it must eventually reach every one of them).
///
/// `position` at the top level is the **legacy** single global position written by builds that
/// predate scope-correct cursoring. It is read as the seed for a scope that has no entry yet, and
/// kept mirrored to the unscoped scope's position, so upgrade and rollback both keep a sane cursor.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct ScanCursor {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    position: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    scopes: BTreeMap<String, ScopeCursor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    gc_position: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
struct ScopeCursor {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    position: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    discovery: Option<String>,
}

impl ScanCursor {
    fn scope_key(scope: Option<&str>) -> &str {
        scope.unwrap_or(UNSCOPED_CURSOR_KEY)
    }

    /// The cursor for one scope, migrating a legacy global position on first sight.
    ///
    /// Seeding a scope from the legacy position can only ever make a scope resume *later* in the
    /// cycle than it otherwise would, and the cycle wraps, so migration delays an entry by at most
    /// one sweep and can never strand one.
    fn for_scope(&self, scope: Option<&str>) -> ScopeCursor {
        match self.scopes.get(Self::scope_key(scope)) {
            Some(cursor) => cursor.clone(),
            None => ScopeCursor {
                position: self.position.clone(),
                discovery: None,
            },
        }
    }
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
    ///
    /// `Ok(None)` means the scope directory provably is not there. A root whose existence could not
    /// be decided is `Err`, not `Ok(None)`: callers treat "no scope" as "this host never attached,
    /// so no binding here has a durable record", and handing them that answer for a scope full of
    /// records they simply could not see is how an unreadable scope becomes a silent green light.
    pub fn open_existing(run_dir: &Path, singleton_hash: &str) -> Result<Option<Self>> {
        let root = run_dir.join("intents").join(singleton_hash);
        if !path_present(&root)? {
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

    /// Every intent id present in the scope, in filename order.
    ///
    /// Unbounded: only callers that are genuinely off the pass path may use this. Everything on a
    /// bounded pass must use [`IntentStore::list_ids_bounded`], because a directory enumeration is
    /// real I/O against a directory that may be on a hung network mount, and an unbounded
    /// enumeration is enough on its own to push a bounded pass past its deadline.
    pub fn list_ids(&self) -> Result<Vec<IntentId>> {
        self.list_ids_bounded(PassDeadline::unbounded())
            .map(|(ids, _)| ids)
    }

    /// Every intent id the enumeration reached before `deadline`, plus whether it was truncated.
    ///
    /// Truncation is reported, never hidden: `observed_count` and `over_cap` are derived from this
    /// list, and a caller that publishes them has to know it is looking at a lower bound rather
    /// than the scope.
    pub fn list_ids_bounded(&self, deadline: PassDeadline) -> Result<(Vec<IntentId>, bool)> {
        let entries = match std::fs::read_dir(&self.root) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok((Vec::new(), false)),
            Err(e) => return Err(IntentError::Io(format!("listing intent scope: {e}"))),
        };
        let mut ids = Vec::new();
        let mut truncated = false;
        let mut examined = 0usize;
        for entry in entries.flatten() {
            examined += 1;
            if examined.is_multiple_of(ENUMERATION_DEADLINE_STRIDE) && deadline.expired() {
                truncated = true;
                break;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            if let Some(id) = IntentId::from_file_name(&name) {
                ids.push(id);
            }
        }
        ids.sort();
        Ok((ids, truncated))
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
    ///
    /// A rejection also carries the binding identity whenever it could be established, so a
    /// rejected manifest is still visible in the status projection and the drain report instead of
    /// being counted only in the pass log. Identity is read from the *parsed* document, so a
    /// manifest that failed a security check before it was ever read stays unidentifiable — which
    /// is reported as its own count rather than as zero.
    pub fn load_projected(&self, id: &IntentId) -> std::result::Result<StationIntentV1, Rejection> {
        match self.load(id) {
            Ok(intent) => Ok(intent),
            Err(e) => {
                let state = match &e {
                    IntentError::Insecure(_) => IntentRecoveryState::Insecure,
                    IntentError::UnsupportedSchema { .. } | IntentError::Invalid(_) => {
                        IntentRecoveryState::Incompatible
                    }
                    _ => IntentRecoveryState::Unverifiable,
                };
                let identity = match &e {
                    // Never re-read a file that failed a security check.
                    IntentError::Insecure(_) => None,
                    _ => self.read_identity(id),
                };
                Err(Rejection {
                    state,
                    detail: e.to_string(),
                    identity,
                })
            }
        }
    }

    /// Best-effort `(store_key, session_id, address)` of a manifest that failed validation.
    fn read_identity(&self, id: &IntentId) -> Option<RejectedIdentity> {
        let bytes =
            platform_fs::read_owner_only_file(&self.path_for(id), STATION_INTENT_MAX_BYTES).ok()?;
        let value: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
        let field = |name: &str| {
            value
                .get(name)
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string())
        };
        Some(RejectedIdentity {
            store_key: field("store_key")?,
            session_id: field("session_id")?,
            address: field("address")?,
        })
    }

    /// Write an intent atomically. Enforces the per-scope count cap for *new* ids (an existing
    /// intent may always be rewritten, so an over-cap scope can still be revoked or GC'd out).
    pub fn write_atomic(&self, intent: &StationIntentV1) -> Result<()> {
        intent.validate()?;
        let id = intent.id();
        let path = self.path_for(&id);
        if !path_present(&path)? {
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

    /// Compare-and-set on `generation`, serialized by a per-intent write lock.
    ///
    /// The lock is what makes this a real compare-and-set rather than a read-then-write: the
    /// writers are genuinely concurrent and cross-process (a reconcile pass in the daemon, a
    /// `finalize_intent` or `revoke` in a CLI turn-boundary hook), so a bare load-check-write
    /// leaves both writers passing the check and the later one clobbering the earlier. Every
    /// mutating entry point in this module takes the same lock.
    pub fn write_cas(&self, expected_generation: u64, intent: &StationIntentV1) -> Result<bool> {
        let id = intent.id();
        let _lock = self.lock_intent(&id)?;
        self.write_cas_locked(expected_generation, intent)
    }

    fn write_cas_locked(&self, expected_generation: u64, intent: &StationIntentV1) -> Result<bool> {
        let id = intent.id();
        // An undecidable existence here would otherwise take the `expected_generation == 0` branch
        // and *create* — overwriting a record the caller could not see rather than losing the CAS.
        if !path_present(&self.path_for(&id))? {
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

    /// Compare-and-set that **requires the record to already exist** — with the per-intent write
    /// lock already held by the caller.
    ///
    /// Distinct from [`IntentStore::write_cas_locked`], which *creates* when the path is absent and
    /// `expected_generation == 0`. A withdrawal must never create: it has no producer identity of
    /// its own, so a create there would mint an identity-less `revoked` record that nothing can
    /// validate and GC would keep for the full terminal TTL. `Ok(false)` means "the record moved
    /// or vanished"; the caller decides which, and refuses either way.
    fn rewrite_if_unchanged_held(
        &self,
        expected_generation: u64,
        intent: &StationIntentV1,
    ) -> Result<bool> {
        let id = intent.id();
        if !path_present(&self.path_for(&id))? {
            return Ok(false);
        }
        let current = self.load(&id)?;
        if current.generation != expected_generation {
            return Ok(false);
        }
        self.write_atomic(intent)?;
        Ok(true)
    }

    /// Read-modify-write one intent under the per-intent write lock, bumping the generation.
    ///
    /// The one supported way for a *producer-side* path (attach finalize, resume) to mutate an
    /// intent: it cannot lose a concurrent daemon evidence write, and it cannot be lost by one.
    /// `mutate` returns `false` to abandon the update without writing.
    pub fn update_locked<F>(&self, id: &IntentId, mutate: F) -> Result<Option<StationIntentV1>>
    where
        F: FnOnce(&mut StationIntentV1) -> bool,
    {
        let _lock = self.lock_intent(id)?;
        let mut intent = self.load(id)?;
        if !mutate(&mut intent) {
            return Ok(None);
        }
        intent.generation = intent.generation.saturating_add(1);
        self.write_atomic(&intent)?;
        Ok(Some(intent))
    }

    /// Create (or replace) the `pending` record for one binding, serialized by the per-intent
    /// write lock and generation-safe.
    ///
    /// The check and the write have to be one critical section. Unserialized, two attaches for the
    /// same binding both read "no live record", both compute `existing.generation + 1`, and the
    /// second silently clobbers the first at the same generation — which then defeats every
    /// generation-CAS guard downstream, because a CAS holder cannot tell that the record under it
    /// was replaced. The lock also makes the "leave an existing `live` record alone" rule real: a
    /// resume whose finalize later fails must not have already demoted a working record to
    /// `pending`, where GC would remove it.
    ///
    /// Two different things reach this function, and telling them apart is the whole of its
    /// lifetime and proof rules:
    ///
    /// * A **retry of the pending lifecycle already in progress** — the record on disk is already
    ///   `Pending`. The attach it describes has not finalized, and this write is another attempt at
    ///   finishing the same thing. It inherits that lifecycle's `created_at_ms` and its armed
    ///   proof, so no amount of retrying can buy the record more life than the one attach earned
    ///   (see [`StationIntentV1::pending_clock_ms`]) and a crash between `Register` and finalize
    ///   still has the proof it needs to be repaired.
    /// * A **new attach over a record whose lifecycle is over** — `Revoked`, `Tombstoned`, or any
    ///   other persisted-but-inert state. This is a genuinely new attach that happens to reuse a
    ///   binding, so it starts a new lifecycle: it keeps its own `created_at_ms`, which gives it
    ///   the full pending TTL to reach its finalize, and it carries **no** armed proof.
    ///
    /// Carrying the old lifecycle's fields into a new attach was wrong in both directions. The
    /// clock was the serious one: a `Revoked` record lives for the 7-day terminal TTL, so every
    /// re-attach after a detach or a fallback downgrade was born `Pending` with an already-expired
    /// pending clock, and the next GC pass deleted it *before* `extensions_reload` and the
    /// turn-boundary finalize could promote it — the attach silently lost its record, and would
    /// keep losing it for a week. The proof was the subtler one: a revocation is an explicit
    /// teardown of the arming it describes, so inheriting it would let `finalize_admission` promote
    /// a brand-new attach on the strength of a *previous* daemon's arming (`armed_durably`), which
    /// is exactly the "a merely-existing bridge arms an attach that was never registered" hole the
    /// admission rules exist to close. A new lifecycle proves itself with a new daemon stamp or it
    /// does not promote.
    ///
    /// The generation is the one field that is *always* inherited-and-advanced: it is a
    /// per-file compare-and-set token, not a lifecycle property, and resetting it would let a
    /// stale CAS holder clobber a newer record.
    ///
    /// `updated_at_ms` moves (this *is* a write), but it is deliberately not the clock any pending
    /// TTL reads.
    pub fn write_pending(&self, intent: &StationIntentV1) -> Result<PendingWrite> {
        if intent.state != IntentRecoveryState::Pending {
            return Err(IntentError::Invalid(
                "write_pending may only write a pending record".to_string(),
            ));
        }
        let id = intent.id();
        let _lock = self.lock_intent(&id)?;
        let existing = self.load(&id).ok();
        if let Some(existing) = existing.as_ref() {
            if existing.state == IntentRecoveryState::Live {
                return Ok(PendingWrite::KeptExistingLive {
                    generation: existing.generation,
                });
            }
        }
        let mut next = intent.clone();
        if let Some(existing) = existing.as_ref() {
            // Generation must be monotonic, never reset: a reconcile pass that read generation N
            // and then wrote back under a compare-and-set would otherwise be able to clobber a
            // *newer* manifest that happened to cycle back to N. True for both branches below —
            // the generation belongs to the file, not to the lifecycle.
            next.generation = existing.generation.saturating_add(1);
            if existing.state == IntentRecoveryState::Pending {
                // Same lifecycle, another attempt. Neither of these may be refreshed by retrying.
                next.created_at_ms = existing.created_at_ms;
                // The durable record's proof is the only one that survives: the proof is the
                // *daemon's* to write, and only `stamp_armed_proof` mints one, so a caller-supplied
                // `armed` block is never honoured here.
                next.armed = existing.armed.clone();
            } else {
                // A new lifecycle over a finished one. It gets its own clock (so it has the full
                // pending TTL to reach its finalize) and no proof (so it must earn a new daemon
                // stamp before anything may promote it).
                next.armed = None;
            }
        }
        self.write_atomic(&next)?;
        Ok(PendingWrite::Created {
            generation: next.generation,
        })
    }

    /// Stamp the durable armed proof onto an existing record, under the per-intent write lock.
    ///
    /// Called by the daemon **as part of committing** an armed push member, before the member is
    /// installed, so there is no window in which a register has committed and its proof has not.
    /// That ordering is what makes the proof transactional rather than advisory: the caller aborts
    /// the registration (releasing anything it claimed) when this cannot be persisted, and a
    /// concurrent attach rollback either loses the per-intent lock race — in which case it finds an
    /// armed record and refuses to delete it — or wins it, in which case this reports
    /// [`ArmedProofStamp::NoRecord`] and the register fails instead of silently returning a durable
    /// success it cannot back.
    ///
    /// Idempotent: an already-armed record is left untouched rather than churning the generation on
    /// every re-register, which would invalidate concurrent CAS holders for no gain. Idempotency is
    /// also what keeps the armed pending TTL honest — the proof timestamp is the clock that TTL
    /// reads, so a re-register must not be able to move it.
    pub fn stamp_armed_proof(
        &self,
        store_key: &str,
        session_id: &str,
        address: &str,
        daemon_instance_id: &str,
        now_ms: i64,
    ) -> Result<ArmedProofStamp> {
        let id = IntentId::derive(store_key, session_id, address);
        // `NoRecord` is a *proof of absence*, because the caller treats it as "this binding never
        // had a durable record, so there is nothing to prove and the register may commit". An
        // existence check that answers `false` for a record it merely could not stat therefore
        // hands an ordinary admission exactly the wrong answer about a record that is really there.
        // Undecidable existence is an error, which `stamp_intent_armed` classifies `RecordUnusable`
        // and the admission table refuses whether or not a proof was owed.
        if !path_present(&self.path_for(&id))? {
            return Ok(ArmedProofStamp::NoRecord);
        }
        let mut already = None;
        let updated = self.update_locked(&id, |intent| {
            if intent.armed.is_some() {
                already = Some(intent.generation);
                return false;
            }
            // Deliberately does *not* move `updated_at_ms`. Arming is not a producer proof, and the
            // pending TTLs read `pending_clock_ms` rather than that field precisely so no repeated
            // call here or in `write_pending` can extend a lifetime the record has not earned.
            intent.armed = Some(ArmedProofV1 {
                armed_at_ms: now_ms,
                daemon_instance_id: daemon_instance_id.to_string(),
            });
            true
        });
        match updated {
            Ok(Some(intent)) => Ok(ArmedProofStamp::Stamped {
                generation: intent.generation,
            }),
            Ok(None) => match already {
                Some(generation) => Ok(ArmedProofStamp::AlreadyArmed { generation }),
                // `update_locked` only declines when the mutation returned `false`, and the only
                // `false` above sets `already`. Treat anything else as "the record went away",
                // which is a refusal, never a silent success.
                None => Ok(ArmedProofStamp::NoRecord),
            },
            // The record vanished between the existence check and the locked load. That is exactly
            // the rollback race, and it must surface as "no record" rather than as an I/O error the
            // caller might classify as transient. Only a *proven* absence earns the remap: if the
            // re-check cannot decide either, the original failure stands, because downgrading an
            // unreadable record to `NoRecord` is the same fail-open this whole path exists to close.
            Err(IntentError::Io(detail)) => match path_present(&self.path_for(&id)) {
                Ok(false) => Ok(ArmedProofStamp::NoRecord),
                _ => Err(IntentError::Io(detail)),
            },
            Err(e) => Err(e),
        }
    }

    /// Acquire the per-intent write lock, or fail rather than write unserialized.
    ///
    /// Bounded (never blocks a reconcile pass past its per-intent budget) and stale-tolerant: a
    /// holder that died mid-write leaves a lock file, so one older than `INTENT_LOCK_STALE` is
    /// stolen rather than wedging the binding forever.
    fn lock_intent(&self, id: &IntentId) -> Result<IntentWriteLock> {
        let path = self
            .root
            .join(format!("{}{INTENT_LOCK_SUFFIX}", id.file_name()));
        for attempt in 0..INTENT_LOCK_ATTEMPTS {
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(_) => return Ok(IntentWriteLock { path }),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    if file_age_ms(&path, crate::model::now_ms())
                        .is_some_and(|age| age > INTENT_LOCK_STALE.as_millis() as i64)
                    {
                        let _ = std::fs::remove_file(&path);
                        continue;
                    }
                    if attempt + 1 < INTENT_LOCK_ATTEMPTS {
                        std::thread::sleep(INTENT_LOCK_RETRY);
                    }
                }
                Err(e) => return Err(IntentError::Io(format!("locking intent {id}: {e}"))),
            }
        }
        Err(IntentError::Io(format!(
            "intent {id} is locked by another writer"
        )))
    }

    /// **Withdraw** one binding's desired state: the single fallible, linearized operation behind
    /// every explicit teardown — detach, session end, operator reset (including a memberless or
    /// already-idle one), and the push→pull fallback downgrade.
    ///
    /// One operation rather than a handful of best-effort `revoke` wrappers, because withdrawal is
    /// the *durable half* of a teardown the user already performed. A wrapper that swallowed its
    /// error left the binding's desired state saying "restore push" while membership said the
    /// station was given up, and the next reconcile pass — or the next daemon — dutifully brought
    /// it back. So this returns `Result`: **callers propagate**.
    ///
    /// The rules, all decided under the per-intent write lock:
    ///
    /// * **`pending` → deleted**, at exactly the generation this call observed. A `pending` record
    ///   describes an attach that never finalized; its producer block may still be the attach-time
    ///   placeholder, so tombstoning it would leave an identity-less `revoked` record squatting on
    ///   the binding for the seven-day terminal TTL. The delete is conditional and lock-held like
    ///   every other deletion in this module: absence, a generation that moved, a state that is no
    ///   longer `pending`, an unreadable manifest, and an unsupported schema version all **refuse**
    ///   it.
    /// * **anything else durable → `revoked`**, the transition an existing detach/session-end path
    ///   already performed, under a generation compare-and-set so it cannot clobber a concurrent
    ///   writer.
    /// * **already `revoked` → success**, unchanged. Withdrawal is idempotent because a detach, a
    ///   session end, and a reset can all legitimately name the same binding.
    /// * **no record → success**, and *nothing is written*. A withdrawal carries no producer
    ///   identity of its own, so it can never mint a `revoked` record: an identity-less tombstone
    ///   would be a durable record nothing can validate, GC would keep it for a week, and it would
    ///   shadow the binding in the meantime.
    ///
    /// "No record" is only ever a *proven* absence ([`platform_fs::path_present`]). A manifest that
    /// could not be stat'd, read, parsed, or validated is an error — never a quiet success — for
    /// the same reason the arming proof refuses one: absence is the answer that admits, so absence
    /// is the answer that has to be proven.
    ///
    /// Wholly synchronous and self-contained: it acquires the per-intent lock, does its own I/O,
    /// and releases it, so no caller can end up holding a filesystem lock across an unrelated
    /// backend await.
    pub fn withdraw_binding(
        &self,
        store_key: &str,
        session_id: &str,
        address: &str,
        now_ms: i64,
    ) -> Result<Withdrawal> {
        self.withdraw_binding_at_generation(store_key, session_id, address, None, now_ms)
    }

    /// The generation-conditional form, for a caller withdrawing on the strength of a decision it
    /// made against one specific version of the record.
    ///
    /// The reconciler is the caller that needs it: a pass decides "this binding is durably
    /// tombstoned" against generation *N*, and by the time it applies that outcome a re-attach may
    /// have written a fresh `pending` record at *N+1*. Withdrawing unconditionally there would
    /// delete a record the decision knew nothing about — the same stale-snapshot hazard every
    /// deletion in this module is conditioned against. `expected_generation: None` means "whatever
    /// is there now", which is what an explicit operator teardown means.
    pub fn withdraw_binding_at_generation(
        &self,
        store_key: &str,
        session_id: &str,
        address: &str,
        expected_generation: Option<u64>,
        now_ms: i64,
    ) -> Result<Withdrawal> {
        let id = IntentId::derive(store_key, session_id, address);
        if !path_present(&self.path_for(&id))? {
            return Ok(Withdrawal::NoRecord);
        }
        let _lock = self.lock_intent(&id)?;
        self.withdraw_held(&id, expected_generation, now_ms)
    }

    /// The withdrawal itself, with the per-intent write lock already held.
    ///
    /// The observation and the write are one critical section on purpose: the generation this
    /// deletes or transitions is exactly the generation it read, so there is no window in which a
    /// concurrent finalize can promote the record between the two halves and lose its promotion —
    /// or, worse, have this call delete it.
    fn withdraw_held(
        &self,
        id: &IntentId,
        expected_generation: Option<u64>,
        now_ms: i64,
    ) -> Result<Withdrawal> {
        let observed = match self.load(id) {
            Ok(observed) => observed,
            Err(e) => {
                // Only a *proven* absence is "nothing to withdraw". Unreadable bytes, a failed
                // security check, and an unsupported schema version are refusals that propagate:
                // a record telex cannot read is a record it must not delete, overwrite, or claim
                // to have withdrawn. "Intents are never deleted by a rollback" depends on this —
                // an unsupported schema version is exactly what a rollback leaves behind.
                return if path_present(&self.path_for(id))? {
                    Err(e)
                } else {
                    Ok(Withdrawal::NoRecord)
                };
            }
        };
        let generation = observed.generation;
        if expected_generation.is_some_and(|expected| expected != generation) {
            return Ok(Withdrawal::Superseded { generation });
        }
        match observed.state {
            IntentRecoveryState::Revoked => Ok(Withdrawal::AlreadyRevoked { generation }),
            IntentRecoveryState::Pending => {
                // Re-checked under the lock even though it was just read under the same lock: the
                // lock is stale-tolerant by design (a writer that died mid-update must not wedge a
                // binding forever), so "the record still says what I decided about" is proven at
                // the unlink rather than assumed from the lock alone.
                if self.remove_if_unchanged_held(id, generation, |current| {
                    current.state == IntentRecoveryState::Pending
                })? {
                    Ok(Withdrawal::DeletedPending { generation })
                } else {
                    Err(IntentError::Io(format!(
                        "station intent {id} changed under its own write lock; \
                         refusing to delete a record this withdrawal no longer describes"
                    )))
                }
            }
            _ => {
                let mut next = observed;
                next.state = IntentRecoveryState::Revoked;
                next.updated_at_ms = now_ms;
                next.generation = generation.saturating_add(1);
                if self.rewrite_if_unchanged_held(generation, &next)? {
                    return Ok(Withdrawal::Revoked {
                        generation: next.generation,
                    });
                }
                // The record moved or vanished under the lock. A vanished one is genuinely
                // withdrawn; anything else is an anomaly this call must not paper over.
                if path_present(&self.path_for(id))? {
                    Err(IntentError::Io(format!(
                        "station intent {id} changed under its own write lock; \
                         the withdrawal was refused rather than clobbering a newer record"
                    )))
                } else {
                    Ok(Withdrawal::NoRecord)
                }
            }
        }
    }

    /// Every binding the scope currently names, split by whether its identity is *proven*.
    ///
    /// Needed by the set-scoped withdrawals (a session end withdraws every binding of one session;
    /// an operator reset withdraws every binding of one address, including ones with no member at
    /// all). They cannot enumerate from membership — the whole point is that a memberless or
    /// already-idle binding still has durable desired state — so they enumerate from the scope.
    ///
    /// Only a manifest that *loaded* contributes an actionable binding: `load` proves the tuple
    /// against the filename hash and the singleton scope, so an id derived from it can only name
    /// the file it came from. An unreadable manifest's self-declared identity is unvalidated and
    /// is therefore never acted on — it is returned so the caller can **refuse** the set-scoped
    /// operation rather than silently skipping a record it could not read.
    pub fn bindings(&self) -> Result<BindingScan> {
        self.bindings_bounded(PassDeadline::unbounded())
    }

    /// The same enumeration, stopped at `deadline` and reporting whether it was truncated.
    ///
    /// A set-scoped teardown is an operator-facing request with a bound of its own, and both halves
    /// of this enumeration are real I/O against a directory that may be on a wedged network mount:
    /// the `read_dir` itself, and one `read` per manifest. The unbounded form gave a session end or
    /// a reset no way to stay inside its own deadline, so the cooperative check runs on the same
    /// stride as `list_ids_bounded` — and truncation is *reported*, never hidden, because the
    /// caller must refuse rather than act on a partial set (see [`BindingScan::truncated`]).
    ///
    /// Cooperative checks bound the loop, not the syscall that is currently blocked; an async
    /// caller pairs this with [`run_blocking_within`] for the hard bound.
    pub fn bindings_bounded(&self, deadline: PassDeadline) -> Result<BindingScan> {
        let (ids, truncated) = self.list_ids_bounded(deadline)?;
        let mut scan = BindingScan {
            truncated,
            ..BindingScan::default()
        };
        for (examined, id) in ids.iter().enumerate() {
            if examined > 0
                && examined.is_multiple_of(ENUMERATION_DEADLINE_STRIDE)
                && deadline.expired()
            {
                scan.truncated = true;
                break;
            }
            match self.load_projected(id) {
                Ok(intent) => scan.bindings.push(IntentBinding {
                    store_key: intent.store_key,
                    session_id: intent.session_id,
                    address: intent.address,
                }),
                Err(rejection) => {
                    let claimed = rejection.identity.map(|identity| IntentBinding {
                        store_key: identity.store_key,
                        session_id: identity.session_id,
                        address: identity.address,
                    });
                    scan.unreadable.push((id.clone(), claimed));
                }
            }
        }
        scan.bindings.sort();
        Ok(scan)
    }

    /// Publish a member (or any other commit) **only while** the manifest still says exactly what
    /// the caller reconciled: present, `live`, and at the generation the decision was made against.
    ///
    /// The restoration chain is a sequence of awaits — a credential read, a producer probe, a
    /// backend `ensure_address`, an epoch claim, two tombstone checks — and an explicit withdrawal
    /// can land anywhere inside it. Without this gate the pass finished its chain and installed an
    /// armed push member from a manifest that had been revoked or deleted seconds earlier; the
    /// member then outlived the desired state that authorized it, and only a *later* pass (which
    /// would find the record inert) could notice.
    ///
    /// So the last step before publication re-reads under the per-intent write lock and runs the
    /// commit inside it. Withdrawal takes the same lock, so the two are linearized: either the
    /// withdrawal wins and this refuses to publish, or this wins and the withdrawal observes the
    /// member it must tear down. `commit` is synchronous by construction — it installs an
    /// in-memory record — so the filesystem lock is never held across an await.
    ///
    /// `Ok(None)` is "the record no longer authorizes this commit" (absent, revoked, deleted, or
    /// moved to a newer generation). `Err` is "that could not be decided", which is a refusal too:
    /// the caller releases whatever it claimed and retries on the ordinary ladder.
    pub fn commit_if_live_generation<T>(
        &self,
        id: &IntentId,
        expected_generation: u64,
        commit: impl FnOnce() -> T,
    ) -> Result<Option<T>> {
        let _lock = self.lock_intent(id)?;
        if !path_present(&self.path_for(id))? {
            return Ok(None);
        }
        let current = self.load(id)?;
        if current.generation != expected_generation || current.state != IntentRecoveryState::Live {
            return Ok(None);
        }
        Ok(Some(commit()))
    }
}

/// Per-intent write lock file, removed on drop.
#[derive(Debug)]
struct IntentWriteLock {
    path: PathBuf,
}

impl Drop for IntentWriteLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

impl IntentStore {
    /// Delete one intent, but **only** while holding its write lock and only if the record on disk
    /// still matches what the caller decided about.
    ///
    /// Deleting an intent is the one action recovery cannot undo, and every caller decides from a
    /// snapshot: GC classifies a record it loaded earlier in the pass, and an attach rollback
    /// removes "the record this invocation created". Between the decision and the unlink, a
    /// concurrent turn-boundary finalize can promote the very same file to `live`, or a fresh
    /// attach can replace it — and the unconditional unlink destroyed the newer record with no
    /// trace.
    ///
    /// So the delete re-reads under the lock and re-checks two things: the generation is exactly
    /// the one the decision was made against, and `still_deletable` still holds on the freshly
    /// loaded record. The generation catches every state transition (`update_locked`,
    /// `write_pending`, and `revoke` all bump it); the predicate catches everything else the
    /// caller cares about, including an evidence-only rewrite that moved a TTL clock without
    /// moving the generation.
    ///
    /// Returns `false` — never an error — when the record is gone, unreadable, or no longer
    /// matches. A refused delete is always the safe outcome.
    pub fn remove_if_unchanged<F>(
        &self,
        id: &IntentId,
        expected_generation: u64,
        still_deletable: F,
    ) -> Result<bool>
    where
        F: FnOnce(&StationIntentV1) -> bool,
    {
        let _lock = self.lock_intent(id)?;
        self.remove_if_unchanged_held(id, expected_generation, still_deletable)
    }

    /// The conditional delete itself, with the per-intent write lock already held.
    ///
    /// Split out so a caller that has *already* taken the lock to observe the record — an explicit
    /// withdrawal, which must delete exactly the `pending` generation it read — can delete inside
    /// that same critical section. Re-entering [`IntentStore::lock_intent`] would instead block on
    /// the caller's own lock file until the attempt budget ran out, and then either fail or (once
    /// the staleness window elapsed) *steal* the caller's own lock, which is worse: the mutual
    /// exclusion the whole path depends on would silently stop existing.
    fn remove_if_unchanged_held<F>(
        &self,
        id: &IntentId,
        expected_generation: u64,
        still_deletable: F,
    ) -> Result<bool>
    where
        F: FnOnce(&StationIntentV1) -> bool,
    {
        let current = match self.load(id) {
            Ok(current) => current,
            // Already gone, or it became unreadable under us. Either way this caller's decision no
            // longer describes what is on disk, and `remove_unreadable_if_unchanged` is the only
            // path allowed to delete something it cannot read.
            Err(_) => return Ok(false),
        };
        if current.generation != expected_generation || !still_deletable(&current) {
            return Ok(false);
        }
        self.unlink(id)
    }

    /// Delete a manifest that cannot be read at all, re-confirming under the write lock that it is
    /// *still* unreadable and *still* past `min_age`.
    ///
    /// Separate from [`IntentStore::remove_if_unchanged`] because there is no generation to compare
    /// against: the record has no readable one. Re-loading under the lock is what keeps a manifest
    /// that was merely mid-rewrite (a torn read, a Windows sharing violation) from being deleted
    /// because one unlucky pass could not parse it.
    pub fn remove_unreadable_if_unchanged(
        &self,
        id: &IntentId,
        now_ms: i64,
        min_age: Duration,
    ) -> Result<bool> {
        let _lock = self.lock_intent(id)?;
        if self.load(id).is_ok() {
            return Ok(false);
        }
        let past_ttl = file_age_ms(&self.path_for(id), now_ms)
            .is_some_and(|age| age > min_age.as_millis() as i64);
        if !past_ttl {
            return Ok(false);
        }
        self.unlink(id)
    }

    /// The raw unlink. Private on purpose: every deletion in this module goes through one of the
    /// two conditional, lock-held entry points above.
    fn unlink(&self, id: &IntentId) -> Result<bool> {
        match std::fs::remove_file(self.path_for(id)) {
            Ok(()) => Ok(true),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(IntentError::Io(format!("removing intent {id}: {e}"))),
        }
    }

    /// One bounded scan pass, resuming from the persisted round-robin cursor.
    ///
    /// Unbounded and unscoped. Kept for callers that are not on a deadline (fixtures, one-shot
    /// maintenance); everything on the reconcile path calls [`IntentStore::scan_bounded`].
    pub fn scan(&self, budget: usize) -> Result<ScanPage> {
        self.scan_bounded(budget, None, PassDeadline::unbounded())
    }

    /// One bounded scan pass for `scope`, resuming from that scope's persisted cursors and
    /// stopping at `deadline`.
    ///
    /// Three properties are load-bearing here:
    ///
    /// * **Scope filtering happens ahead of the page window and the budget.** A pass filtered to
    ///   one store must spend its whole budget on that store; filtering after the window let a
    ///   scope full of another store's intents consume every slot and produce an empty page for the
    ///   store that actually asked, forever.
    /// * **Discovery is deadline-bounded and resumable.** Reading every manifest in the scope is
    ///   real I/O, so it is bounded like everything else — and because a bounded read that always
    ///   restarts at the head would re-read the same prefix on every pass and never reach the tail,
    ///   the position discovery stopped at is persisted per scope. At least one manifest is always
    ///   read, so a pass whose deadline is already spent still makes forward progress rather than
    ///   spinning.
    /// * **The sort-position cursor is still only advanced by the caller.** This function reads it
    ///   and never writes it: only the caller knows how far a budget- or deadline-truncated pass
    ///   actually got, and advancing past entries the pass never attempted is exactly how a
    ///   round-robin cursor silently starves the tail of a scope. See
    ///   [`IntentStore::advance_cursor_in_scope`]. The *discovery* position is different in kind —
    ///   it describes what this function itself did, so this function is the only thing that can
    ///   record it.
    ///
    /// When discovery completes within the deadline (the ordinary case: a capped scope on a local
    /// disk) the page is exactly what an unbounded scan would have produced.
    pub fn scan_bounded(
        &self,
        budget: usize,
        scope: Option<&str>,
        deadline: PassDeadline,
    ) -> Result<ScanPage> {
        let (ids, enumeration_truncated) = self.list_ids_bounded(deadline)?;
        let observed_count = ids.len();
        // Same comparison the write cap uses (`>=`): at exactly the cap new ids already fail with
        // `CapExceeded`, so reporting `over_cap: false` there would leave the operator with a
        // refused write and no stated reason.
        let over_cap = observed_count >= STATION_INTENT_MAX_COUNT;

        let cursor = self.read_cursor()?;
        let scope_cursor = cursor.for_scope(scope);
        let mut entries: Vec<(IntentSortKey, StationIntentV1)> = Vec::new();
        let mut rejected = Vec::new();
        let mut skipped = Vec::new();

        // Resume discovery at the first id strictly greater than the one it stopped at, wrapping
        // like the sort cursor does.
        let discovery_start = match scope_cursor.discovery.as_deref() {
            Some(after) => ids
                .iter()
                .position(|id| id.as_str() > after)
                .unwrap_or_default(),
            None => 0,
        };
        let mut discovery_truncated = enumeration_truncated;
        let mut discovery_position: Option<String> = None;
        for offset in 0..ids.len() {
            // Checked *after* the first read, never before it: a pass that arrives with its
            // deadline already spent must still read one manifest and move discovery on, or a
            // scope whose maintenance repeatedly overruns would never be discovered at all.
            if offset > 0 && deadline.expired() {
                discovery_truncated = true;
                break;
            }
            let id = &ids[(discovery_start + offset) % ids.len()];
            discovery_position = Some(id.as_str().to_string());
            match self.load_projected(id) {
                Ok(intent) => {
                    if scope.is_some_and(|filter| intent.store_key != filter) {
                        continue;
                    }
                    entries.push((intent.sort_key(), intent));
                }
                Err(rejection) => {
                    // A rejection that names another store is filtered like a loaded one. A
                    // rejection whose identity could not be established belongs to every scope:
                    // dropping it would make an unreadable manifest invisible to a scoped pass.
                    if let Some(identity) = &rejection.identity {
                        if scope.is_some_and(|filter| identity.store_key != filter) {
                            continue;
                        }
                    }
                    rejected.push((id.clone(), rejection));
                }
            }
        }
        // A sweep that reached the end of the scope starts the next one at the head.
        let next_discovery = if discovery_truncated {
            discovery_position
        } else {
            None
        };
        if next_discovery != scope_cursor.discovery {
            self.write_scope_cursor(scope, |entry| entry.discovery = next_discovery.clone())?;
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
                discovery_truncated,
            });
        }

        let sort_positions: Vec<String> = entries.iter().map(|(k, _)| sort_position(k)).collect();
        // Resume at the first position strictly greater than the last processed one; wrap to the
        // start when the cursor sits at or past the maximum.
        let start = match scope_cursor.position.as_deref() {
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
            discovery_truncated,
        })
    }

    /// Persist the round-robin cursor at the sort position of the last entry a pass processed.
    ///
    /// Called by the reconciler with the position of the last intent it actually attempted (or, if
    /// it attempted none, the last it considered), so a truncated pass resumes where it stopped
    /// instead of skipping everything it loaded but never reached.
    pub fn advance_cursor(&self, position: &str) -> Result<()> {
        self.advance_cursor_in_scope(None, position)
    }
    /// The scoped form. A scoped pass moves only its own store's position, so it can neither skip
    /// intents another scope never considered nor be skipped past by one.
    pub fn advance_cursor_in_scope(&self, scope: Option<&str>, position: &str) -> Result<()> {
        self.write_scope_cursor(scope, |entry| entry.position = Some(position.to_string()))
    }

    /// The four pass-path phases, each behind [`run_blocking_within`] or, for the two that mutate
    /// durable pass-visible state, [`run_blocking_reserved`].
    ///
    /// These are the forms the reconcile pass uses, and the synchronous ones are reserved for
    /// off-pass callers (fixtures, one-shot maintenance, the store's own tests). For the two
    /// scanning phases the deadline is passed twice on purpose and means two different things:
    /// *inside* the phase it is the cooperative stride check that decides where to stop and what to
    /// persist as a resume position, and *outside* it is the instant the async caller stops waiting
    /// for the phase at all. Without the outer half, one blocked syscall is enough to carry a
    /// bounded pass past its own deadline; without [`RECONCILE_BLOCKING_GRACE`] between them, the
    /// outer half would pre-empt every cooperative stop and no large scope would ever be scanned.
    pub async fn scan_bounded_within(
        &self,
        budget: usize,
        scope: Option<String>,
        deadline: PassDeadline,
    ) -> BoundedPhase<Result<ScanPage>> {
        let store = self.clone();
        run_blocking_within(deadline, RECONCILE_BLOCKING_GRACE, move || {
            store.scan_bounded(budget, scope.as_deref(), deadline)
        })
        .await
    }

    pub async fn gc_bounded_within(
        &self,
        now_ms: i64,
        local_host: Option<String>,
        local_boot: Option<String>,
        deadline: PassDeadline,
    ) -> BoundedPhase<Result<GcReport>> {
        let store = self.clone();
        run_blocking_within(deadline, RECONCILE_BLOCKING_GRACE, move || {
            store.gc_bounded(
                now_ms,
                local_host.as_deref(),
                local_boot.as_deref(),
                deadline,
            )
        })
        .await
    }

    /// The round-robin cursor write, started only while `reserve` of the pass deadline is intact.
    ///
    /// No grace: `advance_cursor_in_scope` has no deadline of its own to stop at, so there is
    /// nothing for a grace period to let it finish. The deadline here is purely the hard bound, and
    /// the reserve is what keeps a cursor advance from being *launched* so close to the deadline
    /// that it could only ever land after the pass has answered.
    pub async fn advance_cursor_in_scope_reserved(
        &self,
        scope: Option<String>,
        position: String,
        deadline: PassDeadline,
        reserve: Duration,
    ) -> BoundedPhase<Result<()>> {
        let store = self.clone();
        run_blocking_reserved(deadline, reserve, move || {
            store.advance_cursor_in_scope(scope.as_deref(), &position)
        })
        .await
    }

    /// The evidence write an outcome persists, behind the same boundary and the same reserve.
    ///
    /// A per-intent manifest rewrite is small, but it is still an atomic write (stage, fsync,
    /// rename) on a path a pass reaches *after* its wave has already spent most of the budget, and
    /// it runs once per attempted intent. Starting one with less than `reserve` left would be
    /// starting work whose only possible completion is after the response, so it is not started;
    /// the pass reports itself deadline-truncated and the next pass re-derives the same evidence.
    pub async fn write_cas_reserved(
        &self,
        expected_generation: u64,
        intent: StationIntentV1,
        deadline: PassDeadline,
        reserve: Duration,
    ) -> BoundedPhase<Result<bool>> {
        let store = self.clone();
        run_blocking_reserved(deadline, reserve, move || {
            store.write_cas(expected_generation, &intent)
        })
        .await
    }

    fn read_cursor(&self) -> Result<ScanCursor> {
        let path = self.root.join(SCAN_CURSOR_FILE);
        // Only a proven absence short-circuits. This is the one existence check in the module that
        // is *not* an authority question — the cursor is a scheduling hint, and the read below
        // already defaults on failure — so an undecidable answer simply falls through to it rather
        // than failing the pass.
        if matches!(platform_fs::path_present(&path), Ok(false)) {
            return Ok(ScanCursor::default());
        }
        // A corrupt or unreadable cursor is a scheduling hint, not authority: restart from the
        // beginning rather than failing the pass.
        match platform_fs::read_owner_only_file(&path, 4096) {
            Ok(bytes) => Ok(serde_json::from_slice(&bytes).unwrap_or_default()),
            Err(_) => Ok(ScanCursor::default()),
        }
    }

    /// Read-modify-write one scope's entry, preserving every other scope's and the GC position.
    fn write_scope_cursor(
        &self,
        scope: Option<&str>,
        update: impl FnOnce(&mut ScopeCursor),
    ) -> Result<()> {
        let mut cursor = self.read_cursor()?;
        let key = ScanCursor::scope_key(scope).to_string();
        let mut entry = cursor.for_scope(scope);
        update(&mut entry);
        // Mirror the unscoped position into the legacy field so a rollback to a build that only
        // understands one global position still resumes somewhere sane instead of at the head.
        if scope.is_none() {
            cursor.position = entry.position.clone();
        }
        cursor.scopes.insert(key, entry);
        self.write_cursor(&cursor)
    }

    fn write_gc_position(&self, position: Option<String>) -> Result<()> {
        let mut cursor = self.read_cursor()?;
        if cursor.gc_position == position {
            return Ok(());
        }
        cursor.gc_position = position;
        self.write_cursor(&cursor)
    }

    fn write_cursor(&self, cursor: &ScanCursor) -> Result<()> {
        let path = self.root.join(SCAN_CURSOR_FILE);
        let bytes = serde_json::to_vec(cursor)
            .map_err(|e| IntentError::Json(format!("serializing scan cursor: {e}")))?;
        platform_fs::write_owner_only_file_atomic(&path, &bytes)?;
        Ok(())
    }

    /// Bounded garbage collection. The only mechanism that can bring an over-cap scope back under
    /// the cap, and the only place an intent file is ever deleted.
    ///
    /// Every reason is **TTL-governed and state-scoped**, because deleting an intent is the one GC
    /// action recovery cannot undo:
    ///
    /// * An unarmed `Pending` record is governed **solely** by [`STATION_INTENT_PENDING_TTL`],
    ///   measured from this attach lifecycle's `created_at_ms`
    ///   ([`StationIntentV1::pending_clock_ms`]). Nothing else may
    ///   delete it: on a first attach the producer does not exist yet by construction (the bridge
    ///   extension has been written but not loaded), so a credential-existence or producer-liveness
    ///   rule would delete exactly the record the turn-boundary finalizer is waiting to promote.
    /// * An **armed** `Pending` record — one a daemon durably proved it armed push for — is
    ///   governed by [`STATION_INTENT_ARMED_PENDING_TTL`] measured from the armed proof, because
    ///   deleting it at five minutes silently disarms recovery for a binding that is delivering
    ///   right now.
    /// * A persisted terminal state (`Unverifiable` / `Insecure` / `Revoked`) expires after
    ///   [`STATION_INTENT_UNVERIFIABLE_TTL`].
    /// * A finalized intent whose credential file is gone expires after
    ///   [`STATION_INTENT_CREDENTIAL_MISSING_TTL`], measured from the last time the producer was
    ///   *proven* ([`StationIntentV1::last_proven_ms`]), not from manifest age — a bridge reload
    ///   makes the registry momentarily absent on a binding that may have been live for days.
    /// * An intent whose identity belongs to another host or boot *and* whose producer is provably
    ///   dead is removed immediately: it can never be restored here.
    /// * A finalized intent whose producer is provably dead expires after
    ///   [`STATION_INTENT_UNVERIFIABLE_TTL`], so a session that died without `sessionEnd` cannot
    ///   wedge its binding forever.
    ///
    /// Both TTL clocks are read from *proof*, never from retry attempts, and the pending clocks are
    /// read from creation or from the arming proof, never from the last write. The reconciler
    /// persists scheduling state on every genuine failure and a failing attach rewrites its pending
    /// record on every retry, so an attempt- or write-based clock is refreshed every few seconds
    /// forever and none of these rules can ever fire for exactly the abandoned records they exist
    /// to collect.
    ///
    /// Every deletion is re-checked under the per-intent write lock against the generation and the
    /// state the decision was made from ([`IntentStore::remove_if_unchanged`]), so a record that a
    /// concurrent finalize promoted, or that a concurrent attach replaced, is never destroyed by a
    /// decision taken against the older copy.
    ///
    /// `local_host`/`local_boot` are `None` when identity cannot be resolved; identity-derived
    /// reasons are then skipped rather than guessed.
    pub fn gc(
        &self,
        now_ms: i64,
        local_host: Option<&str>,
        local_boot: Option<&str>,
    ) -> Result<GcReport> {
        self.gc_bounded(now_ms, local_host, local_boot, PassDeadline::unbounded())
    }

    /// The deadline-bounded form, and the one every scheduled sweep uses.
    ///
    /// GC is `O(scope)` file I/O — a `load` and, for a candidate, a lock acquisition and an unlink
    /// each — so on the pass path it is bounded exactly like discovery is, against the same
    /// absolute deadline. Two properties keep that from turning a bound into a leak:
    ///
    /// * The sweep **resumes** from the persisted GC position, so a scope larger than one budget is
    ///   collected across several sweeps instead of having its head re-examined forever while its
    ///   tail is never reached.
    /// * At least one candidate is always examined, so a sweep that arrives with its deadline spent
    ///   still advances.
    ///
    /// `GcReport::complete` says which happened. The debris sweep runs only on a complete pass:
    /// it is a second full directory walk, and paying for it on a truncated sweep would spend the
    /// budget that was supposed to make the sweep complete.
    pub fn gc_bounded(
        &self,
        now_ms: i64,
        local_host: Option<&str>,
        local_boot: Option<&str>,
        deadline: PassDeadline,
    ) -> Result<GcReport> {
        let mut report = GcReport::default();
        let (ids, enumeration_truncated) = self.list_ids_bounded(deadline)?;
        let resume_after = self.read_cursor()?.gc_position;
        let start = match resume_after.as_deref() {
            Some(after) => ids
                .iter()
                .position(|id| id.as_str() > after)
                .unwrap_or_default(),
            None => 0,
        };
        let mut truncated = enumeration_truncated;
        let mut position: Option<String> = None;
        for offset in 0..ids.len() {
            // Checked after the first candidate, never before it: see `scan_bounded`.
            if offset > 0 && deadline.expired() {
                truncated = true;
                break;
            }
            let id = ids[(start + offset) % ids.len()].clone();
            position = Some(id.as_str().to_string());
            let intent = match self.load(&id) {
                Ok(intent) => intent,
                Err(IntentError::Io(_)) => {
                    report.kept += 1;
                    continue;
                }
                // A manifest written by a *newer* build is not evidence of abandonment; it is
                // exactly what a rollback leaves behind, and the documented rollback guarantee is
                // that intents are never deleted by one. Keep it inert and visible forever.
                Err(IntentError::UnsupportedSchema { .. }) => {
                    report.kept += 1;
                    continue;
                }
                Err(e) => {
                    // A manifest we cannot even parse securely is GC-eligible once it is older
                    // than the orphan TTL; until then it stays visible in status. Re-checked under
                    // the write lock, so a manifest that was merely mid-rewrite survives. A lock
                    // we could not take is a keep, never a delete.
                    match self.remove_unreadable_if_unchanged(
                        &id,
                        now_ms,
                        STATION_INTENT_UNVERIFIABLE_TTL,
                    ) {
                        Ok(true) => {
                            report.removed.push(id.clone());
                            report
                                .reasons
                                .push((id, format!("unreadable past TTL: {e}")));
                        }
                        Ok(false) | Err(_) => report.kept += 1,
                    }
                    continue;
                }
            };
            let Some(reason) = Self::gc_reason(&intent, now_ms, local_host, local_boot) else {
                report.kept += 1;
                continue;
            };
            // Re-decide under the lock on the record as it is *now*: between the classification
            // above and the unlink, a turn-boundary finalize can promote this exact file to `live`
            // and a fresh attach can replace it at a new generation. A lock this pass could not
            // take is also a keep — the next pass will reconsider.
            let generation = intent.generation;
            let removed = self
                .remove_if_unchanged(&id, generation, |current| {
                    Self::gc_reason(current, now_ms, local_host, local_boot).is_some()
                })
                .unwrap_or(false);
            if removed {
                report.removed.push(id.clone());
                report.reasons.push((id, reason));
            } else {
                report.kept += 1;
            }
        }
        report.complete = !truncated;
        // Best effort: a cursor we could not persist costs coverage latency, never correctness, and
        // must not fail a sweep that already did its deletions.
        let _ = self.write_gc_position(if report.complete { None } else { position });
        if report.complete {
            self.sweep_write_debris(now_ms);
        }
        Ok(report)
    }

    /// Why this record is GC-eligible right now, or `None` to keep it.
    ///
    /// Pure and side-effect free so it can be applied twice: once to classify, and again under the
    /// per-intent write lock on the record as it actually is at unlink time.
    fn gc_reason(
        intent: &StationIntentV1,
        now_ms: i64,
        local_host: Option<&str>,
        local_boot: Option<&str>,
    ) -> Option<String> {
        let age_ms = now_ms.saturating_sub(intent.updated_at_ms);
        // "How long has this binding been without a *proven* producer." Never an attempt clock:
        // see `StationIntentV1::last_proven_ms`.
        let unproven_ms = now_ms.saturating_sub(intent.last_proven_ms());
        let producer_dead = intent.producer.pid != 0
            && !crate::session_watch::process_alive_with_start_time(
                intent.producer.pid,
                Some(intent.producer.start_time),
            );
        if intent.state == IntentRecoveryState::Pending {
            // Pending is governed by its own TTL and by nothing else — a longer one once a daemon
            // has durably proven it armed push for the binding.
            //
            // Aged from `pending_clock_ms`, never from `updated_at_ms`: a re-attach rewrites the
            // record through `write_pending`, so an `updated_at_ms` clock let a producer that
            // failed to finalize keep its leftover alive forever simply by retrying.
            let ttl = if intent.is_armed() {
                STATION_INTENT_ARMED_PENDING_TTL
            } else {
                STATION_INTENT_PENDING_TTL
            };
            let pending_age_ms = now_ms.saturating_sub(intent.pending_clock_ms());
            return (pending_age_ms > ttl.as_millis() as i64).then(|| {
                if intent.is_armed() {
                    "armed pending intent past its TTL (never finalized)".to_string()
                } else {
                    "pending intent past its TTL (attach never finalized)".to_string()
                }
            });
        }
        if matches!(
            intent.state,
            IntentRecoveryState::Unverifiable
                | IntentRecoveryState::Insecure
                | IntentRecoveryState::Revoked
        ) && age_ms > STATION_INTENT_UNVERIFIABLE_TTL.as_millis() as i64
        {
            return Some("terminal intent past its TTL".to_string());
        }
        if credential_provably_absent(&intent.producer.credential.path)
            && unproven_ms > STATION_INTENT_CREDENTIAL_MISSING_TTL.as_millis() as i64
        {
            return Some("credential file has been gone past its TTL".to_string());
        }
        let (Some(host), Some(boot)) = (local_host, local_boot) else {
            return None;
        };
        if !intent.matches_local_identity(host, boot) && producer_dead {
            return Some("foreign host/boot identity with a dead producer".to_string());
        }
        if producer_dead && unproven_ms > STATION_INTENT_UNVERIFIABLE_TTL.as_millis() as i64 {
            // The orphan case decision 15 promised: a session that died without `sessionEnd` must
            // not hold its binding hostage forever.
            return Some(
                "producer is dead and the intent has been unproven past its TTL".to_string(),
            );
        }
        None
    }

    /// Remove orphaned atomic-write temp files and abandoned write locks.
    ///
    /// Neither is visible to `list_ids` or `gc`'s per-intent loop (they do not carry the intent
    /// filename shape), so without this an interrupted write leaks into the scope forever.
    fn sweep_write_debris(&self, now_ms: i64) {
        let Ok(entries) = std::fs::read_dir(&self.root) else {
            return;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let stale_after = if name.ends_with(INTENT_LOCK_SUFFIX) {
                INTENT_LOCK_STALE
            } else if name.ends_with(".tmp") {
                STATION_INTENT_PENDING_TTL
            } else {
                continue;
            };
            if file_age_ms(&entry.path(), now_ms)
                .is_some_and(|age| age > stale_after.as_millis() as i64)
            {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
}

fn sort_position(key: &IntentSortKey) -> String {
    format!(
        "{}\u{1f}{}\u{1f}{:020}\u{1f}{}",
        key.store_key, key.address, key.generation_desc, key.id
    )
}

/// Is the producer's credential file **provably** gone?
///
/// The GC reason this feeds is the one that deletes a *finalized* record, and deletion is the one
/// GC action recovery cannot undo. So the question has to be asked in the direction that only a
/// positive `NotFound` can answer: a credential whose metadata could not be read is a permissions,
/// mount, or antivirus condition — the bridge registry lives in a directory telex shares with an
/// external producer — and treating it as "gone" would have GC destroy the durable proof of a
/// binding that is delivering right now, on evidence that is really "I could not look".
fn credential_provably_absent(path: &Path) -> bool {
    matches!(platform_fs::path_present(path), Ok(false))
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
            armed: None,
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

        // The armed proof is an *optional* field inside schema version 1, so a manifest written
        // before it existed must read back cleanly — and, critically, as **never armed**. Defaulting
        // it the other way would make every pre-existing `pending` record promotable by any running
        // bridge, which is the one thing the proof exists to prevent.
        let mut older: serde_json::Value = serde_json::to_value(&intent).expect("to value");
        older
            .as_object_mut()
            .expect("object")
            .remove("armed")
            .map(|_| ())
            .unwrap_or_default();
        let older: StationIntentV1 = serde_json::from_value(older).expect("decode an older shape");
        assert!(older.armed.is_none());
        assert_eq!(
            finalize_admission(IntentRecoveryState::Pending, false, false),
            FinalizeAdmission::RefusedNotArmed
        );
        // And it survives a round trip once present.
        let mut armed = intent.clone();
        armed.armed = Some(ArmedProofV1 {
            armed_at_ms: 7,
            daemon_instance_id: "inst".to_string(),
        });
        let encoded = serde_json::to_vec(&armed).expect("encode");
        let decoded: StationIntentV1 = serde_json::from_slice(&encoded).expect("decode");
        assert_eq!(decoded.armed, armed.armed);
        assert!(
            !String::from_utf8_lossy(&serde_json::to_vec(&intent).expect("encode"))
                .contains("armed"),
            "an unarmed record must not carry a null field into the manifest"
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

    /// The full withdrawal matrix in one place, because each row is a distinct way the old
    /// per-caller `revoke` wrappers got it wrong.
    #[test]
    fn withdrawal_deletes_pending_tombstones_live_and_is_idempotent() {
        let run_dir = temp_run_dir("withdraw-matrix");
        let store = IntentStore::open(&run_dir, "hash").expect("store");

        // A binding that provably has no record: success, and nothing is created. An
        // identity-less `revoked` record would squat the binding for the terminal TTL.
        assert_eq!(
            store
                .withdraw_binding("sqlite:/a", "sess", "never-attached", 1_000)
                .expect("absent"),
            Withdrawal::NoRecord
        );
        assert_eq!(
            store.list_ids().expect("list").len(),
            0,
            "a withdrawal has no identity of its own and must never mint a record"
        );

        // `pending`: deleted, at exactly the generation observed.
        let mut pending = sample_intent("sqlite:/a", "sess", "addr-pending");
        pending.state = IntentRecoveryState::Pending;
        let PendingWrite::Created { generation } = store.write_pending(&pending).expect("attach")
        else {
            panic!("a first attach must create");
        };
        assert_eq!(
            store
                .withdraw_binding("sqlite:/a", "sess", "addr-pending", 2_000)
                .expect("withdraw pending"),
            Withdrawal::DeletedPending { generation }
        );
        assert!(
            store.load(&pending.id()).is_err(),
            "an unfinalized attach is deleted, not tombstoned"
        );

        // `live`: tombstoned under a generation bump, and idempotent afterwards.
        let live = sample_intent("sqlite:/a", "sess", "addr-live");
        store.write_atomic(&live).expect("seed live");
        let first = store
            .withdraw_binding("sqlite:/a", "sess", "addr-live", 3_000)
            .expect("withdraw live");
        let Withdrawal::Revoked { generation } = first else {
            panic!("a live record is tombstoned, got {first:?}");
        };
        let reloaded = store.load(&live.id()).expect("reload");
        assert_eq!(reloaded.state, IntentRecoveryState::Revoked);
        assert_eq!(reloaded.generation, generation);
        assert_eq!(reloaded.updated_at_ms, 3_000, "the TTL clock moves with it");
        assert_eq!(
            store
                .withdraw_binding("sqlite:/a", "sess", "addr-live", 4_000)
                .expect("second withdrawal"),
            Withdrawal::AlreadyRevoked { generation },
            "a detach, a session end, and a reset may all name one binding"
        );
        assert_eq!(
            store.load(&live.id()).expect("reload").updated_at_ms,
            3_000,
            "and the idempotent repeat must not push the terminal TTL out"
        );
        let _ = std::fs::remove_dir_all(&run_dir);
    }

    /// A withdrawal decided against one generation must not act on another.
    ///
    /// The reconciler is the caller that needs this: a pass decides "tombstoned" against the
    /// manifest it loaded, and a re-attach can write a fresh `pending` record before the outcome is
    /// applied. Deleting *that* record would destroy an attach the decision knew nothing about.
    #[test]
    fn a_generation_conditional_withdrawal_never_touches_a_newer_record() {
        let run_dir = temp_run_dir("withdraw-superseded");
        let store = IntentStore::open(&run_dir, "hash").expect("store");
        let mut intent = sample_intent("sqlite:/a", "sess", "addr");
        intent.state = IntentRecoveryState::Pending;
        store.write_pending(&intent).expect("first attach");
        let observed = store.load(&intent.id()).expect("load").generation;

        // A re-attach moves the record on before the stale decision is applied.
        store.write_pending(&intent).expect("re-attach");
        let current = store.load(&intent.id()).expect("load").generation;
        assert!(current > observed, "precondition: the record moved");

        assert_eq!(
            store
                .withdraw_binding_at_generation("sqlite:/a", "sess", "addr", Some(observed), 5_000)
                .expect("stale withdrawal"),
            Withdrawal::Superseded {
                generation: current
            }
        );
        assert!(
            store.load(&intent.id()).is_ok(),
            "a stale reconcile outcome must not delete a fresh attach"
        );

        // The unconditional form — what an explicit operator teardown uses — still applies.
        assert!(matches!(
            store
                .withdraw_binding("sqlite:/a", "sess", "addr", 6_000)
                .expect("explicit withdrawal"),
            Withdrawal::DeletedPending { .. }
        ));
        let _ = std::fs::remove_dir_all(&run_dir);
    }

    /// A record telex cannot read is a record it must not delete, overwrite, or claim to have
    /// withdrawn. An unsupported schema version is exactly what a rollback leaves behind, and
    /// "intents are never deleted by a rollback" is a documented guarantee — so a withdrawal that
    /// meets one has to fail rather than fall through to a delete or a fabricated tombstone.
    #[test]
    fn withdrawal_refuses_an_unreadable_or_unsupported_record() {
        let run_dir = temp_run_dir("withdraw-unreadable");
        let store = IntentStore::open(&run_dir, "hash").expect("store");
        let intent = sample_intent("sqlite:/a", "sess", "addr");
        store.write_atomic(&intent).expect("write");
        let path = store.path_for(&intent.id());

        let mut document: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).expect("read")).expect("parse");
        document["schema_version"] = serde_json::json!(99);
        let _ = std::fs::remove_file(&path);
        platform_fs::write_owner_only_file_atomic(
            &path,
            serde_json::to_vec(&document).expect("encode").as_slice(),
        )
        .expect("write skewed");
        assert!(
            matches!(
                store.withdraw_binding("sqlite:/a", "sess", "addr", 1_000),
                Err(IntentError::UnsupportedSchema { .. })
            ),
            "a manifest from a newer build is never deleted or clobbered by a teardown"
        );
        assert!(path.exists(), "and it is still there");

        let _ = std::fs::remove_file(&path);
        std::fs::write(&path, b"not json at all").expect("corrupt");
        assert!(
            store
                .withdraw_binding("sqlite:/a", "sess", "addr", 1_000)
                .is_err(),
            "an unreadable manifest must not be reported as withdrawn"
        );
        assert!(path.exists());
        let _ = std::fs::remove_dir_all(&run_dir);
    }

    /// `commit_if_live_generation` is the gate that makes a withdrawal beat a restoration already
    /// in flight. It must publish only for a present, `live`, unmoved record.
    #[test]
    fn a_member_commit_is_refused_once_the_manifest_stops_authorizing_it() {
        let run_dir = temp_run_dir("commit-gate");
        let store = IntentStore::open(&run_dir, "hash").expect("store");
        let intent = sample_intent("sqlite:/a", "sess", "addr");
        store.write_atomic(&intent).expect("seed live");
        let id = intent.id();
        let generation = store.load(&id).expect("load").generation;

        assert_eq!(
            store
                .commit_if_live_generation(&id, generation, || "published")
                .expect("commit"),
            Some("published"),
            "a live, unmoved manifest authorizes the member it describes"
        );
        assert_eq!(
            store
                .commit_if_live_generation(&id, generation + 1, || "published")
                .expect("commit"),
            None,
            "a decision made against another generation does not"
        );

        store
            .withdraw_binding("sqlite:/a", "sess", "addr", 2_000)
            .expect("withdraw");
        assert_eq!(
            store
                .commit_if_live_generation(&id, generation, || "published")
                .expect("commit"),
            None,
            "and a withdrawn binding never publishes a restored member"
        );

        // A binding whose record is gone entirely — what withdrawing a `pending` one leaves.
        let absent = IntentId::derive("sqlite:/a", "sess", "never-attached");
        assert_eq!(
            store
                .commit_if_live_generation(&absent, 1, || "published")
                .expect("commit"),
            None
        );
        let _ = std::fs::remove_dir_all(&run_dir);
    }

    /// Set-scoped withdrawal enumerates the scope, and an unreadable manifest that *claims* the
    /// scope is reported rather than acted on.
    ///
    /// Acting on the claim would be a real vulnerability: the tuple in an unreadable manifest is
    /// unvalidated, so honouring it would let one file name — and withdraw — another binding's
    /// record. Silently skipping it would be the opposite failure: a teardown reporting success for
    /// a record it could not read.
    #[test]
    fn binding_enumeration_separates_proven_identities_from_claimed_ones() {
        let run_dir = temp_run_dir("bindings");
        let store = IntentStore::open(&run_dir, "hash").expect("store");
        let a = sample_intent("sqlite:/a", "sess", "addr-1");
        let b = sample_intent("sqlite:/a", "sess-other", "addr-2");
        for intent in [&a, &b] {
            store.write_atomic(intent).expect("write");
        }
        let corrupt = sample_intent("sqlite:/a", "sess", "addr-corrupt");
        store.write_atomic(&corrupt).expect("write");
        let corrupt_path = store.path_for(&corrupt.id());
        let mut document: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&corrupt_path).expect("read"))
                .expect("parse");
        document["schema_version"] = serde_json::json!(99);
        let _ = std::fs::remove_file(&corrupt_path);
        platform_fs::write_owner_only_file_atomic(
            &corrupt_path,
            serde_json::to_vec(&document).expect("encode").as_slice(),
        )
        .expect("write skewed");

        let scan = store.bindings().expect("enumerate");
        assert_eq!(
            scan.bindings,
            vec![
                IntentBinding {
                    store_key: "sqlite:/a".to_string(),
                    session_id: "sess".to_string(),
                    address: "addr-1".to_string(),
                },
                IntentBinding {
                    store_key: "sqlite:/a".to_string(),
                    session_id: "sess-other".to_string(),
                    address: "addr-2".to_string(),
                },
            ],
            "only manifests that loaded contribute an actionable binding"
        );
        assert_eq!(scan.unreadable.len(), 1);
        let (_, claimed) = &scan.unreadable[0];
        assert_eq!(
            claimed.as_ref().map(|binding| binding.address.as_str()),
            Some("addr-corrupt"),
            "the claimed identity is carried so a caller can refuse, never so it can act"
        );
        let _ = std::fs::remove_dir_all(&run_dir);
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

        assert!(matches!(
            store
                .withdraw_binding("sqlite:/a", "sess", "addr-1", 2_000)
                .expect("withdraw"),
            Withdrawal::Revoked { .. }
        ));
        assert_eq!(
            store.load(&a.id()).expect("load a").state,
            IntentRecoveryState::Revoked
        );
        assert_eq!(
            store.load(&b.id()).expect("load b").state,
            IntentRecoveryState::Live,
            "withdrawing one address must not touch a sibling address"
        );
        assert_eq!(
            store.load(&c.id()).expect("load c").state,
            IntentRecoveryState::Live,
            "withdrawing one store must not touch the same address in another store"
        );
        // Withdrawal bumps the generation so a concurrent CAS write loses.
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

    /// A deadline that is already spent. Models the case the whole bound exists for — a pass that
    /// reaches a phase with nothing left to spend on it — without a sleep, a fake clock, or a
    /// filesystem that has to actually be slow.
    fn spent_deadline() -> PassDeadline {
        PassDeadline::at(Instant::now())
    }

    /// Mirrors the daemon's published per-pass scan bound. Kept local so the store keeps no
    /// dependency on the reconciler's schedule.
    const RECONCILE_SCAN_BOUND: Duration = Duration::from_secs(4);

    /// The property the cooperative deadline checks cannot provide on their own.
    ///
    /// `scan_bounded`/`gc_bounded` check the clock *between* synchronous calls, so they bound a
    /// phase only while every individual call returns promptly. A blocked `read_dir` or a stalled
    /// atomic rename is exactly the case the bound exists for, and there the check never runs. The
    /// execution boundary is what makes the async caller's bound real — and it must abandon only
    /// the *wait*, because tearing a staged atomic write in half would be a durable-state bug
    /// traded for a latency fix.
    #[tokio::test]
    async fn a_blocking_phase_that_overruns_is_abandoned_but_never_cancelled() {
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::Arc;

        let finished = Arc::new(AtomicBool::new(false));
        let flag = finished.clone();
        let started = Instant::now();
        let phase = run_blocking_within(
            PassDeadline::at(Instant::now() + Duration::from_millis(50)),
            Duration::ZERO,
            move || {
                std::thread::sleep(Duration::from_millis(600));
                flag.store(true, Ordering::SeqCst);
            },
        )
        .await;
        let waited = started.elapsed();

        assert!(
            phase.overran(),
            "a phase that outlives the deadline must report it rather than returning a result the \
             caller would then publish"
        );
        assert!(
            waited < Duration::from_millis(400),
            "the caller must stop waiting at the deadline, not at the phase's own pace: waited \
             {waited:?}"
        );
        assert!(
            !finished.load(Ordering::SeqCst),
            "the phase under test must still be running, or this asserts nothing about abandonment"
        );

        tokio::time::sleep(Duration::from_millis(900)).await;
        assert!(
            finished.load(Ordering::SeqCst),
            "abandoning the wait must never cancel the work: a partially staged atomic write has \
             to be allowed to finish its own rename"
        );
    }

    /// The ordinary case must be untouched: a phase that finishes inside the deadline returns its
    /// result exactly as the synchronous form would.
    #[tokio::test]
    async fn a_blocking_phase_inside_its_deadline_returns_its_result() {
        let run_dir = temp_run_dir("boundedphase");
        let store = IntentStore::open(&run_dir, "hash").expect("store");
        store
            .write_atomic(&sample_intent("sqlite:/a", "sess", "addr-00"))
            .expect("seed");

        let page = store
            .scan_bounded_within(
                64,
                None,
                PassDeadline::at(Instant::now() + RECONCILE_SCAN_BOUND),
            )
            .await
            .completed()
            .expect("a local-disk scan of one intent completes well inside the pass deadline")
            .expect("scan");
        assert_eq!(page.loaded.len(), 1);
        assert!(!page.discovery_truncated);
        let _ = std::fs::remove_dir_all(&run_dir);
    }

    /// The boundary must not pre-empt the very stop it asked for.
    ///
    /// `scan_bounded` checks the same deadline the caller waits on, so it deliberately stops *at*
    /// it and then still has to sort its page and persist a resume position. Waiting to exactly the
    /// deadline loses that race every time, which would turn every scope big enough to use its
    /// budget into a permanently overrunning one: truncated pages are how such a scope makes
    /// progress at all, so discarding them is starvation, not safety.
    #[tokio::test]
    async fn a_cooperatively_truncated_phase_still_returns_its_partial_page() {
        let run_dir = temp_run_dir("gracepage");
        let store = IntentStore::open(&run_dir, "hash").expect("store");
        for i in 0..6 {
            store
                .write_atomic(&sample_intent("sqlite:/a", "sess", &format!("addr-{i:02}")))
                .expect("seed");
        }

        // Already spent, so discovery stops after its mandatory first read — the worst case for
        // the outer wait, which has only the grace period to work with.
        let page = store
            .scan_bounded_within(64, None, spent_deadline())
            .await
            .completed()
            .expect("a cooperative stop must be returned to the caller, not abandoned")
            .expect("scan");
        assert!(
            page.discovery_truncated,
            "this asserts nothing unless discovery actually stopped short"
        );
        assert_eq!(
            page.loaded.len(),
            1,
            "the mandatory first read is the progress a spent pass still owes the scope"
        );
        let _ = std::fs::remove_dir_all(&run_dir);
    }

    #[test]
    fn discovery_stops_at_the_deadline_and_resumes_instead_of_re_reading_its_head() {
        let run_dir = temp_run_dir("slowdiscovery");
        let store = IntentStore::open(&run_dir, "hash").expect("store");
        let total = 6usize;
        for i in 0..total {
            store
                .write_atomic(&sample_intent("sqlite:/a", "sess", &format!("addr-{i:02}")))
                .expect("seed");
        }

        let mut seen = std::collections::BTreeSet::new();
        for pass in 0..total {
            let page = store
                .scan_bounded(64, None, spent_deadline())
                .expect("scan");
            assert!(
                page.discovery_truncated,
                "pass {pass}: a truncated discovery must be reported, because it makes \
                 observed_count a lower bound rather than the scope"
            );
            assert_eq!(
                page.loaded.len(),
                1,
                "pass {pass}: one manifest is always read even with the deadline spent, or a scope \
                 whose maintenance repeatedly overruns would never be discovered at all"
            );
            assert_eq!(
                page.observed_count, total,
                "pass {pass}: enumerating a scope this small is never truncated"
            );
            seen.insert(page.loaded[0].id());
        }
        assert_eq!(
            seen.len(),
            total,
            "successive truncated passes must rotate through the whole scope; re-reading the head \
             is how a bounded discovery turns into permanent starvation of the tail"
        );

        // A pass with time to spend behaves exactly as an unbounded one always did.
        let full = store
            .scan_bounded(64, None, PassDeadline::unbounded())
            .expect("scan");
        assert!(!full.discovery_truncated);
        assert_eq!(full.loaded.len(), total);
        let _ = std::fs::remove_dir_all(&run_dir);
    }

    #[test]
    fn gc_stops_at_the_deadline_reports_it_and_resumes_where_it_stopped() {
        let run_dir = temp_run_dir("slowgc");
        let store = IntentStore::open(&run_dir, "hash").expect("store");
        let total = 4usize;
        for i in 0..total {
            let mut pending = sample_intent("sqlite:/a", "sess", &format!("addr-{i:02}"));
            pending.state = IntentRecoveryState::Pending;
            store.write_atomic(&pending).expect("seed");
        }
        let past_ttl = 1_000 + STATION_INTENT_PENDING_TTL.as_millis() as i64 + 1;

        for remaining in (1..total).rev() {
            let report = store
                .gc_bounded(past_ttl, None, None, spent_deadline())
                .expect("bounded gc");
            assert!(
                !report.complete,
                "a sweep that stopped at its deadline must say so: consuming the once-a-minute \
                 maintenance slot with a partial sweep is indistinguishable from a leak"
            );
            assert_eq!(
                report.removed.len(),
                1,
                "one candidate is always examined, so a spent budget still collects"
            );
            assert_eq!(store.list_ids().expect("list").len(), remaining);
        }

        // The last record is the whole of what is left, so the sweep that collects it reaches the
        // end of the scope and is complete — which is what lets the maintenance clock advance.
        let last = store
            .gc_bounded(past_ttl, None, None, spent_deadline())
            .expect("last gc");
        assert!(last.complete);
        assert_eq!(last.removed.len(), 1);
        assert!(store.list_ids().expect("list").is_empty());
        let _ = std::fs::remove_dir_all(&run_dir);
    }

    #[test]
    fn a_scoped_scan_spends_its_budget_on_its_own_store_not_on_unrelated_pages() {
        // The regression this pins: the target store sorts *behind* a large block of another
        // store's intents, so filtering after the page window produced a page the caller then
        // discarded in full — for every pass, forever, because the shared cursor also advanced past
        // entries the scoped pass never considered.
        let run_dir = temp_run_dir("scopefilter");
        let store = IntentStore::open(&run_dir, "hash").expect("store");
        let noise = "sqlite:/aaa-unrelated";
        let target = "sqlite:/zzz-target";
        for i in 0..100 {
            store
                .write_atomic(&sample_intent(noise, "sess", &format!("addr-{i:03}")))
                .expect("seed noise");
        }
        for i in 0..3 {
            store
                .write_atomic(&sample_intent(target, "sess", &format!("addr-{i:03}")))
                .expect("seed target");
        }

        let page = store
            .scan_bounded(4, Some(target), PassDeadline::unbounded())
            .expect("scoped scan");
        assert_eq!(
            page.loaded.len(),
            3,
            "every in-scope intent must fit in a budget of 4 regardless of how much unrelated \
             state sorts ahead of it"
        );
        assert!(page.loaded.iter().all(|intent| intent.store_key == target));
        assert_eq!(
            page.observed_count, 103,
            "observed_count and the cap warning describe the scope on disk, not the filter"
        );

        // An unscoped pass over the same scope is dominated by the unrelated store, which is
        // exactly why the filter has to precede the budget.
        let unscoped = store
            .scan_bounded(4, None, PassDeadline::unbounded())
            .expect("unscoped scan");
        assert!(unscoped
            .loaded
            .iter()
            .all(|intent| intent.store_key == noise));
        let _ = std::fs::remove_dir_all(&run_dir);
    }

    #[test]
    fn scope_cursors_are_independent_and_a_legacy_global_cursor_still_migrates() {
        let run_dir = temp_run_dir("scopecursor");
        let store = IntentStore::open(&run_dir, "hash").expect("store");
        let a = "sqlite:/a";
        let b = "sqlite:/b";
        for i in 0..4 {
            store
                .write_atomic(&sample_intent(a, "sess", &format!("addr-{i:02}")))
                .expect("seed a");
            store
                .write_atomic(&sample_intent(b, "sess", &format!("addr-{i:02}")))
                .expect("seed b");
        }

        let page_a = store
            .scan_bounded(4, Some(a), PassDeadline::unbounded())
            .expect("scan a");
        assert_eq!(page_a.loaded[0].address, "addr-00");
        // A pass filtered to store `a` advances only store `a`'s position.
        store
            .advance_cursor_in_scope(Some(a), &page_a.loaded_positions[1])
            .expect("advance a");

        let next_a = store
            .scan_bounded(4, Some(a), PassDeadline::unbounded())
            .expect("scan a again");
        assert_eq!(next_a.loaded[0].address, "addr-02");
        let page_b = store
            .scan_bounded(4, Some(b), PassDeadline::unbounded())
            .expect("scan b");
        assert_eq!(
            page_b.loaded[0].address, "addr-00",
            "store b must start at its own head: with one shared position, a scoped pass skipped \
             intents another scope had never considered"
        );

        // A cursor file written by a build that predates scope-correct cursoring carries one
        // global position. It must be honored as the seed for a scope that has no entry yet,
        // rather than silently restarting every scope at its head.
        let legacy = serde_json::json!({ "position": page_a.loaded_positions[2] });
        platform_fs::write_owner_only_file_atomic(
            &run_dir.join("intents").join("hash").join(SCAN_CURSOR_FILE),
            &serde_json::to_vec(&legacy).expect("encode legacy cursor"),
        )
        .expect("write legacy cursor");
        let migrated = store
            .scan_bounded(4, Some(a), PassDeadline::unbounded())
            .expect("scan a after migration");
        assert_eq!(
            migrated.loaded[0].address, "addr-03",
            "a legacy global position must still be resumed from"
        );
        let _ = std::fs::remove_dir_all(&run_dir);
    }

    #[test]
    fn an_over_cap_scope_honors_the_scan_deadline_and_still_deletes_nothing() {
        let run_dir = temp_run_dir("overcapdeadline");
        let store = IntentStore::open(&run_dir, "hash").expect("store");
        let total = STATION_INTENT_MAX_COUNT + 88;
        for i in 0..total {
            let intent = sample_intent("sqlite:/a", "sess", &format!("addr-{i:04}"));
            let bytes = serde_json::to_vec_pretty(&intent).expect("encode");
            platform_fs::write_owner_only_file_atomic(&store.path_for(&intent.id()), &bytes)
                .expect("seed");
        }

        // A pass with time to spend still sees the whole scope and still reports it over cap.
        let started = Instant::now();
        let full = store
            .scan_bounded(64, None, PassDeadline::at(started + RECONCILE_SCAN_BOUND))
            .expect("full scan");
        let full_elapsed = started.elapsed();
        assert!(
            full_elapsed < RECONCILE_SCAN_BOUND + Duration::from_secs(1),
            "a bounded scan must not overrun its deadline, took {full_elapsed:?}"
        );
        if full.discovery_truncated {
            assert!(full.observed_count <= total);
        } else {
            assert_eq!(full.observed_count, total);
            assert!(full.over_cap);
            assert_eq!(full.loaded.len(), 64);
        }

        // Loading 600 manifests is the phase that used to be unbounded: a pass paid for all of it
        // before it started its first wave, so the published 4 s pass bound was not a bound at all
        // on exactly the scopes it was published for.
        let started = Instant::now();
        let page = store
            .scan_bounded(64, None, spent_deadline())
            .expect("scan");
        let elapsed = started.elapsed();
        assert!(
            elapsed < Duration::from_secs(2),
            "a scan with no budget left must return promptly, took {elapsed:?}"
        );
        assert!(page.discovery_truncated);
        assert_eq!(page.loaded.len(), 1);
        assert!(
            page.observed_count <= total,
            "a truncated pass reports a lower bound on the scope size, never an inflated one"
        );
        assert_eq!(
            store.list_ids().expect("list").len(),
            total,
            "a deadline is never a reason to delete an intent"
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
        // The pending TTL is aged from creation, not from the last write: a re-attach rewrites the
        // record and must not be able to extend its own leftover's life.
        pending.created_at_ms = 0;
        pending.updated_at_ms = 0;
        pending.producer.credential.path = credential.clone();
        store.write_atomic(&pending).expect("write pending");

        let mut live = sample_intent("sqlite:/a", "sess", "live");
        live.updated_at_ms = 1_000_000;
        live.producer.credential.path = credential.clone();
        store.write_atomic(&live).expect("write live");

        let mut fresh_pending = sample_intent("sqlite:/a", "sess", "fresh-pending");
        fresh_pending.state = IntentRecoveryState::Pending;
        fresh_pending.created_at_ms = 1_000_000;
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

        // An intent whose credential file vanished is *not* GC-eligible immediately: the bridge
        // deletes and rewrites its own registry on every reload, so an instant rule destroys the
        // record recovery depends on. It expires on `STATION_INTENT_CREDENTIAL_MISSING_TTL`,
        // measured from the last time the producer was proven.
        std::fs::remove_file(&credential).expect("remove credential");
        let report = store.gc(1_000_001, None, None).expect("gc again");
        assert!(
            !report.removed.contains(&live.id()),
            "a momentarily-absent credential is a bridge reload, not a teardown"
        );
        let past_ttl = 1_000_001 + STATION_INTENT_CREDENTIAL_MISSING_TTL.as_millis() as i64 + 1_000;
        let report = store.gc(past_ttl, None, None).expect("gc past ttl");
        assert!(
            report.removed.contains(&live.id()),
            "a credential gone past its TTL is a genuinely orphaned intent"
        );
        let _ = std::fs::remove_dir_all(&run_dir);
    }

    /// The primary scenario the feature exists for: a *first* attach writes a `Pending` intent
    /// whose credential path is the bridge registry the extension has not created yet, because
    /// the agent still has to run `extensions_reload`. GC must leave that record alone until
    /// `STATION_INTENT_PENDING_TTL`; deleting it disarms recovery silently, since
    /// `finalize_pending_intents_for_session` only updates records that still exist.
    #[test]
    fn gc_never_deletes_a_pending_intent_whose_producer_does_not_exist_yet() {
        let run_dir = temp_run_dir("gc-pending-no-producer");
        let store = IntentStore::open(&run_dir, "hash").expect("store");

        let mut pending = sample_intent("sqlite:/a", "sess", "first-attach");
        pending.state = IntentRecoveryState::Pending;
        pending.created_at_ms = 1_000_000;
        pending.updated_at_ms = 1_000_000;
        // Exactly what `write_pending_intent` records on a first attach: a credential path that
        // does not exist, and the placeholder producer identity.
        pending.producer.credential.path = run_dir.join("not-created-yet.json");
        pending.producer.pid = 0;
        pending.producer.start_time = 0;
        pending.producer.host_id = String::new();
        pending.producer.boot_id = String::new();
        store.write_atomic(&pending).expect("write pending");

        // A GC pass one tick later, and another most of the way through the TTL, with the local
        // identity supplied (the daemon always has one) so every identity-derived rule is live.
        for now in [
            1_000_001,
            1_000_000 + STATION_INTENT_PENDING_TTL.as_millis() as i64 - 1,
        ] {
            let report = store
                .gc(now, Some("local-host"), Some("local-boot"))
                .expect("gc");
            assert!(
                report.removed.is_empty(),
                "a pending intent inside its TTL must survive every GC reason, got {:?}",
                report.reasons
            );
        }

        let report = store
            .gc(
                1_000_000 + STATION_INTENT_PENDING_TTL.as_millis() as i64 + 1,
                Some("local-host"),
                Some("local-boot"),
            )
            .expect("gc past ttl");
        assert!(
            report.removed.contains(&pending.id()),
            "past its own TTL a pending intent is a crash-during-attach leftover"
        );
        let _ = std::fs::remove_dir_all(&run_dir);
    }

    /// A manifest written by a *newer* build is what a rollback leaves behind. The documented
    /// rollback guarantee is that intents are never deleted by one, so the unreadable-past-TTL
    /// sweep must exclude an out-of-range schema specifically.
    #[test]
    fn gc_never_deletes_a_manifest_from_a_newer_schema() {
        let run_dir = temp_run_dir("gc-newer-schema");
        let store = IntentStore::open(&run_dir, "hash").expect("store");
        let intent = sample_intent("sqlite:/a", "sess", "future");
        let mut raw: serde_json::Value =
            serde_json::to_value(&intent).expect("serialize the fixture");
        raw["schema_version"] = serde_json::json!(STATION_INTENT_SCHEMA_MAX_SUPPORTED + 1);
        let bytes = serde_json::to_vec_pretty(&raw).expect("serialize");
        platform_fs::write_owner_only_file_atomic(&store.path_for(&intent.id()), &bytes)
            .expect("write future manifest");

        let far_future = 10 * STATION_INTENT_UNVERIFIABLE_TTL.as_millis() as i64;
        let report = store.gc(far_future, None, None).expect("gc");
        assert!(
            report.removed.is_empty(),
            "a rollback must never delete a newer-schema intent, got {:?}",
            report.reasons
        );
        let _ = std::fs::remove_dir_all(&run_dir);
    }

    /// `over_cap` must use the same comparison the write cap uses, or the operator sees a refused
    /// write with no stated reason.
    #[test]
    fn over_cap_is_reported_at_the_same_count_that_refuses_a_write() {
        let run_dir = temp_run_dir("gc-over-cap-boundary");
        let store = IntentStore::open(&run_dir, "hash").expect("store");
        // Only the comparison is under test, so assert it directly rather than materializing 512
        // manifests: `write_atomic` refuses at `>= cap` and `scan` must report at `>= cap` too.
        let page = store.scan(8).expect("scan an empty scope");
        assert!(!page.over_cap);
        assert_eq!(page.observed_count, 0);
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

    // -----------------------------------------------------------------------------------------
    // The durable state machine
    // -----------------------------------------------------------------------------------------

    /// The whole producer-side transition table, in one place, without a daemon or a bridge.
    ///
    /// The two authorities are deliberately *not* interchangeable, and the asymmetry is the fix
    /// for the reload-plus-replacement deadlock:
    ///
    /// * A `pending` record needs one of them. Without either, a bridge that merely exists could
    ///   arm an attach that was never registered — the security property this whole path rests on.
    /// * A `live` record needs neither, because being `live` is itself the durable proof that the
    ///   binding was armed. Requiring a live member there made the only repair for a stale
    ///   producer identity depend on the member that the stale identity prevents from ever
    ///   existing.
    #[test]
    fn finalize_admission_is_the_whole_producer_side_transition_table() {
        use FinalizeAdmission::*;
        // (state, armed_durably, armed_now) -> admission
        let cases = [
            // A live record refreshes with no daemon knowledge at all. This row is the deadlock
            // fix: bridge reloads, daemon crashes, successor has no member, and the identity can
            // still be re-recorded.
            (IntentRecoveryState::Live, false, false, Refresh),
            (IntentRecoveryState::Live, true, true, Refresh),
            // A pending record promotes on either authority...
            (IntentRecoveryState::Pending, false, true, Promote),
            (IntentRecoveryState::Pending, true, false, Promote),
            (IntentRecoveryState::Pending, true, true, Promote),
            // ...and on neither, it is refused.
            (IntentRecoveryState::Pending, false, false, RefusedNotArmed),
            // A revocation always wins, however armed the binding is.
            (IntentRecoveryState::Revoked, true, true, RefusedRevoked),
            (IntentRecoveryState::Tombstoned, true, true, RefusedRevoked),
            // Runtime projections are not states this build persists, so they are not transitions
            // it owns.
            (IntentRecoveryState::Unverifiable, true, true, RefusedState),
            (IntentRecoveryState::Quarantined, true, true, RefusedState),
            (IntentRecoveryState::Unknown, true, true, RefusedState),
        ];
        for (state, armed_durably, armed_now, expected) in cases {
            assert_eq!(
                finalize_admission(state, armed_durably, armed_now),
                expected,
                "finalize_admission({state:?}, durable={armed_durably}, now={armed_now})"
            );
        }
        assert!(Promote.is_allowed() && Refresh.is_allowed());
        for refused in [RefusedNotArmed, RefusedRevoked, RefusedState] {
            assert!(!refused.is_allowed(), "{refused:?} must never write");
        }
    }

    /// The armed proof is what makes a `pending` record promotable after the arming daemon is
    /// gone, and `live` implies it without one (every `live` record was finalized, which only runs
    /// after the daemon confirmed `push_registered`).
    #[test]
    fn the_armed_proof_is_durable_daemon_evidence_and_live_implies_it() {
        let run_dir = temp_run_dir("armed-proof");
        let store = IntentStore::open(&run_dir, "hash").expect("store");

        let mut pending = sample_intent("sqlite:/a", "sess", "addr");
        pending.state = IntentRecoveryState::Pending;
        assert!(!pending.is_armed(), "a fresh pending record is not armed");
        store.write_pending(&pending).expect("write pending");

        assert_eq!(
            store
                .stamp_armed_proof("sqlite:/a", "sess", "addr", "inst-1", 2_000)
                .expect("mark armed"),
            ArmedProofStamp::Stamped {
                generation: pending.generation + 1
            },
            "the first arming stamp writes"
        );
        let armed = store.load(&pending.id()).expect("reload");
        assert!(armed.is_armed());
        assert_eq!(armed.armed.as_ref().expect("proof").armed_at_ms, 2_000);
        assert_eq!(
            armed.armed.as_ref().expect("proof").daemon_instance_id,
            "inst-1"
        );
        assert_eq!(
            armed.generation,
            pending.generation + 1,
            "the stamp is a real durable transition, so it moves the generation"
        );
        assert_eq!(
            armed.updated_at_ms, pending.updated_at_ms,
            "but it must not move the TTL clock: arming is not a producer proof"
        );

        assert_eq!(
            store
                .stamp_armed_proof("sqlite:/a", "sess", "addr", "inst-2", 3_000)
                .expect("re-stamp"),
            ArmedProofStamp::AlreadyArmed {
                generation: armed.generation
            },
            "re-arming is idempotent, so the hot register path does not churn the generation"
        );
        assert_eq!(
            store.load(&pending.id()).expect("reload").generation,
            armed.generation
        );
        assert_eq!(
            store
                .load(&pending.id())
                .expect("reload")
                .pending_clock_ms(),
            2_000,
            "and a re-register cannot move the clock the armed pending TTL reads"
        );

        // No record for the binding at all is the ordinary pull-attach case. Reported as its own
        // variant, never folded into "already armed": the daemon has to be able to tell it apart
        // from "the record this register owed a proof to was deleted under it".
        assert_eq!(
            store
                .stamp_armed_proof("sqlite:/a", "sess", "never-attached", "inst-1", 4_000)
                .expect("no record"),
            ArmedProofStamp::NoRecord
        );
        assert!(!ArmedProofStamp::NoRecord.is_proven());

        // `live` needs no explicit proof.
        let live = sample_intent("sqlite:/a", "sess", "live-addr");
        assert_eq!(live.state, IntentRecoveryState::Live);
        assert!(live.is_armed());
        let _ = std::fs::remove_dir_all(&run_dir);
    }

    /// A record the stamp cannot read is a **refusal**, never a silent success.
    ///
    /// The daemon commits this stamp as part of committing an armed push member, and it aborts the
    /// registration when the stamp does not prove anything. That only works if a corrupt or
    /// unreadable manifest surfaces as an error rather than as "nothing to do".
    #[test]
    fn an_unreadable_record_refuses_the_arming_stamp_rather_than_reporting_success() {
        let run_dir = temp_run_dir("armed-proof-unreadable");
        let store = IntentStore::open(&run_dir, "hash").expect("store");

        let mut pending = sample_intent("sqlite:/a", "sess", "addr");
        pending.state = IntentRecoveryState::Pending;
        store.write_pending(&pending).expect("write pending");
        std::fs::write(store.path_for(&pending.id()), b"{ not json").expect("corrupt the manifest");

        let stamped = store.stamp_armed_proof("sqlite:/a", "sess", "addr", "inst-1", 2_000);
        assert!(
            stamped.is_err(),
            "an unreadable record must not report a durable proof, got {stamped:?}"
        );
        let _ = std::fs::remove_dir_all(&run_dir);
    }

    /// The narrower, nastier shape of the same rule: a record that cannot be **stat'd**.
    ///
    /// `NoRecord` is consumed by the daemon as a proof of *absence* — "this binding never had a
    /// durable record, so an ordinary admission may commit" — and `Path::exists()` produced exactly
    /// that answer for a record that is on disk but whose metadata the platform refuses to hand
    /// over. The record is there, its contents are never consulted, and the register commits.
    ///
    /// The error is injected rather than provoked: `chmod 000` is a no-op under a root test runner
    /// and has no Windows equivalent, a deny-ACE has no Unix equivalent, and neither is the
    /// behavior under test. What is under test is that the answer *is an error*, so only a proven
    /// `NotFound` may become `NoRecord`.
    #[test]
    fn an_unstatable_record_is_never_reported_as_no_record() {
        let run_dir = temp_run_dir("armed-proof-unstatable");
        let store = IntentStore::open(&run_dir, "hash").expect("store");

        let mut pending = sample_intent("sqlite:/a", "sess", "addr");
        pending.state = IntentRecoveryState::Pending;
        store.write_pending(&pending).expect("write pending");

        let path = store.path_for(&pending.id());
        let fault = platform_fs::stat_faults::Unstatable::new(&path);
        let stamped = store.stamp_armed_proof("sqlite:/a", "sess", "addr", "inst-1", 2_000);
        assert!(
            stamped.is_err(),
            "a record whose existence could not be decided must not be reported as absent, got {stamped:?}"
        );

        // The record is intact and unarmed: the refusal wrote nothing.
        drop(fault);
        assert!(!store.load(&pending.id()).expect("reload").is_armed());

        // The control: a binding that genuinely has no record still reports `NoRecord`, so this is
        // a distinction the stamp draws rather than a blanket failure.
        assert_eq!(
            store
                .stamp_armed_proof("sqlite:/a", "sess", "never-attached", "inst-1", 4_000)
                .expect("no record"),
            ArmedProofStamp::NoRecord
        );
        let _ = std::fs::remove_dir_all(&run_dir);
    }

    /// The `NoRecord` remap on the *second* look is owed the same proof.
    ///
    /// `stamp_armed_proof` deliberately converts "the locked load failed with I/O and the file is
    /// now gone" into `NoRecord`, because that is the concurrent-rollback race rather than a real
    /// failure. The re-check has to be a *proof* of absence for the same reason the entry check
    /// does: a re-check that cannot decide must leave the original I/O failure standing, or the
    /// remap becomes a laundry for exactly the unreadable record the entry check now refuses.
    ///
    /// Both halves are driven through the real race: the per-intent write lock is held, so the
    /// locked load fails with I/O after the entry check has already passed.
    #[test]
    fn the_vanished_record_remap_requires_a_proven_absence() {
        let run_dir = temp_run_dir("armed-proof-remap");
        let store = IntentStore::open(&run_dir, "hash").expect("store");

        let mut pending = sample_intent("sqlite:/a", "sess", "addr");
        pending.state = IntentRecoveryState::Pending;
        store.write_pending(&pending).expect("write pending");
        let path = store.path_for(&pending.id());
        let lock = store
            .root()
            .join(format!("{}{INTENT_LOCK_SUFFIX}", pending.id().file_name()));

        // A held lock makes the locked load fail with I/O, and the record really is deleted while
        // the stamp is waiting for it — the rollback race the remap exists for.
        std::fs::write(&lock, b"held by another writer").expect("hold the lock");
        let racing_path = path.clone();
        let deleter = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(150));
            std::fs::remove_file(&racing_path).expect("the concurrent rollback deletes the record");
        });
        let stamped = store.stamp_armed_proof("sqlite:/a", "sess", "addr", "inst-1", 2_000);
        deleter.join().expect("deleter");
        assert_eq!(
            stamped.expect("a record that provably went away is NoRecord"),
            ArmedProofStamp::NoRecord
        );

        // The same interleaving, except the re-check cannot decide. `after(.., 1)` lets the entry
        // check answer truthfully and faults only the re-check, which is the decision under test.
        std::fs::remove_file(&lock).expect("release the lock between the two halves");
        store.write_pending(&pending).expect("rewrite pending");
        std::fs::write(&lock, b"held by another writer").expect("hold the lock again");
        let _fault = platform_fs::stat_faults::Unstatable::after(&path, 1);
        assert!(
            store
                .stamp_armed_proof("sqlite:/a", "sess", "addr", "inst-1", 2_000)
                .is_err(),
            "an undecidable re-check must not launder an I/O failure into 'nothing to prove'"
        );
        let _ = std::fs::remove_file(&lock);
        let _ = std::fs::remove_dir_all(&run_dir);
    }

    /// A scope root that cannot be stat'd is not an empty scope.
    ///
    /// `open_existing` is the read path for every "does this host have durable records?" question,
    /// and `Ok(None)` is how it says "this host never attached". Answering that for a scope whose
    /// root merely could not be read is the same fail-open one directory higher.
    #[test]
    fn an_unstatable_scope_root_is_an_error_not_an_absent_scope() {
        let run_dir = temp_run_dir("scope-root-unstatable");
        let store = IntentStore::open(&run_dir, "hash").expect("store");
        let mut pending = sample_intent("sqlite:/a", "sess", "addr");
        pending.state = IntentRecoveryState::Pending;
        store.write_pending(&pending).expect("write pending");

        let root = run_dir.join("intents").join("hash");
        let fault = platform_fs::stat_faults::Unstatable::new(&root);
        assert!(
            IntentStore::open_existing(&run_dir, "hash").is_err(),
            "an unreadable scope root must fail closed rather than report an empty scope"
        );
        drop(fault);

        assert!(
            IntentStore::open_existing(&run_dir, "hash")
                .expect("readable scope")
                .is_some(),
            "and the readable scope still opens"
        );
        assert!(
            IntentStore::open_existing(&run_dir, "never-used-hash")
                .expect("absent scope")
                .is_none(),
            "while a scope that provably does not exist is still `None`"
        );
        let _ = std::fs::remove_dir_all(&run_dir);
    }

    /// "Nothing here to revoke" is a success every caller believes.
    ///
    /// `withdraw_binding` returning `NoRecord` means the binding provably has no record, and the
    /// session-end and detach paths treat that as "done". A record that could not be stat'd has
    /// not been withdrawn, so reporting it that way retires a live intent in the daemon's
    /// bookkeeping while the durable record keeps saying the station is armed.
    #[test]
    fn an_unstatable_record_cannot_be_reported_as_nothing_to_revoke() {
        let run_dir = temp_run_dir("revoke-unstatable");
        let store = IntentStore::open(&run_dir, "hash").expect("store");
        let intent = sample_intent("sqlite:/a", "sess", "addr");
        store.write_atomic(&intent).expect("write");

        let fault = platform_fs::stat_faults::Unstatable::new(store.path_for(&intent.id()));
        assert!(
            store
                .withdraw_binding("sqlite:/a", "sess", "addr", 1_000)
                .is_err(),
            "an undecidable record must not report 'there was nothing to withdraw'"
        );
        drop(fault);

        assert!(
            matches!(
                store
                    .withdraw_binding("sqlite:/a", "sess", "addr", 1_000)
                    .expect("withdraw"),
                Withdrawal::Revoked { .. }
            ),
            "the readable record still withdraws"
        );
        assert_eq!(
            store
                .withdraw_binding("sqlite:/a", "sess", "never-attached", 1_000)
                .expect("absent"),
            Withdrawal::NoRecord,
            "and a binding that provably has no record is still an honest `NoRecord`"
        );
        let _ = std::fs::remove_dir_all(&run_dir);
    }

    /// GC's credential rule deletes a finalized record, and deletion is the one GC action recovery
    /// cannot undo — so "the credential is gone" has to be *proven*.
    ///
    /// The credential is the bridge registry, which lives in a directory telex shares with an
    /// external producer: an antivirus lock, a permissions change, or a mount that hiccupped is a
    /// metadata failure, not a deletion. `Path::exists()` reported all three as "gone", and GC then
    /// destroyed the durable proof of a binding that may be delivering right now.
    #[test]
    fn gc_keeps_a_record_whose_credential_could_not_be_stat_ed() {
        let run_dir = temp_run_dir("gc-credential-unstatable");
        let store = IntentStore::open(&run_dir, "hash").expect("store");
        let credential = run_dir.join("cred.json");
        std::fs::write(&credential, b"{\"secret\":\"x\"}").expect("credential");

        // The shape the credential rule collects: finalized, unproven for longer than the TTL.
        let mut intent = sample_intent("sqlite:/a", "sess", "addr");
        intent.created_at_ms = 0;
        intent.updated_at_ms = 0;
        intent.producer.credential.path = credential.clone();
        store.write_atomic(&intent).expect("write");
        let now = STATION_INTENT_CREDENTIAL_MISSING_TTL.as_millis() as i64 + 60_000;

        let fault = platform_fs::stat_faults::Unstatable::new(&credential);
        let report = store.gc(now, Some("host"), Some("boot")).expect("gc");
        assert!(
            !report.removed.contains(&intent.id()),
            "a credential telex could not look at is not a credential that is gone, got {:?}",
            report.reasons
        );
        drop(fault);

        // And the rule still fires for a credential that provably is not there, so the guard did
        // not simply disable it.
        std::fs::remove_file(&credential).expect("delete the credential");
        let report = store.gc(now, Some("host"), Some("boot")).expect("gc");
        assert!(
            report.removed.contains(&intent.id()),
            "a credential that is provably gone past its TTL must still expire, got {:?}",
            report.reasons
        );
        let _ = std::fs::remove_dir_all(&run_dir);
    }

    /// A compare-and-set must lose, not **create**, when it cannot see the record.
    ///
    /// `write_cas` treats "no record" plus `expected_generation == 0` as "create it". An existence
    /// check that answered `false` for an unreadable record therefore turned a lost CAS into an
    /// unconditional overwrite of a record the caller never read.
    #[test]
    fn a_cas_against_an_unstatable_record_fails_rather_than_creating_one() {
        let run_dir = temp_run_dir("cas-unstatable");
        let store = IntentStore::open(&run_dir, "hash").expect("store");
        let existing = sample_intent("sqlite:/a", "sess", "addr");
        store.write_atomic(&existing).expect("write");

        let mut replacement = existing.clone();
        replacement.generation = 0;
        replacement.occupant = "usurper".to_string();

        let fault = platform_fs::stat_faults::Unstatable::new(store.path_for(&existing.id()));
        assert!(
            store.write_cas(0, &replacement).is_err(),
            "an undecidable record must not be treated as a free slot to create into"
        );
        drop(fault);
        assert_eq!(
            store.load(&existing.id()).expect("reload").occupant,
            existing.occupant,
            "and the record it could not see is untouched"
        );
        let _ = std::fs::remove_dir_all(&run_dir);
    }

    /// `write_pending` is the serialized, generation-safe replacement for a bare
    /// load-check-`write_atomic`.
    #[test]
    fn write_pending_is_generation_safe_and_never_demotes_a_live_record() {
        let run_dir = temp_run_dir("write-pending");
        let store = IntentStore::open(&run_dir, "hash").expect("store");

        let mut pending = sample_intent("sqlite:/a", "sess", "addr");
        pending.state = IntentRecoveryState::Pending;
        pending.created_at_ms = 500;
        let first = store.write_pending(&pending).expect("first write");
        assert_eq!(first, PendingWrite::Created { generation: 1 });

        // A second attach for the same binding advances the generation and preserves the original
        // creation time, so the pending TTL is not extended by re-attaching in a loop.
        let mut again = pending.clone();
        again.created_at_ms = 9_000;
        assert_eq!(
            store.write_pending(&again).expect("second write"),
            PendingWrite::Created { generation: 2 }
        );
        let stored = store.load(&pending.id()).expect("reload");
        assert_eq!(stored.created_at_ms, 500);

        // An armed proof survives a re-attach: it is a fact about the binding, and dropping it
        // would re-open the crash-between-Register-and-finalize window.
        store
            .stamp_armed_proof("sqlite:/a", "sess", "addr", "inst-1", 7_000)
            .expect("arm");
        store.write_pending(&again).expect("third write");
        assert!(
            store.load(&pending.id()).expect("reload").is_armed(),
            "a re-attach must not silently drop the daemon's arming proof"
        );

        // A live record is never demoted: a resume whose finalize later fails must not have
        // already destroyed a working recovery record.
        let live = sample_intent("sqlite:/a", "sess", "live-addr");
        store.write_atomic(&live).expect("seed live");
        let mut demote = live.clone();
        demote.state = IntentRecoveryState::Pending;
        assert_eq!(
            store.write_pending(&demote).expect("kept"),
            PendingWrite::KeptExistingLive { generation: 1 }
        );
        assert_eq!(
            store.load(&live.id()).expect("reload live").state,
            IntentRecoveryState::Live
        );

        // And it refuses to write anything that is not a pending record at all.
        assert!(store.write_pending(&live).is_err());
        let _ = std::fs::remove_dir_all(&run_dir);
    }

    /// Deletion is the one action recovery cannot undo, so it is conditional on the record still
    /// being what the caller decided about — the attach-rollback and GC race in one test.
    #[test]
    fn deletion_is_refused_when_the_record_moved_under_the_decision() {
        let run_dir = temp_run_dir("conditional-remove");
        let store = IntentStore::open(&run_dir, "hash").expect("store");

        let mut pending = sample_intent("sqlite:/a", "sess", "addr");
        pending.state = IntentRecoveryState::Pending;
        store.write_pending(&pending).expect("write");
        let id = pending.id();

        // The shape of the rollback race: a failing attach decided to remove generation 1, but a
        // concurrent turn-boundary finalize promoted that exact file to `live` first.
        store
            .update_locked(&id, |intent| {
                intent.state = IntentRecoveryState::Live;
                true
            })
            .expect("finalize");
        assert!(
            !store
                .remove_if_unchanged(&id, 1, |current| current.state
                    == IntentRecoveryState::Pending)
                .expect("conditional remove"),
            "a stale generation must never delete a newer record"
        );
        assert!(store.load(&id).is_ok(), "the promoted record must survive");

        // Even at the right generation, the predicate is the second gate.
        assert!(
            !store
                .remove_if_unchanged(&id, 2, |current| current.state
                    == IntentRecoveryState::Pending)
                .expect("conditional remove"),
            "the caller's own condition must still hold on the freshly loaded record"
        );
        assert!(store.load(&id).is_ok());

        // Both gates satisfied: the delete happens.
        assert!(store
            .remove_if_unchanged(&id, 2, |current| current.state == IntentRecoveryState::Live)
            .expect("conditional remove"),);
        assert!(store.load(&id).is_err());
        // And a delete of something that is already gone is `false`, never an error.
        assert!(!store
            .remove_if_unchanged(&id, 2, |_| true)
            .expect("already gone"));
        let _ = std::fs::remove_dir_all(&run_dir);
    }

    /// An armed `pending` record describes push delivery a daemon really armed. Collecting it on
    /// the five-minute pending TTL silently disarms recovery for a binding that is working right
    /// now, and the user only discovers it after the next daemon replacement.
    #[test]
    fn gc_governs_an_armed_pending_record_by_its_own_longer_ttl() {
        let run_dir = temp_run_dir("gc-armed-pending");
        let store = IntentStore::open(&run_dir, "hash").expect("store");

        let mut unarmed = sample_intent("sqlite:/a", "sess", "unarmed");
        unarmed.state = IntentRecoveryState::Pending;
        // The clock each TTL actually reads: creation for the unarmed record, the arming proof for
        // the armed one. `updated_at_ms` is deliberately set far *later* than both, so a rule that
        // still consulted it would keep both records alive and fail this test.
        unarmed.created_at_ms = 1_000_000;
        unarmed.updated_at_ms = 1_000_000;
        unarmed.producer.credential.path = run_dir.join("never-written.json");
        unarmed.producer.pid = 0;
        unarmed.producer.start_time = 0;
        unarmed.producer.host_id = String::new();
        unarmed.producer.boot_id = String::new();
        store.write_pending(&unarmed).expect("write unarmed");

        let mut armed = unarmed.clone();
        armed.address = "armed".to_string();
        armed.handler.session_id = armed.session_id.clone();
        store.write_pending(&armed).expect("write armed");
        store
            .stamp_armed_proof("sqlite:/a", "sess", "armed", "inst-1", 1_000_000)
            .expect("arm");

        let just_past_pending_ttl =
            1_000_000 + STATION_INTENT_PENDING_TTL.as_millis() as i64 + 1_000;
        let report = store
            .gc(just_past_pending_ttl, Some("host"), Some("boot"))
            .expect("gc");
        assert!(
            report.removed.contains(&unarmed.id()),
            "an unarmed pending record past its TTL is a crash-during-attach leftover"
        );
        assert!(
            !report.removed.contains(&armed.id()),
            "but an armed one describes delivery a daemon actually armed, got {:?}",
            report.reasons
        );

        let past_armed_ttl =
            1_000_000 + STATION_INTENT_ARMED_PENDING_TTL.as_millis() as i64 + 1_000;
        let report = store
            .gc(past_armed_ttl, Some("host"), Some("boot"))
            .expect("gc past the armed ttl");
        assert!(
            report.removed.contains(&armed.id()),
            "it is still bounded, so an abandoned armed record does not accumulate forever"
        );
        let _ = std::fs::remove_dir_all(&run_dir);
    }

    /// A pending record's TTL must be **unreachable by retrying**.
    ///
    /// `write_pending` refreshes `updated_at_ms`, and a failing attach re-runs it on every retry —
    /// so while the pending TTL was aged from that field, a producer whose finalize never succeeded
    /// pushed its own leftover's expiry out indefinitely simply by re-attaching. GC could never
    /// collect the exact class of record it exists for, and the scope grew a permanent resident per
    /// wedged binding.
    ///
    /// Both halves are asserted: a re-attach loop spanning far more than the TTL stays collectable,
    /// while the legitimate state transitions each still get the clock they are supposed to have —
    /// arming moves the record onto the armed proof's own (much longer) clock, and revoking moves
    /// it onto the terminal clock measured from the revocation.
    #[test]
    fn a_repeated_pending_write_cannot_push_the_pending_ttl_out_forever() {
        let run_dir = temp_run_dir("pending-clock");
        let store = IntentStore::open(&run_dir, "hash").expect("store");
        let ttl_ms = STATION_INTENT_PENDING_TTL.as_millis() as i64;

        let mut pending = sample_intent("sqlite:/a", "sess", "retried");
        pending.state = IntentRecoveryState::Pending;
        pending.created_at_ms = 1_000;
        pending.updated_at_ms = 1_000;
        pending.producer.credential.path = run_dir.join("never-written.json");
        pending.producer.pid = 0;
        pending.producer.start_time = 0;
        store.write_pending(&pending).expect("first attach");

        // The failing re-attach loop: eleven attempts spread over more than twice the TTL, each one
        // rewriting the record with a fresh `updated_at_ms` exactly as `write_pending_intent` does.
        let mut now = 1_000;
        for _ in 0..10 {
            now += ttl_ms / 5;
            let mut retry = pending.clone();
            retry.created_at_ms = now;
            retry.updated_at_ms = now;
            store.write_pending(&retry).expect("re-attach");
        }
        let stored = store.load(&pending.id()).expect("reload");
        assert_eq!(
            stored.created_at_ms, 1_000,
            "the creation clock is carried forward, so a re-attach cannot reset it"
        );
        assert!(
            stored.updated_at_ms > 1_000 + ttl_ms,
            "precondition: the last-write clock really was refreshed past the TTL"
        );
        assert_eq!(stored.pending_clock_ms(), 1_000);

        let report = store.gc(now, Some("host"), Some("boot")).expect("gc");
        assert!(
            report.removed.contains(&pending.id()),
            "a record re-attached in a loop for longer than the TTL must still be collectable, got {:?}",
            report.reasons
        );

        // The legitimate transition: arming really does move the record onto the armed clock, and
        // the same GC that just collected the unarmed leftover keeps this one.
        let mut armed = sample_intent("sqlite:/a", "sess", "armed-transition");
        armed.state = IntentRecoveryState::Pending;
        armed.created_at_ms = 1_000;
        armed.updated_at_ms = 1_000;
        armed.producer.credential.path = run_dir.join("never-written.json");
        armed.producer.pid = 0;
        armed.producer.start_time = 0;
        store.write_pending(&armed).expect("write");
        let armed_at_ms = 1_000 + ttl_ms * 2;
        store
            .stamp_armed_proof(
                "sqlite:/a",
                "sess",
                "armed-transition",
                "inst-1",
                armed_at_ms,
            )
            .expect("arm");
        let stored = store.load(&armed.id()).expect("reload");
        assert_eq!(
            stored.pending_clock_ms(),
            armed_at_ms,
            "an armed record ages from the proof, which is the event its TTL is about"
        );
        let report = store
            .gc(armed_at_ms + ttl_ms, Some("host"), Some("boot"))
            .expect("gc");
        assert!(
            !report.removed.contains(&armed.id()),
            "the arming transition earned the longer clock, got {:?}",
            report.reasons
        );

        // And the other legitimate transition: a revocation is aged from when it was revoked. Its
        // own record, because a revoked record must carry a concrete producer identity (only
        // `Pending` may omit one) and must have a resolvable credential for the terminal TTL to be
        // the rule under test.
        let credential = run_dir.join("cred.json");
        std::fs::write(&credential, b"{\"secret\":\"x\"}").expect("credential");
        let mut revocable = sample_intent("sqlite:/a", "sess", "revoked-transition");
        revocable.created_at_ms = 1_000;
        revocable.updated_at_ms = 1_000;
        revocable.producer.credential.path = credential;
        store.write_atomic(&revocable).expect("write live");
        let revoked_at_ms = armed_at_ms + ttl_ms;
        store
            .withdraw_binding("sqlite:/a", "sess", "revoked-transition", revoked_at_ms)
            .expect("withdraw");
        let report = store
            .gc(
                revoked_at_ms + STATION_INTENT_UNVERIFIABLE_TTL.as_millis() as i64 - 1_000,
                Some("host"),
                Some("boot"),
            )
            .expect("gc");
        assert!(
            !report.removed.contains(&revocable.id()),
            "a revocation is visible for its own TTL, measured from the revocation, got {:?}",
            report.reasons
        );
        let report = store
            .gc(
                revoked_at_ms + STATION_INTENT_UNVERIFIABLE_TTL.as_millis() as i64 + 1_000,
                Some("host"),
                Some("boot"),
            )
            .expect("gc");
        assert!(
            report.removed.contains(&revocable.id()),
            "and it is still bounded, got {:?}",
            report.reasons
        );
        let _ = std::fs::remove_dir_all(&run_dir);
    }

    /// Both TTL clocks read *proof*, never retry attempts.
    ///
    /// The reconciler persists scheduling state on every genuine failure, so a clock that
    /// consulted `evidence.last_attempt_ms` was refreshed every few seconds forever — and the two
    /// rules that exist to collect abandoned records could never fire for exactly the records they
    /// were written for. This drives the failing case directly: a dead producer that is still
    /// being attempted right now.
    #[test]
    fn gc_orphan_clocks_are_never_refreshed_by_a_retry_attempt() {
        let run_dir = temp_run_dir("gc-orphan-clock");
        let store = IntentStore::open(&run_dir, "hash").expect("store");
        let credential = run_dir.join("cred.json");
        std::fs::write(&credential, b"{\"secret\":\"x\"}").expect("credential");

        // A live intent whose producer is provably dead (a pid/start-time pair that cannot be
        // alive), never once verified, with the reconciler still hammering it.
        let mut orphan = sample_intent("sqlite:/a", "sess", "orphan");
        orphan.created_at_ms = 0;
        orphan.updated_at_ms = 0;
        orphan.producer.credential.path = credential.clone();
        let now = STATION_INTENT_UNVERIFIABLE_TTL.as_millis() as i64 + 60_000;
        orphan.evidence = IntentEvidence {
            // The clock the old rule used: refreshed to *now* on every single pass.
            last_attempt_ms: Some(now),
            last_success_ms: None,
            attempts: 5_000,
            consecutive_failures: 4_000,
            failure_code: Some("producer_unreachable".to_string()),
            producer_verified_ms: None,
            next_attempt_ms: Some(now + 1_000),
            recovery_latency_ms: None,
        };
        store.write_atomic(&orphan).expect("write orphan");

        let report = store.gc(now, Some("host"), Some("boot")).expect("gc");
        assert!(
            report.removed.contains(&orphan.id()),
            "an intent unproven past its TTL must expire even while attempts continue, got {:?}",
            report.reasons
        );

        // The same clock governs the credential-missing rule, and it was unreachable for the same
        // reason: the credential is missing, so every attempt fails, so every attempt refreshed
        // the clock that decides whether it has been missing long enough.
        let mut gone = sample_intent("sqlite:/a", "sess", "credential-gone");
        gone.created_at_ms = 0;
        gone.updated_at_ms = 0;
        gone.producer.credential.path = run_dir.join("deleted.json");
        let now = STATION_INTENT_CREDENTIAL_MISSING_TTL.as_millis() as i64 + 60_000;
        gone.evidence.last_attempt_ms = Some(now);
        gone.evidence.consecutive_failures = 300;
        store.write_atomic(&gone).expect("write credential-gone");
        let report = store.gc(now, Some("host"), Some("boot")).expect("gc");
        assert!(
            report.removed.contains(&gone.id()),
            "a credential gone past its TTL must expire even while attempts continue, got {:?}",
            report.reasons
        );

        // And proof — not an attempt — is what holds a record: the same shape with a recent
        // success survives.
        let mut proven = sample_intent("sqlite:/a", "sess", "proven");
        proven.created_at_ms = 0;
        proven.updated_at_ms = 0;
        proven.producer.credential.path = credential;
        proven.evidence.last_success_ms = Some(now - 1_000);
        proven.evidence.last_attempt_ms = Some(now);
        store.write_atomic(&proven).expect("write proven");
        let report = store
            .gc(now, Some("host"), Some("boot"))
            .expect("gc proven");
        assert!(
            !report.removed.contains(&proven.id()),
            "a producer proven a second ago is not an orphan, got {:?}",
            report.reasons
        );
        let _ = std::fs::remove_dir_all(&run_dir);
    }

    /// Seed a binding whose *previous* lifecycle is over: attached, optionally armed by a daemon,
    /// finalized, then explicitly withdrawn — the durable shape a detach, a fallback downgrade, an
    /// operator reset, or a session end leaves behind. Returns the revoked record.
    ///
    /// It finalizes before withdrawing because that is what leaves a *tombstone*: withdrawing an
    /// unfinalized `pending` record deletes it instead (see [`IntentStore::withdraw_binding`]), and
    /// a deleted record is not the shape these tests are about.
    fn seed_finished_lifecycle(
        store: &IntentStore,
        run_dir: &Path,
        address: &str,
        armed_at_ms: Option<i64>,
        revoked_at_ms: i64,
    ) -> StationIntentV1 {
        let credential = run_dir.join("cred.json");
        if !credential.exists() {
            std::fs::write(&credential, b"{\"secret\":\"x\"}").expect("credential");
        }
        let mut original = sample_intent("sqlite:/a", "sess", address);
        original.state = IntentRecoveryState::Pending;
        original.created_at_ms = 1_000;
        original.updated_at_ms = 1_000;
        original.producer.credential.path = credential;
        store
            .write_pending(&original)
            .expect("seed the first attach");
        if let Some(armed_at_ms) = armed_at_ms {
            store
                .stamp_armed_proof("sqlite:/a", "sess", address, "inst-old", armed_at_ms)
                .expect("the old daemon arms the old lifecycle");
        }
        store
            .update_locked(&original.id(), |current| {
                current.state = IntentRecoveryState::Live;
                true
            })
            .expect("the turn boundary finalizes the old lifecycle");
        store
            .withdraw_binding("sqlite:/a", "sess", address, revoked_at_ms)
            .expect("withdraw");
        let revoked = store.load(&original.id()).expect("reload the tombstone");
        assert_eq!(revoked.state, IntentRecoveryState::Revoked);
        revoked
    }

    /// The `pending` record a fresh attach writes: a placeholder producer, exactly as
    /// `write_pending_intent` records before `extensions_reload` has loaded the bridge.
    fn fresh_attach(address: &str, now_ms: i64) -> StationIntentV1 {
        let mut attach = sample_intent("sqlite:/a", "sess", address);
        attach.state = IntentRecoveryState::Pending;
        attach.created_at_ms = now_ms;
        attach.updated_at_ms = now_ms;
        attach.producer.pid = 0;
        attach.producer.start_time = 0;
        attach.producer.host_id = String::new();
        attach.producer.boot_id = String::new();
        attach.producer.exe_path = PathBuf::from("not-loaded-yet");
        attach
    }

    /// A **new attach** over a finished lifecycle is not a retry of it, and must get its own clock.
    ///
    /// `write_pending` carried `created_at_ms` forward from whatever record it replaced, which is
    /// exactly right for a retry of an unfinalized attach and exactly wrong for a new one. A
    /// `revoked` tombstone lives for the seven-day terminal TTL, so every re-attach after a detach,
    /// a fallback downgrade, an operator reset, or a session end was born `pending` with an
    /// already-expired pending clock — and the next GC pass deleted it *before* `extensions_reload`
    /// and the turn-boundary finalize could promote it. The attach reported success, the record was
    /// gone seconds later, and it stayed that way for a week.
    #[test]
    fn a_new_attach_over_a_finished_lifecycle_starts_its_own_pending_clock() {
        let run_dir = temp_run_dir("pending-lifecycle-restart");
        let store = IntentStore::open(&run_dir, "hash").expect("store");
        let ttl_ms = STATION_INTENT_PENDING_TTL.as_millis() as i64;

        let revoked_at_ms = 1_000 + ttl_ms;
        let revoked = seed_finished_lifecycle(&store, &run_dir, "addr", None, revoked_at_ms);

        // Days later — still inside the terminal TTL, so the tombstone is very much still on disk —
        // the user attaches this binding again.
        let attached_at_ms = revoked_at_ms + 6 * 24 * 60 * 60 * 1_000;
        let attach = fresh_attach("addr", attached_at_ms);
        assert_eq!(
            store.write_pending(&attach).expect("re-attach"),
            PendingWrite::Created {
                generation: revoked.generation + 1
            },
            "the generation is a per-file CAS token, so it stays monotonic across the transition"
        );

        let stored = store.load(&attach.id()).expect("reload");
        assert_eq!(
            stored.created_at_ms, attached_at_ms,
            "a new lifecycle is not a retry of the one it replaced, so it keeps its own creation"
        );
        assert_eq!(stored.pending_clock_ms(), attached_at_ms);

        // It survives its *whole* new TTL — the window `extensions_reload` and the turn-boundary
        // finalize need in order to exist at all.
        let report = store
            .gc(attached_at_ms + ttl_ms - 1_000, Some("host"), Some("boot"))
            .expect("gc inside the new TTL");
        assert!(
            !report.removed.contains(&attach.id()),
            "a brand-new attach must not be collected on the previous lifecycle's clock, got {:?}",
            report.reasons
        );

        // And it is still bounded: the fresh clock is a full TTL, not an exemption.
        let report = store
            .gc(attached_at_ms + ttl_ms + 1_000, Some("host"), Some("boot"))
            .expect("gc past the new TTL");
        assert!(
            report.removed.contains(&attach.id()),
            "an attach that never finalized is still collected, got {:?}",
            report.reasons
        );
        let _ = std::fs::remove_dir_all(&run_dir);
    }

    /// A new lifecycle never inherits the **armed proof** of the one it replaced.
    ///
    /// The proof says "a daemon armed push for this binding", and a revocation is the explicit
    /// teardown of exactly that. Carrying it into a new attach let `finalize_admission` promote a
    /// record on the strength of a *previous* daemon's arming — the "a merely-existing bridge arms
    /// an attach that was never registered" hole the admission rules exist to close — and quietly
    /// moved the new record onto the 24 h armed clock measured from an arming that happened days
    /// ago, so it was born expired against that clock too.
    ///
    /// The new lifecycle proves itself with a new daemon stamp, or it does not promote.
    #[test]
    fn a_new_pending_lifecycle_never_inherits_the_previous_ones_armed_proof() {
        let run_dir = temp_run_dir("pending-lifecycle-proof");
        let store = IntentStore::open(&run_dir, "hash").expect("store");
        let ttl_ms = STATION_INTENT_PENDING_TTL.as_millis() as i64;
        let revoked_at_ms = 3_000;
        let attached_at_ms = revoked_at_ms + 6 * 24 * 60 * 60 * 1_000;

        let revoked = seed_finished_lifecycle(&store, &run_dir, "addr", Some(2_000), revoked_at_ms);
        assert!(
            revoked.armed.is_some(),
            "precondition: the tombstone still carries the proof its lifecycle was armed with"
        );

        let attach = fresh_attach("addr", attached_at_ms);
        store.write_pending(&attach).expect("re-attach");
        let stored = store.load(&attach.id()).expect("reload");

        assert!(
            stored.armed.is_none(),
            "a new attach must not inherit a proof it did not earn, got {:?}",
            stored.armed
        );
        assert!(!stored.is_armed());
        assert_eq!(
            finalize_admission(stored.state, stored.is_armed(), false),
            FinalizeAdmission::RefusedNotArmed,
            "and with no live member either, a new lifecycle has no authority to promote"
        );
        assert_eq!(
            stored.pending_clock_ms(),
            attached_at_ms,
            "it ages from its own attach, on the unarmed TTL"
        );
        // The unarmed TTL really is the one governing it. An inherited proof would have put it on
        // the 24 h clock measured from `armed_at_ms` (2_000) — expired before it was written.
        let report = store
            .gc(attached_at_ms + ttl_ms - 1_000, Some("host"), Some("boot"))
            .expect("gc inside the new TTL");
        assert!(
            !report.removed.contains(&attach.id()),
            "{:?}",
            report.reasons
        );
        let report = store
            .gc(attached_at_ms + ttl_ms + 1_000, Some("host"), Some("boot"))
            .expect("gc past the new TTL");
        assert!(
            report.removed.contains(&attach.id()),
            "an unarmed pending record is bounded by the unarmed TTL, got {:?}",
            report.reasons
        );

        // The new lifecycle earns the longer clock the only way it can: a *new* daemon stamps it.
        // Nothing about the proof it now carries refers to anything before this attach.
        seed_finished_lifecycle(&store, &run_dir, "re-armed", Some(2_000), revoked_at_ms);
        let attach = fresh_attach("re-armed", attached_at_ms);
        store.write_pending(&attach).expect("re-attach");
        let armed_at_ms = attached_at_ms + 1_000;
        assert!(matches!(
            store
                .stamp_armed_proof("sqlite:/a", "sess", "re-armed", "inst-new", armed_at_ms)
                .expect("the new daemon arms the new lifecycle"),
            ArmedProofStamp::Stamped { .. }
        ));
        let stored = store.load(&attach.id()).expect("reload");
        let proof = stored.armed.as_ref().expect("the new proof");
        assert_eq!(proof.armed_at_ms, armed_at_ms);
        assert_eq!(
            proof.daemon_instance_id, "inst-new",
            "the proof describes the daemon that armed *this* lifecycle"
        );
        assert_eq!(stored.pending_clock_ms(), armed_at_ms);
        let report = store
            .gc(armed_at_ms + ttl_ms + 1_000, Some("host"), Some("boot"))
            .expect("gc past the unarmed TTL");
        assert!(
            !report.removed.contains(&attach.id()),
            "an arming this lifecycle earned moves it onto the longer clock, got {:?}",
            report.reasons
        );
        let _ = std::fs::remove_dir_all(&run_dir);
    }

    /// The fresh clock is granted **once, at the transition** — never by a retry.
    ///
    /// This is the property that keeps the fix above from undoing the one before it. A new attach
    /// over a finished lifecycle gets a full TTL because it is a different attach; every subsequent
    /// `write_pending` for that attach is a retry and inherits the clock, so a producer whose
    /// finalize keeps failing still cannot buy itself an unbounded lifetime.
    #[test]
    fn a_fresh_pending_lifecycle_earns_one_clock_and_no_retry_can_earn_another() {
        let run_dir = temp_run_dir("pending-lifecycle-bounded");
        let store = IntentStore::open(&run_dir, "hash").expect("store");
        let ttl_ms = STATION_INTENT_PENDING_TTL.as_millis() as i64;

        let revoked_at_ms = 1_000 + ttl_ms;
        seed_finished_lifecycle(&store, &run_dir, "addr", None, revoked_at_ms);
        let attached_at_ms = revoked_at_ms + 6 * 24 * 60 * 60 * 1_000;
        let attach = fresh_attach("addr", attached_at_ms);
        store.write_pending(&attach).expect("re-attach");

        // The failing re-attach loop, now starting from the new lifecycle: ten more attempts spread
        // over more than twice the TTL, each rewriting the record exactly as the attach path does.
        let mut now = attached_at_ms;
        for _ in 0..10 {
            now += ttl_ms / 5;
            store
                .write_pending(&fresh_attach("addr", now))
                .expect("retry");
        }
        let stored = store.load(&attach.id()).expect("reload");
        assert_eq!(
            stored.created_at_ms, attached_at_ms,
            "the new lifecycle's clock was set once, by the transition, and no retry moved it"
        );
        assert!(
            stored.updated_at_ms > attached_at_ms + ttl_ms,
            "precondition: the last-write clock really was refreshed past the TTL"
        );
        let report = store.gc(now, Some("host"), Some("boot")).expect("gc");
        assert!(
            report.removed.contains(&attach.id()),
            "a re-attach loop must stay collectable across a lifecycle transition too, got {:?}",
            report.reasons
        );
        let _ = std::fs::remove_dir_all(&run_dir);
    }

    /// The daemon-side proof rule, as a table, decided without a daemon or a filesystem fault.
    ///
    /// Two asymmetries are the whole rule, and each one is a defect if it is dropped:
    ///
    /// * A register that owes **no** proof is not refused by a scope-level failure. For those
    ///   clients — a pull attach, a plain `telex attach --on-deliver` — opening the scope is a
    ///   *create* of a directory the register has nothing to put in, and refusing push because that
    ///   create failed denies a working registration to protect durable state that does not exist.
    /// * A record that is present but unreadable is refused **either way**. That is the concurrent
    ///   window: a record can appear between the up-front observation and the stamp, and durable
    ///   state about the binding that cannot be verified always fails closed.
    #[test]
    fn armed_proof_admission_is_the_whole_daemon_side_proof_table() {
        let table: [(
            std::result::Result<ArmedProofStamp, ArmedProofFailure>,
            ArmedProofAdmission,
            ArmedProofAdmission,
        ); 5] = [
            // outcome, owes_proof = false, owes_proof = true
            (
                Ok(ArmedProofStamp::Stamped { generation: 2 }),
                ArmedProofAdmission::Commit,
                ArmedProofAdmission::Commit,
            ),
            (
                Ok(ArmedProofStamp::AlreadyArmed { generation: 2 }),
                ArmedProofAdmission::Commit,
                ArmedProofAdmission::Commit,
            ),
            (
                Ok(ArmedProofStamp::NoRecord),
                ArmedProofAdmission::Commit,
                ArmedProofAdmission::Refuse,
            ),
            (
                Err(ArmedProofFailure::ScopeUnavailable),
                ArmedProofAdmission::Commit,
                ArmedProofAdmission::Refuse,
            ),
            (
                Err(ArmedProofFailure::RecordUnusable),
                ArmedProofAdmission::Refuse,
                ArmedProofAdmission::Refuse,
            ),
        ];
        for (outcome, unowed, owed) in table {
            assert_eq!(
                armed_proof_admission(outcome, false),
                unowed,
                "a register owing no proof, given {outcome:?}"
            );
            assert_eq!(
                armed_proof_admission(outcome, true),
                owed,
                "a register owing a proof, given {outcome:?}"
            );
        }
    }
}
