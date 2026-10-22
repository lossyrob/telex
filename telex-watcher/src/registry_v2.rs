use crate::evidence::*;
use crate::ownership::Ownership;
use crate::protocol_v2::{hash_value, Registration};
use crate::recovery::{classify, Resolution};
use anyhow::{anyhow, bail, Context, Result};
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::Path;
use uuid::Uuid;

pub(crate) const MAX_GRANT: u32 = 100;
pub(crate) const MAX_GRANT_SECONDS: u64 = 86400;
pub(crate) const MAX_QUERY: u32 = 1000;

pub(crate) fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

pub(crate) fn timestamp(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .expect("supported system clock timestamp")
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Refusal {
    InvalidRegistration,
    InvalidState,
    NoPendingOperation,
    RecoveryUnavailable,
    NotFound,
    IdAlreadyAllocated,
    PendingOperation,
    InvalidLifecycle,
    ImmutableRegistration,
    UnsupportedRegistrationVersion,
    BlockedReasonUncorrected,
    InvalidGrant,
    WrongRecoveryProof,
    AttemptChanged,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Mutation {
    pub watch: Option<Watch>,
    pub refusal: Option<Refusal>,
    pub diagnostic: Option<String>,
    pub recovery: Option<SendEvidence>,
}

#[derive(Debug, Clone)]
pub enum Change {
    Pause,
    Resume,
    Update {
        registration: Box<Registration>,
        state: Option<Value>,
    },
    GrantReconciliation {
        attempts: u32,
        seconds: u64,
    },
    GrantSendRetry {
        attempts: u32,
        seconds: u64,
    },
    CloseNotAccepted,
    Remove,
}

pub(crate) struct Registry {
    connection: Connection,
    _ownership: Ownership,
    pub responsibility: String,
    warning_rows: u64,
    warning_bytes: u64,
    #[cfg(test)]
    pub fail_next_accept_commit: bool,
}

impl Registry {
    pub fn open(path: &Path, warning_rows: u64, warning_bytes: u64) -> Result<Self> {
        if warning_rows == 0 || warning_bytes == 0 {
            bail!("retention warning thresholds must be positive");
        }
        let ownership = Ownership::acquire(path)?;
        let connection = Connection::open(&ownership.path)?;
        connection.busy_timeout(std::time::Duration::from_secs(5))?;
        let has_version: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='watcher_v2_meta')",
            [], |row| row.get(0))?;
        if has_version {
            let version: u32 = connection.query_row(
                "SELECT version FROM watcher_v2_meta WHERE singleton=1",
                [],
                |row| row.get(0),
            )?;
            anyhow::ensure!(version == 2, "unsupported-registry-version: {version}");
        }
        connection.execute_batch(
            "PRAGMA foreign_keys=ON;
             PRAGMA synchronous=FULL;
             CREATE TABLE IF NOT EXISTS watcher_v2_meta (
               singleton INTEGER PRIMARY KEY CHECK(singleton=1),
               version INTEGER NOT NULL, revision INTEGER NOT NULL, responsibility TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS watcher_v2_watches (
               id TEXT PRIMARY KEY, data TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS watcher_v2_revisions (
               watch_id TEXT NOT NULL, revision INTEGER NOT NULL, data TEXT NOT NULL,
               PRIMARY KEY(watch_id,revision));
             CREATE TABLE IF NOT EXISTS watcher_v2_attempts (
               id TEXT PRIMARY KEY, watch_id TEXT NOT NULL, started INTEGER NOT NULL, data TEXT NOT NULL);
             CREATE INDEX IF NOT EXISTS watcher_v2_attempt_watch ON watcher_v2_attempts(watch_id,started);
             CREATE TABLE IF NOT EXISTS watcher_v2_operations (
               id TEXT PRIMARY KEY, watch_id TEXT NOT NULL, sequence INTEGER NOT NULL,
               event_id TEXT NOT NULL UNIQUE, data TEXT NOT NULL,
               UNIQUE(watch_id,sequence));
             CREATE TABLE IF NOT EXISTS watcher_v2_audits (
               id INTEGER PRIMARY KEY, watch_id TEXT, at_ms INTEGER NOT NULL,
               action TEXT NOT NULL, data TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS watcher_v2_operation_evidence (
               id INTEGER PRIMARY KEY, operation_id TEXT NOT NULL, watch_id TEXT NOT NULL,
               at_ms INTEGER NOT NULL, data TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS watcher_v2_runtimes (
               id TEXT PRIMARY KEY, data TEXT NOT NULL);",
        )?;
        connection.execute(
            "INSERT OR IGNORE INTO watcher_v2_meta VALUES(1,2,0,?1)",
            [format!("watcher:{}", Uuid::new_v4())],
        )?;
        let (version, responsibility): (u32, String) = connection.query_row(
            "SELECT version,responsibility FROM watcher_v2_meta WHERE singleton=1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        if version != 2 {
            bail!("unsupported-registry-version: {version}");
        }
        let mut registry = Self {
            connection,
            _ownership: ownership,
            responsibility,
            warning_rows,
            warning_bytes,
            #[cfg(test)]
            fail_next_accept_commit: false,
        };
        registry.isolate_v1()?;
        Ok(registry)
    }

    fn isolate_v1(&mut self) -> Result<()> {
        let legacy: bool = self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='watches')",
            [],
            |row| row.get(0),
        )?;
        if !legacy {
            return Ok(());
        }
        let rows = {
            let mut statement = self
                .connection
                .prepare("SELECT id,state_json FROM watches")?;
            let rows = statement
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            rows
        };
        for (id, state) in rows {
            if self.get(&id)?.is_some() {
                continue;
            }
            let mut watch = new_watch(id, None, serde_json::from_str(&state)?, now_ms());
            watch.schema_version = 1;
            watch.block(BlockedReason::UnsupportedRegistrationVersion);
            let tx = self.connection.transaction()?;
            save_watch(&tx, &watch)?;
            audit(&tx, Some(&watch.id), "isolate-v1", &json!({"version": 1}))?;
            bump(&tx)?;
            tx.commit()?;
        }
        Ok(())
    }

    pub fn revision(&self) -> Result<u64> {
        Ok(self.connection.query_row(
            "SELECT revision FROM watcher_v2_meta WHERE singleton=1",
            [],
            |row| row.get(0),
        )?)
    }

    pub fn containment_directory(&self) -> std::path::PathBuf {
        self._ownership.path.with_extension("containment")
    }

    pub fn get(&self, id: &str) -> Result<Option<Watch>> {
        get_watch(&self.connection, id)
    }

    pub fn list(&self) -> Result<Vec<Watch>> {
        read_many(
            &self.connection,
            "SELECT data FROM watcher_v2_watches ORDER BY id",
            [],
        )
    }

    pub fn register(&mut self, registration: Registration, now: i64) -> Result<Mutation> {
        let tx = self.connection.transaction()?;
        let id = &registration.id;
        if let Some(watch) = get_watch(&tx, id)? {
            return mutation_refused(tx, Some(watch), id, "register", Refusal::IdAlreadyAllocated);
        }
        let watch = new_watch(
            id.clone(),
            Some(registration.clone()),
            registration.initial_state.clone(),
            now,
        );
        save_watch(&tx, &watch)?;
        revision(&tx, &watch)?;
        audit(&tx, Some(id), "register", &json!({"revision": 1}))?;
        bump(&tx)?;
        tx.commit()?;
        Ok(Mutation {
            watch: Some(watch),
            refusal: None,
            diagnostic: None,
            recovery: None,
        })
    }

    pub fn change(&mut self, id: &str, change: Change, now: i64) -> Result<Mutation> {
        let tx = self.connection.transaction()?;
        let Some(mut watch) = get_watch(&tx, id)? else {
            return mutation_refused(tx, None, id, "change", Refusal::NotFound);
        };
        let mut pending = watch
            .pending_operation_id
            .as_deref()
            .map(|id| get_operation(&tx, id))
            .transpose()?
            .flatten();
        let action = match &change {
            Change::Pause => "pause",
            Change::Resume => "resume",
            Change::Update { .. } => "update",
            Change::GrantReconciliation { .. } => "grant-reconciliation",
            Change::GrantSendRetry { .. } => "grant-send-retry",
            Change::CloseNotAccepted => "close-not-accepted",
            Change::Remove => "remove",
        };
        let refusal = check_change(&watch, pending.as_ref(), &change);
        if let Some(reason) = refusal {
            return mutation_refused(tx, Some(watch), id, action, reason);
        }
        let previous_state_hash = hash_value(&watch.state)?;
        if watch.running_attempt_id.is_some()
            && matches!(
                &change,
                Change::Pause | Change::Update { .. } | Change::Remove
            )
        {
            watch.running_cancelled = true;
        }
        match change {
            Change::Pause => {
                watch.lifecycle = Lifecycle::Paused;
                watch.eligibility = Eligibility::Inactive;
                watch.health = Health::Inactive;
                watch.blocked_reason = None;
            }
            Change::Resume => {
                watch.lifecycle = Lifecycle::Active;
                watch.eligibility = Eligibility::Eligible;
                watch.health = Health::Ready;
                watch.blocked_reason = None;
                watch.activated_at = timestamp(now);
                watch.next_due_ms = now;
                watch.next_attempt_ms = now;
                watch.consecutive_failures = 0;
                watch.diagnostic_category = None;
            }
            Change::Update {
                registration,
                state,
            } => {
                watch.registration = Some(*registration);
                watch.registration_revision = watch
                    .registration_revision
                    .checked_add(1)
                    .ok_or_else(|| anyhow!("registration revision exhausted"))?;
                if let Some(state) = state {
                    watch.state = state;
                }
                revision(&tx, &watch)?;
            }
            Change::GrantReconciliation { attempts, seconds } => {
                let operation = pending.as_mut().expect("checked pending operation");
                operation.reconciliation_budget.remaining = attempts;
                operation.reconciliation_budget.deadline_ms = deadline(now, seconds)?;
                operation.status = OperationStatus::QueryRequired;
                operation.query_only = true;
                operation.next_recovery_ms = now;
                watch.lifecycle = Lifecycle::Active;
                watch.pending();
            }
            Change::GrantSendRetry { attempts, seconds } => {
                let operation = pending.as_mut().expect("checked pending operation");
                operation.send_budget.remaining = attempts;
                operation.send_budget.deadline_ms = deadline(now, seconds)?;
                operation.status = OperationStatus::RetryPermitted;
                operation.query_only = false;
                operation.next_recovery_ms = now;
                watch.lifecycle = Lifecycle::Active;
                watch.pending();
            }
            Change::CloseNotAccepted => {
                let operation = pending.as_mut().expect("checked pending operation");
                operation.status = OperationStatus::ClosedNotAccepted;
                watch.pending_operation_id = None;
                watch.blocked_reason = None;
                watch.health = Health::Inactive;
                watch.diagnostic_category = None;
            }
            Change::Remove => {
                watch.lifecycle = Lifecycle::Removed;
                watch.eligibility = Eligibility::Inactive;
                watch.health = Health::Inactive;
                watch.blocked_reason = None;
            }
        }
        if let Some(operation) = pending {
            save_operation(&tx, &operation)?;
        }
        save_watch(&tx, &watch)?;
        audit(
            &tx,
            Some(id),
            action,
            &json!({
                "revision": watch.registration_revision,
                "oldStateHash": previous_state_hash, "newStateHash": hash_value(&watch.state)?
            }),
        )?;
        bump(&tx)?;
        tx.commit()?;
        Ok(Mutation {
            watch: Some(watch),
            refusal: None,
            diagnostic: None,
            recovery: None,
        })
    }

    pub fn refuse(
        &mut self,
        id: Option<&str>,
        action: &str,
        reason: Refusal,
        diagnostic: &str,
    ) -> Result<Mutation> {
        let tx = self.connection.transaction()?;
        let watch = id.map(|id| get_watch(&tx, id)).transpose()?.flatten();
        let diagnostic: String = diagnostic.chars().take(4096).collect();
        audit(
            &tx,
            id,
            action,
            &json!({"refusal":reason,"diagnostic":diagnostic}),
        )?;
        tx.commit()?;
        Ok(Mutation {
            watch,
            refusal: Some(reason),
            diagnostic: Some(diagnostic),
            recovery: None,
        })
    }

    pub fn recovery_result(&self, id: &str) -> Result<Mutation> {
        let watch = get_watch(&self.connection, id)?;
        let latest = self.events(id, 1)?.into_iter().next();
        Ok(Mutation {
            watch,
            refusal: None,
            diagnostic: None,
            recovery: latest.and_then(|operation| operation.last_evidence),
        })
    }

    pub fn bind_store(&mut self, id: &str, store: &str) -> Result<bool> {
        let tx = self.connection.transaction()?;
        let mut watch = require_watch(&tx, id)?;
        let same = watch
            .logical_store_id
            .as_deref()
            .is_none_or(|old| old == store);
        if same {
            watch.logical_store_id = Some(store.to_string());
        } else {
            watch.block(BlockedReason::LogicalStoreMismatch);
        }
        save_watch(&tx, &watch)?;
        tx.commit()?;
        Ok(same)
    }

    pub fn coalesce(&mut self, now: i64) -> Result<()> {
        let watches = self.list()?;
        let tx = self.connection.transaction()?;
        for mut watch in watches {
            if watch.running_attempt_id.is_some() && now >= watch.next_due_ms {
                let period = cadence_ms(&watch);
                let missed = ((now - watch.next_due_ms) / period + 1) as u64;
                watch.suppressed_due_count = watch.suppressed_due_count.saturating_add(missed);
                watch.coalesced_due = true;
                watch.next_due_ms = now.saturating_add(period);
                save_watch(&tx, &watch)?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub fn begin_attempt(&mut self, id: &str, runtime: &str, now: i64) -> Result<(Watch, Attempt)> {
        let tx = self.connection.transaction()?;
        let mut watch = require_watch(&tx, id)?;
        if watch.lifecycle != Lifecycle::Active
            || watch.eligibility != Eligibility::Eligible
            || watch.running_attempt_id.is_some()
            || watch.pending_operation_id.is_some()
        {
            bail!("attempt-ineligible");
        }
        let registration = watch
            .registration
            .as_ref()
            .context("unsupported registration")?;
        let attempt = Attempt {
            schema_version: 2,
            id: format!("attempt-{}", Uuid::new_v4()),
            watch_id: id.to_owned(),
            runtime_id: runtime.to_owned(),
            registration_revision: watch.registration_revision,
            started_at: timestamp(now),
            started_at_ms: now,
            finished_at_ms: None,
            command: registration.command.clone(),
            working_directory: registration.working_directory.clone(),
            prior_state_hash: hash_value(&watch.state)?,
            outcome: AttemptOutcome::Running,
            diagnostic_category: None,
            diagnostic: None,
            containment: None,
            state_committed: false,
        };
        watch.running_attempt_id = Some(attempt.id.clone());
        watch.running_cancelled = false;
        watch.last_attempt_id = Some(attempt.id.clone());
        watch.coalesced_due = false;
        watch.next_due_ms = now.saturating_add(cadence_ms(&watch));
        save_attempt(&tx, &attempt)?;
        save_watch(&tx, &watch)?;
        tx.commit()?;
        Ok((watch, attempt))
    }

    pub fn containment(&mut self, attempt_id: &str, containment: Value) -> Result<()> {
        let tx = self.connection.transaction()?;
        let mut attempt = require_attempt(&tx, attempt_id)?;
        if attempt.outcome != AttemptOutcome::Running {
            bail!("attempt no longer running");
        }
        attempt.containment = Some(containment);
        save_attempt(&tx, &attempt)?;
        tx.commit()?;
        Ok(())
    }

    pub fn attempt_is_current(&self, attempt: &Attempt) -> Result<bool> {
        let watch = require_watch(&self.connection, &attempt.watch_id)?;
        Ok(watch.running_attempt_id.as_deref() == Some(&attempt.id)
            && !watch.running_cancelled
            && watch.registration_revision == attempt.registration_revision
            && watch.lifecycle == Lifecycle::Active
            && watch.pending_operation_id.is_none()
            && hash_value(&watch.state)? == attempt.prior_state_hash)
    }

    pub fn finish_direct(
        &mut self,
        attempt_id: &str,
        state: Value,
        terminal: bool,
        now: i64,
    ) -> Result<()> {
        let tx = self.connection.transaction()?;
        let mut attempt = require_attempt(&tx, attempt_id)?;
        let mut watch = require_watch(&tx, &attempt.watch_id)?;
        ensure_current(&watch, &attempt)?;
        watch.state = state;
        success(&mut watch, &attempt.started_at, terminal);
        finish(&mut watch, &mut attempt, now);
        attempt.outcome = if terminal {
            AttemptOutcome::Terminal
        } else {
            AttemptOutcome::Idle
        };
        attempt.state_committed = true;
        save_attempt(&tx, &attempt)?;
        save_watch(&tx, &watch)?;
        tx.commit()?;
        Ok(())
    }

    pub fn fail_attempt(
        &mut self,
        attempt_id: &str,
        category: DiagnosticCategory,
        diagnostic: &str,
        now: i64,
    ) -> Result<()> {
        let tx = self.connection.transaction()?;
        let mut attempt = require_attempt(&tx, attempt_id)?;
        let mut watch = require_watch(&tx, &attempt.watch_id)?;
        if attempt.outcome != AttemptOutcome::Running {
            bail!("attempt already completed");
        }
        finish(&mut watch, &mut attempt, now);
        attempt.outcome = AttemptOutcome::Failed;
        attempt.diagnostic_category = Some(category);
        attempt.diagnostic = Some(diagnostic.chars().take(4096).collect());
        if watch.lifecycle == Lifecycle::Active {
            watch.health = Health::Degraded;
            watch.diagnostic_category = Some(category);
            watch.consecutive_failures = watch.consecutive_failures.saturating_add(1);
            watch.next_attempt_ms = now.saturating_add(backoff_ms(watch.consecutive_failures));
        }
        save_attempt(&tx, &attempt)?;
        save_watch(&tx, &watch)?;
        tx.commit()?;
        Ok(())
    }

    pub fn stage(&mut self, operation: &Operation, now: i64) -> Result<()> {
        let tx = self.connection.transaction()?;
        let mut watch = require_watch(&tx, &operation.watch_id)?;
        let mut attempt = require_attempt(&tx, &operation.attempt_id)?;
        ensure_current(&watch, &attempt)?;
        if watch.last_allocated_event_sequence.checked_add(1) != Some(operation.event_sequence)
            || watch.state != operation.prior_state
            || operation.registration_revision != watch.registration_revision
            || watch.logical_store_id.as_deref() != Some(&operation.recovery.logical_store_id.0)
        {
            bail!("staging identity changed");
        }
        watch.last_allocated_event_sequence = operation.event_sequence;
        watch.pending_operation_id = Some(operation.operation_id.clone());
        watch.pending();
        finish(&mut watch, &mut attempt, now);
        attempt.outcome = AttemptOutcome::Pending;
        tx.execute(
            "INSERT INTO watcher_v2_operations(id,watch_id,sequence,event_id,data) VALUES(?1,?2,?3,?4,?5)",
            params![operation.operation_id,operation.watch_id,operation.event_sequence,
                operation.event_id,serde_json::to_string(operation)?],
        )?;
        save_watch(&tx, &watch)?;
        save_attempt(&tx, &attempt)?;
        audit(
            &tx,
            Some(&watch.id),
            "stage",
            &json!({
                "operationId": operation.operation_id, "eventSequence": operation.event_sequence
            }),
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn operation(&self, id: &str) -> Result<Option<Operation>> {
        get_operation(&self.connection, id)
    }

    pub fn attempts(&self, id: &str, limit: u32) -> Result<Vec<Attempt>> {
        read_many(
            &self.connection,
            "SELECT data FROM watcher_v2_attempts WHERE watch_id=?1 ORDER BY started DESC LIMIT ?2",
            params![id, limit.min(MAX_QUERY)],
        )
    }

    pub fn events(&self, id: &str, limit: u32) -> Result<Vec<Operation>> {
        read_many(&self.connection,
            "SELECT data FROM watcher_v2_operations WHERE watch_id=?1 ORDER BY sequence DESC LIMIT ?2",
            params![id, limit.min(MAX_QUERY)])
    }

    pub fn reserve_recovery(
        &mut self,
        id: &str,
        send: bool,
        manual: bool,
        now: i64,
    ) -> Result<bool> {
        let tx = self.connection.transaction()?;
        let mut operation = get_operation(&tx, id)?.context("pending operation missing")?;
        let mut watch = require_watch(&tx, &operation.watch_id)?;
        if watch.pending_operation_id.as_deref() != Some(id) {
            return Ok(false);
        }

        if !manual && (watch.lifecycle != Lifecycle::Active || now < operation.next_recovery_ms) {
            return Ok(false);
        }
        if send
            && !matches!(
                operation.status,
                OperationStatus::Staged | OperationStatus::RetryPermitted
            )
        {
            return Ok(false);
        }
        if send && watch.lifecycle != Lifecycle::Active {
            return Ok(false);
        }
        let budget = if send {
            &mut operation.send_budget
        } else {
            &mut operation.reconciliation_budget
        };
        if !manual && (budget.remaining == 0 || now >= budget.deadline_ms) {
            watch.block(if send {
                BlockedReason::SendRetryExhausted
            } else {
                BlockedReason::ReceiptOutcomeUnresolved
            });
            save_watch(&tx, &watch)?;
            tx.commit()?;
            return Ok(false);
        }
        if !manual {
            budget.remaining -= 1;
        }
        budget.used = budget.used.saturating_add(1);
        operation.next_recovery_ms = now.saturating_add(backoff_ms(budget.used));
        // Persist before polling a send future. A crash must cause a query, not
        // an inference that a staged or previously rejected operation was unsent.
        if send {
            operation.status = OperationStatus::QueryRequired;
        }
        save_operation(&tx, &operation)?;
        audit(
            &tx,
            Some(&watch.id),
            if send { "send-intent" } else { "query-intent" },
            &json!({"operationId": id, "manual": manual}),
        )?;
        tx.commit()?;
        Ok(true)
    }

    pub fn expire_recovery(&mut self, id: &str, now: i64) -> Result<bool> {
        let tx = self.connection.transaction()?;
        let operation = get_operation(&tx, id)?.context("operation missing")?;
        let mut watch = require_watch(&tx, &operation.watch_id)?;
        if watch.lifecycle != Lifecycle::Active || watch.pending_operation_id.as_deref() != Some(id)
        {
            return Ok(false);
        }
        let sending = matches!(
            operation.status,
            OperationStatus::Staged | OperationStatus::RetryPermitted
        );
        let budget = if sending {
            &operation.send_budget
        } else {
            &operation.reconciliation_budget
        };
        if budget.remaining == 0 || now >= budget.deadline_ms {
            let reason = if sending {
                BlockedReason::SendRetryExhausted
            } else {
                BlockedReason::ReceiptOutcomeUnresolved
            };
            watch.block(reason);
            save_watch(&tx, &watch)?;
            audit(
                &tx,
                Some(&watch.id),
                "budget-exhausted",
                &json!({"operationId": id, "reason": reason}),
            )?;
            tx.commit()?;
            return Ok(false);
        }
        Ok(true)
    }

    pub fn evidence(&mut self, id: &str, evidence: SendEvidence, now: i64) -> Result<()> {
        let tx = self.connection.transaction()?;
        let mut operation = get_operation(&tx, id)?.context("operation missing")?;
        let mut watch = require_watch(&tx, &operation.watch_id)?;
        if watch.pending_operation_id.as_deref() != Some(id) {
            bail!("operation is not pending");
        }
        let resolution = classify(&operation, &evidence);
        let observation = ReceiptEvidence {
            schema_version: 2,
            operation_id: id.to_owned(),
            observed_at: timestamp(now),
            evidence: evidence.clone(),
        };
        tx.execute(
            "INSERT INTO watcher_v2_operation_evidence(operation_id,watch_id,at_ms,data) VALUES(?1,?2,?3,?4)",
            params![id,operation.watch_id,now,serde_json::to_string(&observation)?],
        )?;
        operation.last_evidence = Some(evidence);
        operation.last_evidence_at = Some(timestamp(now));
        let removed = watch.lifecycle == Lifecycle::Removed;
        let blocked = watch.lifecycle == Lifecycle::Paused;
        match resolution {
            Resolution::Accepted(receipt) => {
                #[cfg(test)]
                if self.fail_next_accept_commit {
                    self.fail_next_accept_commit = false;
                    bail!("injected acceptance commit failure");
                }
                operation.status = OperationStatus::Accepted;
                operation.receipt = Some(*receipt);
                watch.state = operation.proposed_state.clone();
                watch.last_committed_event_sequence = operation.event_sequence;
                watch.pending_operation_id = None;
                watch.last_event_at = Some(timestamp(now));
                update_recurrence(&mut watch, &operation, now);
                success(&mut watch, &operation.attempt_now, operation.terminal);
                watch.next_attempt_ms = now;
                let mut attempt = require_attempt(&tx, &operation.attempt_id)?;
                attempt.outcome = AttemptOutcome::Accepted;
                attempt.state_committed = true;
                save_attempt(&tx, &attempt)?;
            }
            Resolution::Retry => {
                operation.status = OperationStatus::RetryPermitted;
                if removed {
                    operation.status = OperationStatus::RemovedNotRecorded;
                    watch.pending_operation_id = None;
                } else if blocked
                    || operation.query_only
                    || operation.send_budget.remaining == 0
                    || now >= operation.send_budget.deadline_ms
                {
                    watch.block(BlockedReason::SendRetryExhausted);
                } else {
                    watch.pending();
                }
            }
            Resolution::PermanentRejection => {
                if removed {
                    operation.status = OperationStatus::RemovedRejected;
                    watch.pending_operation_id = None;
                } else {
                    watch.block(BlockedReason::SendPermanentlyRejected);
                }
            }
            Resolution::Indeterminate => {
                operation.status = OperationStatus::QueryRequired;
                if !blocked && !removed {
                    if operation.reconciliation_budget.remaining == 0
                        || now >= operation.reconciliation_budget.deadline_ms
                    {
                        watch.block(BlockedReason::ReceiptOutcomeUnresolved);
                    } else {
                        watch.pending();
                    }
                }
            }
            Resolution::Conflict(reason) => watch.block(reason),
        }
        save_operation(&tx, &operation)?;
        save_watch(&tx, &watch)?;
        audit(
            &tx,
            Some(&watch.id),
            "receipt-evidence",
            &json!({"operationId": id, "status": operation.status, "blockedReason": watch.blocked_reason}),
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn start_runtime(
        &mut self,
        runtime_id: &str,
        now: i64,
        mut containment_ended: impl FnMut(Option<&Value>) -> bool,
    ) -> Result<RuntimeHealth> {
        let mut runtimes: Vec<RuntimeHealth> =
            read_many(&self.connection, "SELECT data FROM watcher_v2_runtimes", [])?;
        let watches = self.list()?;
        let tx = self.connection.transaction()?;
        for old in &mut runtimes {
            if old.status == RuntimeStatus::Running {
                old.status = RuntimeStatus::Interrupted;
                old.production_ready = false;
                save_runtime(&tx, old)?;
            }
        }
        for mut watch in watches {
            if let Some(id) = &watch.running_attempt_id {
                let mut attempt = require_attempt(&tx, id)?;
                let ended = containment_ended(attempt.containment.as_ref());
                attempt.outcome = AttemptOutcome::Interrupted;
                attempt.diagnostic_category = Some(DiagnosticCategory::RuntimeInterrupted);
                finish(&mut watch, &mut attempt, now);
                if !ended {
                    watch.block(BlockedReason::OrphanContainmentUnproven);
                } else if watch.lifecycle == Lifecycle::Active {
                    watch.health = Health::Degraded;
                    watch.diagnostic_category = Some(DiagnosticCategory::RuntimeInterrupted);
                }
                save_attempt(&tx, &attempt)?;
            }
            if watch.lifecycle == Lifecycle::Active {
                watch.next_due_ms = now;
                watch.next_attempt_ms = now;
                watch.coalesced_due = false;
                if let Some(id) = &watch.pending_operation_id {
                    let mut op =
                        get_operation(&tx, id)?.context("pending operation missing at restart")?;
                    op.status = OperationStatus::QueryRequired;
                    op.next_recovery_ms = now;
                    save_operation(&tx, &op)?;
                    watch.pending();
                }
            }
            save_watch(&tx, &watch)?;
        }
        tx.commit()?;
        let health = RuntimeHealth {
            schema_version: 2,
            runtime_id: runtime_id.to_owned(),
            pid: std::process::id(),
            started_at: timestamp(now),
            heartbeat_at: timestamp(now),
            status: RuntimeStatus::Running,
            production_ready: false,
            registry_revision: self.revision()?,
            restart_reconciled: true,
            senders: Vec::new(),
            retention: self.retention(None)?,
            diagnostics: Vec::new(),
            detach: Vec::new(),
        };
        self.save_runtime(&health)?;
        Ok(health)
    }

    pub fn unfinished_containment(&self) -> Result<Vec<(String, Option<Value>)>> {
        let mut unfinished = Vec::new();
        for watch in self.list()? {
            if let Some(id) = watch.running_attempt_id {
                let attempt = require_attempt(&self.connection, &id)?;
                unfinished.push((id, attempt.containment));
            }
        }
        Ok(unfinished)
    }

    pub fn receipt_evidence(&self, operation_id: &str, limit: u32) -> Result<Vec<ReceiptEvidence>> {
        read_many(&self.connection,
            "SELECT data FROM watcher_v2_operation_evidence WHERE operation_id=?1 ORDER BY id DESC LIMIT ?2",
            params![operation_id,limit.min(MAX_QUERY)])
    }

    pub fn containment_for_resume(&self, id: &str) -> Result<Option<Value>> {
        let Some(watch) = get_watch(&self.connection, id)? else {
            return Ok(None);
        };
        if watch.blocked_reason != Some(BlockedReason::OrphanContainmentUnproven) {
            return Ok(None);
        }
        let attempt_id = watch
            .last_attempt_id
            .as_deref()
            .context("containment attempt missing")?;
        Ok(require_attempt(&self.connection, attempt_id)?.containment)
    }

    pub fn block_watch(&mut self, id: &str, reason: BlockedReason) -> Result<()> {
        let tx = self.connection.transaction()?;
        let mut watch = require_watch(&tx, id)?;
        watch.block(reason);
        save_watch(&tx, &watch)?;
        audit(&tx, Some(id), "watch-blocked", &json!({"reason": reason}))?;
        tx.commit()?;
        Ok(())
    }

    pub fn block_operation(&mut self, id: &str, reason: BlockedReason) -> Result<()> {
        let tx = self.connection.transaction()?;
        let operation = get_operation(&tx, id)?.context("operation missing")?;
        let mut watch = require_watch(&tx, &operation.watch_id)?;
        watch.block(reason);
        save_watch(&tx, &watch)?;
        audit(
            &tx,
            Some(&watch.id),
            "operation-refused",
            &json!({"operationId": id, "reason": reason}),
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn recheck_containment(
        &mut self,
        id: &str,
        ended: impl FnOnce(&Value) -> bool,
    ) -> Result<()> {
        let tx = self.connection.transaction()?;
        let mut watch = require_watch(&tx, id)?;
        if watch.blocked_reason == Some(BlockedReason::OrphanContainmentUnproven) {
            let attempt_id = watch
                .last_attempt_id
                .as_deref()
                .context("containment attempt missing")?;
            let attempt = require_attempt(&tx, attempt_id)?;
            if attempt.containment.as_ref().is_some_and(ended) {
                watch.blocked_reason = None;
                watch.health = Health::Inactive;
                audit(
                    &tx,
                    Some(id),
                    "containment-ended",
                    &json!({"attemptId": attempt_id}),
                )?;
                save_watch(&tx, &watch)?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub fn save_runtime(&self, health: &RuntimeHealth) -> Result<()> {
        save_runtime(&self.connection, health)
    }

    pub fn runtime(&self, id: &str) -> Result<Option<RuntimeHealth>> {
        read_one(
            &self.connection,
            "SELECT data FROM watcher_v2_runtimes WHERE id=?1",
            [id],
        )
    }

    pub fn retention(&self, watch_id: Option<&str>) -> Result<Retention> {
        let mut rows = 0_u64;
        let mut bytes = 0_u64;
        for (table, key) in [
            ("watcher_v2_watches", "id"),
            ("watcher_v2_attempts", "watch_id"),
            ("watcher_v2_operations", "watch_id"),
            ("watcher_v2_revisions", "watch_id"),
            ("watcher_v2_audits", "watch_id"),
            ("watcher_v2_operation_evidence", "watch_id"),
        ] {
            let query = format!("SELECT count(*),coalesce(sum(length(CAST(data AS BLOB))),0) FROM {table} WHERE ?1 IS NULL OR {key}=?1");
            let (count, size): (u64, u64) =
                self.connection
                    .query_row(&query, [watch_id], |row| Ok((row.get(0)?, row.get(1)?)))?;
            rows += count;
            bytes += size;
        }
        if watch_id.is_none() {
            let (count,size): (u64,u64) = self.connection.query_row(
                "SELECT count(*),coalesce(sum(length(CAST(data AS BLOB))),0) FROM watcher_v2_runtimes",
                [], |row| Ok((row.get(0)?,row.get(1)?)))?;
            rows += count;
            bytes += size;
        }
        Ok(Retention {
            rows,
            bytes,
            warning_rows: self.warning_rows,
            warning_bytes: self.warning_bytes,
            warning: rows >= self.warning_rows || bytes >= self.warning_bytes,
        })
    }

    pub fn health(&self, id: &str, sender_ready: bool) -> Result<WatchHealth> {
        let watch = require_watch(&self.connection, id)?;
        let pending = watch
            .pending_operation_id
            .as_deref()
            .map(|id| self.operation(id))
            .transpose()?
            .flatten();
        Ok(WatchHealth {
            schema_version: 2,
            id: watch.id,
            registration_revision: watch.registration_revision,
            detector_schema_version: watch.schema_version,
            activated_at: watch.activated_at,
            lifecycle: watch.lifecycle,
            eligibility: watch.eligibility,
            health: watch.health,
            blocked_reason: watch.blocked_reason,
            last_allocated_event_sequence: watch.last_allocated_event_sequence,
            last_committed_event_sequence: watch.last_committed_event_sequence,
            pending: pending.as_ref().map(PendingHealth::from),
            last_attempt_id: watch.last_attempt_id,
            last_success_at: watch.last_success_at,
            last_event_at: watch.last_event_at,
            next_attempt_ms: watch.next_attempt_ms,
            latest_attempt_duration_ms: watch.latest_attempt_duration_ms,
            suppressed_due_count: watch.suppressed_due_count,
            consecutive_failures: watch.consecutive_failures,
            diagnostic_category: watch.diagnostic_category,
            recurrence: watch.recurrence,
            sender_ready,
            retention: self.retention(Some(id))?,
        })
    }
}

fn new_watch(id: String, registration: Option<Registration>, state: Value, now: i64) -> Watch {
    Watch {
        schema_version: 2,
        id,
        registration,
        registration_revision: 1,
        lifecycle: Lifecycle::Active,
        eligibility: Eligibility::Eligible,
        health: Health::Ready,
        blocked_reason: None,
        activated_at: timestamp(now),
        last_success_at: None,
        last_event_at: None,
        last_attempt_id: None,
        running_attempt_id: None,
        running_cancelled: false,
        state,
        logical_store_id: None,
        last_allocated_event_sequence: 0,
        last_committed_event_sequence: 0,
        pending_operation_id: None,
        next_due_ms: now,
        next_attempt_ms: now,
        suppressed_due_count: 0,
        coalesced_due: false,
        consecutive_failures: 0,
        latest_attempt_duration_ms: None,
        diagnostic_category: None,
        recurrence: Recurrence::default(),
    }
}

fn check_change(watch: &Watch, pending: Option<&Operation>, change: &Change) -> Option<Refusal> {
    if watch.lifecycle == Lifecycle::Removed {
        return Some(Refusal::InvalidLifecycle);
    }
    if watch.schema_version != 2 && !matches!(change, Change::Remove) {
        return Some(Refusal::UnsupportedRegistrationVersion);
    }
    match change {
        Change::Pause | Change::Resume | Change::Update { .. } if pending.is_some() => {
            Some(Refusal::PendingOperation)
        }
        Change::Pause if watch.lifecycle != Lifecycle::Active => Some(Refusal::InvalidLifecycle),
        Change::Resume if watch.lifecycle != Lifecycle::Paused => Some(Refusal::InvalidLifecycle),
        Change::Resume if watch.blocked_reason.is_some() => Some(Refusal::BlockedReasonUncorrected),
        Change::Update { registration, .. } => {
            let old = watch.registration.as_ref()?;
            if old.id != registration.id
                || old.backend_profile != registration.backend_profile
                || old.sender != registration.sender
                || old.target != registration.target
            {
                Some(Refusal::ImmutableRegistration)
            } else {
                None
            }
        }
        Change::GrantReconciliation { attempts, seconds }
        | Change::GrantSendRetry { attempts, seconds }
            if *attempts == 0
                || *attempts > MAX_GRANT
                || *seconds == 0
                || *seconds > MAX_GRANT_SECONDS =>
        {
            Some(Refusal::InvalidGrant)
        }
        Change::GrantReconciliation { .. }
            if watch.blocked_reason != Some(BlockedReason::ReceiptOutcomeUnresolved)
                || pending.is_none() =>
        {
            Some(Refusal::WrongRecoveryProof)
        }
        Change::GrantSendRetry { .. }
            if watch.blocked_reason != Some(BlockedReason::SendRetryExhausted)
                || !pending.is_some_and(|op| {
                    op.last_evidence
                        .as_ref()
                        .is_some_and(|e| matches!(classify(op, e), Resolution::Retry))
                }) =>
        {
            Some(Refusal::WrongRecoveryProof)
        }
        Change::CloseNotAccepted
            if watch.blocked_reason != Some(BlockedReason::SendPermanentlyRejected)
                || !pending.is_some_and(|op| {
                    op.last_evidence
                        .as_ref()
                        .is_some_and(|e| matches!(classify(op, e), Resolution::PermanentRejection))
                }) =>
        {
            Some(Refusal::WrongRecoveryProof)
        }
        _ => None,
    }
}

fn ensure_current(watch: &Watch, attempt: &Attempt) -> Result<()> {
    if watch.running_attempt_id.as_deref() != Some(&attempt.id)
        || watch.running_cancelled
        || watch.registration_revision != attempt.registration_revision
        || watch.lifecycle != Lifecycle::Active
        || watch.pending_operation_id.is_some()
        || hash_value(&watch.state)? != attempt.prior_state_hash
    {
        bail!("attempt-changed");
    }
    Ok(())
}

fn finish(watch: &mut Watch, attempt: &mut Attempt, now: i64) {
    watch.running_attempt_id = None;
    watch.running_cancelled = false;
    watch.latest_attempt_duration_ms =
        Some(now.saturating_sub(attempt.started_at_ms).max(0) as u64);
    attempt.finished_at_ms = Some(now);
}

fn success(watch: &mut Watch, attempt_now: &str, terminal: bool) {
    watch.last_success_at = Some(attempt_now.to_owned());
    watch.consecutive_failures = 0;
    watch.diagnostic_category = None;
    if watch.lifecycle != Lifecycle::Removed {
        watch.lifecycle = if terminal {
            Lifecycle::Terminal
        } else {
            Lifecycle::Active
        };
        watch.eligibility = if terminal {
            Eligibility::Inactive
        } else {
            Eligibility::Eligible
        };
        watch.health = if terminal {
            Health::Inactive
        } else {
            Health::Ready
        };
        watch.blocked_reason = None;
    }
}

fn update_recurrence(watch: &mut Watch, op: &Operation, now: i64) {
    let r = &mut watch.recurrence;
    if r.last_recurrence_hash.as_deref() == Some(&op.recurrence_hash) {
        r.consecutive_identical_recurrences = r.consecutive_identical_recurrences.saturating_add(1);
    } else {
        r.consecutive_identical_recurrences = 1;
        r.first_identical_at = Some(timestamp(now));
    }
    r.last_recurrence_hash = Some(op.recurrence_hash.clone());
    r.last_identical_at = Some(timestamp(now));
    r.last_identical_event_sequence = Some(op.event_sequence);
}

pub(crate) fn cadence_ms(watch: &Watch) -> i64 {
    let interval = watch
        .registration
        .as_ref()
        .map_or(1, |r| r.interval_seconds)
        * 1000;
    let hash = watch.id.bytes().fold(0xcbf29ce484222325_u64, |h, b| {
        (h ^ b as u64).wrapping_mul(0x100000001b3)
    });
    (interval + hash % (interval / 10 + 1)) as i64
}

fn backoff_ms(failures: u32) -> i64 {
    (1000_i64 << failures.saturating_sub(1).min(10)).min(300_000)
}

fn deadline(now: i64, seconds: u64) -> Result<i64> {
    now.checked_add(i64::try_from(seconds)? * 1000)
        .context("deadline overflow")
}

fn get_watch(connection: &Connection, id: &str) -> Result<Option<Watch>> {
    read_one(
        connection,
        "SELECT data FROM watcher_v2_watches WHERE id=?1",
        [id],
    )
}
fn require_watch(connection: &Connection, id: &str) -> Result<Watch> {
    get_watch(connection, id)?.context("watch not found")
}
fn require_attempt(connection: &Connection, id: &str) -> Result<Attempt> {
    read_one(
        connection,
        "SELECT data FROM watcher_v2_attempts WHERE id=?1",
        [id],
    )?
    .context("attempt not found")
}
fn get_operation(connection: &Connection, id: &str) -> Result<Option<Operation>> {
    read_one(
        connection,
        "SELECT data FROM watcher_v2_operations WHERE id=?1",
        [id],
    )
}
fn save_watch(connection: &Connection, watch: &Watch) -> Result<()> {
    connection.execute("INSERT INTO watcher_v2_watches VALUES(?1,?2) ON CONFLICT(id) DO UPDATE SET data=excluded.data",
        params![watch.id, serde_json::to_string(watch)?])?;
    Ok(())
}
fn save_operation(connection: &Connection, operation: &Operation) -> Result<()> {
    let updated = connection.execute(
        "UPDATE watcher_v2_operations SET data=?5 WHERE id=?1 AND watch_id=?2 AND sequence=?3 AND event_id=?4",
        params![operation.operation_id, operation.watch_id, operation.event_sequence,
            operation.event_id, serde_json::to_string(operation)?])?;
    anyhow::ensure!(updated == 1, "operation-identity-conflict");
    Ok(())
}
fn save_attempt(connection: &Connection, attempt: &Attempt) -> Result<()> {
    connection.execute("INSERT INTO watcher_v2_attempts VALUES(?1,?2,?3,?4) ON CONFLICT(id) DO UPDATE SET data=excluded.data",
        params![attempt.id, attempt.watch_id, attempt.started_at_ms, serde_json::to_string(attempt)?])?;
    Ok(())
}
fn save_runtime(connection: &Connection, health: &RuntimeHealth) -> Result<()> {
    connection.execute("INSERT INTO watcher_v2_runtimes VALUES(?1,?2) ON CONFLICT(id) DO UPDATE SET data=excluded.data",
        params![health.runtime_id, serde_json::to_string(health)?])?;
    Ok(())
}
fn revision(connection: &Connection, watch: &Watch) -> Result<()> {
    connection.execute(
        "INSERT INTO watcher_v2_revisions VALUES(?1,?2,?3)",
        params![
            watch.id,
            watch.registration_revision,
            serde_json::to_string(&watch.registration)?
        ],
    )?;
    Ok(())
}
fn bump(connection: &Connection) -> Result<()> {
    connection.execute(
        "UPDATE watcher_v2_meta SET revision=revision+1 WHERE singleton=1",
        [],
    )?;
    Ok(())
}
fn audit(connection: &Connection, id: Option<&str>, action: &str, data: &Value) -> Result<()> {
    connection.execute(
        "INSERT INTO watcher_v2_audits(watch_id,at_ms,action,data) VALUES(?1,?2,?3,?4)",
        params![id, now_ms(), action, serde_json::to_string(data)?],
    )?;
    Ok(())
}
fn mutation_refused(
    tx: Transaction<'_>,
    watch: Option<Watch>,
    id: &str,
    action: &str,
    refusal: Refusal,
) -> Result<Mutation> {
    audit(&tx, Some(id), action, &json!({"refusal": refusal}))?;
    tx.commit()?;
    Ok(Mutation {
        watch,
        refusal: Some(refusal),
        diagnostic: None,
        recovery: None,
    })
}
fn read_one<T: DeserializeOwned>(
    connection: &Connection,
    sql: &str,
    params: impl rusqlite::Params,
) -> Result<Option<T>> {
    let raw: Option<String> = connection
        .query_row(sql, params, |row| row.get(0))
        .optional()?;
    raw.map(|raw| serde_json::from_str(&raw).map_err(Into::into))
        .transpose()
}
fn read_many<T: DeserializeOwned>(
    connection: &Connection,
    sql: &str,
    params: impl rusqlite::Params,
) -> Result<Vec<T>> {
    let mut statement = connection.prepare(sql)?;
    let rows = statement
        .query_map(params, |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    rows.into_iter()
        .map(|raw| serde_json::from_str(&raw).map_err(Into::into))
        .collect()
}

#[cfg(test)]
#[path = "registry_tests.rs"]
mod tests;
