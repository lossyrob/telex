//! Daemon-owned station-intent reconciliation (issue #106 / ADR 0052).
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
    self, ArmedProofFailure, ArmedProofStamp, BoundedPhase, IntentBinding, IntentEvidence,
    IntentId, IntentStore, PassDeadline, ProducerTransport, StationIntentV1, Withdrawal,
    BRIDGE_PROBE_TIMEOUT, RECONCILE_BLOCKING_GRACE, STATION_INTENT_MAX_COUNT,
};
use std::collections::BTreeSet;
use std::sync::OnceLock;

/// A stamp that could not be performed, with the classification the register's admission rule
/// reads and the operator-facing detail it reports.
///
/// The classification is separate from the detail on purpose: the *decision* must be made from a
/// small closed set (see `station_intent::armed_proof_admission`), never by matching on a message.
#[derive(Debug, Clone)]
pub(crate) struct ArmedProofRefusal {
    pub(crate) failure: ArmedProofFailure,
    pub(crate) detail: String,
}

/// A held per-`MemberKey` delivery-admission guard.
///
/// Owned rather than borrowed so a teardown can hold it across the whole transition it is
/// linearizing — a durable lease release, a member removal, and the withdrawal — instead of only
/// across the last step of it.
pub(crate) struct BindingAdmission {
    _guard: tokio::sync::OwnedMutexGuard<()>,
}

/// Which bindings a set-scoped teardown is about.
///
/// A closed set rather than a predicate closure, because the enumeration it drives runs on the
/// blocking pool behind a deadline and therefore has to be `'static` and `Send`.
#[derive(Debug, Clone)]
pub(crate) enum TeardownScope {
    Session(String),
    Address(String),
}

impl TeardownScope {
    fn selects(&self, binding: &IntentBinding) -> bool {
        match self {
            TeardownScope::Session(session_id) => binding.session_id == *session_id,
            TeardownScope::Address(address) => binding.address == *address,
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Constants and published bounds
// ---------------------------------------------------------------------------------------------

/// Reconciliation rides the existing heartbeat tick rather than adding a second loop.
/// `HEARTBEAT_INTERVAL` is deliberately **not** made env-overridable: it carries a documented,
/// test-enforced invariant against `ON_DELIVER_DEFERRED_BACKSTOP`.
pub const RECONCILE_INTERVAL: Duration = HEARTBEAT_INTERVAL;

/// Wall-clock ceiling for one pass **as observed by whoever asked for it**. Strictly less than
/// `RECONCILE_INTERVAL`, so a pass can never overrun the tick that started it.
///
/// This is the published, end-to-end number, and it is the only one: the admin request bound and
/// the client ceiling are the same value, enforced by *sharing a deadline* rather than by running a
/// second clock beside the pass (see [`RECONCILE_RESPONSE_RESERVE`]).
///
/// Enforced as a **single absolute deadline** computed once at the top of the pass, against which
/// every phase — GC, discovery, and each wave — measures itself. A chain of independent per-phase
/// timeouts would bound each phase and nothing at all.
pub const RECONCILE_PASS_DEADLINE: Duration = Duration::from_secs(4);

/// The slice of the published bound reserved for *answering*, and therefore not available to the
/// pass's own phases.
///
/// A pass that budgeted its work to the full four seconds published a bound it could only ever meet
/// by accident: after the last phase returns there is still a task join, a report clone, a JSON
/// encode, and a socket write between the pass and the caller. The reserve makes that tail
/// explicit, so `RECONCILE_PASS_WORK_BUDGET + RECONCILE_RESPONSE_RESERVE = RECONCILE_PASS_DEADLINE`
/// is the whole arithmetic of the published number.
///
/// It is also what lets the admin handler stop running a clock of its own. The previous shape —
/// `tokio::spawn(pass)` plus a same-length `tokio::time::timeout` on the handler — had two clocks
/// started at two different instants for the same four seconds, so the handler's could fire while
/// the pass was still mid-wave. The caller got `ran: false` and the pass went on to register
/// members, advance cursors, and publish a report *after* the answer it was not in. One deadline,
/// originated by the request and passed into the pass, is what removes that window.
pub const RECONCILE_RESPONSE_RESERVE: Duration = Duration::from_millis(200);

/// What a pass's own phases actually get: the published bound minus the response reserve.
///
/// Every internal budget below is sized against *this*, not against `RECONCILE_PASS_DEADLINE`.
pub const RECONCILE_PASS_WORK_BUDGET: Duration = Duration::from_millis(3_800);

/// Minimum remaining budget for *starting* a durable pass-visible write (the evidence CAS, the
/// round-robin cursor) and joining it.
///
/// Bounding only the wait is the right answer for a phase whose result the pass merely reports; it
/// is the wrong answer for one that mutates state a caller can observe, because the write then
/// lands after the response. So such a write is never launched below this reserve: it is reported
/// as deadline-truncated having not run, and the next pass re-derives it. See
/// [`station_intent::run_blocking_reserved`] for the one residual case this cannot remove.
pub const RECONCILE_DURABLE_WRITE_RESERVE: Duration = Duration::from_millis(100);

/// Minimum remaining budget for *starting* an evidence-log append and joining it.
///
/// Smaller than the durable-write reserve because the append is smaller and because skipping it is
/// cheaper: the reconcile event log is diagnostics, never authority.
pub const RECONCILE_EVENT_LOG_RESERVE: Duration = Duration::from_millis(25);

/// Slice of the pass deadline that maintenance may consume before the first wave starts.
///
/// Sized so that `RECONCILE_MAINTENANCE_BUDGET + RECONCILE_BLOCKING_GRACE +
/// RECONCILE_PER_INTENT_TIMEOUT + RECONCILE_SCHEDULING_RESERVE` still fits inside
/// `RECONCILE_PASS_WORK_BUDGET`. That is what keeps the guaranteed-minimum-progress property true:
/// even a pass whose maintenance runs right up to its budget still has room to start — and finish —
/// a whole first wave *and* apply its outcomes inside the published bound. Both GC and discovery
/// resume from their own persisted positions, so capping them delays coverage rather than losing
/// it.
///
/// The grace term is in the sum because maintenance is the one phase whose *hard* bound is wider
/// than its cooperative one: a truncated scan has to be allowed to hand back the partial page it
/// stopped on, or a large scope would be discarded on every pass instead of advancing through it.
pub const RECONCILE_MAINTENANCE_BUDGET: Duration = Duration::from_millis(300);

/// Slice of the maintenance budget a due GC sweep may consume, leaving the rest for discovery.
pub const RECONCILE_GC_BUDGET: Duration = Duration::from_millis(150);

/// Whole per-intent budget: probe + local validation + backend claim.
pub const RECONCILE_PER_INTENT_TIMEOUT: Duration = Duration::from_secs(3);

/// One **total** internal deadline for a daemon-owned teardown (reset, session end).
///
/// Set-scoped teardowns used to be unbounded twice over: an unbounded synchronous scope scan, then
/// a fresh `RECONCILE_PER_INTENT_TIMEOUT` admission wait per binding. A reset of an address with a
/// dozen bindings could therefore sit on the request handler for well over half a minute, and a
/// scope on a wedged mount could sit there forever — neither of which is a bound anything
/// published. Each operation now computes this deadline once, at its top, and every phase inside it
/// — enumeration, each per-binding admission, each withdrawal — measures itself against what is
/// left. Expiry is an explicit **incomplete teardown** error, and no further state is published.
///
/// The same value as the per-intent budget: one contended binding is exactly the case an operator
/// waits through today, and a teardown that touches several is no more entitled to the operator's
/// patience than one that touches a single one.
pub const TEARDOWN_DEADLINE: Duration = RECONCILE_PER_INTENT_TIMEOUT;

/// The pass's own overhead, reserved rather than assumed to be free.
///
/// A pass is not just maintenance plus waves. Between and after them it spawns and joins wave
/// tasks, takes the index lock, folds every outcome (`apply_outcome`: a live-member lookup, an
/// evidence CAS write, and — for a terminal outcome — a linearized withdrawal that has to wait for
/// the binding's admission guard), advances the round-robin cursor, and appends to the event log.
/// Budgeting maintenance and waves to exactly the pass deadline left that work no time at all, so
/// the arithmetic said a pass fit while every real pass overran by however long its bookkeeping
/// took.
///
/// Reserving it has two consequences, both intended: a wave only starts when the reserve is still
/// intact behind it, and every post-wave phase is itself bounded by the remaining pass deadline
/// rather than by a fresh per-operation timeout.
pub const RECONCILE_SCHEDULING_RESERVE: Duration = Duration::from_millis(250);

/// Outer bound on the admin `ReconcileIntents` request, as observed by the daemon.
///
/// It is the **same** bound as the pass, and — since M3 — it is the same *deadline*, not merely the
/// same duration. The handler computes one absolute instant when the request arrives, hands it to
/// [`reconcile_once_until`], and awaits the pass against it. Two clocks of equal length started at
/// two different instants are not one bound: the handler's `timeout` could fire while the pass it
/// had spawned was still mid-wave, and the pass then went on registering members, advancing cursors,
/// and publishing a report after the caller had already been told `ran: false`. Sharing the deadline
/// removes the window rather than narrowing it.
///
/// The pass is spawned rather than polled inline so that a client that hangs up mid-request cannot
/// tear a pass in half at an arbitrary await point; the handler still *joins* it, so nothing is
/// detached behind an answered request.
pub const RECONCILE_ADMIN_DEADLINE: Duration = RECONCILE_PASS_DEADLINE;

/// Client-side ceiling for one `ReconcileIntents` round trip.
///
/// The same 4-second bound again, and callers additionally clamp it to whatever their own caller
/// left them (`remaining.min(RECONCILE_REQUEST_DEADLINE)`), so a request never outlives the budget
/// the operator actually gave the command. A client ceiling *wider* than the daemon's own bound
/// published a number nobody was enforcing: the daemon always answers by its admin bound, so time
/// spent waiting beyond it is time spent waiting for a connection that is already answering or
/// already lost.
pub const RECONCILE_REQUEST_DEADLINE: Duration = RECONCILE_PASS_DEADLINE;

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
pub const RECONCILE_EVENT_LOG_FILE: &str = "reconcile-events.ndjson";
const RECONCILE_EVENT_LOG_ROTATE_BYTES: u64 = 1_048_576;

/// Compile-time assertions for the invariants the published bounds are derived from. If any of
/// these ever stops holding, the documented recovery bounds stop being true, so they are enforced
/// here rather than only asserted in a test.
const _: () = {
    assert!(RECONCILE_PASS_DEADLINE.as_millis() < RECONCILE_INTERVAL.as_millis());
    // The published bound is the pass's own work budget plus the tail reserved for answering.
    // Nothing else may be added to either side of it.
    assert!(
        RECONCILE_PASS_WORK_BUDGET.as_millis() + RECONCILE_RESPONSE_RESERVE.as_millis()
            == RECONCILE_PASS_DEADLINE.as_millis()
    );
    assert!(RECONCILE_PER_INTENT_TIMEOUT.as_millis() <= RECONCILE_PASS_WORK_BUDGET.as_millis());
    assert!(BRIDGE_PROBE_TIMEOUT.as_millis() < RECONCILE_PER_INTENT_TIMEOUT.as_millis());
    assert!(RECONCILE_DEFERRED_LEASE_RETRY.as_millis() <= RECONCILE_INTERVAL.as_millis());
    assert!(RECONCILE_MAX_CONCURRENCY <= RECONCILE_PASS_BUDGET);
    // Guaranteed minimum progress: maintenance, one whole wave, and the pass's own bookkeeping all
    // fit inside the *work* budget. Drop the reserve from this sum and the arithmetic still "fits"
    // while every real pass overruns by however long its scheduling and outcome application take.
    assert!(
        RECONCILE_MAINTENANCE_BUDGET.as_millis()
            + RECONCILE_BLOCKING_GRACE.as_millis()
            + RECONCILE_PER_INTENT_TIMEOUT.as_millis()
            + RECONCILE_SCHEDULING_RESERVE.as_millis()
            <= RECONCILE_PASS_WORK_BUDGET.as_millis()
    );
    assert!(RECONCILE_GC_BUDGET.as_millis() < RECONCILE_MAINTENANCE_BUDGET.as_millis());
    // Both start-gates have to be payable out of the reserve the pass keeps for its own
    // bookkeeping, or a pass that spent exactly its budget could never persist anything at all.
    assert!(RECONCILE_DURABLE_WRITE_RESERVE.as_millis() < RECONCILE_SCHEDULING_RESERVE.as_millis());
    assert!(RECONCILE_EVENT_LOG_RESERVE.as_millis() < RECONCILE_DURABLE_WRITE_RESERVE.as_millis());
    // The published end-to-end bound is one number, so neither the admin backstop nor the client
    // ceiling may sit outside it.
    assert!(RECONCILE_ADMIN_DEADLINE.as_millis() <= RECONCILE_PASS_DEADLINE.as_millis());
    assert!(RECONCILE_REQUEST_DEADLINE.as_millis() <= RECONCILE_PASS_DEADLINE.as_millis());
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
    last_gc_ms: std::sync::atomic::AtomicI64,
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
            last_gc_ms: std::sync::atomic::AtomicI64::new(0),
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

    /// Publish an explicit withdrawal's outcome into the in-memory index, **generation-aware**.
    ///
    /// The index is a projection, not a source of truth, and a withdrawal races the reconcile pass
    /// that maintains it. Publishing unconditionally let a withdrawal that deleted generation 4
    /// stamp `revoked` over an entry a re-attach had already advanced to generation 5, so
    /// `telex status` showed a live binding as revoked until the next pass corrected it.
    ///
    /// It also never *creates* an entry. A withdrawal has no identity to publish: an entry minted
    /// here would be a `revoked` row for a binding with no manifest behind it — the projection
    /// equivalent of the identity-less tombstone the store itself refuses to write.
    fn index_publish_withdrawal(&self, key: IntentKey, outcome: Withdrawal) {
        let Some(mut entry) = self.index_entry(&key) else {
            return;
        };
        match outcome {
            // A newer record than the one this call decided about; the pass that wrote the entry
            // knows more than this withdrawal does.
            Withdrawal::NoRecord | Withdrawal::Superseded { .. } => {}
            Withdrawal::DeletedPending { generation } => {
                if entry.generation <= generation {
                    self.index_remove(&key);
                }
            }
            Withdrawal::Revoked { generation } | Withdrawal::AlreadyRevoked { generation } => {
                if entry.generation <= generation {
                    entry.state = IntentRecoveryState::Revoked;
                    entry.generation = generation;
                    self.index_upsert(key, entry);
                }
            }
        }
    }

    /// Withdraw one binding's durable desired state under the **same per-station admission
    /// ordering** a reconcile pass uses.
    ///
    /// The ordering is the point. Withdrawal and restoration are the two writers of a station's
    /// membership, and they were previously serialized by nothing at all: a pass could be halfway
    /// through its restore chain — credential read, producer probe, `ensure_address`, epoch claim —
    /// when a detach revoked the record, and it would still go on to install the armed member the
    /// revoked record no longer authorized. Taking the delivery-admission guard makes the two
    /// mutually exclusive per binding, and the guard is the *outermost* lock, so the ordering is
    /// uniformly admission guard → per-intent file lock everywhere.
    ///
    /// This is the **acquiring** form, for a caller whose whole teardown is this one withdrawal —
    /// today only the reconcile pass applying a terminal outcome. A caller that also mutates
    /// lifecycle or member state — a detach, a reset, a session end, a fallback downgrade — must
    /// instead take [`DaemonState::admit_binding`] *before* that mutation and call
    /// [`DaemonState::withdraw_intent_admitted`] inside it. Withdrawing under a guard the mutation
    /// did not hold is not a linearization: the reconciler could publish an armed push member in
    /// between, and the teardown would then revoke the manifest while leaving the member it
    /// authorized installed.
    ///
    /// The admission budget is explicit rather than a fixed [`RECONCILE_PER_INTENT_TIMEOUT`]. That
    /// constant is the right budget for a caller starting a fresh operation, and the wrong one for
    /// the reconcile pass applying a terminal outcome: that caller is already inside an absolute
    /// pass deadline it has mostly spent, and a fresh three-second admission wait there is added
    /// *on top* of the wave — which is exactly how a pass bounded at four seconds answered an admin
    /// request at seven. Such a caller passes what the pass has left, and treats exhaustion as a
    /// deferral rather than as a teardown failure.
    ///
    /// Fallible on purpose — see [`IntentStore::withdraw_binding`]. Callers propagate.
    #[cfg(feature = "sqlite")]
    pub(crate) async fn withdraw_intent_at_generation_within(
        &self,
        store_key: &str,
        session_id: &str,
        address: &str,
        expected_generation: Option<u64>,
        admission_budget: Duration,
    ) -> std::result::Result<Withdrawal, String> {
        self.withdraw_intent_at_generation_until(
            store_key,
            session_id,
            address,
            expected_generation,
            PassDeadline::at(Instant::now() + admission_budget),
        )
        .await
    }

    /// The deadline-carrying form used by reconciliation after an intent has been admitted.
    ///
    /// Admission and the filesystem withdrawal share this one absolute deadline. In particular, a
    /// pass that waited for admission must not begin a fresh synchronous withdrawal after its caller
    /// has exhausted the pass budget.
    pub(crate) async fn withdraw_intent_at_generation_until(
        &self,
        store_key: &str,
        session_id: &str,
        address: &str,
        expected_generation: Option<u64>,
        deadline: PassDeadline,
    ) -> std::result::Result<Withdrawal, String> {
        let admission_budget = deadline
            .remaining()
            .unwrap_or(RECONCILE_PER_INTENT_TIMEOUT)
            .min(RECONCILE_PER_INTENT_TIMEOUT);
        if admission_budget.is_zero() {
            return Err(format!(
                "timed out before admitting station intent {session_id}/{address}"
            ));
        }
        let _admit = self
            .admit_binding(store_key, session_id, address, admission_budget)
            .await?;
        if deadline.expired() {
            return Err(format!(
                "timed out after admitting station intent {session_id}/{address}"
            ));
        }
        let Some(store) = self.intent_store_readonly()? else {
            return Ok(Withdrawal::NoRecord);
        };
        let store_key = store_key.to_string();
        let session_id = session_id.to_string();
        let address = address.to_string();
        let index_key = IntentKey {
            store_key: store_key.clone(),
            session_id: session_id.clone(),
            address: address.clone(),
        };
        let withdrawal_store_key = store_key.clone();
        let withdrawal_session_id = session_id.clone();
        let withdrawal_address = address.clone();
        let outcome = match station_intent::run_blocking_cancellable_within(
            deadline,
            Duration::ZERO,
            move |cancelled| {
                if cancelled.load(Ordering::Acquire) || deadline.expired() {
                    return Err("withdrawal deadline expired before durable mutation".to_string());
                }
                store
                    .withdraw_binding_at_generation(
                        &withdrawal_store_key,
                        &withdrawal_session_id,
                        &withdrawal_address,
                        expected_generation,
                        now_ms(),
                    )
                    .map_err(|e| {
                        format!(
                            "withdrawing station intent {withdrawal_session_id}/{withdrawal_address}: {e}"
                        )
                    })
            },
        )
        .await
        {
            BoundedPhase::Completed(outcome) => outcome?,
            BoundedPhase::Overran => {
                return Err(format!(
                    "timed out withdrawing station intent {session_id}/{address}"
                ))
            }
        };
        self.index_publish_withdrawal(index_key, outcome);
        Ok(outcome)
    }

    /// Acquire this binding's delivery-admission guard, bounded.
    ///
    /// The guard is the outermost lock in the daemon and it is **not reentrant**: never call this
    /// from a context that already holds one (`register_member`, `reconcile_intent_locked`, or an
    /// enclosing teardown). It is held across backend awaits by design — a register does, and so
    /// does a detach, because "release the durable lease, then tear the member down, then withdraw"
    /// is one transition and splitting it across the guard is what let a reconcile pass slip an
    /// armed member into the middle of it.
    ///
    /// The bound is what keeps a wedged guard from turning into a request handler that never
    /// returns; a set-scoped teardown passes what is left of its *one* total deadline rather than a
    /// fresh per-binding budget.
    pub(crate) async fn admit_binding(
        &self,
        store_key: &str,
        session_id: &str,
        address: &str,
        budget: Duration,
    ) -> std::result::Result<BindingAdmission, String> {
        let admission = self
            .delivery_admission(
                store_key,
                session_id,
                address,
                DeliveryAdmissionKind::Register,
            )
            .await;
        let guard = tokio::time::timeout(budget, admission.lock_owned())
            .await
            .map_err(|_| {
                format!(
                    "timed out waiting to admit station intent {session_id}/{address}: \
                     another delivery operation holds the admission guard"
                )
            })?;
        Ok(BindingAdmission { _guard: guard })
    }

    /// The withdrawal itself, for a caller that **already holds** this binding's admission guard.
    ///
    /// Synchronous and self-contained: it acquires the per-intent file lock, does its own I/O, and
    /// releases it, so a caller may hold the admission guard across backend awaits without ever
    /// holding a filesystem lock across one.
    pub(crate) fn withdraw_intent_admitted(
        &self,
        store_key: &str,
        session_id: &str,
        address: &str,
        expected_generation: Option<u64>,
    ) -> std::result::Result<Withdrawal, String> {
        // Open-existing: a withdrawal must never bring a scope into existence. A host that never
        // attached has nothing to withdraw, and an unreadable scope is a refusal, not an absence.
        let Some(store) = self.intent_store_readonly()? else {
            return Ok(Withdrawal::NoRecord);
        };
        let outcome = store
            .withdraw_binding_at_generation(
                store_key,
                session_id,
                address,
                expected_generation,
                now_ms(),
            )
            .map_err(|e| format!("withdrawing station intent {session_id}/{address}: {e}"))?;
        self.index_publish_withdrawal(
            IntentKey {
                store_key: store_key.to_string(),
                session_id: session_id.to_string(),
                address: address.to_string(),
            },
            outcome,
        );
        Ok(outcome)
    }

    /// Every binding of one session that the durable scope names, enumerated inside `deadline`.
    ///
    /// Used by the daemon's own session-end paths (`sessionEnd`, watch-pid death, definite end), so
    /// an ended session can never be re-attended by a stale intent. Enumerates from the *scope*,
    /// not from membership: a binding whose member was already released still has durable desired
    /// state, and it is precisely that record which would bring the ended session back.
    pub(crate) async fn session_teardown_bindings(
        &self,
        store_key: &str,
        session_id: &str,
        deadline: PassDeadline,
    ) -> std::result::Result<Vec<IntentBinding>, String> {
        self.teardown_bindings(
            store_key,
            TeardownScope::Session(session_id.to_string()),
            deadline,
        )
        .await
    }

    /// Every binding of one address that the durable scope names, whatever session owns it.
    ///
    /// This is what an operator reset needs. Reset used to derive its withdrawal set from the
    /// members it *changed*, which silently excluded the two cases that matter most: a station with
    /// no member at all (the manifest is the only thing left, and the next pass restores it), and a
    /// member that was already idle (marking it idle changes nothing, so nothing was withdrawn).
    /// Enumerating the scope covers both.
    pub(crate) async fn address_teardown_bindings(
        &self,
        store_key: &str,
        address: &str,
        deadline: PassDeadline,
    ) -> std::result::Result<Vec<IntentBinding>, String> {
        self.teardown_bindings(
            store_key,
            TeardownScope::Address(address.to_string()),
            deadline,
        )
        .await
    }

    /// Enumerate a teardown's binding set, bounded, refusing every partial answer.
    ///
    /// Enumeration is separated from mutation because each binding is torn down under *its own*
    /// admission guard, and holding one binding's guard while enumerating (or while tearing another
    /// down) would invert the documented outermost-guard ordering.
    ///
    /// Three ways this refuses rather than returning a set:
    ///
    /// * The blocking scan overran the operation's deadline. It is run through
    ///   [`station_intent::run_blocking_within`] rather than inline, because the cooperative checks
    ///   inside the scan cannot bound the `read_dir` or the `read` that is currently blocked — and
    ///   a scope on a wedged mount is exactly the case a teardown deadline exists for.
    /// * The scan completed but was truncated by its own deadline. A partial list is not the set.
    /// * A manifest that could not be read *claims* to belong to the set. It cannot be acted on —
    ///   its identity is unvalidated, so honouring the claim would let one manifest redirect a
    ///   withdrawal onto another binding's record — and it cannot be silently skipped either,
    ///   because "I could not read the record you asked me to withdraw" is exactly the answer a
    ///   teardown must not swallow.
    ///
    /// In all three the caller must publish no further state: an incomplete teardown is reported as
    /// a failed one.
    async fn teardown_bindings(
        &self,
        store_key: &str,
        scope: TeardownScope,
        deadline: PassDeadline,
    ) -> std::result::Result<Vec<IntentBinding>, String> {
        let Some(store) = self.intent_store_readonly()? else {
            return Ok(Vec::new());
        };
        let scan = match station_intent::run_blocking_within(
            deadline,
            RECONCILE_BLOCKING_GRACE,
            move || store.bindings_bounded(deadline),
        )
        .await
        {
            BoundedPhase::Completed(scan) => {
                scan.map_err(|e| format!("enumerating station intents in {store_key}: {e}"))?
            }
            BoundedPhase::Overran => {
                return Err(format!(
                    "timed out enumerating station intents in {store_key}; \
                     refusing to report a teardown that did not complete"
                ))
            }
        };
        if scan.truncated {
            return Err(format!(
                "the station-intent scope for {store_key} could not be enumerated within the \
                 teardown deadline; refusing to report a teardown that did not complete"
            ));
        }
        for (id, claimed) in &scan.unreadable {
            let matches_scope = claimed
                .as_ref()
                .is_some_and(|binding| binding.store_key == store_key && scope.selects(binding));
            if matches_scope {
                return Err(format!(
                    "station intent {id} is unreadable but names this scope; \
                     refusing to report a withdrawal that did not happen"
                ));
            }
        }
        Ok(scan
            .bindings
            .into_iter()
            .filter(|binding| binding.store_key == store_key && scope.selects(binding))
            .collect())
    }

    /// Stamp the durable **armed proof** on a binding's intent record, as part of committing an
    /// armed push registration.
    ///
    /// Called by `register_member` **before** it installs the member, not after it returns. That
    /// placement is the whole point: it closes the window between `Register` committing and the
    /// producer-side finalize, in which a crash (of the CLI, of the agent, of the machine) left a
    /// `pending` record that the five-minute pending TTL then deleted while push delivery went on
    /// working — and it closes the narrower window in which a *concurrent* attach's rollback could
    /// delete the record between the member commit and the stamp, leaving an armed station with no
    /// durable proof at all while the register still reported success.
    ///
    /// Opens the scope through the **read** path, which never creates it. A binding with no durable
    /// record has nothing to prove, and a scope with no directory holds no records, so that case is
    /// [`ArmedProofStamp::NoRecord`] rather than a failure — a register that owes no proof must not
    /// be refused because a directory it had nothing to put in could not be created. The stamp
    /// still *writes* through this handle whenever the scope does exist, which is the only case in
    /// which a record can be there to stamp.
    ///
    /// Deliberately **not** best effort, and deliberately not swallowing its result: the caller
    /// decides (through `station_intent::armed_proof_admission`), and a register that owes a proof
    /// it could not persist is aborted rather than reported as a durable push registration.
    /// Idempotent, so the hot re-register path costs a stat and nothing else.
    pub(crate) fn stamp_intent_armed(
        &self,
        store_key: &str,
        session_id: &str,
        address: &str,
    ) -> std::result::Result<ArmedProofStamp, ArmedProofRefusal> {
        let store = match self.intent_store_readonly() {
            Ok(Some(store)) => store,
            // No scope on disk at all, so no record for this binding either.
            Ok(None) => return Ok(ArmedProofStamp::NoRecord),
            Err(detail) => {
                return Err(ArmedProofRefusal {
                    failure: ArmedProofFailure::ScopeUnavailable,
                    detail,
                })
            }
        };
        // Every error `stamp_armed_proof` can return is about the record itself: an absent manifest
        // is reported as `NoRecord`, never as an error.
        store
            .stamp_armed_proof(store_key, session_id, address, &self.instance_id, now_ms())
            .map_err(|e| ArmedProofRefusal {
                failure: ArmedProofFailure::RecordUnusable,
                detail: e.to_string(),
            })
    }

    /// Whether the durable intent scope currently holds a record for this binding.
    ///
    /// Read before an arming register runs, so the daemon knows whether that register *owes* a
    /// durable proof. Without it, "no record at the stamp" is ambiguous: it is the ordinary pull
    /// or plain-`--on-deliver` attach (nothing to prove) and it is also "a concurrent rollback
    /// deleted the record this register was going to stamp" (everything to prove).
    ///
    /// `Err` is "the scope exists but could not be read", which the caller fails closed on exactly
    /// as the anti-downgrade guard does — an unreadable scope is the `Insecure` condition the rest
    /// of this design refuses to guess about. A scope that does not exist at all is `Ok(false)`:
    /// this host has never attached, so there is genuinely nothing to prove.
    ///
    /// The record probe is [`platform_fs::path_present`], never `Path::exists()`. `Ok(false)` here
    /// means "this register owes no proof", so a record that is present but unstatable — an ACL, an
    /// untraversable parent, a volume that went away — must not be able to produce it: the register
    /// would then owe nothing, the stamp would report `NoRecord` for the same reason, and an
    /// ordinary admission would commit an armed member over a durable record it never proved.
    pub(crate) fn durable_intent_present(
        &self,
        store_key: &str,
        session_id: &str,
        address: &str,
    ) -> std::result::Result<bool, String> {
        match self.intent_store_readonly()? {
            None => Ok(false),
            Some(store) => {
                let path = store.path_for(&IntentId::derive(store_key, session_id, address));
                platform_fs::path_present(&path)
                    .map_err(|e| format!("checking {}: {e}", path.display()))
            }
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
        // `Absent` is the answer that lets the downgrade through, so it is owed a proof of absence.
        // `Path::exists()` gave it away for a record it merely could not stat, which fails the
        // guard **open** for exactly the record it is meant to protect.
        match platform_fs::path_present(&store.path_for(&id)) {
            Ok(true) => {}
            Ok(false) => return LiveIntentLookup::Absent,
            Err(e) => {
                return LiveIntentLookup::Unavailable(format!(
                    "checking {}: {e}",
                    store.path_for(&id).display()
                ))
            }
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
    /// Built from the cached index, with a **bounded durable backfill** for bindings the index has
    /// no fresh answer for. No probe, no connection, no backend or network I/O — so it still
    /// cannot push a graceful drain past `--drain-timeout-ms` — and it is evaluated *before* the
    /// lease-release loop, so it describes what a successor will find.
    ///
    /// The backfill exists because the index is only refreshed by a reconcile pass, while the
    /// durable record is written by a *producer-side* finalize in a different process. `attach`
    /// immediately followed by `upgrade` therefore drained with `recoverable = 0` for a binding
    /// that was fully armed and finalized seconds earlier, and the successor-verification step
    /// skipped itself on "no recoverable station intents" — silently turning the one path that is
    /// supposed to hand push delivery across a daemon replacement into a no-op.
    ///
    /// The two sources are combined so neither can mask the other:
    ///
    /// * A cached projection that names a *problem* (degraded, incompatible, revoked) wins **only
    ///   while it still describes the manifest on disk**. That is real evidence from an attempt,
    ///   and a successor will hit the same wall — but only if the record it will read is the one
    ///   the attempt failed against. Generation is what decides that: every durable state
    ///   transition (a finalize, a producer-identity refresh, an arming stamp, a re-attach) moves
    ///   it, while the reconciler's own evidence writes deliberately do not. A cached
    ///   `producer_identity_mismatch` for generation N therefore stops applying the moment the
    ///   turn-boundary hook writes generation N+1, and holding on to it made `upgrade` report a
    ///   freshly repaired binding as degraded and skip the hand-off it exists to perform.
    /// * Otherwise the durable manifest wins, because it is what the successor will actually read.
    pub(crate) fn drain_intent_report(&self) -> DrainIntentReport {
        let snapshot = self.intent_index_snapshot();
        let durable = self.durable_intent_states();
        let mut report = DrainIntentReport {
            over_cap: snapshot.over_cap,
            observed_count: snapshot.observed_count,
            index_as_of_ms: snapshot.as_of_ms,
            ..Default::default()
        };
        // `(state, generation)` throughout: the state alone cannot say which record it describes.
        let mut states: BTreeMap<IntentKey, (IntentRecoveryState, u64)> = BTreeMap::new();
        for (key, entry) in snapshot.entries.iter() {
            states.insert(key.clone(), (entry.state, entry.generation));
        }
        for (key, (durable_state, durable_generation)) in durable {
            let cached = states.get(&key).copied();
            let effective = match cached {
                // A cached failure/incompatibility/revocation is evidence the successor will hit
                // the same wall — provided it was recorded against the generation the successor
                // will read. Anything else (never seen, still `pending` because no pass has run
                // since the finalize, an already-recoverable projection, or a projection that is
                // now stale because the manifest moved on) defers to the manifest.
                Some((cached_state, cached_generation))
                    if cached_generation >= durable_generation
                        && !cached_state.is_recoverable()
                        && cached_state != IntentRecoveryState::Pending
                        && cached_state != IntentRecoveryState::Unknown =>
                {
                    cached_state
                }
                _ => durable_state,
            };
            let generation = cached
                .map(|(_, cached_generation)| cached_generation.max(durable_generation))
                .unwrap_or(durable_generation);
            states.insert(key, (effective, generation));
        }
        for (state, _) in states.values() {
            match state {
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
        // carried separately rather than lost.
        report.unidentifiable = snapshot.unidentifiable;
        report.degraded += snapshot.unidentifiable;
        report
    }

    /// The durable state **and generation** of every readable record in the scope, read without
    /// creating it.
    ///
    /// The generation travels with the state because the caller has to compare it against the
    /// cached projection's: a cached problem only describes the record a successor will read while
    /// the two generations agree, and a manifest that moved on has invalidated the attempt that
    /// produced the cached verdict.
    ///
    /// Bounded by the per-scope cap and free of any network or probe I/O. An unreadable scope or
    /// an unreadable manifest simply contributes nothing: the cached index (and the pass's own
    /// `unidentifiable` count) already carries what is known about those, and inventing a state
    /// for a record we could not read would be worse than deferring to what the last pass proved.
    fn durable_intent_states(&self) -> BTreeMap<IntentKey, (IntentRecoveryState, u64)> {
        let mut states = BTreeMap::new();
        let Ok(Some(store)) = self.intent_store_readonly() else {
            return states;
        };
        let Ok(ids) = store.list_ids() else {
            return states;
        };
        for id in ids.into_iter().take(STATION_INTENT_MAX_COUNT) {
            let Ok(intent) = store.load(&id) else {
                continue;
            };
            states.insert(
                IntentKey {
                    store_key: intent.store_key.clone(),
                    session_id: intent.session_id.clone(),
                    address: intent.address.clone(),
                },
                (intent.state, intent.generation),
            );
        }
        states
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
///
/// `pub(super)` so the daemon's own test module can pin the missing-versus-unreadable distinction
/// below directly; it has no other caller outside this module.
pub(super) async fn backend_open_existing_only(
    state: &Arc<DaemonState>,
    store_key: &str,
) -> std::result::Result<Arc<dyn Backend>, String> {
    if let Some(entry) = state.stores.lock().unwrap().get(store_key).cloned() {
        return Ok(entry.backend);
    }
    #[cfg(feature = "sqlite")]
    if let Some(path) = store_key.strip_prefix("sqlite:") {
        // Only a proven absence is `store_missing`, because that code is **terminal**: it parks the
        // intent on the hour-long quarantine cadence on the reasoning that a store which does not
        // exist will not start existing. A store file whose metadata could not be read is the
        // opposite kind of condition — a lock, an ACL, a mount that came back — so it takes the
        // ordinary retry ladder with every other transient backend failure.
        match platform_fs::path_present(Path::new(path)) {
            Ok(true) => {}
            Ok(false) => return Err("store_missing".to_string()),
            Err(e) => return Err(format!("store_unreadable: {e}")),
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
/// The order is the point: peer verification runs **before anything is sent**, so the secret only
/// ever reaches a process that has already been proven to be the same user, the recorded
/// executable, and the recorded `(pid, start_time)`. Identity is never inferred from the answer.
/// That order, the response cap, and the connect/exchange budgets all live in
/// [`super::verified_peer`], which the producer-side finalize and push paths use too — the rule is
/// implemented once and every caller that hands out a credential inherits it.
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

    let nonce = probe_nonce();
    let request = serde_json::json!({
        "op": "probe",
        "nonce": nonce,
        "protocol": BRIDGE_PROBE_MIN_PROTOCOL,
        "secret": secret,
    });
    let line = serde_json::to_string(&request)
        .map_err(|_| IntentOutcome::failed("probe_encode_failed"))?;

    let response = verified_peer::exchange(
        &endpoint,
        verified_peer::ExpectedPeer {
            exe_path: &intent.producer.exe_path,
            pid: intent.producer.pid,
            start_time: intent.producer.start_time,
        },
        verified_peer::LineExchange {
            request_line: &line,
            // Frame cap on the producer's answer, mirroring the daemon's own
            // `MAX_JSONL_FRAME_BYTES` policy. This is the only place the daemon reads an external
            // JSON line, and the producer is verified but never *trusted*: an unbounded read lets
            // a buggy or hostile bridge stream until the probe timeout and hand an arbitrarily
            // large string to the status projection, which then exceeds the IPC frame cap and
            // fails every `telex status` call.
            max_response_bytes: PROBE_MAX_RESPONSE_BYTES,
            connect_timeout: BRIDGE_PROBE_TIMEOUT,
            exchange_timeout: BRIDGE_PROBE_TIMEOUT,
        },
    )
    .await
    .map_err(|e| match e {
        verified_peer::ExchangeError::Connect(_) => IntentOutcome::failed("producer_unreachable"),
        verified_peer::ExchangeError::ConnectTimeout => {
            IntentOutcome::failed("producer_connect_timeout")
        }
        // Retryable, not terminal: the overwhelmingly common cause is a bridge *reload*
        // (`extensions_reload`, `/clear`, an extension-host restart), which gives the producer a
        // new pid and start time while the manifest still names the old pair. The turn-boundary
        // hook refreshes the recorded identity from the live registry, so the ladder is what lets
        // this heal on its own instead of parking the binding for the quarantine hour.
        verified_peer::ExchangeError::PeerUnverified(_) => {
            IntentOutcome::failed("producer_identity_mismatch")
        }
        verified_peer::ExchangeError::Io(_) => IntentOutcome::failed("probe_io_failed"),
        verified_peer::ExchangeError::ExchangeTimeout => IntentOutcome::failed("probe_timeout"),
        verified_peer::ExchangeError::ResponseTooLarge => {
            IntentOutcome::failed("probe_response_too_large")
        }
    })?;
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
                    return match commit_member_if_intent_live(state, intent, || {
                        state.insert_member(refreshed)
                    }) {
                        Ok(true) => IntentOutcome::Restored,
                        Ok(false) => {
                            IntentOutcome::terminal(IntentRecoveryState::Revoked, "withdrawn")
                        }
                        Err(_) => IntentOutcome::failed("intent_recheck_failed"),
                    };
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
                let published = refreshed.clone();
                match commit_member_if_intent_live(state, intent, || state.insert_member(published))
                {
                    Ok(true) => {}
                    Ok(false) => {
                        return IntentOutcome::terminal(IntentRecoveryState::Revoked, "withdrawn")
                    }
                    Err(_) => return IntentOutcome::failed("intent_recheck_failed"),
                }
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
        capability: crate::daemon_ipc::StationCapability::Bidirectional,
        store_key: intent.store_key.clone(),
        backend: backend.kind().to_string(),
        session_id: intent.session_id.clone(),
        application_responsibility: None,
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
    let published = record.clone();
    // The last gate before an armed push member exists: a withdrawal that landed anywhere in the
    // chain above must beat this publication, and the lease claimed for it has to go back.
    match commit_member_if_intent_live(state, intent, || state.insert_member(published)) {
        Ok(true) => {}
        Ok(false) => {
            let _ = backend
                .release_epoch_lease(
                    &intent.address,
                    &claimed_owner_instance_id,
                    claimed_lease_epoch,
                )
                .await;
            return IntentOutcome::terminal(IntentRecoveryState::Revoked, "withdrawn");
        }
        Err(_) => {
            let _ = backend
                .release_epoch_lease(
                    &intent.address,
                    &claimed_owner_instance_id,
                    claimed_lease_epoch,
                )
                .await;
            return IntentOutcome::failed("intent_recheck_failed");
        }
    }
    spawn_on_deliver_backlog(state.clone(), record);
    IntentOutcome::Restored
}

/// Publish a restored member **only while** the manifest that authorized it is still live at the
/// generation this pass reconciled.
///
/// The restore chain between reading a manifest and installing a member is a long sequence of
/// awaits — a credential read, a producer probe, `ensure_address`, an epoch claim, two tombstone
/// queries — and an explicit withdrawal can land anywhere inside it. The tombstone checks catch a
/// *detach*, because detach writes a durable backend tombstone; nothing caught an operator reset, a
/// session end, or a fallback downgrade, none of which write one. The pass finished its chain and
/// armed a push member the desired state no longer authorized, and only a later pass could notice.
///
/// Re-checking under the intent's own write lock — the same lock withdrawal takes — linearizes the
/// two: either the withdrawal lands first and this refuses to publish, or this publishes first and
/// the withdrawal sees the member it must tear down. The commit is synchronous (an in-memory
/// insert), so the file lock is never held across an await.
///
/// `Ok(false)` is "the manifest no longer authorizes this member"; `Err` is "that could not be
/// decided", which is a refusal too — an unreadable manifest must never be read as consent.
fn commit_member_if_intent_live(
    state: &Arc<DaemonState>,
    intent: &StationIntentV1,
    commit: impl FnOnce(),
) -> std::result::Result<bool, String> {
    let Some(store) = state.intent_store_readonly()? else {
        // The scope vanished under the pass. Whatever authorized this member is gone with it.
        return Ok(false);
    };
    store
        .commit_if_live_generation(&intent.id(), intent.generation, commit)
        .map(|committed| committed.is_some())
        .map_err(|e| format!("re-checking station intent for {}: {e}", intent.address))
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

/// Run one bounded reconciliation pass on the pass's own budget.
///
/// The scheduled entry point (heartbeat tick, trigger pulse, startup scan), where nobody is waiting
/// on a socket. Request-originated callers must use [`reconcile_once_until`] so the deadline the
/// pass measures itself against is the *same instant* the caller is bounded by.
///
/// Acquires the per-scope single-flight guard and the per-`MemberKey` admission guard for each
/// intent. Never call this from a context that already holds an admission guard — use
/// [`reconcile_intent_locked`] there.
pub async fn reconcile_once(state: Arc<DaemonState>, scope: Option<String>) -> ReconcileReport {
    reconcile_once_until(
        state,
        scope,
        Instant::now() + RECONCILE_PASS_WORK_BUDGET,
        PassOrigin::Scheduled,
    )
    .await
}

/// Who asked for this pass, which is the only thing that differs between the two entry points.
///
/// It changes no budget and no phase; it exists so the event log can distinguish a pass a caller was
/// blocked on from a scheduled one, and so the reason a pass was given a particular deadline is
/// carried with the pass instead of inferred.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PassOrigin {
    /// A heartbeat tick, a trigger pulse, or the startup scan.
    Scheduled,
    /// An admin `ReconcileIntents` request, whose caller is blocked on this pass and shares its
    /// deadline.
    Request,
}

impl PassOrigin {
    fn as_str(self) -> &'static str {
        match self {
            PassOrigin::Scheduled => "scheduled",
            PassOrigin::Request => "request",
        }
    }
}

/// Run one bounded reconciliation pass against a **caller-supplied absolute deadline**.
///
/// This is the shape the published four-second bound actually rests on. A request-originated pass
/// takes `request_arrival + RECONCILE_PASS_WORK_BUDGET`, so the instant the pass stops starting work
/// and the instant its caller must be answered by are derived from the same origin, and the reserve
/// between them ([`RECONCILE_RESPONSE_RESERVE`]) is what pays for the join, the encode, and the
/// socket write.
///
/// The property that buys is worth stating precisely, because it is the one the previous
/// spawn-and-race shape could not state: **every member registration, cursor advance, evidence
/// write, and report publication this pass performs happens before it returns.** Waves are joined,
/// not detached; a wave is only admitted while a whole per-intent timeout plus the scheduling
/// reserve still fits, and each intent inside it is individually timed out; post-wave work measures
/// itself against `deadline`; and the two durable writes are only *started* while
/// [`RECONCILE_DURABLE_WRITE_RESERVE`] remains. So a caller who receives this pass's report can say
/// that nothing further from it is coming.
pub async fn reconcile_once_until(
    state: Arc<DaemonState>,
    scope: Option<String>,
    deadline: Instant,
    origin: PassOrigin,
) -> ReconcileReport {
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

    // One absolute deadline for the whole pass, **originated by whoever asked for it**. Every
    // bounded phase below measures itself against *this* instant rather than against its own
    // elapsed time, which is what makes the published bound a property of the pass instead of a
    // property of each phase in isolation — and, for a request-originated pass, a property the
    // caller shares rather than one it races.
    let pass_deadline = deadline;
    // Maintenance (GC + discovery) is confined to the head of the pass so the reserve behind it is
    // still large enough for a full first wave. Clamped to the pass deadline, because a caller may
    // hand over less than a whole work budget (a client that clamped its own request to what its
    // caller left it), and a maintenance window wider than the pass would be no window at all.
    let maintenance_deadline =
        PassDeadline::at((started + RECONCILE_MAINTENANCE_BUDGET).min(pass_deadline));

    // Maintenance GC is wall-clock scheduled. Skipped passes deliberately do not move this clock,
    // so drain suppression or single-flight contention cannot consume a maintenance slot.
    let now = now_ms();
    let last_gc_ms = state.intents.last_gc_ms.load(Ordering::Relaxed);
    if last_gc_ms == 0 || now.saturating_sub(last_gc_ms) >= RECONCILE_GC_INTERVAL.as_millis() as i64
    {
        run_intent_gc(
            &state,
            PassDeadline::at(started + RECONCILE_GC_BUDGET).earliest(maintenance_deadline),
            PassDeadline::at(pass_deadline),
        )
        .await;
    }

    // Scope filtering happens inside the scan, ahead of the page window: a pass filtered to one
    // store must spend its whole budget on that store, and its cursor must be that store's own.
    //
    // Run behind the blocking boundary, and awaited only until the maintenance deadline: discovery
    // is `read_dir` plus one owner-checked read per manifest, and a pass that had to wait for a
    // hung one had no bound at all no matter how many cooperative checks sat between the calls.
    let scan = store
        .scan_bounded_within(RECONCILE_PASS_BUDGET, scope.clone(), maintenance_deadline)
        .await;
    let page = match scan {
        BoundedPhase::Completed(Ok(page)) => page,
        BoundedPhase::Completed(Err(e)) => {
            state.push_recent_error("StationIntent", format!("scanning intent scope: {e}"));
            report = ReconcileReport::skipped_pass(pass_seq, "scan_failed");
            publish(&state, &report);
            return report;
        }
        BoundedPhase::Overran => {
            // Discovery is still running and will still persist its own resume position, but this
            // pass never saw the scope: it must publish nothing derived from it and must not go on
            // to attempt a wave it has no page for. `ran: false` is the existing protocol for
            // "no result to report", so an admin caller retries instead of reading an empty page
            // as a scope with nothing in it.
            report = ReconcileReport::skipped_pass(pass_seq, "discovery_deadline");
            report.deadline_reached = true;
            report.duration_ms = started.elapsed().as_millis() as u64;
            publish(&state, &report);
            return report;
        }
    };
    report.observed_count = page.observed_count;
    report.over_cap = page.over_cap;
    report.skipped = page.skipped.len();
    if page.discovery_truncated {
        // A truncated discovery saw a *prefix* of the scope, so its counts are lower bounds.
        // Publishing them as-is would let one slow pass silently retract an over-cap warning (and
        // shrink the observed count) for a scope nothing has reclaimed.
        report.deadline_reached = true;
    }
    {
        let mut index = state.intents.index.lock().unwrap();
        if page.discovery_truncated {
            index.observed_count = index.observed_count.max(page.observed_count);
            index.over_cap = index.over_cap || page.over_cap;
        } else {
            index.observed_count = page.observed_count;
            index.over_cap = page.over_cap;
        }
        index.as_of_ms = now_ms();
        report.observed_count = index.observed_count;
        report.over_cap = index.over_cap;
    }
    if page.over_cap {
        state.push_recent_error(
            "StationIntent",
            format!(
                "station-intent scope holds {} entries, over the {STATION_INTENT_MAX_COUNT} write cap; \
                 existing records remain updateable or withdrawable, but revocation does not free \
                 capacity until the seven-day terminal TTL expires and daemon GC removes the record",
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
        log_event_within(
            &store,
            PassDeadline::at(pass_deadline),
            serde_json::json!({
                "event": "intent_rejected",
                "pass_seq": pass_seq,
                "intent_id": id.to_string(),
                "state": rejection.state,
                "identified": rejection.identity.is_some(),
                "detail": rejection.detail,
            }),
        )
        .await;
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
    //
    // Only *reconcilable* records compete for the slot, and the claim is taken **after** the
    // `is_reconcilable` check rather than before it. Taking it first let an inert record — a
    // `revoked` generation 3 for one session — consume the address and shadow a `live`
    // generation 2 for another, which is not merely a scheduling delay: the shadowed record was
    // `continue`d before it was indexed, so for up to a full TTL it was invisible to the status
    // projection, the drain report, and the turn guard, and no pass would ever attempt it.
    let mut seen_addresses: BTreeSet<(String, String)> = BTreeSet::new();
    // The position of the last entry this pass *considered* but did not attempt. Used only when the
    // pass attempts nothing at all, so the cursor still advances and the next pass moves on.
    let mut last_considered_position: Option<String> = None;
    for (intent, position) in page.loaded.into_iter().zip(page.loaded_positions) {
        last_considered_position = Some(position.clone());
        let key = IntentKey::from_intent(&intent);
        // Seed the index from the manifest header for *every* record this pass loaded, including
        // the ones it will not attempt, so the drain report and status are never blind to an entry
        // the budget skipped or the per-address dedupe shadowed.
        let mut entry = state.index_entry(&key).unwrap_or_default();
        let first_sight = entry.first_seen_ms.is_none();
        // A durable *state transition* on the record — a finalize, a producer-identity refresh, an
        // arming stamp, a re-attach — moves the generation. An evidence write does not (it rewrites
        // under the same generation on purpose), so this is precisely "the record materially
        // changed since the schedule that is parking it was computed".
        let generation_moved = !first_sight && entry.generation != intent.generation;
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
            if let Some(projected) = terminal_evidence_projection(&entry) {
                // Older builds persisted a one-hour terminal park while leaving the manifest
                // `Live`. Do not inherit that process-scoped delay: project it honestly and retry
                // once in this successor so `apply_outcome` can rewrite the evidence without the
                // stale park.
                entry.state = projected;
                entry.next_attempt_ms = None;
            } else if entry.consecutive_failures >= RECONCILE_QUARANTINE_AFTER {
                entry.state = IntentRecoveryState::Quarantined;
            }
        } else if generation_moved {
            // The ladder is earned by a *descriptor*, not by a binding. The canonical case is the
            // one this whole recovery path exists for: a bridge reloads, every pass fails
            // `producer_identity_mismatch` against the stale `(pid, start_time)`, and the
            // turn-boundary hook then re-records the live identity. Carrying the old ladder across
            // that transition means the repair is followed by a wait the repaired record did not
            // earn — and, past `RECONCILE_QUARANTINE_AFTER`, an hour of it. The failures describe a
            // producer that no longer exists, so they are dropped with it.
            entry.consecutive_failures = 0;
            entry.next_attempt_ms = None;
            entry.failure_code = None;
            entry.state = intent.state;
        }
        if entry.state == IntentRecoveryState::default() || !intent.is_reconcilable() {
            entry.state = intent.state;
        }
        state.index_upsert(key.clone(), entry.clone());

        if !intent.is_reconcilable() {
            report.inert += 1;
            continue;
        }
        if !seen_addresses.insert((intent.store_key.clone(), intent.address.clone())) {
            report.skipped += 1;
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
    // `RECONCILE_PER_INTENT_TIMEOUT`, and no wave is ever started that could outlive the pass
    // deadline — including the first.
    //
    // The first wave used to be exempt, on the reasoning that it is what guarantees a minimum of
    // `RECONCILE_MAX_CONCURRENCY` intents of progress per pass. That exemption is what made the
    // published bound untrue in the one case it mattered: when the phases *before* the first wave
    // (GC, discovery) had already consumed the deadline, the exempt wave then added a further
    // `RECONCILE_PER_INTENT_TIMEOUT` on top of it, so a pass could overrun its own tick and an
    // admin request could sit behind the overrun. The minimum-progress property is preserved by
    // budgeting maintenance instead: `RECONCILE_MAINTENANCE_BUDGET` leaves a reserve large enough
    // for one whole wave, so the check below only bites when maintenance itself overran, and in
    // that case the honest answer is a deadline-truncated pass whose cursors resume next tick.
    let mut cursor = 0usize;
    let mut last_attempted_position: Option<String> = None;
    while cursor < due.len() {
        // A wave may only start when the whole of it *plus* the bookkeeping that follows it still
        // fits. Gating on the per-intent timeout alone reserved nothing for spawning and joining
        // the wave, folding its outcomes (each of which may write evidence and, for a terminal one,
        // wait on the binding's admission guard), advancing the cursor, and logging the pass — so a
        // wave that started with exactly its own timeout left guaranteed an overrun.
        let remaining = pass_deadline.checked_duration_since(Instant::now());
        match remaining {
            Some(remaining)
                if remaining >= RECONCILE_PER_INTENT_TIMEOUT + RECONCILE_SCHEDULING_RESERVE => {}
            _ => {
                report.deadline_reached = true;
                report.skipped += due.len() - cursor;
                break;
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
                    apply_outcome(
                        &state,
                        &store,
                        &intent,
                        outcome,
                        &mut report,
                        pass_seq,
                        pass_deadline,
                    )
                    .await;
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
    //
    // Behind the blocking boundary like every other filesystem phase, and — because the cursor is
    // durable state a caller can observe — **started only while the durable-write reserve is still
    // intact**. Bounding the wait alone was enough to keep the pass punctual and not enough to keep
    // it honest: an abandoned cursor write launched at the deadline lands after the response, so a
    // caller that had been told the pass was truncated could still watch its cursor move. Below the
    // reserve the write is not attempted at all, the pass reports itself deadline-truncated, and the
    // next pass re-derives the same position. Nothing is ever cancelled mid-write.
    if let Some(position) = last_attempted_position.or(last_considered_position) {
        match store
            .advance_cursor_in_scope_reserved(
                scope.clone(),
                position,
                PassDeadline::at(pass_deadline),
                RECONCILE_DURABLE_WRITE_RESERVE,
            )
            .await
        {
            BoundedPhase::Completed(Ok(())) => {}
            BoundedPhase::Completed(Err(e)) => state.push_recent_error(
                "StationIntent",
                format!("persisting the reconcile scan cursor: {e}"),
            ),
            BoundedPhase::Overran => report.deadline_reached = true,
        }
    }

    report.duration_ms = started.elapsed().as_millis() as u64;
    report.index_as_of_ms = now_ms();
    {
        let mut index = state.intents.index.lock().unwrap();
        index.as_of_ms = report.index_as_of_ms;
    }
    log_event_within(
        &store,
        PassDeadline::at(pass_deadline),
        serde_json::json!({
            "event": "reconcile_pass",
            "pass_seq": pass_seq,
            "origin": origin.as_str(),
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
    )
    .await;
    publish(&state, &report);
    report
}

fn publish(state: &Arc<DaemonState>, report: &ReconcileReport) {
    let _ = state.intents.report_tx.send(report.clone());
}

/// The report an admin caller receives when its own outer bound fires before the pass answers.
///
/// Carries the current pass sequence so the caller can tell *which* pass it stopped waiting for,
/// and `ran: false` so it is never mistaken for a pass that ran and restored nothing. The pass
/// itself is still running and will publish its own report on the watch channel.
pub fn abandoned_pass_report(state: &Arc<DaemonState>, reason: &str) -> ReconcileReport {
    ReconcileReport::skipped_pass(state.intents.pass_seq.load(Ordering::SeqCst), reason)
}

/// Fold one outcome into the index, the durable evidence fields, and the pass report.
///
/// `pass_deadline` is the pass's own absolute deadline, not a fresh per-operation budget. Outcome
/// application runs *after* a wave has already spent most of the pass, and both of the things it
/// can block on — the linearized withdrawal's admission guard and the evidence CAS write — used to
/// start their own clocks at that point. A terminal outcome could therefore add a further
/// `RECONCILE_PER_INTENT_TIMEOUT` of admission wait on top of a wave that had just consumed the
/// whole budget, which is the same "a chain of per-phase timeouts bounds nothing" defect the
/// absolute pass deadline exists to remove.
async fn apply_outcome(
    state: &Arc<DaemonState>,
    store: &IntentStore,
    intent: &StationIntentV1,
    outcome: IntentOutcome,
    report: &mut ReconcileReport,
    pass_seq: u64,
    pass_deadline: Instant,
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
                // A durable tombstone (or an operator reset) outranks the local manifest: withdraw
                // it so the next pass does not even attempt it.
                //
                // Generation-conditional, and through the same linearized withdrawal every
                // explicit teardown uses. The pass decided "tombstoned" against the generation it
                // loaded, and a re-attach may have written a fresh `pending` record since —
                // withdrawing unconditionally would delete a record this decision knows nothing
                // about. `Superseded` is therefore a correct, silent no-op: the newer record gets
                // its own pass.
                //
                // Withdrawal bumps (or removes) the generation, so the evidence write below is
                // deliberately skipped: writing back the pre-withdrawal copy under the old
                // generation would either fail the CAS or, worse, resurrect `Live`.
                //
                // Bounded by what is left of the *pass*, never by a fresh admission timeout. A
                // withdrawal that cannot take the guard in the time the pass has left is deferred
                // to the next pass and reported as deadline-truncated: the record stays exactly as
                // the pass found it, the terminal projection is still published to the index, and
                // the next pass re-derives the same decision. Waiting instead is what turned a
                // 4-second pass into a 7-second one.
                match state
                    .withdraw_intent_at_generation_until(
                        &intent.store_key,
                        &intent.session_id,
                        &intent.address,
                        Some(intent.generation),
                        PassDeadline::at(pass_deadline),
                    )
                    .await
                {
                    Ok(_) => {}
                    Err(e) => {
                        report.deadline_reached |= Instant::now() >= pass_deadline;
                        state.push_recent_error("StationIntent", e)
                    }
                }
                entry.next_attempt_ms = None;
                state.index_upsert(key, entry);
                log_outcome_event(store, intent, &outcome, pass_seq, pass_deadline).await;
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
    let persisted_next_attempt_ms = if matches!(&outcome, IntentOutcome::Terminal { .. }) {
        // A terminal outcome may be process- or lifecycle-scoped. Park it in this daemon's index,
        // but never make a successor inherit the hour-long delay from a still-Live manifest.
        None
    } else {
        entry.next_attempt_ms
    };
    let evidence = IntentEvidence {
        last_attempt_ms: entry.last_attempt_ms,
        last_success_ms: entry.last_success_ms,
        attempts: entry.attempts,
        consecutive_failures: entry.consecutive_failures,
        failure_code: entry.failure_code.clone(),
        producer_verified_ms: entry.producer_verified_ms,
        next_attempt_ms: persisted_next_attempt_ms,
        recovery_latency_ms: entry.recovery_latency_ms,
    };
    if evidence_write_due(&intent.evidence, &evidence, now)
        || refreshed_watermark != intent.cc_watermark_ms
    {
        let mut updated = intent.clone();
        updated.evidence = evidence;
        updated.cc_watermark_ms = refreshed_watermark;
        match store
            .write_cas_reserved(
                intent.generation,
                updated,
                PassDeadline::at(pass_deadline),
                RECONCILE_DURABLE_WRITE_RESERVE,
            )
            .await
        {
            BoundedPhase::Completed(Ok(_)) => {}
            BoundedPhase::Completed(Err(e)) => state.push_recent_error(
                "StationIntent",
                format!("persisting intent evidence for {}: {e}", intent.address),
            ),
            // Either the write was never started (the durable-write reserve was already gone, so
            // nothing can land after the response) or it was started with the reserve intact and
            // has not returned yet. In both cases this pass publishes nothing derived from it and
            // reports a truncated tail rather than a failure it cannot substantiate; the write
            // itself is never cancelled, and its CAS is what protects a late one.
            BoundedPhase::Overran => report.deadline_reached = true,
        }
    }

    state.index_upsert(key, entry);
    log_outcome_event(store, intent, &outcome, pass_seq, pass_deadline).await;
}

/// Fold an inline anti-downgrade success into the cached projection without counting it as a
/// scheduled pass attempt. The caller has already run the full reconcile operation and only needs
/// status/drain consumers to observe the success immediately.
pub(crate) fn apply_inline_success_projection(
    state: &Arc<DaemonState>,
    intent: &StationIntentV1,
    outcome: &IntentOutcome,
) {
    if !outcome.is_success() {
        return;
    }
    let now = now_ms();
    let key = IntentKey::from_intent(intent);
    let mut entry = state.index_entry(&key).unwrap_or_default();
    entry.generation = intent.generation;
    entry.wake_on_cc = intent.wake_on_cc;
    entry.cc_watermark_ms = state
        .get_member(&intent.store_key, &intent.session_id, &intent.address)
        .and_then(|member| member.on_deliver_cc_after_ms)
        .or(intent.cc_watermark_ms);
    entry.state = outcome.projected_state();
    entry.failure_code = None;
    entry.consecutive_failures = 0;
    entry.next_attempt_ms = None;
    entry.last_success_ms = Some(now);
    entry.first_seen_ms.get_or_insert(now);
    if matches!(outcome, IntentOutcome::Restored) {
        entry.recovery_latency_ms = entry.first_seen_ms.map(|first| now.saturating_sub(first));
    }
    state.index_upsert(key, entry);
}

/// Recover the runtime projection encoded by a terminal evidence row from an older daemon.
///
/// Genuine failures always increment `consecutive_failures`; terminal outcomes do not. That lets a
/// successor distinguish a terminal park without expanding the persisted-state enum beyond
/// `pending | live | revoked`.
fn terminal_evidence_projection(entry: &IntentIndexEntry) -> Option<IntentRecoveryState> {
    if entry.consecutive_failures != 0 {
        return None;
    }
    let code = entry.failure_code.as_deref()?;
    Some(match code {
        "credential_insecure" | "credential_outside_root" => IntentRecoveryState::Insecure,
        "handler_kind_unregistered" | "handler_argv_invalid" => IntentRecoveryState::Incompatible,
        "legacy_producer" => IntentRecoveryState::LegacyProducer,
        "address_attended" => IntentRecoveryState::OwnershipConflict,
        "tombstoned" | "operator_reset" => IntentRecoveryState::Revoked,
        _ => IntentRecoveryState::Unverifiable,
    })
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

async fn log_outcome_event(
    store: &IntentStore,
    intent: &StationIntentV1,
    outcome: &IntentOutcome,
    pass_seq: u64,
    pass_deadline: Instant,
) {
    log_event_within(
        store,
        PassDeadline::at(pass_deadline),
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
    )
    .await;
}

/// Exponential backoff with +/- jitter, capped at `RECONCILE_BACKOFF_MAX`.
///
/// `consecutive_failures` is 1-based — it is the count *including* the failure being scheduled —
/// so the exponent is `failures - 1`. Without that, the first transient failure waited
/// `2 * RECONCILE_BACKOFF_INITIAL`, and the published ladder ("5 s → 5 min") was wrong at exactly
/// the rung that matters most: a bridge mid-reload, which is over within a tick or two.
pub fn backoff_delay(consecutive_failures: u32) -> Duration {
    let base = RECONCILE_BACKOFF_INITIAL.as_millis() as u64;
    let max = RECONCILE_BACKOFF_MAX.as_millis() as u64;
    let exponent = consecutive_failures.saturating_sub(1);
    let scaled = base.saturating_mul(1u64 << exponent.min(16)).min(max);
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

/// Append one NDJSON reconcile event, bounded by the pass deadline it was produced under.
///
/// Diagnostics, never authority — and that is what makes the bound below the right trade. The
/// append is three synchronous filesystem calls (a `metadata`, sometimes a `rename`, then an
/// `open`+`write`) against a path that may live on a network share or behind a filter driver, and
/// it used to run **inline on the pass's own task**. A pass that had just spent its whole budget
/// therefore did its rejection, outcome, and final-summary logging with no bound at all, on the one
/// task an admin caller was blocked on. "Best effort" described what happened to write *errors*, not
/// to write *latency*.
///
/// Two changes make it best-effort in the sense that matters:
///
/// * The work runs on the blocking pool, so a wedged log file stalls a pool thread rather than the
///   pass.
/// * It is only **started** while [`RECONCILE_EVENT_LOG_RESERVE`] of the deadline remains, and it is
///   joined inside that deadline.
///
/// **Evidence behavior, which is deliberate and load-bearing:** a pass that is at or past its
/// deadline *skips* its event-log appends entirely — including its own `reconcile_pass` summary
/// line. The log is therefore a record of what a pass had budget to narrate, not a complete audit of
/// every pass. The authoritative record of a truncated pass is the report the caller receives
/// (`deadline_reached`, or `ran: false` with a `skipped_reason`) and the durable evidence block on
/// each manifest; neither is ever elided to make a deadline. Reading a missing `reconcile_pass` line
/// as "no pass ran" is the one wrong inference, and it is why the summary line carries `origin` and
/// `duration_ms`: a gap in `pass_seq` between two logged lines means the passes in between were too
/// short of budget to write, not that they did not happen.
///
/// No secret, no credential path, and no raw argv is ever written here.
async fn log_event_within(
    store: &IntentStore,
    deadline: PassDeadline,
    mut event: serde_json::Value,
) {
    if let Some(map) = event.as_object_mut() {
        map.insert("ts_ms".to_string(), serde_json::json!(now_ms()));
        // Defense in depth: if a future field ever carried a secret, redact it rather than log it.
        for key in ["secret", "credential", "argv"] {
            if map.contains_key(key) {
                map.insert(key.to_string(), serde_json::json!(REDACTED_SECRET));
            }
        }
    }
    let Ok(mut line) = serde_json::to_string(&event) else {
        return;
    };
    line.push('\n');
    let path = store.root().join(RECONCILE_EVENT_LOG_FILE);
    let rotated = store.root().join(format!("{RECONCILE_EVENT_LOG_FILE}.1"));
    let _: BoundedPhase<()> =
        station_intent::run_blocking_reserved(deadline, RECONCILE_EVENT_LOG_RESERVE, move || {
            append_event_line(&path, &rotated, &line)
        })
        .await;
}

/// The synchronous half of [`log_event_within`], run only on the blocking pool.
fn append_event_line(path: &Path, rotated: &Path, line: &str) {
    if std::fs::metadata(path).is_ok_and(|m| m.len() > RECONCILE_EVENT_LOG_ROTATE_BYTES) {
        let _ = std::fs::remove_file(rotated);
        let _ = std::fs::rename(path, rotated);
    }
    use std::io::Write;
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
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
        // Bounded like every other sweep: it is off the accept path, but an unbounded startup GC
        // still delays the first reconcile pass by however long the filesystem takes, and the
        // sweep resumes from its persisted position on the maintenance cadence anyway.
        let deadline = PassDeadline::at(Instant::now() + RECONCILE_PASS_WORK_BUDGET);
        run_intent_gc(&state, deadline, deadline).await;
        reconcile_once(state, None).await;
    });
}

/// Wall-clock interval between maintenance GC runs. GC is O(scope) I/O, so it runs less often than
/// reconciliation itself: once a minute, plus once at startup.
pub const RECONCILE_GC_INTERVAL: Duration = Duration::from_secs(60);

/// Bounded intent GC: the only place an intent file is deleted, and the only mechanism that brings
/// an over-cap scope back under the cap.
///
/// The maintenance clock is only advanced by a sweep that actually **completed**. A sweep truncated
/// by its budget has examined a slice of the scope and persisted where it stopped, so letting it
/// consume the once-a-minute slot would leave the rest of the scope uncollected for a minute per
/// slice — on a scope large enough to truncate, that is indistinguishable from a leak. Re-running
/// it on the next pass instead is bounded work (`RECONCILE_GC_BUDGET`) that resumes where it
/// stopped.
/// `deadline` bounds the sweep itself (the maintenance slice); `log_deadline` bounds the removal
/// events it narrates afterwards. They are two different instants because a sweep that used its
/// whole maintenance budget has still done real, durable work, and dropping the record of which
/// files it deleted to save a maintenance millisecond is the wrong trade — the wider bound is the
/// pass deadline, which is what the append must not outlive.
async fn run_intent_gc(
    state: &Arc<DaemonState>,
    deadline: PassDeadline,
    log_deadline: PassDeadline,
) {
    let Some(store) = state.intent_store() else {
        return;
    };
    let identity = local_identity().ok();
    // Behind the blocking boundary: a sweep is a directory walk plus a `load`, a lock
    // acquisition, and an unlink per candidate, and the cooperative check between them cannot
    // bound the one that is currently blocked.
    let sweep = store
        .gc_bounded_within(
            now_ms(),
            identity.as_ref().map(|(host, _)| host.to_string()),
            identity.as_ref().map(|(_, boot)| boot.to_string()),
            deadline,
        )
        .await;
    match sweep {
        BoundedPhase::Completed(Ok(report)) => {
            if report.complete {
                state.intents.last_gc_ms.store(now_ms(), Ordering::Relaxed);
            }
            if !report.removed.is_empty() {
                state.index_prune_removed(&report.removed);
                for (id, reason) in &report.reasons {
                    log_event_within(
                        &store,
                        log_deadline,
                        serde_json::json!({
                            "event": "intent_gc_removed",
                            "intent_id": id.to_string(),
                            "reason": reason,
                        }),
                    )
                    .await;
                }
            }
        }
        BoundedPhase::Completed(Err(e)) => {
            // A failed sweep still consumed its slot: retrying an erroring scope every tick
            // would spend the maintenance budget on the same failure.
            state.intents.last_gc_ms.store(now_ms(), Ordering::Relaxed);
            state.push_recent_error("StationIntent", format!("intent GC: {e}"))
        }
        // The sweep is still running and will finish its own deletions and cursor write; this
        // pass simply stops waiting. The maintenance clock is deliberately *not* advanced —
        // an unobserved sweep must not consume the once-a-minute slot — and nothing is pruned
        // from the index, because a report this pass never received cannot be published from.
        BoundedPhase::Overran => {}
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
        // The published bound is the pass's work budget plus the tail reserved for answering, and
        // nothing else. A pass that budgeted its phases to the whole four seconds published a
        // number it could only meet by accident: the join, the encode, and the socket write all
        // happen after the last phase returns.
        assert_eq!(
            RECONCILE_PASS_WORK_BUDGET + RECONCILE_RESPONSE_RESERVE,
            RECONCILE_PASS_DEADLINE
        );
        // No wave is ever started that could outlive the budget the phases actually have.
        assert!(RECONCILE_PER_INTENT_TIMEOUT <= RECONCILE_PASS_WORK_BUDGET);
        // The probe leaves room for local validation and the backend claim inside a per-intent
        // budget.
        assert!(BRIDGE_PROBE_TIMEOUT < RECONCILE_PER_INTENT_TIMEOUT);
        // DeferredLease retries at most once per tick, with no exponential growth.
        assert_eq!(RECONCILE_DEFERRED_LEASE_RETRY, RECONCILE_INTERVAL);
        // Every pass makes at least `RECONCILE_MAX_CONCURRENCY` intents of progress, which is only
        // meaningful if a wave fits inside the budget.
        assert!(RECONCILE_MAX_CONCURRENCY <= RECONCILE_PASS_BUDGET.max(1));
        // The first wave is no longer exempt from the deadline, so guaranteed minimum progress is
        // now a property of the *maintenance* budget: whatever GC and discovery are allowed to
        // consume must still leave a whole per-intent timeout inside the pass work budget — plus
        // the pass's own scheduling and outcome-application overhead, which is reserved rather than
        // assumed free.
        assert!(
            RECONCILE_MAINTENANCE_BUDGET
                + RECONCILE_BLOCKING_GRACE
                + RECONCILE_PER_INTENT_TIMEOUT
                + RECONCILE_SCHEDULING_RESERVE
                <= RECONCILE_PASS_WORK_BUDGET
        );
        // The gate a wave is admitted through is exactly that sum, so a pass whose maintenance ran
        // right up to its budget — grace included, because a truncated scan is allowed to return
        // late rather than be discarded — still starts a first wave. Without this the reserve would
        // buy punctuality at the cost of the minimum-progress guarantee.
        assert!(
            RECONCILE_PASS_WORK_BUDGET - RECONCILE_MAINTENANCE_BUDGET - RECONCILE_BLOCKING_GRACE
                >= RECONCILE_PER_INTENT_TIMEOUT + RECONCILE_SCHEDULING_RESERVE
        );
        // GC is one phase of maintenance, never the whole of it: discovery has to fit too.
        assert!(RECONCILE_GC_BUDGET < RECONCILE_MAINTENANCE_BUDGET);
        // The two start-gates are payable out of the reserve the pass keeps for its own
        // bookkeeping. If a durable write cost more than the whole reserve, a pass that spent
        // exactly its budget could never persist anything and the evidence block would stop
        // advancing under load — the opposite of what the reserve is for.
        assert!(RECONCILE_DURABLE_WRITE_RESERVE < RECONCILE_SCHEDULING_RESERVE);
        assert!(RECONCILE_EVENT_LOG_RESERVE < RECONCILE_DURABLE_WRITE_RESERVE);
        // One published end-to-end number, not three. The admin bound is now the *same deadline*
        // rather than a second clock of the same length, and the client ceiling sits inside it.
        assert!(RECONCILE_ADMIN_DEADLINE <= RECONCILE_PASS_DEADLINE);
        assert!(RECONCILE_REQUEST_DEADLINE <= RECONCILE_ADMIN_DEADLINE);
        assert_eq!(RECONCILE_PASS_DEADLINE, Duration::from_secs(4));
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
        // `consecutive_failures` counts the failure being scheduled, so the *first* transient
        // failure must wait `RECONCILE_BACKOFF_INITIAL`, not twice it. The exponent used to be the
        // count itself, which made the published "5 s → 5 min" ladder wrong at exactly the rung
        // that matters most: a bridge mid-reload, which is over within a tick or two.
        let band = |ms: u64| {
            let jitter = ms * RECONCILE_BACKOFF_JITTER_PCT / 100;
            (ms - jitter, ms + jitter)
        };
        for (failures, expected) in [(1u32, 5_000u64), (2, 10_000), (3, 20_000), (4, 40_000)] {
            let (low, high) = band(expected);
            let delay = backoff_delay(failures).as_millis() as u64;
            assert!(
                delay >= low && delay <= high,
                "failure {failures} must schedule around {expected} ms, got {delay}"
            );
        }
        assert_eq!(
            RECONCILE_BACKOFF_INITIAL.as_millis() as u64,
            5_000,
            "the documented ladder starts at the constant, and the first rung must use it"
        );
        // A zero count (the `DeferredPullWaiter` cadence, which never advances the counter) must
        // not underflow the exponent.
        let (low, high) = band(5_000);
        let zero = backoff_delay(0).as_millis() as u64;
        assert!(zero >= low && zero <= high);

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
