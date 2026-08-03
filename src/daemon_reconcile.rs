//! Daemon-owned station-intent reconciliation (issue #106 / ADR 0050).
//!
//! Reconciliation is **its own daemon operation**, not a side effect of `Register`. That framing is
//! what makes the safety properties statable:
//!
//! * It is single-flight per scope, suppressed while draining, budgeted by both a per-pass count
//!   and a per-pass wall-clock deadline shorter than the tick that starts it, and per-intent timed
//!   out — so a pass can never overrun its own tick and one wedged intent can never starve others.
//! * It checks the durable detach tombstone **unconditionally** before and after the epoch claim,
//!   and the tombstone-*clearing* branch is structurally unreachable from this module. An
//!   explicitly detached station therefore never auto-returns.
//! * It preserves retry state and the CC watermark, so repeated passes are idempotent and no
//!   message committed during a restart gap becomes invisible.
//! * It never force-steals an incumbent — reconciled or fresh. `AlreadyOwned` on a lease that is
//!   merely *not stale yet* is a **waiting** outcome (`DeferredLease`) on a fixed cadence, not a
//!   failure, which is exactly what makes the published crash-recovery bound derivable.
//!
//! ## Two-level API
//!
//! [`reconcile_once`] is the **acquiring** entry point: it owns the single-flight guard, the pass
//! scheduling, and it takes the per-`MemberKey` admission guard for each intent.
//! [`reconcile_intent_locked`] is the **guard-free inner** routine: it *assumes* the caller already
//! holds that guard. The anti-downgrade check inside `register_member` may only call the latter,
//! because `register_member` already holds the admission guard for the whole call and that guard is
//! documented as outermost and non-reentrant — calling `reconcile_once` there would self-deadlock
//! the hottest register path.

use super::*;
use crate::daemon_ipc::{
    DrainIntentReport, IntentRecoveryState, IntentStatus, ReconcileReport, REDACTED_SECRET,
};
use crate::handler_kinds::{self, StoreSelector};
use crate::platform_fs;
use crate::station_intent::{
    self, IntentEvidence, IntentId, IntentStore, ProducerTransport, StationIntentV1,
    BRIDGE_PROBE_TIMEOUT, STATION_INTENT_MAX_COUNT,
};
use std::collections::BTreeSet;
use std::sync::OnceLock;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt};

// ---------------------------------------------------------------------------------------------
// Constants and published bounds
// ---------------------------------------------------------------------------------------------

/// Reconciliation rides the existing heartbeat tick rather than adding a second loop.
/// `HEARTBEAT_INTERVAL` is deliberately **not** made env-overridable: it carries a documented,
/// test-enforced invariant against `ON_DELIVER_DEFERRED_BACKSTOP`.
pub const RECONCILE_INTERVAL: Duration = HEARTBEAT_INTERVAL;

/// Wall-clock ceiling for one pass. Strictly less than `RECONCILE_INTERVAL`, so a pass can never
/// overrun the tick that started it and the single-flight guard never has to skip a tick in the
/// normal case.
pub const RECONCILE_PASS_DEADLINE: Duration = Duration::from_secs(4);

/// Whole per-intent budget: probe + local validation + backend claim.
pub const RECONCILE_PER_INTENT_TIMEOUT: Duration = Duration::from_secs(3);

/// Upper bound on intents attempted per pass — an upper bound, not a guarantee the pass completes;
/// the deadline may cut it short, and the round-robin cursor resumes where it stopped.
pub const RECONCILE_PASS_BUDGET: usize = 64;

/// Herd cap, and simultaneously the guaranteed *minimum* progress per pass: the first wave always
/// runs, so even when every intent consumes its full timeout a pass advances by this many.
pub const RECONCILE_MAX_CONCURRENCY: usize = 4;

/// Per-intent **failure** backoff ladder. Never applied to `DeferredLease`.
pub const RECONCILE_BACKOFF_INITIAL: Duration = Duration::from_secs(5);
pub const RECONCILE_BACKOFF_MAX: Duration = Duration::from_secs(5 * 60);
/// Jitter as a percentage of the computed delay, applied +/- to break herds.
pub const RECONCILE_BACKOFF_JITTER_PCT: u64 = 20;

/// Retry cadence for `DeferredLease` — an incumbent lease that is simply not stale yet. Fixed, no
/// exponential growth, no jitter: on the crash path this is the *expected* outcome of every attempt
/// during `liveness_window_secs()`, so treating it as failure would push recovery past its bound.
pub const RECONCILE_DEFERRED_LEASE_RETRY: Duration = RECONCILE_INTERVAL;

/// Consecutive genuine failures before an intent drops to the slow retry cadence.
pub const RECONCILE_QUARANTINE_AFTER: u32 = 10;
pub const RECONCILE_QUARANTINE_RETRY: Duration = Duration::from_secs(60 * 60);

/// How often an *unchanged* healthy intent's evidence block is refreshed on disk.
///
/// Long enough that a steady state costs one manifest rewrite per intent per minute rather than
/// per tick, short enough that `evidence.last_success_ms` stays a usable "the producer was proven
/// this recently" clock for GC and for a successor daemon seeding its retry state.
pub const EVIDENCE_REFRESH_INTERVAL: Duration = Duration::from_secs(60);

/// Client-side capability gate: below this daemon minor, a client must not write or finalize an
/// intent, because the daemon would never act on it.
pub const RECONCILE_MIN_DAEMON_MINOR: u16 = 5;

/// Below this producer protocol the producer is *legacy*, not *failed*: its liveness is unprovable,
/// so it is never auto-restored, but it never wedges anything either.
pub const BRIDGE_PROBE_MIN_PROTOCOL: u32 = 2;

/// Byte cap for a producer credential file. Generous enough for a real bridge registry, small
/// enough that a hostile file cannot be used to exhaust memory.
pub const CREDENTIAL_MAX_BYTES: u64 = 64 * 1024;

/// Frame cap for a probe response, and the length cap for any failure code derived from it. The
/// producer is peer-verified but never trusted to be well-behaved, and this is the only external
/// JSON line the daemon reads.
pub const PROBE_MAX_RESPONSE_BYTES: u64 = 16 * 1024;
pub const FAILURE_CODE_MAX_CHARS: usize = 64;

/// Reduce a producer-supplied error code to something safe to retain and re-serialize: lowercase
/// `[a-z0-9_]`, length-capped, never empty.
fn sanitize_failure_code(code: &str) -> String {
    let mut out = String::with_capacity(FAILURE_CODE_MAX_CHARS);
    for ch in code.chars() {
        if out.len() >= FAILURE_CODE_MAX_CHARS {
            break;
        }
        let ch = ch.to_ascii_lowercase();
        if ch.is_ascii_alphanumeric() || ch == '_' {
            out.push(ch);
        } else if !out.ends_with('_') && !out.is_empty() {
            out.push('_');
        }
    }
    let trimmed = out.trim_matches('_');
    if trimmed.is_empty() {
        "rejected".to_string()
    } else {
        trimmed.to_string()
    }
}

/// Rotating reconcile event log, reusing the hook-log idiom (single rotation, size cap).
const RECONCILE_EVENT_LOG_FILE: &str = "reconcile-events.ndjson";
const RECONCILE_EVENT_LOG_ROTATE_BYTES: u64 = 1_048_576;

/// Compile-time assertions for the invariants the published bounds are derived from. If any of
/// these ever stops holding, the documented recovery bounds stop being true, so they are enforced
/// here rather than only asserted in a test.
const _: () = {
    assert!(RECONCILE_PASS_DEADLINE.as_millis() < RECONCILE_INTERVAL.as_millis());
    assert!(RECONCILE_PER_INTENT_TIMEOUT.as_millis() <= RECONCILE_PASS_DEADLINE.as_millis());
    assert!(BRIDGE_PROBE_TIMEOUT.as_millis() < RECONCILE_PER_INTENT_TIMEOUT.as_millis());
    assert!(RECONCILE_DEFERRED_LEASE_RETRY.as_millis() <= RECONCILE_INTERVAL.as_millis());
    assert!(RECONCILE_MAX_CONCURRENCY <= RECONCILE_PASS_BUDGET);
};

/// Graceful drain / upgrade recovery bound, in milliseconds, derived from the constants rather than
/// asserted: one tick, plus a probe, plus a local-validation and backend-claim allowance.
/// Qualified to an intent in a scope whose live-intent count is `<= RECONCILE_PASS_BUDGET` and
/// which is not in failure backoff, pull-waiter backoff, or quarantine.
pub fn graceful_recovery_bound_ms() -> u64 {
    RECONCILE_INTERVAL.as_millis() as u64
        + BRIDGE_PROBE_TIMEOUT.as_millis() as u64
        + (RECONCILE_PER_INTENT_TIMEOUT.as_millis() as u64
            - BRIDGE_PROBE_TIMEOUT.as_millis() as u64)
}

/// Hard-crash recovery bound, in milliseconds: a crashed predecessor never releases its lease, so
/// every attempt inside `liveness_window_secs()` is `DeferredLease`. The first attempt at or after
/// the stale cutoff lands within one `RECONCILE_DEFERRED_LEASE_RETRY` of it. Same qualifications.
pub fn crash_recovery_bound_ms() -> u64 {
    (liveness_window_secs().max(0) as u64) * 1000 + graceful_recovery_bound_ms()
}

/// Deterministic maximum queue delay for a scope larger than one pass budget. Not a recovery
/// bound: a *ceiling* on how long an intent can wait to be attempted, using the guaranteed
/// per-pass progress in the pathological case where every intent consumes its full timeout.
pub fn max_queue_delay_ms(live_intents: usize) -> u64 {
    let passes = live_intents.div_ceil(RECONCILE_MAX_CONCURRENCY.max(1));
    passes as u64 * RECONCILE_INTERVAL.as_millis() as u64
}

// ---------------------------------------------------------------------------------------------
// Outcomes
// ---------------------------------------------------------------------------------------------

/// Per-intent outcome class. The retry policy is a function of this type and nothing else, so
/// "which outcomes back off" is decidable by reading one `match`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IntentOutcome {
    /// A member was created (or push was restored onto an existing member).
    Restored,
    /// The member already exists with this push handler; nothing was written anywhere.
    RefreshedNoOp,
    /// The incumbent epoch lease is simply not stale yet. A waiting state on a fixed cadence:
    /// never backed off, never counted toward quarantine.
    DeferredLease,
    /// A live armed pull waiter owns the address. Pull-waiter precedence is preserved, so the
    /// intent waits (with its own backoff) rather than forcing the conflict.
    DeferredPullWaiter,
    /// A genuine failure: probe, credential, backend, timeout, or a lost claim to a *fresh*
    /// competing owner. The only class that enters the exponential ladder or quarantine.
    Failed { code: String },
    /// Terminal or inert. Surfaced and GC-governed, never retried on the fast cadence.
    Terminal {
        state: IntentRecoveryState,
        code: String,
    },
}

impl IntentOutcome {
    fn failed(code: impl Into<String>) -> Self {
        IntentOutcome::Failed { code: code.into() }
    }

    fn terminal(state: IntentRecoveryState, code: impl Into<String>) -> Self {
        IntentOutcome::Terminal {
            state,
            code: code.into(),
        }
    }

    /// The recovery state this outcome projects to.
    pub fn projected_state(&self) -> IntentRecoveryState {
        match self {
            IntentOutcome::Restored | IntentOutcome::RefreshedNoOp => IntentRecoveryState::Restored,
            IntentOutcome::DeferredLease => IntentRecoveryState::DeferredLease,
            IntentOutcome::DeferredPullWaiter => IntentRecoveryState::DeferredPullWaiter,
            IntentOutcome::Failed { .. } => IntentRecoveryState::Unverifiable,
            IntentOutcome::Terminal { state, .. } => *state,
        }
    }

    pub fn failure_code(&self) -> Option<&str> {
        match self {
            IntentOutcome::Failed { code } => Some(code),
            IntentOutcome::Terminal { code, .. } => Some(code),
            _ => None,
        }
    }

    fn is_success(&self) -> bool {
        matches!(self, IntentOutcome::Restored | IntentOutcome::RefreshedNoOp)
    }
}

// ---------------------------------------------------------------------------------------------
// The cached in-memory intent index
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct IntentKey {
    pub store_key: String,
    pub session_id: String,
    pub address: String,
}

impl IntentKey {
    fn from_intent(intent: &StationIntentV1) -> Self {
        Self {
            store_key: intent.store_key.clone(),
            session_id: intent.session_id.clone(),
            address: intent.address.clone(),
        }
    }

    fn intent_id(&self) -> IntentId {
        IntentId::derive(&self.store_key, &self.session_id, &self.address)
    }
}

#[derive(Debug, Clone, Default)]
pub struct IntentIndexEntry {
    pub state: IntentRecoveryState,
    pub generation: u64,
    pub wake_on_cc: bool,
    pub cc_watermark_ms: Option<i64>,
    pub last_attempt_ms: Option<i64>,
    pub last_success_ms: Option<i64>,
    pub attempts: u64,
    pub consecutive_failures: u32,
    pub failure_code: Option<String>,
    pub producer_verified_ms: Option<i64>,
    pub next_attempt_ms: Option<i64>,
    pub recovery_latency_ms: Option<i64>,
    /// When the intent was first observed live by this daemon, so `recovery_latency_ms` measures
    /// "how long did recovery actually take" rather than "how old is the manifest".
    pub first_seen_ms: Option<i64>,
}

#[derive(Debug, Default)]
pub struct IntentIndexSnapshot {
    pub entries: BTreeMap<IntentKey, IntentIndexEntry>,
    pub as_of_ms: i64,
    pub over_cap: bool,
    pub observed_count: usize,
    /// Manifests the last pass rejected before it could establish which binding they name. They
    /// cannot reach the keyed index at all, so they are carried here rather than lost.
    pub unidentifiable: usize,
}

/// Result of consulting the durable intent scope for one binding on a read path.
#[derive(Debug)]
pub(crate) enum LiveIntentLookup {
    /// The scope was readable and holds a live intent for this binding.
    Live(Box<StationIntentV1>),
    /// The scope was readable and holds no live intent for this binding.
    Absent,
    /// The scope (or the manifest) could not be read. Callers must fail closed.
    Unavailable(String),
}

/// Everything the daemon holds for reconciliation: the cached index, the single-flight guard, and
/// the trigger/report seam.
pub struct IntentRuntime {
    index: Mutex<IntentIndexSnapshot>,
    single_flight: AsyncMutex<()>,
    /// The one scheduling seam. Startup, the heartbeat tick, upgrade/rollback, `ReconcileIntents`,
    /// and tests all pulse this; nothing bypasses per-intent `next_attempt_ms`.
    pub(crate) trigger: Notify,
    report_tx: tokio::sync::watch::Sender<ReconcileReport>,
    report_rx: tokio::sync::watch::Receiver<ReconcileReport>,
    pass_seq: AtomicU64,
}

impl Default for IntentRuntime {
    fn default() -> Self {
        let (report_tx, report_rx) = tokio::sync::watch::channel(ReconcileReport::default());
        Self {
            index: Mutex::new(IntentIndexSnapshot::default()),
            single_flight: AsyncMutex::new(()),
            trigger: Notify::new(),
            report_tx,
            report_rx,
            pass_seq: AtomicU64::new(0),
        }
    }
}

impl IntentRuntime {
    /// The cached index, for the test-support harness only: a test that models a daemon
    /// replacement has to be able to drop the projection while the durable scope survives.
    #[cfg(feature = "sqlite")]
    pub(crate) fn index_for_test(&self) -> &Mutex<IntentIndexSnapshot> {
        &self.index
    }
}

impl DaemonState {
    /// Pulse the reconcile trigger. Schedules work; never bypasses backoff, quarantine, or a
    /// deferred outcome's next-attempt time.
    pub fn pulse_reconcile(&self) {
        self.intents.trigger.notify_one();
    }
    /// Subscribe to per-pass reports. Callers await the next `pass_seq` instead of polling a clock.
    pub fn reconcile_reports(&self) -> tokio::sync::watch::Receiver<ReconcileReport> {
        self.intents.report_rx.clone()
    }

    pub fn intent_index_snapshot(&self) -> IntentIndexSnapshot {
        let index = self.intents.index.lock().unwrap();
        IntentIndexSnapshot {
            entries: index.entries.clone(),
            as_of_ms: index.as_of_ms,
            over_cap: index.over_cap,
            observed_count: index.observed_count,
            unidentifiable: index.unidentifiable,
        }
    }

    fn index_entry(&self, key: &IntentKey) -> Option<IntentIndexEntry> {
        self.intents.index.lock().unwrap().entries.get(key).cloned()
    }

    fn index_upsert(&self, key: IntentKey, entry: IntentIndexEntry) {
        let mut index = self.intents.index.lock().unwrap();
        index.as_of_ms = now_ms();
        index.entries.insert(key, entry);
    }

    fn index_remove(&self, key: &IntentKey) {
        let mut index = self.intents.index.lock().unwrap();
        index.as_of_ms = now_ms();
        index.entries.remove(key);
    }

    /// Drop index entries for intents GC just deleted, so the cache can never project a row for a
    /// manifest that no longer exists.
    fn index_prune_removed(&self, removed: &[IntentId]) {
        if removed.is_empty() {
            return;
        }
        let removed: BTreeSet<&str> = removed.iter().map(|id| id.as_str()).collect();
        let stale: Vec<IntentKey> = self
            .intents
            .index
            .lock()
            .unwrap()
            .entries
            .keys()
            .filter(|key| removed.contains(key.intent_id().as_str()))
            .cloned()
            .collect();
        for key in stale {
            self.index_remove(&key);
        }
    }

    /// The intent store for this daemon's scope, opened lazily.
    ///
    /// Creating: used by the reconciler and by every path that may legitimately have to bring the
    /// scope into existence. Read-only callers use [`DaemonState::intent_store_readonly`].
    pub(crate) fn intent_store(&self) -> Option<IntentStore> {
        match IntentStore::open(&self.paths.run_dir, &self.paths.singleton_hash) {
            Ok(store) => Some(store),
            Err(e) => {
                self.push_recent_error(
                    "StationIntent",
                    format!("cannot open the station-intent scope: {e}"),
                );
                None
            }
        }
    }

    /// The intent scope for **read paths**, which must not create it as a side effect.
    ///
    /// `Ok(None)` is "this host never attached"; `Err` is "the scope exists but could not be
    /// opened", which callers must treat as fail-closed rather than as "no intents". A status call
    /// on a host that never attached now genuinely creates nothing — the previous comment claimed
    /// this while calling the creating variant.
    pub(crate) fn intent_store_readonly(&self) -> std::result::Result<Option<IntentStore>, String> {
        IntentStore::open_existing(&self.paths.run_dir, &self.paths.singleton_hash)
            .map_err(|e| e.to_string())
    }

    /// Revoke every intent for a session, in every store the daemon knows about. Used by the
    /// daemon's own session-end paths (`sessionEnd`, watch-pid death, definite end), so an ended
    /// session can never be re-attended by a stale intent.
    pub(crate) fn revoke_intents_for_session(&self, store_key: &str, session_id: &str) {
        let Some(store) = self.intent_store() else {
            return;
        };
        match store.revoke_session(store_key, session_id, now_ms()) {
            Ok(0) => {}
            Ok(count) => {
                let mut index = self.intents.index.lock().unwrap();
                index.as_of_ms = now_ms();
                for (key, entry) in index.entries.iter_mut() {
                    if key.store_key == store_key && key.session_id == session_id {
                        entry.state = IntentRecoveryState::Revoked;
                    }
                }
                drop(index);
                self.push_recent_error(
                    "StationIntent",
                    format!(
                        "revoked {count} station intent(s) for ended session {session_id} in {store_key}"
                    ),
                );
            }
            Err(e) => self.push_recent_error(
                "StationIntent",
                format!("revoking station intents for session {session_id}: {e}"),
            ),
        }
    }

    /// Revoke exactly one binding's intent. Used by detach and by the fallback downgrade, where the
    /// durable tombstone is written *first* so a crash between the two leaves tombstone-wins.
    pub(crate) fn revoke_intent_for_binding(
        &self,
        store_key: &str,
        session_id: &str,
        address: &str,
    ) {
        let Some(store) = self.intent_store() else {
            return;
        };
        match store.revoke(store_key, session_id, address, now_ms()) {
            Ok(true) => {
                let key = IntentKey {
                    store_key: store_key.to_string(),
                    session_id: session_id.to_string(),
                    address: address.to_string(),
                };
                if let Some(mut entry) = self.index_entry(&key) {
                    entry.state = IntentRecoveryState::Revoked;
                    self.index_upsert(key, entry);
                }
            }
            Ok(false) => {}
            Err(e) => self.push_recent_error(
                "StationIntent",
                format!("revoking station intent {session_id}/{address}: {e}"),
            ),
        }
    }

    /// Status projection for intent rows, built from the cached index only.
    pub(crate) fn intent_statuses(&self, store_filter: Option<&str>) -> Vec<IntentStatus> {
        let snapshot = self.intent_index_snapshot();
        let mut rows = Vec::new();
        for (key, entry) in snapshot.entries.iter() {
            if let Some(filter) = store_filter {
                if key.store_key != filter {
                    continue;
                }
            }
            let has_member = self
                .get_member(&key.store_key, &key.session_id, &key.address)
                .is_some();
            rows.push(IntentStatus {
                store_key: key.store_key.clone(),
                session_id: key.session_id.clone(),
                address: key.address.clone(),
                state: entry.state,
                generation: entry.generation,
                delivery_mode: DeliveryMode::Push,
                wake_on_cc: entry.wake_on_cc,
                has_member,
                cc_watermark_ms: entry.cc_watermark_ms,
                last_attempt_ms: entry.last_attempt_ms,
                last_success_ms: entry.last_success_ms,
                attempts: entry.attempts,
                failure_code: entry.failure_code.clone(),
                producer_verified_ms: entry.producer_verified_ms,
                next_attempt_ms: entry.next_attempt_ms,
                recovery_latency_ms: entry.recovery_latency_ms,
                index_as_of_ms: Some(snapshot.as_of_ms),
            });
        }
        rows
    }

    /// Whether this binding has a `live` push intent, according to the cached index.
    pub(crate) fn live_push_intent(&self, key: &IntentKey) -> Option<IntentIndexEntry> {
        self.index_entry(key).filter(|entry| {
            matches!(
                entry.state,
                IntentRecoveryState::Live
                    | IntentRecoveryState::Restored
                    | IntentRecoveryState::DeferredLease
                    | IntentRecoveryState::DeferredPullWaiter
            )
        })
    }

    /// Three-way lookup of the durable live intent for a binding.
    ///
    /// The index is a cache; the manifest is the truth, so the anti-downgrade guard re-reads it
    /// rather than acting on a possibly-stale cached state. It needs the third case as well:
    /// "the scope could not be opened" is exactly the `Insecure` condition the rest of the design
    /// fails closed on, and folding it into "no intent" made the guard fail **open** in the one
    /// window it exists for — a daemon replacement where the index has not been populated yet.
    pub(crate) fn lookup_live_intent(&self, key: &IntentKey) -> LiveIntentLookup {
        let store = match self.intent_store_readonly() {
            Ok(Some(store)) => store,
            Ok(None) => return LiveIntentLookup::Absent,
            Err(e) => return LiveIntentLookup::Unavailable(e),
        };
        let id = IntentId::derive(&key.store_key, &key.session_id, &key.address);
        if !store.path_for(&id).exists() {
            return LiveIntentLookup::Absent;
        }
        match store.load(&id) {
            Ok(intent) if intent.is_reconcilable() => LiveIntentLookup::Live(Box::new(intent)),
            Ok(_) => LiveIntentLookup::Absent,
            Err(e) => LiveIntentLookup::Unavailable(e.to_string()),
        }
    }

    /// Whether a `pending` (not yet finalized) push intent exists for this binding.
    ///
    /// Read from the durable manifest rather than the index: a pending intent is never reconciled,
    /// so no pass ever indexes it, and this is the one question the index cannot answer.
    pub(crate) fn pending_push_intent(
        &self,
        store_key: &str,
        session_id: &str,
        address: &str,
    ) -> bool {
        let Ok(Some(store)) = self.intent_store_readonly() else {
            return false;
        };
        let id = IntentId::derive(store_key, session_id, address);
        store
            .load(&id)
            .is_ok_and(|intent| intent.state == IntentRecoveryState::Pending)
    }

    /// The pre/post-drain intent signal.
    ///
    /// Computed from the cached index only — no directory scan, no probe, no network I/O — so it
    /// can never push a graceful drain past `--drain-timeout-ms`. It is evaluated *before* the
    /// lease-release loop, so it describes what a successor will find.
    pub(crate) fn drain_intent_report(&self) -> DrainIntentReport {
        let snapshot = self.intent_index_snapshot();
        let mut report = DrainIntentReport {
            over_cap: snapshot.over_cap,
            observed_count: snapshot.observed_count,
            index_as_of_ms: snapshot.as_of_ms,
            ..Default::default()
        };
        for entry in snapshot.entries.values() {
            match entry.state {
                IntentRecoveryState::Live
                | IntentRecoveryState::Restored
                | IntentRecoveryState::DeferredLease
                | IntentRecoveryState::DeferredPullWaiter => report.recoverable += 1,
                // `Pending` is **not** recoverable: it is never reconciled by construction
                // (`is_reconcilable` is `Live`-only), so counting it as "a compatible successor
                // will restore this automatically" overstated the pre-drain signal and made
                // `upgrade` wait out its successor timeout for a pass that could restore nothing.
                // It gets its own counter so the number is still visible.
                IntentRecoveryState::Pending => report.pending += 1,
                IntentRecoveryState::Unverifiable
                | IntentRecoveryState::Insecure
                | IntentRecoveryState::Quarantined
                | IntentRecoveryState::OwnershipConflict => report.degraded += 1,
                IntentRecoveryState::Incompatible | IntentRecoveryState::LegacyProducer => {
                    report.incompatible += 1
                }
                IntentRecoveryState::Revoked | IntentRecoveryState::Tombstoned => {}
                IntentRecoveryState::Unknown => report.unknown += 1,
            }
        }
        // Entries the scan could not identify at all (a manifest rejected before its
        // `(store, session, address)` could be read) cannot reach the keyed index, so they are
        // carried separately rather than silently reported as zero.
        report.unidentifiable = snapshot.unidentifiable;
        report.degraded += snapshot.unidentifiable;
        report
    }
}

// ---------------------------------------------------------------------------------------------
// Local identity, cached
// ---------------------------------------------------------------------------------------------

/// Local host/boot identity, resolved once. Both fail closed: an intent whose identity cannot be
/// recomputed is `Unverifiable`, never verified.
fn local_identity() -> std::result::Result<&'static (String, String), &'static str> {
    static IDENTITY: OnceLock<std::result::Result<(String, String), String>> = OnceLock::new();
    match IDENTITY.get_or_init(|| {
        let host = platform_fs::host_id().map_err(|e| e.to_string())?;
        let boot = platform_fs::boot_id().map_err(|e| e.to_string())?;
        Ok((host, boot))
    }) {
        Ok(identity) => Ok(identity),
        Err(_) => Err("host_boot_identity_unresolved"),
    }
}

// ---------------------------------------------------------------------------------------------
// Store selector resolution
// ---------------------------------------------------------------------------------------------

/// Map an opaque `store_key` back to the CLI selector flags a restored handler needs.
///
/// The daemon holds only a store key, so this mapping is unavoidable; it is named and fallible
/// rather than hidden. Failure — no match, an ambiguous match, or a profile that no longer exists —
/// yields `Unverifiable` with `store_selector_unresolved`: no member is created and no store is
/// opened.
pub fn store_selector_for_key(store_key: &str) -> std::result::Result<StoreSelector, String> {
    #[cfg(feature = "sqlite")]
    if let Some(path) = store_key.strip_prefix("sqlite:") {
        if path.is_empty() {
            return Err("empty sqlite store path".to_string());
        }
        return Ok(StoreSelector::new(None, Some(path.to_string())));
    }
    #[cfg(feature = "postgres")]
    if store_key.starts_with("pg:") || store_key.starts_with("postgres") {
        return match resolve_postgres_profile_for_store_key(store_key) {
            Ok((name, _profile)) => Ok(StoreSelector::new(Some(name), None)),
            Err(response) => Err(match response {
                Response::Error { message, .. } => message,
                other => format!("{other:?}"),
            }),
        };
    }
    // Fall back to the profile scan for any other shape, so a named SQLite profile still resolves.
    #[cfg(feature = "postgres")]
    {
        if let Ok((name, _)) = resolve_postgres_profile_for_store_key(store_key) {
            return Ok(StoreSelector::new(Some(name), None));
        }
    }
    Err(format!(
        "no configured backend profile matches store key {store_key}"
    ))
}

/// Open a backend for a store key **without ever creating one**.
///
/// A missing SQLite file or an unconfigured profile marks the intent `Unverifiable`; reconciliation
/// must never bring a store into existence as a side effect of restoring a handler.
async fn backend_open_existing_only(
    state: &Arc<DaemonState>,
    store_key: &str,
) -> std::result::Result<Arc<dyn Backend>, String> {
    if let Some(entry) = state.stores.lock().unwrap().get(store_key).cloned() {
        return Ok(entry.backend);
    }
    #[cfg(feature = "sqlite")]
    if let Some(path) = store_key.strip_prefix("sqlite:") {
        if !Path::new(path).exists() {
            return Err("store_missing".to_string());
        }
    }
    match state.backend_for(store_key).await {
        Ok(backend) => Ok(backend),
        Err(Response::Error { code, message, .. }) => Err(format!("{code}: {message}")),
        Err(other) => Err(format!("{other:?}")),
    }
}

// ---------------------------------------------------------------------------------------------
// Producer probe
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct ProbeSuccess {
    #[allow(dead_code)]
    bridge_generation: u64,
    verified_at_ms: i64,
}

/// Resolve the producer credential.
///
/// Two independent axes, deliberately kept apart:
///
/// * **Projected state** — a failed *security* check is `Insecure`, an unresolvable or stale
///   credential is `Unverifiable`. That is what status and the drain report show.
/// * **Retry policy** — `IntentOutcome::Terminal` means "inert, retried only on the quarantine
///   cadence"; `IntentOutcome::Failed` means "take the 5 s → 5 min ladder". Every *transient*
///   producer condition takes the ladder even though it projects `Unverifiable`: the credential is
///   the bridge registry, rewritten on a heartbeat and deleted/recreated on every reload, so a
///   stale mtime, a truncated read, or a missing field is routinely a one-tick condition. Parking
///   those for an hour replaces the published recovery bounds with a wedge that never self-heals.
///
/// In no failure mode is a secret read, a connection made, or a probe sent.
fn resolve_credential(intent: &StationIntentV1) -> std::result::Result<String, IntentOutcome> {
    let credential = &intent.producer.credential;
    let resolved = handler_kinds::resolve_credential_path(&credential.root_id, &credential.path)
        .map_err(|e| match e {
            handler_kinds::RegistryError::UnknownProducerRoot(_) => IntentOutcome::terminal(
                IntentRecoveryState::Unverifiable,
                "credential_root_unregistered",
            ),
            // A containment *decision* is a security verdict; a containment check that could not
            // be made because the file is not there right now is not. `resolve_credential_path`
            // keeps the two apart so an absent registry during a bridge reload is retried rather
            // than declared insecure.
            handler_kinds::RegistryError::ContainmentUnreadable(_) => {
                IntentOutcome::failed("credential_unresolved")
            }
            handler_kinds::RegistryError::Containment(_) => {
                IntentOutcome::terminal(IntentRecoveryState::Insecure, "credential_outside_root")
            }
            _ => IntentOutcome::failed("credential_unresolved"),
        })?;

    // One open handle for the age gate *and* the read: Plan decision 2 requires that the age or
    // size decision can never be made against a different inode than the bytes that were used.
    // The size, owner, DACL, and reparse checks all run on that handle before any byte is copied.
    // The trade-off this makes explicit: a stale file's bytes do reach memory, but the age gate
    // below runs before the secret is ever extracted from them, connected to, or sent.
    let (bytes, meta) =
        platform_fs::read_owner_only_file_with_meta(&resolved, CREDENTIAL_MAX_BYTES).map_err(
            |e| match e {
                platform_fs::FsError::Unsupported { .. } => {
                    IntentOutcome::terminal(IntentRecoveryState::Insecure, "credential_insecure")
                }
                platform_fs::FsError::Io { .. } => IntentOutcome::failed("credential_unreadable"),
            },
        )?;
    let max_age_ms = credential.clamped_max_age_ms();
    let now = now_ms();
    match meta.modified_ms {
        Some(modified_ms) if now.saturating_sub(modified_ms) > max_age_ms => {
            return Err(IntentOutcome::failed("credential_stale"));
        }
        Some(_) => {}
        None => return Err(IntentOutcome::failed("credential_age_unknown")),
    }

    let document: serde_json::Value = serde_json::from_slice(&bytes)
        .map_err(|_| IntentOutcome::failed("credential_malformed"))?;
    station_intent::json_pointer_str(&document, &credential.pointer)
        .map(|s| s.to_string())
        .ok_or_else(|| IntentOutcome::failed("credential_field_missing"))
}

fn producer_endpoint(intent: &StationIntentV1) -> Option<Endpoint> {
    match intent.producer.transport {
        #[cfg(windows)]
        ProducerTransport::NamedPipe => {
            Some(Endpoint::WindowsPipe(intent.producer.endpoint_path.clone()))
        }
        #[cfg(unix)]
        ProducerTransport::UnixSocket => Some(Endpoint::UnixSocket(PathBuf::from(
            &intent.producer.endpoint_path,
        ))),
        _ => None,
    }
}

/// Connect, prove the peer, then probe.
///
/// The order is the point: `verify_server_peer` runs **before anything is sent**, so the secret
/// only ever reaches a process that has already been proven to be the same user, the recorded
/// executable, and the recorded `(pid, start_time)`. Identity is never inferred from the answer.
async fn probe_producer(
    intent: &StationIntentV1,
    secret: &str,
) -> std::result::Result<ProbeSuccess, IntentOutcome> {
    let Some(endpoint) = producer_endpoint(intent) else {
        return Err(IntentOutcome::terminal(
            IntentRecoveryState::Unverifiable,
            "producer_transport_unsupported",
        ));
    };
    let conn = match tokio::time::timeout(BRIDGE_PROBE_TIMEOUT, platform::connect(&endpoint)).await
    {
        Ok(Ok(conn)) => conn,
        Ok(Err(_)) => return Err(IntentOutcome::failed("producer_unreachable")),
        Err(_) => return Err(IntentOutcome::failed("producer_connect_timeout")),
    };
    // Same-user ownership, executable match, and pid+start-time identity, all in one call, before
    // a single byte leaves this process. A platform that cannot resolve the peer fails closed.
    if platform::verify_server_peer(
        &conn,
        &intent.producer.exe_path,
        Some(intent.producer.pid),
        Some(intent.producer.start_time),
    )
    .is_err()
    {
        // Retryable, not terminal: the overwhelmingly common cause is a bridge *reload*
        // (`extensions_reload`, `/clear`, an extension-host restart), which gives the producer a
        // new pid and start time while the manifest still names the old pair. The turn-boundary
        // hook refreshes the recorded identity from the live registry, so the ladder is what lets
        // this heal on its own instead of parking the binding for the quarantine hour.
        return Err(IntentOutcome::failed("producer_identity_mismatch"));
    }

    let nonce = probe_nonce();
    let request = serde_json::json!({
        "op": "probe",
        "nonce": nonce,
        "protocol": BRIDGE_PROBE_MIN_PROTOCOL,
        "secret": secret,
    });
    let mut line = serde_json::to_string(&request)
        .map_err(|_| IntentOutcome::failed("probe_encode_failed"))?;
    line.push('\n');

    let (read_half, mut write_half) = tokio::io::split(conn);
    // Frame cap on the producer's answer, mirroring the daemon's own `MAX_JSONL_FRAME_BYTES`
    // policy. This is the only place the daemon reads an external JSON line, and the producer is
    // verified but never *trusted*: an unbounded `read_line` lets a buggy or hostile bridge
    // stream until the probe timeout and hand an arbitrarily large string to the status
    // projection, which then exceeds the IPC frame cap and fails every `telex status` call.
    let mut reader = BufReader::new(read_half.take(PROBE_MAX_RESPONSE_BYTES));
    let exchange = async {
        write_half.write_all(line.as_bytes()).await?;
        write_half.flush().await?;
        let mut response = String::new();
        reader.read_line(&mut response).await?;
        Ok::<String, std::io::Error>(response)
    };
    let response = match tokio::time::timeout(BRIDGE_PROBE_TIMEOUT, exchange).await {
        Ok(Ok(response)) => response,
        Ok(Err(_)) => return Err(IntentOutcome::failed("probe_io_failed")),
        Err(_) => return Err(IntentOutcome::failed("probe_timeout")),
    };
    if response.len() as u64 >= PROBE_MAX_RESPONSE_BYTES {
        return Err(IntentOutcome::failed("probe_response_too_large"));
    }
    let parsed: serde_json::Value = serde_json::from_str(response.trim())
        .map_err(|_| IntentOutcome::failed("probe_malformed_response"))?;
    if parsed.get("ok").and_then(|v| v.as_bool()) != Some(true) {
        let code = parsed
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("probe_rejected");
        // A producer that does not know the verb is legacy, not failed: its liveness is
        // unprovable, but it must not wedge anything.
        if code == "unsupported_op" || code == "unsupported_protocol" {
            return Err(IntentOutcome::terminal(
                IntentRecoveryState::LegacyProducer,
                "legacy_producer",
            ));
        }
        // Clamped and charset-restricted before it becomes a `failure_code`: this string is
        // retained in the in-memory index and copied into every `Status` response.
        return Err(IntentOutcome::failed(format!(
            "probe_{}",
            sanitize_failure_code(code)
        )));
    }
    if parsed.get("nonce").and_then(|v| v.as_str()) != Some(nonce.as_str()) {
        return Err(IntentOutcome::failed("probe_nonce_mismatch"));
    }
    if parsed.get("sessionId").and_then(|v| v.as_str()) != Some(intent.session_id.as_str()) {
        return Err(IntentOutcome::failed("probe_session_mismatch"));
    }
    let protocol = parsed.get("protocol").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    if protocol < BRIDGE_PROBE_MIN_PROTOCOL {
        return Err(IntentOutcome::terminal(
            IntentRecoveryState::LegacyProducer,
            "legacy_producer",
        ));
    }
    Ok(ProbeSuccess {
        bridge_generation: parsed
            .get("bridgeGeneration")
            .and_then(|v| v.as_u64())
            .unwrap_or(0),
        verified_at_ms: now_ms(),
    })
}

fn probe_nonce() -> String {
    let mut bytes = [0u8; 16];
    if getrandom::getrandom(&mut bytes).is_err() {
        let seed = now_ms() as u64 ^ monotonic_nonce();
        bytes[..8].copy_from_slice(&seed.to_le_bytes());
        bytes[8..].copy_from_slice(&monotonic_nonce().to_le_bytes());
    }
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

// ---------------------------------------------------------------------------------------------
// Member creation
// ---------------------------------------------------------------------------------------------

/// Create (or push-restore) the member for a verified intent.
///
/// Deliberately a **distinct path** from `register_member`, owning its own ordering rather than
/// inheriting a flag-coupled one:
///
/// * The durable detach tombstone is checked **unconditionally** before the claim and again after
///   it. `register_member` gates both checks on its `recovery` flag and *clears* the tombstone on
///   the `recovery = false` branch — that clearing branch is the hazard, and it is structurally
///   unreachable from here: this function contains no call to `clear_detach_tombstone`.
/// * A member that already exists is refreshed with `on_deliver = None, replace_on_deliver = false`
///   semantics, so retry/backoff state is preserved and no backend write happens for a no-op.
/// * `cc_watermark_ms` is passed through from the intent instead of recomputing "now", so every CC
///   message committed during the restart gap stays visible.
async fn register_member_reconciled(
    state: &Arc<DaemonState>,
    intent: &StationIntentV1,
    argv: Vec<String>,
) -> IntentOutcome {
    let backend = match backend_open_existing_only(state, &intent.store_key).await {
        // A store that genuinely does not exist is terminal: reconciliation is open-existing-only
        // and must never bring one into being.
        Err(code) if code == "store_missing" => {
            return IntentOutcome::terminal(IntentRecoveryState::Unverifiable, "store_missing")
        }
        // Anything else is a *transient* backend condition — a Postgres connect error, a DNS blip,
        // a failover. Classifying it terminal would park every push station for the quarantine
        // retry (an hour) after the database came back, with no self-healing path, so it takes the
        // ordinary failure ladder like every other backend error in this function.
        Err(code) => return IntentOutcome::failed(format!("backend_unavailable: {code}")),
        Ok(backend) => backend,
    };

    // Tombstone check #1: unconditional, before the claim.
    match backend
        .detach_tombstone(&intent.session_id, &intent.address)
        .await
    {
        Ok(Some(_)) => return IntentOutcome::terminal(IntentRecoveryState::Revoked, "tombstoned"),
        Ok(None) => {}
        Err(_) => return IntentOutcome::failed("tombstone_check_failed"),
    }

    // An address already attended by a *different* session in this daemon is never stolen. The
    // `idle` filter is deliberately absent for the conflict check that matters below: an idle
    // member is still this daemon's record of who attends the address.
    if let Some(conflict) = state
        .members
        .lock()
        .unwrap()
        .values()
        .find(|m| {
            m.store_key == intent.store_key
                && m.address == intent.address
                && !m.idle
                && m.session_id != intent.session_id
        })
        .cloned()
    {
        let _ = conflict;
        return IntentOutcome::terminal(IntentRecoveryState::OwnershipConflict, "address_attended");
    }

    if let Some(existing) = state.get_member(&intent.store_key, &intent.session_id, &intent.address)
    {
        match backend
            .heartbeat_epoch(
                &intent.address,
                &existing.owner_instance_id,
                existing.lease_epoch,
            )
            .await
        {
            Ok(true) => {
                if existing.on_deliver.is_some() {
                    if existing.on_deliver.as_deref() == Some(argv.as_slice()) {
                        // Already push-covered with the argv this daemon would build: a no-op
                        // refresh performs no backend write at all, and must not reset
                        // retry/backoff state or re-scan the backlog.
                        return IntentOutcome::RefreshedNoOp;
                    }
                    // Same member, *different* argv — in practice a stale `--daemon-instance`
                    // epoch fence from an attach that raced a daemon replacement. Left alone,
                    // every push to this station dead-letters permanently, and the no-op
                    // short-circuit means nothing would ever rebuild it. Repair it in place.
                    let mut refreshed = existing.clone();
                    refreshed.on_deliver = Some(argv);
                    refreshed.on_deliver_wake_on_cc = intent.wake_on_cc;
                    refreshed.on_deliver_cc_after_ms =
                        max_watermark(existing.on_deliver_cc_after_ms, intent.cc_watermark_ms);
                    state.insert_member(refreshed);
                    return IntentOutcome::Restored;
                }
                let mut refreshed = existing.clone();
                refreshed.on_deliver = Some(argv);
                refreshed.on_deliver_wake_on_cc = intent.wake_on_cc;
                // Never *lower* a live member's watermark: the manifest value is the floor that
                // keeps gap-committed CC messages visible, but a member that has already advanced
                // past it must not be rewound into replaying its own history.
                refreshed.on_deliver_cc_after_ms =
                    max_watermark(existing.on_deliver_cc_after_ms, intent.cc_watermark_ms);
                refreshed.idle = false;
                refreshed.idle_rearmable = false;
                state.insert_member(refreshed.clone());
                spawn_on_deliver_backlog(state.clone(), refreshed);
                return IntentOutcome::Restored;
            }
            Ok(false) => {
                self_demote_member(
                    state,
                    &existing,
                    "reconcile: epoch heartbeat returned 0 rows",
                );
            }
            Err(_) => return IntentOutcome::failed("epoch_heartbeat_failed"),
        }
    }

    if let Err(e) = backend
        .ensure_address(
            &intent.address,
            intent.description.as_deref(),
            intent.scope.as_deref(),
            intent.tags.as_deref(),
        )
        .await
    {
        let _ = e;
        return IntentOutcome::failed("ensure_address_failed");
    }

    let (claimed_lease_epoch, claimed_owner_instance_id) = match backend
        .claim_epoch_lease(&intent.address, &state.instance_id, liveness_window_secs())
        .await
    {
        Ok(EpochClaimResult::Claimed(claimed)) => (claimed.lease_epoch, claimed.owner_instance_id),
        Ok(EpochClaimResult::AlreadyOwned {
            lease_epoch,
            owner_instance_id,
            lease_row,
        }) => {
            if owner_instance_id == state.instance_id {
                // We already own this address and simply have no in-memory member for it — the
                // shape a lost or forgotten member leaves behind. Adopt the lease we already hold
                // rather than deferring forever against ourselves, which would wedge the binding.
                //
                // Restricted to the case where **no other session in this daemon** holds the
                // address at all (idle or not): "we already own it" is a statement about the
                // daemon, not about this session, so adopting it unconditionally would let two
                // sessions' intents for one address both end up with an armed member and deliver
                // the same message twice.
                let held_by_other_session = state.members.lock().unwrap().values().any(|m| {
                    m.store_key == intent.store_key
                        && m.address == intent.address
                        && m.session_id != intent.session_id
                });
                if held_by_other_session {
                    return IntentOutcome::terminal(
                        IntentRecoveryState::OwnershipConflict,
                        "address_attended",
                    );
                }
                (lease_epoch, owner_instance_id)
            } else {
                // The distinction that makes the crash bound derivable: an incumbent whose lease is
                // merely not stale yet is a *waiting* outcome on a fixed cadence, while a genuinely
                // fresh competing owner is a failure. Neither is ever force-stolen.
                let durable_now = backend
                    .durable_clock_now_ms()
                    .await
                    .unwrap_or_else(|_| now_ms());
                let stale_cutoff_ms = durable_now - liveness_window_secs() * 1000;
                if lease_row.heartbeat_at_ms > stale_cutoff_ms {
                    return IntentOutcome::DeferredLease;
                }
                return IntentOutcome::failed("epoch_claim_lost");
            }
        }
        Err(_) => return IntentOutcome::failed("epoch_claim_failed"),
    };

    // Tombstone check #2: unconditional, after the claim, so a tombstone written between the
    // pre-check and the claim is still honored. On a hit the lease is released immediately.
    match backend
        .detach_tombstone(&intent.session_id, &intent.address)
        .await
    {
        Ok(Some(_)) => {
            let _ = backend
                .release_epoch_lease(
                    &intent.address,
                    &claimed_owner_instance_id,
                    claimed_lease_epoch,
                )
                .await;
            return IntentOutcome::terminal(IntentRecoveryState::Revoked, "tombstoned");
        }
        Ok(None) => {}
        Err(_) => {
            let _ = backend
                .release_epoch_lease(
                    &intent.address,
                    &claimed_owner_instance_id,
                    claimed_lease_epoch,
                )
                .await;
            return IntentOutcome::failed("tombstone_recheck_failed");
        }
    }
    // NOTE: there is deliberately no `clear_detach_tombstone` call anywhere in this function.
    // Clearing a tombstone is an explicit-attach-only operation; a reconciler that could clear one
    // would resurrect an explicitly detached station.

    let record = MemberRecord {
        address: intent.address.clone(),
        store_key: intent.store_key.clone(),
        backend: backend.kind().to_string(),
        session_id: intent.session_id.clone(),
        occupant: intent.occupant.clone(),
        host: crate::config::hostname(),
        waiters: 0,
        watch_pids: Vec::new(),
        description: intent.description.clone(),
        scope: intent.scope.clone(),
        tags: intent.tags.clone(),
        lease_epoch: claimed_lease_epoch,
        owner_instance_id: claimed_owner_instance_id.clone(),
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
        on_deliver: Some(argv),
        on_deliver_wake_on_cc: intent.wake_on_cc,
        // Preserved, not recomputed: recomputing the lower bound as "now" would make every CC
        // message committed during the restart gap permanently invisible.
        on_deliver_cc_after_ms: intent.cc_watermark_ms,
    };
    state.check_session_id_reuse_tripwire(&record);
    state.insert_member(record.clone());
    spawn_on_deliver_backlog(state.clone(), record);
    IntentOutcome::Restored
}

/// The later of a live member's CC watermark and the manifest's, treating `None` as "no bound".
///
/// Monotonic by construction: the manifest value is a floor that keeps gap-committed CC messages
/// visible, and a member that has already advanced past it must never be rewound.
fn max_watermark(existing: Option<i64>, from_intent: Option<i64>) -> Option<i64> {
    match (existing, from_intent) {
        (Some(a), Some(b)) => Some(a.max(b)),
        (Some(a), None) => Some(a),
        (None, b) => b,
    }
}

// ---------------------------------------------------------------------------------------------
// The guard-free inner routine
// ---------------------------------------------------------------------------------------------

/// Reconcile exactly one intent.
///
/// **Contract: the caller already holds the per-`MemberKey` `delivery_admission` guard for this
/// intent's key, and this function never takes it.** That is what lets `register_member` call it
/// inline for the anti-downgrade check without self-deadlocking, and it is why `reconcile_once`
/// (which *does* acquire the guard) must never be called from a context that already holds one.
pub async fn reconcile_intent_locked(
    state: Arc<DaemonState>,
    intent: &StationIntentV1,
) -> IntentOutcome {
    if state.is_draining() {
        return IntentOutcome::terminal(IntentRecoveryState::Unverifiable, "draining");
    }
    if !intent.is_reconcilable() {
        return IntentOutcome::terminal(intent.state, "not_reconcilable");
    }

    // Host/boot binding. This closes both the Linux boot-relative `(pid, start_time)`
    // reproducibility hole and the synced/network-home hole: an intent written by another machine
    // or another boot can never be restored here.
    match local_identity() {
        Ok((host, boot)) => {
            if !intent.matches_local_identity(host, boot) {
                return IntentOutcome::terminal(
                    IntentRecoveryState::Unverifiable,
                    "foreign_host_or_boot",
                );
            }
        }
        Err(code) => return IntentOutcome::terminal(IntentRecoveryState::Unverifiable, code),
    }

    if !handler_kinds::handler_kind_registered(&intent.handler.kind) {
        return IntentOutcome::terminal(
            IntentRecoveryState::Incompatible,
            "handler_kind_unregistered",
        );
    }
    if intent.producer.protocol.max < BRIDGE_PROBE_MIN_PROTOCOL {
        return IntentOutcome::terminal(IntentRecoveryState::LegacyProducer, "legacy_producer");
    }

    // Pull-waiter precedence is preserved: a live armed waiter wins, and the intent waits rather
    // than forcing the conflict. This is the scoping of the anti-downgrade guarantee.
    if state.has_live_waiter_for(&intent.store_key, &intent.session_id, &intent.address) {
        return IntentOutcome::DeferredPullWaiter;
    }

    let selector = match store_selector_for_key(&intent.store_key) {
        Ok(selector) => selector,
        Err(_) => {
            return IntentOutcome::terminal(
                IntentRecoveryState::Unverifiable,
                "store_selector_unresolved",
            )
        }
    };

    // The reconciling daemon's own binary: matched-version by construction, and identical to what
    // `bridge_handler_argv` bakes on the attach path. Argv is re-derived, never persisted, which is
    // what makes upgrade and rollback work without any path-stability assumption.
    let exe = match canonical_current_exe() {
        Ok(exe) => exe,
        Err(_) => {
            return IntentOutcome::terminal(IntentRecoveryState::Unverifiable, "exe_unresolved")
        }
    };
    let argv = match handler_kinds::build_handler_argv(
        &intent.handler.kind,
        &exe,
        &selector,
        &intent.handler.session_id,
        &state.instance_id,
    ) {
        Ok(argv) => argv,
        Err(handler_kinds::RegistryError::UnknownHandlerKind(_)) => {
            return IntentOutcome::terminal(
                IntentRecoveryState::Incompatible,
                "handler_kind_unregistered",
            )
        }
        Err(_) => {
            return IntentOutcome::terminal(
                IntentRecoveryState::Incompatible,
                "handler_argv_invalid",
            )
        }
    };

    // If the member already exists and is push-covered with exactly the argv this daemon builds,
    // nothing needs proving: skip the probe entirely so a healthy steady state costs no I/O at
    // all. A *different* argv means the stored handler names a stale daemon instance, which must
    // be repaired rather than short-circuited.
    if let Some(existing) = state.get_member(&intent.store_key, &intent.session_id, &intent.address)
    {
        if existing.on_deliver.is_some()
            && !existing.idle
            && existing.on_deliver.as_deref() == Some(argv.as_slice())
        {
            return IntentOutcome::RefreshedNoOp;
        }
        // An operator `telex station reset` marks the member idle **and** clears
        // `idle_rearmable`, which is precisely "do not re-arm this automatically". Reset is the
        // one deliberate operator action with no durable tombstone, so it is the one the
        // reconciler could not otherwise see — restoring over it silently undid the reset within
        // a tick. Treat it as a revocation of the intent (the same terminal outcome a durable
        // tombstone produces), so it is durable, visible, and reversible only by an explicit
        // `telex --address <station> copilot resume`.
        if existing.idle && !existing.idle_rearmable {
            return IntentOutcome::terminal(IntentRecoveryState::Revoked, "operator_reset");
        }
    }

    let secret = match resolve_credential(intent) {
        Ok(secret) => secret,
        Err(outcome) => return outcome,
    };
    let probe = match probe_producer(intent, &secret).await {
        Ok(probe) => probe,
        Err(outcome) => return outcome,
    };
    drop(secret);

    let outcome = register_member_reconciled(&state, intent, argv).await;
    if outcome.is_success() {
        let key = IntentKey::from_intent(intent);
        if let Some(mut entry) = state.index_entry(&key) {
            entry.producer_verified_ms = Some(probe.verified_at_ms);
            state.index_upsert(key, entry);
        }
    }
    outcome
}

// ---------------------------------------------------------------------------------------------
// The acquiring entry point
// ---------------------------------------------------------------------------------------------

/// Run one bounded reconciliation pass.
///
/// Acquires the per-scope single-flight guard and the per-`MemberKey` admission guard for each
/// intent. Never call this from a context that already holds an admission guard — use
/// [`reconcile_intent_locked`] there.
pub async fn reconcile_once(state: Arc<DaemonState>, scope: Option<String>) -> ReconcileReport {
    let pass_seq = state.intents.pass_seq.fetch_add(1, Ordering::SeqCst) + 1;
    let started = Instant::now();
    let mut report = ReconcileReport {
        pass_seq,
        ran: true,
        index_as_of_ms: now_ms(),
        ..Default::default()
    };

    // Drain suppression, exactly as `heartbeat_members_once` does: a draining daemon must not arm
    // new delivery. Reported as a *skipped* pass so a caller (upgrade's successor verification,
    // the turn-boundary hook) can tell "did not run" from "ran and found nothing" and retry,
    // rather than printing `restored 0` for a recovery that never started.
    if state.is_draining() {
        report = ReconcileReport::skipped_pass(pass_seq, "draining");
        publish(&state, &report);
        return report;
    }
    // Single-flight per scope. A pass that would overlap simply does not start; because the pass
    // deadline is shorter than the tick, this is not expected to trigger in the normal case.
    let Ok(_single_flight) = state.intents.single_flight.try_lock() else {
        report = ReconcileReport::skipped_pass(pass_seq, "single_flight");
        publish(&state, &report);
        return report;
    };

    let Some(store) = state.intent_store() else {
        report = ReconcileReport::skipped_pass(pass_seq, "scope_unavailable");
        publish(&state, &report);
        return report;
    };

    // Maintenance GC on its own slower cadence, inside the same bounded tick.
    if pass_seq.is_multiple_of(RECONCILE_GC_EVERY_PASSES) {
        run_intent_gc(&state);
    }

    let page = match store.scan(RECONCILE_PASS_BUDGET) {
        Ok(page) => page,
        Err(e) => {
            state.push_recent_error("StationIntent", format!("scanning intent scope: {e}"));
            report = ReconcileReport::skipped_pass(pass_seq, "scan_failed");
            publish(&state, &report);
            return report;
        }
    };
    report.observed_count = page.observed_count;
    report.over_cap = page.over_cap;
    report.skipped = page.skipped.len();
    {
        let mut index = state.intents.index.lock().unwrap();
        index.observed_count = page.observed_count;
        index.over_cap = page.over_cap;
        index.as_of_ms = now_ms();
    }
    if page.over_cap {
        state.push_recent_error(
            "StationIntent",
            format!(
                "station-intent scope holds {} entries, over the {STATION_INTENT_MAX_COUNT} write cap; \
                 nothing is deleted for being over cap - GC brings it back down",
                page.observed_count
            ),
        );
    }

    // Entries rejected by a security or schema check are indexed whenever the binding they name
    // could be established (so they are visible in status and the drain report) but never
    // attempted. A rejection that could not even be identified is counted, so the operator-facing
    // number is never silently zero.
    let mut unidentifiable = 0usize;
    for (id, rejection) in &page.rejected {
        report.inert += 1;
        match &rejection.identity {
            Some(identity) => {
                let key = IntentKey {
                    store_key: identity.store_key.clone(),
                    session_id: identity.session_id.clone(),
                    address: identity.address.clone(),
                };
                let mut entry = state.index_entry(&key).unwrap_or_default();
                // Highest-precedence-wins: several states can apply to one binding at once (a
                // cached projection plus a fresh rejection), and the rejection must not be masked
                // by a stale success.
                entry.state = rejection.state.max(entry.state);
                entry.failure_code = Some(sanitize_failure_code(&rejection.detail));
                entry.last_attempt_ms = Some(now_ms());
                entry.first_seen_ms.get_or_insert(now_ms());
                state.index_upsert(key, entry);
            }
            None => unidentifiable += 1,
        }
        log_event(
            &store,
            serde_json::json!({
                "event": "intent_rejected",
                "pass_seq": pass_seq,
                "intent_id": id.to_string(),
                "state": rejection.state,
                "identified": rejection.identity.is_some(),
                "detail": rejection.detail,
            }),
        );
    }
    {
        let mut index = state.intents.index.lock().unwrap();
        index.unidentifiable = unidentifiable;
    }

    let now = now_ms();
    let mut due: Vec<(String, StationIntentV1)> = Vec::new();
    // Deduped by `(store_key, address)`, not by the full key: two *different sessions* can hold a
    // live intent for one address, and the deterministic `(store_key, address, generation desc)`
    // scan order makes the first one seen the highest-generation one. Keying this on the full
    // `IntentKey` made the filter a no-op (the filename is derived from all three fields, so two
    // entries can never share a key) and let both sessions be reconciled in the same wave.
    let mut seen_addresses: BTreeSet<(String, String)> = BTreeSet::new();
    // The position of the last entry this pass *considered* but did not attempt. Used only when the
    // pass attempts nothing at all, so the cursor still advances and the next pass moves on.
    let mut last_considered_position: Option<String> = None;
    for (intent, position) in page
        .loaded
        .into_iter()
        .zip(page.loaded_positions.into_iter())
    {
        if let Some(filter) = scope.as_deref() {
            // Below the scope filter, deliberately: the cursor is shared by every store, so
            // advancing it past an out-of-scope intent would let a scoped pass skip intents it
            // never considered for a full round — weakening exactly the guaranteed-progress
            // property the queue-delay bound rests on.
            if intent.store_key != filter {
                continue;
            }
        }
        last_considered_position = Some(position.clone());
        let key = IntentKey::from_intent(&intent);
        if !seen_addresses.insert((intent.store_key.clone(), intent.address.clone())) {
            report.skipped += 1;
            continue;
        }
        // Seed the index from the manifest header even for intents this pass will not attempt, so
        // the drain report and status are never blind to an entry the budget skipped.
        let mut entry = state.index_entry(&key).unwrap_or_default();
        let first_sight = entry.first_seen_ms.is_none();
        entry.generation = intent.generation;
        entry.wake_on_cc = intent.wake_on_cc;
        entry.cc_watermark_ms = intent.cc_watermark_ms;
        entry.first_seen_ms.get_or_insert(now);
        if first_sight {
            // Seed retry state from the durable evidence block on first sight. Without this,
            // backoff and quarantine reset on every daemon replacement — the event most likely to
            // follow a crash loop — so a wedged intent got a fresh full-rate retry budget every
            // time, and `recovery_latency_ms` restarted the clock on the exact event it measures.
            entry.attempts = intent.evidence.attempts;
            entry.consecutive_failures = intent.evidence.consecutive_failures;
            entry.next_attempt_ms = intent.evidence.next_attempt_ms;
            entry.last_attempt_ms = intent.evidence.last_attempt_ms;
            entry.last_success_ms = intent.evidence.last_success_ms;
            entry.producer_verified_ms = intent.evidence.producer_verified_ms;
            entry.failure_code = intent.evidence.failure_code.clone();
            if entry.consecutive_failures >= RECONCILE_QUARANTINE_AFTER {
                entry.state = IntentRecoveryState::Quarantined;
            }
        }
        if entry.state == IntentRecoveryState::default() || !intent.is_reconcilable() {
            entry.state = intent.state;
        }
        state.index_upsert(key.clone(), entry.clone());

        if !intent.is_reconcilable() {
            report.inert += 1;
            continue;
        }
        if entry
            .next_attempt_ms
            .is_some_and(|next_attempt_ms| next_attempt_ms > now)
        {
            // Backoff, quarantine, and deferred cadences are respected: a pulse schedules work, it
            // never bypasses a next-attempt time.
            report.skipped += 1;
            continue;
        }
        due.push((position, intent));
    }

    // Wave scheduling: waves of at most `RECONCILE_MAX_CONCURRENCY`, each intent bounded by
    // `RECONCILE_PER_INTENT_TIMEOUT`, and no wave is ever started that could outlive the deadline.
    // The first wave always runs, which is what guarantees a minimum of `RECONCILE_MAX_CONCURRENCY`
    // intents of progress per pass even when every intent times out.
    let mut cursor = 0usize;
    let mut last_attempted_position: Option<String> = None;
    while cursor < due.len() {
        if cursor > 0 {
            let remaining = RECONCILE_PASS_DEADLINE.checked_sub(started.elapsed());
            match remaining {
                Some(remaining) if remaining >= RECONCILE_PER_INTENT_TIMEOUT => {}
                _ => {
                    report.deadline_reached = true;
                    report.skipped += due.len() - cursor;
                    break;
                }
            }
        }
        let wave: Vec<(String, StationIntentV1)> = due[cursor..]
            .iter()
            .take(RECONCILE_MAX_CONCURRENCY)
            .cloned()
            .collect();
        cursor += wave.len();

        let mut handles = Vec::with_capacity(wave.len());
        for (position, intent) in wave {
            let state = state.clone();
            handles.push(tokio::spawn(async move {
                // The acquiring half of the two-level API: take the per-station admission guard,
                // then call the guard-free inner routine.
                //
                // Guard *acquisition* is inside the timeout, not just the reconcile: `register_member`
                // holds this same guard across backend work, so an unbounded wait here would let one
                // slow station stall the wave — and, because the heartbeat loop awaits the pass
                // inline, stall every member's epoch heartbeat with it.
                let outcome = match tokio::time::timeout(RECONCILE_PER_INTENT_TIMEOUT, async {
                    let admission = state
                        .delivery_admission(
                            &intent.store_key,
                            &intent.session_id,
                            &intent.address,
                            DeliveryAdmissionKind::Register,
                        )
                        .await;
                    let _guard = admission.lock().await;
                    reconcile_intent_locked(state.clone(), &intent).await
                })
                .await
                {
                    Ok(outcome) => outcome,
                    Err(_) => IntentOutcome::failed("per_intent_timeout"),
                };
                (position, intent, outcome)
            }));
        }
        for handle in handles {
            match handle.await {
                Ok((position, intent, outcome)) => {
                    report.scanned += 1;
                    last_attempted_position = Some(position);
                    apply_outcome(&state, &store, &intent, outcome, &mut report, pass_seq);
                }
                Err(_) => {
                    report.scanned += 1;
                    report.failed += 1;
                }
            }
        }
    }

    // Advance the round-robin cursor to the last intent actually attempted, so a pass cut short by
    // the deadline resumes where it stopped rather than skipping everything it loaded. When nothing
    // was attempted (all inert or backed off) the cursor still moves past what was considered, so
    // the next pass makes progress instead of re-examining the same head forever.
    if let Some(position) = last_attempted_position.or(last_considered_position) {
        if let Err(e) = store.advance_cursor(&position) {
            state.push_recent_error(
                "StationIntent",
                format!("persisting the reconcile scan cursor: {e}"),
            );
        }
    }

    report.duration_ms = started.elapsed().as_millis() as u64;
    report.index_as_of_ms = now_ms();
    {
        let mut index = state.intents.index.lock().unwrap();
        index.as_of_ms = report.index_as_of_ms;
    }
    log_event(
        &store,
        serde_json::json!({
            "event": "reconcile_pass",
            "pass_seq": pass_seq,
            "scanned": report.scanned,
            "restored": report.restored,
            "refreshed_no_op": report.refreshed_no_op,
            "deferred_lease": report.deferred_lease,
            "deferred_pull_waiter": report.deferred_pull_waiter,
            "failed": report.failed,
            "inert": report.inert,
            "skipped": report.skipped,
            "over_cap": report.over_cap,
            "observed_count": report.observed_count,
            "duration_ms": report.duration_ms,
            "deadline_reached": report.deadline_reached,
            "ran": report.ran,
        }),
    );
    publish(&state, &report);
    report
}

fn publish(state: &Arc<DaemonState>, report: &ReconcileReport) {
    let _ = state.intents.report_tx.send(report.clone());
}

/// Fold one outcome into the index, the durable evidence fields, and the pass report.
fn apply_outcome(
    state: &Arc<DaemonState>,
    store: &IntentStore,
    intent: &StationIntentV1,
    outcome: IntentOutcome,
    report: &mut ReconcileReport,
    pass_seq: u64,
) {
    let now = now_ms();
    let key = IntentKey::from_intent(intent);
    let mut entry = state.index_entry(&key).unwrap_or_default();
    entry.generation = intent.generation;
    entry.wake_on_cc = intent.wake_on_cc;
    entry.cc_watermark_ms = intent.cc_watermark_ms;
    entry.attempts = entry.attempts.saturating_add(1);
    entry.last_attempt_ms = Some(now);
    entry.state = outcome.projected_state();
    entry.failure_code = outcome.failure_code().map(|c| c.to_string());

    match &outcome {
        IntentOutcome::Restored => {
            report.restored += 1;
            entry.consecutive_failures = 0;
            entry.next_attempt_ms = None;
            entry.last_success_ms = Some(now);
            entry.recovery_latency_ms = entry.first_seen_ms.map(|first| now.saturating_sub(first));
        }
        IntentOutcome::RefreshedNoOp => {
            report.refreshed_no_op += 1;
            entry.consecutive_failures = 0;
            entry.next_attempt_ms = None;
            entry.last_success_ms = Some(now);
        }
        IntentOutcome::DeferredLease => {
            report.deferred_lease += 1;
            // Fixed cadence, no exponential growth, no jitter, and the failure counter is NOT
            // advanced: this is a waiting state, and treating it as failure would push crash
            // recovery past its published bound.
            entry.next_attempt_ms = Some(now + RECONCILE_DEFERRED_LEASE_RETRY.as_millis() as i64);
        }
        IntentOutcome::DeferredPullWaiter => {
            report.deferred_pull_waiter += 1;
            // Deferred with its own backoff (the wait is unbounded in principle), but still not a
            // failure: the loser is not permanent.
            let delay = backoff_delay(entry.consecutive_failures.min(3));
            entry.next_attempt_ms = Some(now + delay.as_millis() as i64);
        }
        IntentOutcome::Failed { .. } => {
            report.failed += 1;
            entry.consecutive_failures = entry.consecutive_failures.saturating_add(1);
            if entry.consecutive_failures >= RECONCILE_QUARANTINE_AFTER {
                entry.state = IntentRecoveryState::Quarantined;
                entry.next_attempt_ms = Some(now + RECONCILE_QUARANTINE_RETRY.as_millis() as i64);
            } else {
                let delay = backoff_delay(entry.consecutive_failures);
                entry.next_attempt_ms = Some(now + delay.as_millis() as i64);
            }
        }
        IntentOutcome::Terminal {
            state: projected, ..
        } => {
            report.inert += 1;
            // Terminal and inert classes are surfaced and GC-governed, never retried on the fast
            // cadence, and they do not advance the failure counter.
            entry.next_attempt_ms = Some(now + RECONCILE_QUARANTINE_RETRY.as_millis() as i64);
            if *projected == IntentRecoveryState::Revoked {
                // A durable tombstone (or an operator reset) outranks the local manifest: revoke
                // it so the next pass does not even attempt it. `revoke` bumps the generation, so
                // the evidence write below is deliberately skipped — writing back the pre-revoke
                // copy under the old generation would either fail the CAS or, worse, resurrect
                // `Live`.
                match store.revoke(&intent.store_key, &intent.session_id, &intent.address, now) {
                    Ok(_) => {}
                    Err(e) => state.push_recent_error(
                        "StationIntent",
                        format!("revoking intent for {}: {e}", intent.address),
                    ),
                }
                entry.next_attempt_ms = None;
                state.index_upsert(key, entry);
                log_outcome_event(store, intent, &outcome, pass_seq);
                return;
            }
        }
    }

    // Refresh the durable CC watermark from the live member.
    //
    // The manifest value is written once at finalize and would otherwise never move, while the
    // member's `on_deliver_cc_after_ms` advances in memory and dies with the daemon. A restored
    // member then received the *attach-time* bound and re-injected every CC message the station
    // had seen since attach. Persisting the live value bounds replay to the outage window while
    // keeping the pass-through property that makes gap-committed CC messages visible: the
    // persisted value lags "now" by at most one tick and is never recomputed as "now".
    let live_watermark = state
        .get_member(&intent.store_key, &intent.session_id, &intent.address)
        .and_then(|member| member.on_deliver_cc_after_ms);
    let refreshed_watermark = if outcome.is_success() {
        max_watermark(live_watermark, intent.cc_watermark_ms)
    } else {
        intent.cc_watermark_ms
    };
    entry.cc_watermark_ms = refreshed_watermark;

    // Persist evidence with a generation CAS so a concurrent attach or revoke wins over a pass
    // that read a now-stale manifest.
    //
    // Only when something actually changed, and **without** bumping `updated_at_ms`: rewriting
    // every live manifest on every 5 s tick to record an unchanged `RefreshedNoOp` cost one atomic
    // file rewrite per intent per tick, contradicted the "a healthy steady state costs no I/O at
    // all" optimization it sits beside, and — because GC ages an intent from `updated_at_ms` —
    // reset every orphan TTL on every attempt, so no live intent could ever expire.
    let evidence = IntentEvidence {
        last_attempt_ms: entry.last_attempt_ms,
        last_success_ms: entry.last_success_ms,
        attempts: entry.attempts,
        consecutive_failures: entry.consecutive_failures,
        failure_code: entry.failure_code.clone(),
        producer_verified_ms: entry.producer_verified_ms,
        next_attempt_ms: entry.next_attempt_ms,
        recovery_latency_ms: entry.recovery_latency_ms,
    };
    if evidence_write_due(&intent.evidence, &evidence, now)
        || refreshed_watermark != intent.cc_watermark_ms
    {
        let mut updated = intent.clone();
        updated.evidence = evidence;
        updated.cc_watermark_ms = refreshed_watermark;
        if let Err(e) = store.write_cas(intent.generation, &updated) {
            state.push_recent_error(
                "StationIntent",
                format!("persisting intent evidence for {}: {e}", intent.address),
            );
        }
    }

    state.index_upsert(key, entry);
    log_outcome_event(store, intent, &outcome, pass_seq);
}

/// Whether the evidence block has changed enough to be worth an atomic manifest rewrite.
///
/// Any change to the scheduling-relevant fields is persisted immediately (that state has to
/// survive a daemon replacement); an unchanged healthy intent is refreshed at most once per
/// `EVIDENCE_REFRESH_INTERVAL`, which is what keeps `last_success_ms` a usable "the producer was
/// proven this recently" clock for GC without paying a write every tick.
fn evidence_write_due(current: &IntentEvidence, next: &IntentEvidence, now: i64) -> bool {
    if current.consecutive_failures != next.consecutive_failures
        || current.failure_code != next.failure_code
        || current.next_attempt_ms != next.next_attempt_ms
        || current.recovery_latency_ms != next.recovery_latency_ms
        || current.producer_verified_ms != next.producer_verified_ms
    {
        return true;
    }
    let last_persisted = current.last_success_ms.or(current.last_attempt_ms);
    match last_persisted {
        Some(persisted) => {
            now.saturating_sub(persisted) >= EVIDENCE_REFRESH_INTERVAL.as_millis() as i64
        }
        None => true,
    }
}

fn log_outcome_event(
    store: &IntentStore,
    intent: &StationIntentV1,
    outcome: &IntentOutcome,
    pass_seq: u64,
) {
    log_event(
        store,
        serde_json::json!({
            "event": "intent_outcome",
            "pass_seq": pass_seq,
            "store_key": intent.store_key,
            "session_id": intent.session_id,
            "address": intent.address,
            "generation": intent.generation,
            "state": outcome.projected_state(),
            "failure_code": outcome.failure_code(),
        }),
    );
}

/// Exponential backoff with +/- jitter, capped at `RECONCILE_BACKOFF_MAX`.
pub fn backoff_delay(consecutive_failures: u32) -> Duration {
    let base = RECONCILE_BACKOFF_INITIAL.as_millis() as u64;
    let max = RECONCILE_BACKOFF_MAX.as_millis() as u64;
    let scaled = base
        .saturating_mul(1u64 << consecutive_failures.min(16))
        .min(max);
    let jitter_span = scaled * RECONCILE_BACKOFF_JITTER_PCT / 100;
    if jitter_span == 0 {
        return Duration::from_millis(scaled);
    }
    let mut bytes = [0u8; 8];
    let roll = if getrandom::getrandom(&mut bytes).is_ok() {
        u64::from_le_bytes(bytes)
    } else {
        monotonic_nonce()
    };
    let offset = roll % (jitter_span * 2 + 1);
    Duration::from_millis(scaled.saturating_add(offset).saturating_sub(jitter_span))
}

// ---------------------------------------------------------------------------------------------
// Event log
// ---------------------------------------------------------------------------------------------

/// Append one NDJSON reconcile event, with a single rotation at the size cap.
///
/// Best effort by design: diagnostics must never be able to fail a reconcile pass. No secret, no
/// credential path, and no raw argv is ever written here.
fn log_event(store: &IntentStore, mut event: serde_json::Value) {
    if let Some(map) = event.as_object_mut() {
        map.insert("ts_ms".to_string(), serde_json::json!(now_ms()));
        // Defense in depth: if a future field ever carried a secret, redact it rather than log it.
        for key in ["secret", "credential", "argv"] {
            if map.contains_key(key) {
                map.insert(key.to_string(), serde_json::json!(REDACTED_SECRET));
            }
        }
    }
    let path = store.root().join(RECONCILE_EVENT_LOG_FILE);
    if std::fs::metadata(&path).is_ok_and(|m| m.len() > RECONCILE_EVENT_LOG_ROTATE_BYTES) {
        let rotated = store.root().join(format!("{RECONCILE_EVENT_LOG_FILE}.1"));
        let _ = std::fs::remove_file(&rotated);
        let _ = std::fs::rename(&path, &rotated);
    }
    let Ok(mut line) = serde_json::to_string(&event) else {
        return;
    };
    line.push('\n');
    use std::io::Write;
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        let _ = file.write_all(line.as_bytes());
    }
}

// ---------------------------------------------------------------------------------------------
// Scheduling entry points
// ---------------------------------------------------------------------------------------------

/// The startup scan.
///
/// Asynchronous and non-blocking: `serve()` accepts connections immediately and spawns the first
/// pass, which is budgeted and per-intent timed out, so a large or corrupt intent set can never
/// delay daemon readiness. Ordinary client recovery is not gated on it either, because the
/// anti-downgrade guard lives inside `register_member` and applies whether or not the scan has run.
pub fn spawn_startup_scan(state: Arc<DaemonState>) {
    tokio::spawn(async move {
        // GC first so a scope full of crash leftovers does not consume the first pass budget.
        run_intent_gc(&state);
        reconcile_once(state, None).await;
    });
}

/// How many passes between maintenance GC runs. GC is O(scope) I/O, so it runs on a slower cadence
/// than reconciliation itself: once a minute at the 5 s tick, plus once at startup.
const RECONCILE_GC_EVERY_PASSES: u64 = 12;

/// Bounded intent GC: the only place an intent file is deleted, and the only mechanism that brings
/// an over-cap scope back under the cap.
fn run_intent_gc(state: &Arc<DaemonState>) {
    let Some(store) = state.intent_store() else {
        return;
    };
    let identity = local_identity().ok();
    match store.gc(
        now_ms(),
        identity.map(|(host, _)| host.as_str()),
        identity.map(|(_, boot)| boot.as_str()),
    ) {
        Ok(report) => {
            if !report.removed.is_empty() {
                state.index_prune_removed(&report.removed);
                for (id, reason) in &report.reasons {
                    log_event(
                        &store,
                        serde_json::json!({
                            "event": "intent_gc_removed",
                            "intent_id": id.to_string(),
                            "reason": reason,
                        }),
                    );
                }
            }
        }
        Err(e) => state.push_recent_error("StationIntent", format!("intent GC: {e}")),
    }
}

/// Await the next completed pass with a `pass_seq` strictly greater than `after_seq`.
///
/// This is the seam that lets upgrade, rollback, and tests wait for a *reconcile result* rather
/// than poll a wall clock.
pub async fn await_next_report(
    mut reports: tokio::sync::watch::Receiver<ReconcileReport>,
    after_seq: u64,
    timeout: Duration,
) -> Option<ReconcileReport> {
    let deadline = Instant::now() + timeout;
    loop {
        {
            let current = reports.borrow_and_update().clone();
            if current.pass_seq > after_seq {
                return Some(current);
            }
        }
        let remaining = deadline.checked_duration_since(Instant::now())?;
        if tokio::time::timeout(remaining, reports.changed())
            .await
            .is_err()
        {
            return None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pass_scheduling_constants_keep_the_published_bounds_true() {
        // A pass can never overrun the tick that started it.
        assert!(RECONCILE_PASS_DEADLINE < RECONCILE_INTERVAL);
        // No wave is ever started that could outlive the deadline.
        assert!(RECONCILE_PER_INTENT_TIMEOUT <= RECONCILE_PASS_DEADLINE);
        // The probe leaves room for local validation and the backend claim inside a per-intent
        // budget.
        assert!(BRIDGE_PROBE_TIMEOUT < RECONCILE_PER_INTENT_TIMEOUT);
        // DeferredLease retries at most once per tick, with no exponential growth.
        assert_eq!(RECONCILE_DEFERRED_LEASE_RETRY, RECONCILE_INTERVAL);
        // Every pass makes at least `RECONCILE_MAX_CONCURRENCY` intents of progress, which is only
        // meaningful if a wave fits inside the budget.
        assert!(RECONCILE_MAX_CONCURRENCY <= RECONCILE_PASS_BUDGET.max(1));
    }

    #[test]
    fn published_bounds_are_derived_not_asserted() {
        // Graceful: one tick + probe + validation/claim allowance.
        assert_eq!(graceful_recovery_bound_ms(), 5_000 + 3_000);
        assert!(
            graceful_recovery_bound_ms() <= 10_000,
            "documented as <= 10 s"
        );
        // Crash: the liveness window plus the graceful bound, read from the runtime value rather
        // than a literal.
        let expected = (liveness_window_secs() as u64) * 1000 + graceful_recovery_bound_ms();
        assert_eq!(crash_recovery_bound_ms(), expected);
    }

    #[test]
    fn queue_delay_formula_matches_the_documented_shape() {
        // ceil(N / P_min) passes at one pass per interval.
        assert_eq!(max_queue_delay_ms(0), 0);
        assert_eq!(max_queue_delay_ms(4), 5_000);
        assert_eq!(max_queue_delay_ms(5), 10_000);
        assert_eq!(
            max_queue_delay_ms(STATION_INTENT_MAX_COUNT),
            128 * 5_000,
            "the 512-intent pathological ceiling published for large scopes"
        );
    }

    #[test]
    fn backoff_grows_and_is_capped_with_jitter_inside_the_stated_band() {
        let first = backoff_delay(1);
        assert!(first >= Duration::from_millis(8_000) && first <= Duration::from_millis(12_000));
        let capped = backoff_delay(30);
        let max = RECONCILE_BACKOFF_MAX.as_millis() as u64;
        let band = max * RECONCILE_BACKOFF_JITTER_PCT / 100;
        assert!(capped.as_millis() as u64 >= max - band);
        assert!(capped.as_millis() as u64 <= max + band);
    }

    #[test]
    fn outcome_classes_map_to_the_intended_states() {
        assert_eq!(
            IntentOutcome::Restored.projected_state(),
            IntentRecoveryState::Restored
        );
        assert_eq!(
            IntentOutcome::RefreshedNoOp.projected_state(),
            IntentRecoveryState::Restored
        );
        assert_eq!(
            IntentOutcome::DeferredLease.projected_state(),
            IntentRecoveryState::DeferredLease
        );
        assert!(IntentRecoveryState::DeferredLease.is_waiting());
        assert!(IntentRecoveryState::DeferredLease.is_recoverable());
        assert_eq!(
            IntentOutcome::failed("x").projected_state(),
            IntentRecoveryState::Unverifiable
        );
        assert_eq!(
            IntentOutcome::terminal(IntentRecoveryState::Revoked, "tombstoned").projected_state(),
            IntentRecoveryState::Revoked
        );
    }

    #[test]
    fn the_reconcile_path_contains_no_tombstone_clearing_call() {
        // A source-level assertion of the structural claim in decision 6: the reconciler must not
        // be able to reach `clear_detach_tombstone`, because that is what would resurrect an
        // explicitly detached station. This is checked here (rather than only behaviorally) so a
        // future edit that reintroduces the call fails immediately and unambiguously.
        //
        // Only the production half of the file is inspected; the test module below necessarily
        // mentions the symbol in order to assert about it.
        let source = include_str!("daemon_reconcile.rs");
        let test_module_start = source
            .find("mod tests {")
            .expect("daemon_reconcile.rs must end with its test module");
        let production = &source[..test_module_start];
        for (index, line) in production.lines().enumerate() {
            if line.contains("clear_detach_tombstone") {
                assert!(
                    line.trim_start().starts_with("//"),
                    "daemon_reconcile.rs:{} calls clear_detach_tombstone outside a comment",
                    index + 1
                );
            }
        }
    }

    #[test]
    fn a_producer_supplied_failure_code_is_clamped_before_it_enters_the_status_projection() {
        // The producer is peer-verified but never trusted to be well-behaved, and `failure_code`
        // is retained in the in-memory index and copied into every `Status` response — so an
        // unbounded string there pushes the status frame past `MAX_JSONL_FRAME_BYTES` and makes
        // every `telex status` call fail until the daemon is restarted.
        let long = "x".repeat(4096);
        let clamped = sanitize_failure_code(&long);
        assert!(clamped.len() <= FAILURE_CODE_MAX_CHARS);
        assert_eq!(sanitize_failure_code("Bridge Busy!"), "bridge_busy");
        assert_eq!(sanitize_failure_code("   "), "rejected");
        assert_eq!(sanitize_failure_code("\u{1b}[31mred"), "31mred");
        assert!(
            sanitize_failure_code("a\nb\r\nc")
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_'),
            "no control characters may survive into a status field"
        );
    }

    #[test]
    fn a_restore_never_lowers_a_live_members_cc_watermark() {
        assert_eq!(max_watermark(Some(9), Some(2)), Some(9));
        assert_eq!(max_watermark(Some(2), Some(9)), Some(9));
        assert_eq!(max_watermark(None, Some(2)), Some(2));
        assert_eq!(max_watermark(Some(2), None), Some(2));
        assert_eq!(max_watermark(None, None), None);
    }

    #[test]
    fn unchanged_evidence_is_not_rewritten_on_every_tick() {
        // The healthy steady state used to rewrite every live manifest every 5 s to record an
        // unchanged `RefreshedNoOp`, which contradicted the "a healthy steady state costs no I/O
        // at all" optimization next to it and — because GC ages an intent from `updated_at_ms` —
        // reset every orphan TTL on every attempt.
        let base = IntentEvidence {
            last_attempt_ms: Some(1_000),
            last_success_ms: Some(1_000),
            attempts: 3,
            consecutive_failures: 0,
            failure_code: None,
            producer_verified_ms: Some(900),
            next_attempt_ms: None,
            recovery_latency_ms: Some(10),
        };
        let mut next = base.clone();
        next.attempts = 4;
        next.last_attempt_ms = Some(1_500);
        next.last_success_ms = Some(1_500);
        assert!(
            !evidence_write_due(&base, &next, 1_500),
            "an unchanged healthy intent must not be rewritten every tick"
        );
        assert!(
            evidence_write_due(
                &base,
                &next,
                1_000 + EVIDENCE_REFRESH_INTERVAL.as_millis() as i64
            ),
            "but it must be refreshed periodically so `last_success_ms` stays a usable clock"
        );

        let mut failed = base.clone();
        failed.consecutive_failures = 1;
        failed.failure_code = Some("credential_stale".to_string());
        failed.next_attempt_ms = Some(1_100);
        assert!(
            evidence_write_due(&base, &failed, 1_001),
            "scheduling state must be persisted immediately: it has to survive a replacement"
        );
    }

    #[test]
    fn a_transient_credential_condition_is_not_a_terminal_outcome() {
        // Retry policy and projected state are separate axes: these all project `Unverifiable`
        // for status, and all take the backoff ladder rather than the one-hour quarantine cadence.
        for code in [
            "credential_stale",
            "credential_age_unknown",
            "credential_unreadable",
            "credential_malformed",
            "credential_field_missing",
            "credential_unresolved",
        ] {
            let outcome = IntentOutcome::failed(code);
            assert_eq!(outcome.projected_state(), IntentRecoveryState::Unverifiable);
            assert!(matches!(outcome, IntentOutcome::Failed { .. }));
        }
        // The security and genuinely-inert classes stay terminal.
        for (state, code) in [
            (IntentRecoveryState::Insecure, "credential_outside_root"),
            (IntentRecoveryState::Insecure, "credential_insecure"),
            (
                IntentRecoveryState::Unverifiable,
                "credential_root_unregistered",
            ),
            (IntentRecoveryState::Unverifiable, "foreign_host_or_boot"),
            (IntentRecoveryState::LegacyProducer, "legacy_producer"),
        ] {
            let outcome = IntentOutcome::terminal(state, code);
            assert!(matches!(outcome, IntentOutcome::Terminal { .. }));
        }
    }

    #[test]
    fn a_skipped_pass_is_distinguishable_from_an_empty_one() {
        let skipped = ReconcileReport::skipped_pass(7, "draining");
        assert!(!skipped.ran);
        assert_eq!(skipped.skipped_reason.as_deref(), Some("draining"));
        assert_eq!(skipped.restored, 0);
        // A report deserialized from a daemon that predates the field describes a pass that ran.
        let legacy: ReconcileReport =
            serde_json::from_str(r#"{"pass_seq":1,"scanned":0,"restored":0,"refreshed_no_op":0,"deferred_lease":0,"deferred_pull_waiter":0,"failed":0,"skipped":0,"inert":0,"over_cap":false,"observed_count":0,"duration_ms":0,"deadline_reached":false,"index_as_of_ms":0}"#)
                .expect("older report shape");
        assert!(legacy.ran);
    }

    #[test]
    fn store_selector_resolves_a_sqlite_store_key() {
        #[cfg(feature = "sqlite")]
        {
            let selector = store_selector_for_key("sqlite:/tmp/telex.db").expect("sqlite selector");
            assert_eq!(selector.db.as_deref(), Some("/tmp/telex.db"));
            assert_eq!(selector.backend, None);
            assert!(store_selector_for_key("sqlite:").is_err());
        }
        assert!(store_selector_for_key("totally-unknown-store-key").is_err());
    }
}
