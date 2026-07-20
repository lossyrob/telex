use crate::protocol::{hash_value, parse_state, sha256, state_json, MAX_STATE_BYTES};
use anyhow::{anyhow, bail, Context, Result};
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const SCHEMA_VERSION: i64 = 1;
const ATTEMPT_RETENTION: i64 = 200;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WatchSpec {
    pub id: String,
    pub command: Vec<String>,
    pub script_path: PathBuf,
    pub working_directory: PathBuf,
    pub script_mode: ScriptMode,
    #[serde(default)]
    pub script_digest: Option<String>,
    pub sender: String,
    pub target: String,
    pub interval_seconds: u64,
    pub timeout_seconds: u64,
    #[serde(default = "default_attention")]
    pub attention: String,
    #[serde(default)]
    pub requires_disposition: bool,
    #[serde(default)]
    pub environment_allowlist: Vec<String>,
    #[serde(default = "empty_object")]
    pub parameters: Value,
    #[serde(default = "empty_object")]
    pub state: Value,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ScriptMode {
    Pinned,
    FollowPath,
}

impl ScriptMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pinned => "pinned",
            Self::FollowPath => "follow-path",
        }
    }

    fn parse(raw: &str) -> Result<Self> {
        match raw {
            "pinned" => Ok(Self::Pinned),
            "follow-path" => Ok(Self::FollowPath),
            _ => bail!("unknown script mode {raw:?}"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WatchStatus {
    Active,
    Paused,
    Terminal,
    Removed,
}

impl WatchStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Paused => "paused",
            Self::Terminal => "terminal",
            Self::Removed => "removed",
        }
    }

    fn parse(raw: &str) -> Result<Self> {
        match raw {
            "active" => Ok(Self::Active),
            "paused" => Ok(Self::Paused),
            "terminal" => Ok(Self::Terminal),
            "removed" => Ok(Self::Removed),
            _ => bail!("unknown watch status {raw:?}"),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Watch {
    pub id: String,
    pub command: Vec<String>,
    pub script_path: PathBuf,
    pub working_directory: PathBuf,
    pub script_mode: ScriptMode,
    pub script_digest: Option<String>,
    pub sender: String,
    pub target: String,
    pub interval_seconds: u64,
    pub timeout_seconds: u64,
    pub attention: String,
    pub requires_disposition: bool,
    pub environment_allowlist: Vec<String>,
    pub parameters: Value,
    pub state: Value,
    pub status: String,
    pub next_due_ms: i64,
    pub failure_count: u32,
    pub last_diagnostic: Option<String>,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Attempt {
    pub id: String,
    pub watch_id: String,
    pub started_at_ms: i64,
    pub finished_at_ms: Option<i64>,
    pub script_digest: Option<String>,
    pub prior_state_hash: String,
    pub outcome: Option<String>,
    pub result_json: Option<Value>,
    pub event_id: Option<String>,
    pub envelope_hash: Option<String>,
    pub receipt_json: Option<Value>,
    pub state_committed: bool,
    pub diagnostic: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SentEvent {
    pub watch_id: String,
    pub event_id: String,
    pub prior_state_hash: String,
    pub next_state_hash: String,
    pub envelope_hash: String,
    pub script_digest: String,
    pub sender: String,
    pub target: String,
    pub message_id: i64,
    pub receipt_json: Value,
    pub attempt_id: String,
    pub accepted_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ReconciledStartup {
    /// Prior runtimes that were still `running` and have now been marked `interrupted`.
    pub runtime_session_ids: Vec<String>,
    /// Attempts that were still unfinished and have now been failed as `runtime-interrupted`.
    pub attempt_ids: Vec<String>,
    /// Watches whose failure backoff/next-due were extended to fence a possibly surviving detector.
    pub watch_ids: Vec<String>,
}

impl ReconciledStartup {
    pub fn is_empty(&self) -> bool {
        self.runtime_session_ids.is_empty()
            && self.attempt_ids.is_empty()
            && self.watch_ids.is_empty()
    }
}

pub struct Registry {
    connection: Connection,
}

impl Registry {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("create registry directory {}", parent.display()))?;
        }
        let connection = Connection::open(path)
            .with_context(|| format!("open Watcher registry {}", path.display()))?;
        connection.busy_timeout(std::time::Duration::from_secs(5))?;
        let mut registry = Self { connection };
        registry.initialize()?;
        Ok(registry)
    }

    fn initialize(&mut self) -> Result<()> {
        self.connection.execute_batch(
            "
            PRAGMA foreign_keys = ON;
            CREATE TABLE IF NOT EXISTS metadata (
                key TEXT PRIMARY KEY NOT NULL,
                value TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS watches (
                id TEXT PRIMARY KEY NOT NULL,
                command_json TEXT NOT NULL,
                script_path TEXT NOT NULL,
                working_directory TEXT NOT NULL,
                script_mode TEXT NOT NULL,
                script_digest TEXT,
                sender TEXT NOT NULL,
                target TEXT NOT NULL,
                interval_seconds INTEGER NOT NULL,
                timeout_seconds INTEGER NOT NULL,
                attention TEXT NOT NULL,
                requires_disposition INTEGER NOT NULL,
                environment_allowlist_json TEXT NOT NULL,
                parameters_json TEXT NOT NULL,
                state_json TEXT NOT NULL,
                status TEXT NOT NULL,
                next_due_ms INTEGER NOT NULL,
                failure_count INTEGER NOT NULL,
                last_diagnostic TEXT,
                created_at_ms INTEGER NOT NULL,
                updated_at_ms INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS attempts (
                id TEXT PRIMARY KEY NOT NULL,
                watch_id TEXT NOT NULL REFERENCES watches(id),
                started_at_ms INTEGER NOT NULL,
                finished_at_ms INTEGER,
                script_digest TEXT,
                prior_state_hash TEXT NOT NULL,
                outcome TEXT,
                result_json TEXT,
                event_id TEXT,
                envelope_hash TEXT,
                receipt_json TEXT,
                state_committed INTEGER NOT NULL DEFAULT 0,
                diagnostic TEXT
            );
            CREATE INDEX IF NOT EXISTS attempts_watch_started ON attempts(watch_id, started_at_ms DESC);
            CREATE TABLE IF NOT EXISTS sent_events (
                watch_id TEXT NOT NULL REFERENCES watches(id),
                event_id TEXT NOT NULL,
                prior_state_hash TEXT NOT NULL,
                next_state_hash TEXT NOT NULL,
                envelope_hash TEXT NOT NULL,
                script_digest TEXT NOT NULL,
                sender TEXT NOT NULL,
                target TEXT NOT NULL,
                message_id INTEGER NOT NULL,
                receipt_json TEXT NOT NULL,
                attempt_id TEXT NOT NULL REFERENCES attempts(id),
                accepted_at_ms INTEGER NOT NULL,
                PRIMARY KEY (watch_id, event_id)
            );
            CREATE INDEX IF NOT EXISTS sent_events_watch_accepted ON sent_events(watch_id, accepted_at_ms DESC);
            CREATE TABLE IF NOT EXISTS runtime_sessions (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                runtime_session_id TEXT NOT NULL,
                watcher_pid INTEGER NOT NULL,
                started_at_ms INTEGER NOT NULL,
                ended_at_ms INTEGER,
                status TEXT NOT NULL,
                detail_json TEXT
            );
            ",
        )?;
        let schema: Option<String> = self
            .connection
            .query_row(
                "SELECT value FROM metadata WHERE key = 'schema_version'",
                [],
                |row| row.get(0),
            )
            .optional()?;
        match schema.as_deref() {
            None => {
                self.connection.execute(
                    "INSERT OR IGNORE INTO metadata(key, value) VALUES ('schema_version', ?1), ('revision', '0')",
                    [SCHEMA_VERSION.to_string()],
                )?;
            }
            Some("1") => {}
            Some(value) => bail!("unsupported Watcher registry schema version {value}"),
        }
        Ok(())
    }

    pub fn add(&mut self, mut spec: WatchSpec) -> Result<Watch> {
        normalize_addresses(&mut spec);
        validate_spec(&mut spec.clone())?;
        let spec = canonicalize_spec(spec)?;
        let now = now_ms();
        let tx = self.connection.transaction()?;
        let exists: Option<String> = tx
            .query_row("SELECT id FROM watches WHERE id = ?1", [&spec.id], |row| {
                row.get(0)
            })
            .optional()?;
        if exists.is_some() {
            bail!("watch {:?} already exists", spec.id);
        }
        insert_watch(&tx, &spec, WatchStatus::Active, now, now)?;
        increment_revision(&tx)?;
        tx.commit()?;
        self.get(&spec.id)?
            .ok_or_else(|| anyhow!("added watch disappeared from registry"))
    }

    pub fn update(&mut self, id: &str, mut spec: WatchSpec) -> Result<Watch> {
        normalize_addresses(&mut spec);
        validate_spec(&mut spec.clone())?;
        if id != spec.id {
            bail!("update file id must match the requested watch id");
        }
        let spec = canonicalize_spec(spec)?;
        let previous = self
            .get(id)?
            .ok_or_else(|| anyhow!("watch {id:?} does not exist"))?;
        if previous.sender != spec.sender || previous.target != spec.target {
            bail!("sender and target are immutable; create a new watch to reroute");
        }
        if WatchStatus::parse(&previous.status)? == WatchStatus::Removed {
            bail!("removed watches cannot be updated");
        }
        let now = now_ms();
        let tx = self.connection.transaction()?;
        tx.execute(
            "UPDATE watches SET command_json=?2, script_path=?3, working_directory=?4,
             script_mode=?5, script_digest=?6, interval_seconds=?7, timeout_seconds=?8,
             attention=?9, requires_disposition=?10, environment_allowlist_json=?11,
             parameters_json=?12, state_json=?13, next_due_ms=?14, failure_count=0,
             last_diagnostic=NULL, updated_at_ms=?15 WHERE id=?1",
            params![
                id,
                json_text(&spec.command)?,
                path_text(&spec.script_path),
                path_text(&spec.working_directory),
                spec.script_mode.as_str(),
                spec.script_digest,
                spec.interval_seconds as i64,
                spec.timeout_seconds as i64,
                spec.attention,
                bool_int(spec.requires_disposition),
                json_text(&spec.environment_allowlist)?,
                json_text(&spec.parameters)?,
                state_json(&spec.state)?,
                now,
                now,
            ],
        )?;
        increment_revision(&tx)?;
        tx.commit()?;
        self.get(id)?
            .ok_or_else(|| anyhow!("updated watch disappeared from registry"))
    }

    pub fn list(&self) -> Result<Vec<Watch>> {
        let mut statement = self.connection.prepare(
            "SELECT id, command_json, script_path, working_directory, script_mode, script_digest,
                    sender, target, interval_seconds, timeout_seconds, attention, requires_disposition,
                    environment_allowlist_json, parameters_json, state_json, status, next_due_ms,
                    failure_count, last_diagnostic, updated_at_ms
             FROM watches ORDER BY id",
        )?;
        let rows = statement.query_map([], row_to_watch)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    pub fn get(&self, id: &str) -> Result<Option<Watch>> {
        self.connection
            .query_row(
                "SELECT id, command_json, script_path, working_directory, script_mode, script_digest,
                        sender, target, interval_seconds, timeout_seconds, attention, requires_disposition,
                        environment_allowlist_json, parameters_json, state_json, status, next_due_ms,
                        failure_count, last_diagnostic, updated_at_ms
                 FROM watches WHERE id=?1",
                [id],
                row_to_watch,
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn set_status(&mut self, id: &str, status: WatchStatus) -> Result<Watch> {
        let now = now_ms();
        let changed = self.connection.execute(
            "UPDATE watches SET status=?2, next_due_ms=?3, updated_at_ms=?3 WHERE id=?1
             AND status != 'removed'",
            params![id, status.as_str(), now],
        )?;
        if changed != 1 {
            bail!("watch {id:?} does not exist or has been removed");
        }
        self.bump_revision()?;
        self.get(id)?
            .ok_or_else(|| anyhow!("changed watch disappeared from registry"))
    }

    pub fn remove(&mut self, id: &str) -> Result<Watch> {
        self.set_status(id, WatchStatus::Removed)
    }

    /// Spread active overdue watches by a bounded, deterministic 0-10% of their interval before the
    /// first due batch after a restart. This avoids a thundering herd of simultaneous detector
    /// launches when many watches were overdue while the runtime was down, while preserving the
    /// one-run catch-up guarantee (the jitter never exceeds 10% of the interval). The delay is a
    /// stable function of the watch id so a given watch is spread consistently across restarts, and
    /// it does NOT bump the configuration revision (jitter is not a lifecycle mutation).
    pub fn apply_restart_jitter(
        &mut self,
        selected: &BTreeSet<String>,
        now: i64,
    ) -> Result<Vec<(String, i64)>> {
        let overdue: Vec<Watch> = self
            .list()?
            .into_iter()
            .filter(|watch| {
                watch.status == WatchStatus::Active.as_str()
                    && watch.next_due_ms <= now
                    && (selected.is_empty() || selected.contains(&watch.id))
            })
            .collect();
        let mut applied = Vec::new();
        let tx = self.connection.transaction()?;
        for watch in overdue {
            let span_ms = (watch.interval_seconds as i64).saturating_mul(1000) / 10;
            let jitter_ms = if span_ms > 0 {
                (stable_hash(&watch.id) % (span_ms as u64 + 1)) as i64
            } else {
                0
            };
            if jitter_ms == 0 {
                continue;
            }
            let next_due = now.saturating_add(jitter_ms);
            tx.execute(
                "UPDATE watches SET next_due_ms=?2 WHERE id=?1",
                params![watch.id, next_due],
            )?;
            applied.push((watch.id, jitter_ms));
        }
        tx.commit()?;
        Ok(applied)
    }

    pub fn due(&self, selected: &BTreeSet<String>, now: i64) -> Result<Vec<Watch>> {
        let watches = self.list()?;
        Ok(watches
            .into_iter()
            .filter(|watch| {
                watch.status == WatchStatus::Active.as_str()
                    && watch.next_due_ms <= now
                    && (selected.is_empty() || selected.contains(&watch.id))
            })
            .collect())
    }

    pub fn configured_senders(&self) -> Result<Vec<String>> {
        let mut statement = self.connection.prepare(
            "SELECT DISTINCT sender FROM watches WHERE status != 'removed' ORDER BY sender",
        )?;
        let rows = statement.query_map([], |row| row.get(0))?;
        rows.collect::<std::result::Result<Vec<String>, _>>()
            .map_err(Into::into)
    }

    pub fn revision(&self) -> Result<i64> {
        self.connection
            .query_row(
                "SELECT value FROM metadata WHERE key = 'revision'",
                [],
                |row| row.get::<_, String>(0),
            )?
            .parse()
            .map_err(|error| anyhow!("invalid registry revision: {error}"))
    }

    pub fn begin_attempt(
        &mut self,
        attempt_id: &str,
        watch: &Watch,
        script_digest: Option<&str>,
    ) -> Result<String> {
        let prior_state_hash = hash_value(&watch.state)?;
        self.connection.execute(
            "INSERT INTO attempts(id, watch_id, started_at_ms, script_digest, prior_state_hash)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                attempt_id,
                watch.id,
                now_ms(),
                script_digest,
                prior_state_hash
            ],
        )?;
        Ok(prior_state_hash)
    }

    pub fn event(&self, watch_id: &str, event_id: &str) -> Result<Option<SentEvent>> {
        self.connection
            .query_row(
                "SELECT watch_id, event_id, prior_state_hash, next_state_hash, envelope_hash,
                        script_digest, sender, target, message_id, receipt_json, attempt_id, accepted_at_ms
                 FROM sent_events WHERE watch_id=?1 AND event_id=?2",
                params![watch_id, event_id],
                |row| {
                    Ok(SentEvent {
                        watch_id: row.get(0)?,
                        event_id: row.get(1)?,
                        prior_state_hash: row.get(2)?,
                        next_state_hash: row.get(3)?,
                        envelope_hash: row.get(4)?,
                        script_digest: row.get(5)?,
                        sender: row.get(6)?,
                        target: row.get(7)?,
                        message_id: row.get(8)?,
                        receipt_json: parse_json(row.get::<_, String>(9)?).map_err(to_sql_error)?,
                        attempt_id: row.get(10)?,
                        accepted_at_ms: row.get(11)?,
                    })
                },
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn commit_idle(
        &mut self,
        watch: &Watch,
        attempt_id: &str,
        next_state: Value,
        terminal: bool,
        result: &Value,
    ) -> Result<()> {
        let tx = self.connection.transaction()?;
        let now = now_ms();
        let status = if terminal {
            WatchStatus::Terminal
        } else {
            WatchStatus::Active
        };
        tx.execute(
            "UPDATE watches SET state_json=?2, status=?3, next_due_ms=?4, failure_count=0,
             last_diagnostic=NULL, updated_at_ms=?5 WHERE id=?1",
            params![
                watch.id,
                state_json(&next_state)?,
                status.as_str(),
                schedule_after(watch.interval_seconds, now),
                now
            ],
        )?;
        finish_attempt(
            &tx,
            attempt_id,
            "success",
            Some(result),
            None,
            None,
            None,
            true,
            None,
        )?;
        prune_attempts(&tx, &watch.id)?;
        tx.commit()?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn commit_event(
        &mut self,
        watch: &Watch,
        attempt_id: &str,
        event_id: &str,
        prior_state_hash: &str,
        next_state: Value,
        envelope_hash: &str,
        script_digest: &str,
        message_id: i64,
        receipt: &Value,
        terminal: bool,
        result: &Value,
    ) -> Result<()> {
        let tx = self.connection.transaction()?;
        let now = now_ms();
        let next_state_hash = hash_value(&next_state)?;
        tx.execute(
            "UPDATE watches SET state_json=?2, status=?3, next_due_ms=?4, failure_count=0,
             last_diagnostic=NULL, updated_at_ms=?5 WHERE id=?1",
            params![
                watch.id,
                state_json(&next_state)?,
                if terminal {
                    WatchStatus::Terminal.as_str()
                } else {
                    WatchStatus::Active.as_str()
                },
                schedule_after(watch.interval_seconds, now),
                now
            ],
        )?;
        tx.execute(
            "INSERT INTO sent_events(watch_id, event_id, prior_state_hash, next_state_hash,
             envelope_hash, script_digest, sender, target, message_id, receipt_json, attempt_id,
             accepted_at_ms) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                watch.id,
                event_id,
                prior_state_hash,
                next_state_hash,
                envelope_hash,
                script_digest,
                watch.sender,
                watch.target,
                message_id,
                json_text(receipt)?,
                attempt_id,
                now
            ],
        )?;
        finish_attempt(
            &tx,
            attempt_id,
            "event-sent",
            Some(result),
            Some(event_id),
            Some(envelope_hash),
            Some(receipt),
            true,
            None,
        )?;
        prune_attempts(&tx, &watch.id)?;
        tx.commit()?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn finish_noop(
        &mut self,
        watch: &Watch,
        attempt_id: &str,
        outcome: &str,
        result: Option<&Value>,
        event_id: Option<&str>,
        envelope_hash: Option<&str>,
        diagnostic: Option<&str>,
    ) -> Result<()> {
        let tx = self.connection.transaction()?;
        let now = now_ms();
        tx.execute(
            "UPDATE watches SET next_due_ms=?2, last_diagnostic=?3, updated_at_ms=?4 WHERE id=?1",
            params![
                watch.id,
                schedule_after(watch.interval_seconds, now),
                diagnostic,
                now
            ],
        )?;
        finish_attempt(
            &tx,
            attempt_id,
            outcome,
            result,
            event_id,
            envelope_hash,
            None,
            false,
            diagnostic,
        )?;
        prune_attempts(&tx, &watch.id)?;
        tx.commit()?;
        Ok(())
    }

    pub fn record_failure(
        &mut self,
        watch: &Watch,
        attempt_id: &str,
        outcome: &str,
        diagnostic: &str,
        result: Option<&Value>,
        script_digest: Option<&str>,
    ) -> Result<()> {
        let tx = self.connection.transaction()?;
        let now = now_ms();
        let failures = watch.failure_count.saturating_add(1);
        tx.execute(
            "UPDATE watches SET failure_count=?2, next_due_ms=?3, last_diagnostic=?4,
             updated_at_ms=?5 WHERE id=?1",
            params![
                watch.id,
                failures,
                now.saturating_add(backoff_ms(failures)),
                diagnostic,
                now
            ],
        )?;
        tx.execute(
            "UPDATE attempts SET script_digest=COALESCE(?2, script_digest) WHERE id=?1",
            params![attempt_id, script_digest],
        )?;
        finish_attempt(
            &tx,
            attempt_id,
            outcome,
            result,
            None,
            None,
            None,
            false,
            Some(diagnostic),
        )?;
        prune_attempts(&tx, &watch.id)?;
        tx.commit()?;
        Ok(())
    }

    pub fn attempts(&self, watch_id: &str, limit: u32) -> Result<Vec<Attempt>> {
        let mut statement = self.connection.prepare(
            "SELECT id, watch_id, started_at_ms, finished_at_ms, script_digest, prior_state_hash,
                    outcome, result_json, event_id, envelope_hash, receipt_json, state_committed,
                    diagnostic FROM attempts WHERE watch_id=?1 ORDER BY started_at_ms DESC LIMIT ?2",
        )?;
        let rows = statement.query_map(
            params![watch_id, limit.min(ATTEMPT_RETENTION as u32)],
            |row| {
                Ok(Attempt {
                    id: row.get(0)?,
                    watch_id: row.get(1)?,
                    started_at_ms: row.get(2)?,
                    finished_at_ms: row.get(3)?,
                    script_digest: row.get(4)?,
                    prior_state_hash: row.get(5)?,
                    outcome: row.get(6)?,
                    result_json: row
                        .get::<_, Option<String>>(7)?
                        .map(parse_json)
                        .transpose()
                        .map_err(to_sql_error)?,
                    event_id: row.get(8)?,
                    envelope_hash: row.get(9)?,
                    receipt_json: row
                        .get::<_, Option<String>>(10)?
                        .map(parse_json)
                        .transpose()
                        .map_err(to_sql_error)?,
                    state_committed: row.get::<_, i64>(11)? != 0,
                    diagnostic: row.get(12)?,
                })
            },
        )?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    pub fn events(&self, watch_id: &str, limit: u32) -> Result<Vec<SentEvent>> {
        let mut statement = self.connection.prepare(
            "SELECT watch_id, event_id, prior_state_hash, next_state_hash, envelope_hash,
                    script_digest, sender, target, message_id, receipt_json, attempt_id, accepted_at_ms
             FROM sent_events WHERE watch_id=?1 ORDER BY accepted_at_ms DESC LIMIT ?2",
        )?;
        let rows = statement.query_map(params![watch_id, limit.min(1000)], |row| {
            Ok(SentEvent {
                watch_id: row.get(0)?,
                event_id: row.get(1)?,
                prior_state_hash: row.get(2)?,
                next_state_hash: row.get(3)?,
                envelope_hash: row.get(4)?,
                script_digest: row.get(5)?,
                sender: row.get(6)?,
                target: row.get(7)?,
                message_id: row.get(8)?,
                receipt_json: parse_json(row.get::<_, String>(9)?).map_err(to_sql_error)?,
                attempt_id: row.get(10)?,
                accepted_at_ms: row.get(11)?,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    /// Reconcile the wreckage of a Watcher runtime that died abruptly (no clean shutdown), so a
    /// fresh runtime never inherits a `running` session row or an unfinished attempt. This runs
    /// once at startup, after the scheduler file lock is held and before the new runtime row is
    /// recorded, in a single transaction. Detector `state_json` and the `sent_events` ledger are
    /// intentionally left untouched; only liveness bookkeeping and failure backoff change.
    pub fn reconcile_interrupted_runtimes(&mut self) -> Result<ReconciledStartup> {
        let now = now_ms();
        let tx = self.connection.transaction()?;

        let mut reconciled = ReconciledStartup::default();

        // 1. Prior runtimes still marked running are interrupted.
        {
            let mut statement = tx.prepare(
                "SELECT runtime_session_id FROM runtime_sessions WHERE status='running'",
            )?;
            let ids = statement.query_map([], |row| row.get::<_, String>(0))?;
            for id in ids {
                reconciled.runtime_session_ids.push(id?);
            }
        }
        if !reconciled.runtime_session_ids.is_empty() {
            let detail = json_text(&serde_json::json!({
                "reason": "runtime-interrupted",
                "detail": "prior Watcher runtime ended without a clean shutdown; reconciled at startup",
            }))?;
            tx.execute(
                "UPDATE runtime_sessions SET status='interrupted', ended_at_ms=?1, detail_json=?2 \
                 WHERE status='running'",
                params![now, detail],
            )?;
        }

        // 2. Attempts left unfinished become visible runtime-interrupted failures. We never invent
        //    a receipt or an event commit: state_committed stays false and result/receipt are null.
        let unfinished: Vec<(String, String)> = {
            let mut statement =
                tx.prepare("SELECT id, watch_id FROM attempts WHERE finished_at_ms IS NULL")?;
            let rows = statement.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?;
            rows.collect::<std::result::Result<Vec<_>, _>>()?
        };
        if !unfinished.is_empty() {
            tx.execute(
                "UPDATE attempts SET finished_at_ms=?1, outcome='runtime-interrupted', \
                 state_committed=0, diagnostic=?2 WHERE finished_at_ms IS NULL",
                params![
                    now,
                    "Watcher runtime was interrupted before this attempt finished; no send or state commit occurred"
                ],
            )?;
        }
        for (attempt_id, _) in &unfinished {
            reconciled.attempt_ids.push(attempt_id.clone());
        }

        // 3. Each affected watch takes one failure and a next-due fence of at least its configured
        //    timeout plus the normal failure backoff, so a detector descendant that outlived the
        //    dead runtime cannot immediately overlap the fresh runtime's next attempt.
        let mut affected_watches: Vec<String> = Vec::new();
        for (_, watch_id) in &unfinished {
            if !affected_watches.contains(watch_id) {
                affected_watches.push(watch_id.clone());
            }
        }
        for watch_id in &affected_watches {
            let row: Option<(i64, i64)> = tx
                .query_row(
                    "SELECT timeout_seconds, failure_count FROM watches WHERE id=?1",
                    params![watch_id],
                    |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
                )
                .optional()?;
            let Some((timeout_seconds, failure_count)) = row else {
                continue;
            };
            let failures = (failure_count as u32).saturating_add(1);
            let fence = timeout_seconds
                .saturating_mul(1000)
                .saturating_add(backoff_ms(failures));
            let next_due = now.saturating_add(fence);
            tx.execute(
                "UPDATE watches SET failure_count=?2, next_due_ms=?3, last_diagnostic=?4, \
                 updated_at_ms=?5 WHERE id=?1",
                params![
                    watch_id,
                    failures,
                    next_due,
                    "runtime interrupted mid-attempt; retry delayed to fence a possible surviving detector",
                    now
                ],
            )?;
            reconciled.watch_ids.push(watch_id.clone());
        }

        tx.commit()?;
        Ok(reconciled)
    }

    pub fn runtime_started(&mut self, runtime_session_id: &str, pid: u32) -> Result<i64> {
        self.connection.execute(
            "INSERT INTO runtime_sessions(runtime_session_id, watcher_pid, started_at_ms, status, detail_json)
             VALUES (?1, ?2, ?3, 'running', ?4)",
            params![
                runtime_session_id,
                pid,
                now_ms(),
                json_text(&serde_json::json!({"runtimeSessionId": runtime_session_id}))?,
            ],
        )?;
        Ok(self.connection.last_insert_rowid())
    }

    pub fn runtime_finished(&mut self, id: i64, status: &str, detail: &Value) -> Result<()> {
        self.connection.execute(
            "UPDATE runtime_sessions SET ended_at_ms=?2, status=?3, detail_json=?4 WHERE id=?1",
            params![id, now_ms(), status, json_text(detail)?],
        )?;
        Ok(())
    }

    fn bump_revision(&mut self) -> Result<()> {
        self.connection.execute(
            "UPDATE metadata SET value=CAST(value AS INTEGER)+1 WHERE key='revision'",
            [],
        )?;
        Ok(())
    }
}

fn validate_spec(spec: &mut WatchSpec) -> Result<()> {
    if spec.id.trim().is_empty() || spec.id.len() > 128 || spec.id.contains(char::is_whitespace) {
        bail!("watch id must be a non-empty identifier up to 128 bytes without whitespace");
    }
    if spec.command.is_empty()
        || spec
            .command
            .iter()
            .any(|part| part.is_empty() || part.len() > 8192)
    {
        bail!("command must contain non-empty argv entries no longer than 8192 bytes");
    }
    if !(1..=86_400).contains(&spec.interval_seconds) {
        bail!("intervalSeconds must be between 1 and 86400");
    }
    if !(1..=3_600).contains(&spec.timeout_seconds) {
        bail!("timeoutSeconds must be between 1 and 3600");
    }
    if !matches!(
        spec.attention.as_str(),
        "interrupt" | "next-checkpoint" | "background" | "fyi"
    ) {
        bail!("attention must be interrupt, next-checkpoint, background, or fyi");
    }
    validate_address("sender", &spec.sender)?;
    validate_address("target", &spec.target)?;
    let mut environment = BTreeSet::new();
    for name in &spec.environment_allowlist {
        if !valid_env_name(name) {
            bail!("invalid environment variable name {name:?}");
        }
        if !environment.insert(name.clone()) {
            bail!("environment allowlist contains duplicate {name:?}");
        }
    }
    crate::protocol::state_json(&spec.state)?;
    ensure_json_cap("parameters", &spec.parameters, MAX_STATE_BYTES)?;
    Ok(())
}

fn canonicalize_spec(mut spec: WatchSpec) -> Result<WatchSpec> {
    let requested_script_path = spec.script_path.clone();
    spec.script_path = normalized_canonical_path(
        fs::canonicalize(&spec.script_path)
            .with_context(|| format!("canonicalize script path {}", spec.script_path.display()))?,
    );
    if !spec.script_path.is_file() {
        bail!("script path is not a file: {}", spec.script_path.display());
    }
    spec.working_directory = normalized_canonical_path(
        fs::canonicalize(&spec.working_directory).with_context(|| {
            format!(
                "canonicalize working directory {}",
                spec.working_directory.display()
            )
        })?,
    );
    if !spec.working_directory.is_dir() {
        bail!(
            "working directory is not a directory: {}",
            spec.working_directory.display()
        );
    }
    let mut command_contains_script = false;
    for arg in &mut spec.command {
        if Path::new(arg) == requested_script_path {
            *arg = path_text(&spec.script_path);
            command_contains_script = true;
        }
    }
    if !command_contains_script {
        bail!(
            "command argv must include the registered scriptPath {}; shell snippets are not supported",
            requested_script_path.display()
        );
    }
    let digest = script_digest(&spec.script_path)?;
    match spec.script_mode {
        ScriptMode::Pinned => {
            let expected = spec
                .script_digest
                .as_deref()
                .ok_or_else(|| anyhow!("pinned script mode requires scriptDigest"))?;
            if expected != digest {
                bail!(
                    "pinned scriptDigest does not match {}",
                    spec.script_path.display()
                );
            }
        }
        ScriptMode::FollowPath => {
            spec.script_digest = None;
        }
    }
    Ok(spec)
}

pub fn script_digest(path: &Path) -> Result<String> {
    Ok(sha256(&fs::read(path).with_context(|| {
        format!("read script {}", path.display())
    })?))
}

fn insert_watch(
    tx: &Transaction<'_>,
    spec: &WatchSpec,
    status: WatchStatus,
    next_due_ms: i64,
    now: i64,
) -> Result<()> {
    tx.execute(
        "INSERT INTO watches(id, command_json, script_path, working_directory, script_mode,
         script_digest, sender, target, interval_seconds, timeout_seconds, attention,
         requires_disposition, environment_allowlist_json, parameters_json, state_json, status,
         next_due_ms, failure_count, last_diagnostic, created_at_ms, updated_at_ms)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16,
         ?17, 0, NULL, ?18, ?18)",
        params![
            spec.id,
            json_text(&spec.command)?,
            path_text(&spec.script_path),
            path_text(&spec.working_directory),
            spec.script_mode.as_str(),
            spec.script_digest,
            spec.sender,
            spec.target,
            spec.interval_seconds as i64,
            spec.timeout_seconds as i64,
            spec.attention,
            bool_int(spec.requires_disposition),
            json_text(&spec.environment_allowlist)?,
            json_text(&spec.parameters)?,
            state_json(&spec.state)?,
            status.as_str(),
            next_due_ms,
            now
        ],
    )?;
    Ok(())
}

fn row_to_watch(row: &rusqlite::Row<'_>) -> rusqlite::Result<Watch> {
    let mode: String = row.get(4)?;
    let state: String = row.get(14)?;
    let environment: String = row.get(12)?;
    let parameters: String = row.get(13)?;
    Ok(Watch {
        id: row.get(0)?,
        command: parse_json(row.get::<_, String>(1)?).map_err(to_sql_error)?,
        script_path: PathBuf::from(row.get::<_, String>(2)?),
        working_directory: PathBuf::from(row.get::<_, String>(3)?),
        script_mode: ScriptMode::parse(&mode).map_err(to_sql_error)?,
        script_digest: row.get(5)?,
        sender: row.get(6)?,
        target: row.get(7)?,
        interval_seconds: row.get::<_, i64>(8)? as u64,
        timeout_seconds: row.get::<_, i64>(9)? as u64,
        attention: row.get(10)?,
        requires_disposition: row.get::<_, i64>(11)? != 0,
        environment_allowlist: parse_json(environment).map_err(to_sql_error)?,
        parameters: parse_json(parameters).map_err(to_sql_error)?,
        state: parse_state(&state).map_err(to_sql_error)?,
        status: row.get(15)?,
        next_due_ms: row.get(16)?,
        failure_count: row.get::<_, i64>(17)? as u32,
        last_diagnostic: row.get(18)?,
        updated_at_ms: row.get(19)?,
    })
}

#[allow(clippy::too_many_arguments)]
fn finish_attempt(
    tx: &Transaction<'_>,
    attempt_id: &str,
    outcome: &str,
    result: Option<&Value>,
    event_id: Option<&str>,
    envelope_hash: Option<&str>,
    receipt: Option<&Value>,
    state_committed: bool,
    diagnostic: Option<&str>,
) -> Result<()> {
    tx.execute(
        "UPDATE attempts SET finished_at_ms=?2, outcome=?3, result_json=?4, event_id=?5,
         envelope_hash=?6, receipt_json=?7, state_committed=?8, diagnostic=?9 WHERE id=?1",
        params![
            attempt_id,
            now_ms(),
            outcome,
            result.map(json_text).transpose()?,
            event_id,
            envelope_hash,
            receipt.map(json_text).transpose()?,
            bool_int(state_committed),
            diagnostic
        ],
    )?;
    Ok(())
}

fn prune_attempts(tx: &Transaction<'_>, watch_id: &str) -> Result<()> {
    tx.execute(
        "DELETE FROM attempts WHERE id IN (
           SELECT id FROM attempts WHERE watch_id=?1 ORDER BY started_at_ms DESC LIMIT -1 OFFSET ?2
         )",
        params![watch_id, ATTEMPT_RETENTION],
    )?;
    Ok(())
}

fn increment_revision(tx: &Transaction<'_>) -> Result<()> {
    tx.execute(
        "UPDATE metadata SET value=CAST(value AS INTEGER)+1 WHERE key='revision'",
        [],
    )?;
    Ok(())
}

fn schedule_after(interval_seconds: u64, now: i64) -> i64 {
    now.saturating_add((interval_seconds as i64).saturating_mul(1000))
}

fn backoff_ms(failures: u32) -> i64 {
    let power = failures.saturating_sub(1).min(6);
    let seconds = 5u64.saturating_mul(1u64 << power).min(300);
    (seconds * 1000) as i64
}

/// FNV-1a 64-bit hash of a watch id. Deterministic across restarts and platforms so the same watch
/// receives the same restart jitter without introducing a random-number dependency.
fn stable_hash(value: &str) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in value.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

fn normalize_addresses(spec: &mut WatchSpec) {
    // Approved Plan address normalization: trim surrounding whitespace, preserve case, and let
    // `validate_address` reject empty or internal-whitespace values after trimming.
    spec.sender = spec.sender.trim().to_string();
    spec.target = spec.target.trim().to_string();
}

fn validate_address(field: &str, raw: &str) -> Result<()> {
    if raw.trim().is_empty() || raw.trim() != raw || raw.len() > 1024 {
        bail!("{field} must be a non-empty trimmed Telex address up to 1024 bytes");
    }
    if raw.chars().any(char::is_control) || raw.chars().any(char::is_whitespace) {
        bail!("{field} must not contain whitespace or control characters");
    }
    Ok(())
}

fn valid_env_name(name: &str) -> bool {
    let mut chars = name.chars();
    matches!(chars.next(), Some(first) if first == '_' || first.is_ascii_alphabetic())
        && chars.all(|character| character == '_' || character.is_ascii_alphanumeric())
}

fn json_text<T: Serialize>(value: &T) -> Result<String> {
    Ok(serde_json::to_string(value)?)
}

fn parse_json<T: for<'de> Deserialize<'de>>(value: String) -> Result<T> {
    serde_json::from_str(&value).map_err(|error| anyhow!("invalid registry JSON: {error}"))
}

fn ensure_json_cap(field: &str, value: &Value, cap: usize) -> Result<()> {
    if serde_json::to_vec(value)?.len() > cap {
        bail!("{field} exceeded {cap} serialized bytes");
    }
    Ok(())
}

fn path_text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

#[cfg(windows)]
fn normalized_canonical_path(path: PathBuf) -> PathBuf {
    let text = path.to_string_lossy();
    PathBuf::from(text.strip_prefix(r"\\?\").unwrap_or(&text))
}

#[cfg(not(windows))]
fn normalized_canonical_path(path: PathBuf) -> PathBuf {
    path
}

fn bool_int(value: bool) -> i64 {
    i64::from(value)
}

fn to_sql_error(error: impl std::fmt::Display) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(
        0,
        rusqlite::types::Type::Text,
        Box::new(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            error.to_string(),
        )),
    )
}

pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(i64::MAX)
}

fn default_attention() -> String {
    "background".to_string()
}

fn empty_object() -> Value {
    Value::Object(Default::default())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::ops::Deref;
    use std::sync::atomic::{AtomicU64, Ordering};

    /// A unique scratch directory under the OS temp dir that removes itself on drop, so test runs
    /// never leave repository-local artifacts even if a test panics before its explicit cleanup.
    struct TestDir {
        path: PathBuf,
    }

    impl TestDir {
        fn new(name: &str) -> Self {
            static COUNTER: AtomicU64 = AtomicU64::new(0);
            let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "telex-watcher-registry-{name}-{}-{unique}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).unwrap();
            Self { path }
        }
    }

    impl Deref for TestDir {
        type Target = Path;
        fn deref(&self) -> &Path {
            &self.path
        }
    }

    impl AsRef<Path> for TestDir {
        fn as_ref(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    fn test_dir(name: &str) -> TestDir {
        TestDir::new(name)
    }

    fn fixture_spec(dir: &Path) -> WatchSpec {
        let script = dir.join("detector.cmd");
        fs::write(&script, "@echo off\r\necho {}\r\n").unwrap();
        WatchSpec {
            id: "fixture-watch".into(),
            command: vec![script.to_string_lossy().into_owned()],
            script_path: script,
            working_directory: dir.to_path_buf(),
            script_mode: ScriptMode::FollowPath,
            script_digest: None,
            sender: "service:watcher".into(),
            target: "project:telex".into(),
            interval_seconds: 60,
            timeout_seconds: 10,
            attention: "background".into(),
            requires_disposition: false,
            environment_allowlist: vec![],
            parameters: serde_json::json!({}),
            state: serde_json::json!({"cursor": 1}),
        }
    }

    #[test]
    fn add_trims_surrounding_whitespace_and_preserves_case() {
        let dir = test_dir("addr-trim");
        let mut registry = Registry::open(&dir.join("watcher.sqlite")).unwrap();
        let mut spec = fixture_spec(&dir);
        spec.sender = "  Service:Watcher  ".into();
        spec.target = "\tProject:Telex\n".into();
        let watch = registry.add(spec).unwrap();
        assert_eq!(watch.sender, "Service:Watcher");
        assert_eq!(watch.target, "Project:Telex");
    }

    #[test]
    fn add_rejects_internal_whitespace_after_trimming() {
        let dir = test_dir("addr-internal-ws");
        let mut registry = Registry::open(&dir.join("watcher.sqlite")).unwrap();
        let mut spec = fixture_spec(&dir);
        spec.sender = "  service watcher  ".into();
        let error = registry.add(spec).unwrap_err().to_string();
        assert!(
            error.contains("whitespace"),
            "internal whitespace must be rejected: {error}"
        );
    }

    #[test]
    fn add_rejects_addresses_that_are_only_whitespace() {
        let dir = test_dir("addr-empty-ws");
        let mut registry = Registry::open(&dir.join("watcher.sqlite")).unwrap();
        let mut spec = fixture_spec(&dir);
        spec.target = "   ".into();
        let error = registry.add(spec).unwrap_err().to_string();
        assert!(
            error.contains("non-empty"),
            "whitespace-only address must be rejected: {error}"
        );
    }

    #[test]
    fn update_trims_surrounding_whitespace_before_immutability_check() {
        let dir = test_dir("addr-update-trim");
        let mut registry = Registry::open(&dir.join("watcher.sqlite")).unwrap();
        let base = fixture_spec(&dir);
        let watch = registry.add(base.clone()).unwrap();
        let mut updated = base;
        // Padded values must normalize to the persisted sender/target so the immutability guard
        // treats them as unchanged rather than an attempted reroute.
        updated.sender = "  service:watcher  ".into();
        updated.target = "  project:telex  ".into();
        updated.interval_seconds = 120;
        let result = registry.update(&watch.id, updated).unwrap();
        assert_eq!(result.sender, "service:watcher");
        assert_eq!(result.interval_seconds, 120);
    }

    #[test]
    fn restart_jitter_spreads_overdue_watches_within_deterministic_bounds() {
        let dir = test_dir("jitter-bounds");
        let mut registry = Registry::open(&dir.join("watcher.sqlite")).unwrap();
        let interval = 60u64;
        let ids = [
            "watch-alpha",
            "watch-bravo",
            "watch-charlie",
            "watch-delta",
            "watch-echo",
        ];
        for id in ids {
            let mut spec = fixture_spec(&dir);
            spec.id = id.to_string();
            registry.add(spec).unwrap();
        }
        let revision_before = registry.revision().unwrap();
        let now = now_ms();
        let applied = registry
            .apply_restart_jitter(&BTreeSet::new(), now)
            .unwrap();

        let span = (interval as i64) * 1000 / 10;
        for (id, delay) in &applied {
            // Bounded by 10% of the interval, and exactly the deterministic per-id value.
            assert!(
                *delay > 0 && *delay <= span,
                "{id} delay {delay} outside (0, {span}]"
            );
            let expected = (stable_hash(id) % (span as u64 + 1)) as i64;
            assert_eq!(*delay, expected, "jitter must be deterministic for {id}");
            let watch = registry.get(id).unwrap().unwrap();
            assert_eq!(watch.next_due_ms, now + delay);
        }

        let distinct: BTreeSet<i64> = applied.iter().map(|(_, delay)| *delay).collect();
        assert!(
            distinct.len() > 1,
            "jitter must not schedule every overdue watch identically: {applied:?}"
        );
        assert_eq!(
            registry.revision().unwrap(),
            revision_before,
            "restart jitter must not bump the configuration revision"
        );
    }

    #[test]
    fn restart_jitter_leaves_non_overdue_watches_untouched() {
        let dir = test_dir("jitter-non-overdue");
        let mut registry = Registry::open(&dir.join("watcher.sqlite")).unwrap();
        let mut spec = fixture_spec(&dir);
        spec.id = "future-watch".into();
        let watch = registry.add(spec).unwrap();
        let now = now_ms();
        // Schedule this watch well into the future so it is not overdue at restart.
        let future = now + 10_000_000;
        registry
            .connection
            .execute(
                "UPDATE watches SET next_due_ms=?2 WHERE id=?1",
                params![watch.id, future],
            )
            .unwrap();

        let applied = registry
            .apply_restart_jitter(&BTreeSet::new(), now)
            .unwrap();
        assert!(
            applied.is_empty(),
            "no overdue watch should be spread: {applied:?}"
        );
        assert_eq!(
            registry.get(&watch.id).unwrap().unwrap().next_due_ms,
            future,
            "a non-overdue watch must keep its schedule"
        );
    }

    #[test]
    fn accepted_event_commits_state_and_provenance_together() {
        let dir = test_dir("accepted");
        let mut registry = Registry::open(&dir.join("watcher.sqlite")).unwrap();
        let watch = registry.add(fixture_spec(&dir)).unwrap();
        let attempt = "attempt-1";
        let prior = registry
            .begin_attempt(attempt, &watch, Some("digest"))
            .unwrap();
        registry
            .commit_event(
                &watch,
                attempt,
                "provider:1",
                &prior,
                serde_json::json!({"cursor": 2}),
                "envelope",
                "digest",
                42,
                &serde_json::json!({"receipt": "delivered"}),
                false,
                &serde_json::json!({"outcome": "event"}),
            )
            .unwrap();

        assert_eq!(
            registry.get(&watch.id).unwrap().unwrap().state,
            serde_json::json!({"cursor": 2})
        );
        assert_eq!(registry.events(&watch.id, 10).unwrap().len(), 1);
        assert!(registry.attempts(&watch.id, 10).unwrap()[0].state_committed);
        drop(registry);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn failure_does_not_advance_state() {
        let dir = test_dir("failure");
        let mut registry = Registry::open(&dir.join("watcher.sqlite")).unwrap();
        let watch = registry.add(fixture_spec(&dir)).unwrap();
        registry
            .begin_attempt("attempt-1", &watch, Some("digest"))
            .unwrap();
        registry
            .record_failure(
                &watch,
                "attempt-1",
                "send-failed",
                "Telex unavailable",
                None,
                Some("digest"),
            )
            .unwrap();
        let after = registry.get(&watch.id).unwrap().unwrap();
        assert_eq!(after.state, serde_json::json!({"cursor": 1}));
        assert_eq!(after.failure_count, 1);
        drop(registry);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn existing_event_id_never_advances_a_newly_proposed_state() {
        let dir = test_dir("duplicate");
        let mut registry = Registry::open(&dir.join("watcher.sqlite")).unwrap();
        let watch = registry.add(fixture_spec(&dir)).unwrap();
        let first_prior = registry
            .begin_attempt("attempt-1", &watch, Some("digest"))
            .unwrap();
        registry
            .commit_event(
                &watch,
                "attempt-1",
                "provider:1",
                &first_prior,
                serde_json::json!({"cursor": 2}),
                "envelope-a",
                "digest",
                1,
                &serde_json::json!({"receipt": "delivered"}),
                false,
                &serde_json::json!({"outcome": "event"}),
            )
            .unwrap();

        let advanced = registry.get(&watch.id).unwrap().unwrap();
        registry
            .begin_attempt("attempt-2", &advanced, Some("digest"))
            .unwrap();
        registry
            .finish_noop(
                &advanced,
                "attempt-2",
                "stale-duplicate",
                Some(&serde_json::json!({"outcome": "event"})),
                Some("provider:1"),
                Some("envelope-a"),
                Some("already committed"),
            )
            .unwrap();
        assert_eq!(
            registry.get(&watch.id).unwrap().unwrap().state,
            serde_json::json!({"cursor": 2})
        );

        let after_duplicate = registry.get(&watch.id).unwrap().unwrap();
        registry
            .begin_attempt("attempt-3", &after_duplicate, Some("digest"))
            .unwrap();
        registry
            .finish_noop(
                &after_duplicate,
                "attempt-3",
                "duplicate-event-conflict",
                Some(&serde_json::json!({"outcome": "terminal"})),
                Some("provider:1"),
                Some("envelope-b"),
                Some("different evidence"),
            )
            .unwrap();
        let after_conflict = registry.get(&watch.id).unwrap().unwrap();
        assert_eq!(after_conflict.state, serde_json::json!({"cursor": 2}));
        assert_eq!(after_conflict.status, WatchStatus::Active.as_str());
        drop(registry);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn runtime_session_id_is_persisted_with_runtime_diagnostics() {
        let dir = test_dir("runtime-session-id");
        let mut registry = Registry::open(&dir.join("watcher.sqlite")).unwrap();
        let runtime_id = "runtime-session-id";
        let record_id = registry.runtime_started(runtime_id, 42).unwrap();
        registry
            .runtime_finished(
                record_id,
                "stopped",
                &serde_json::json!({"runtimeSessionId": runtime_id}),
            )
            .unwrap();

        let persisted: String = registry
            .connection
            .query_row(
                "SELECT runtime_session_id FROM runtime_sessions WHERE id=?1",
                [record_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(persisted, runtime_id);
        drop(registry);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn fresh_registry_initializes_schema_version_one() {
        let dir = test_dir("schema-v1");
        let registry = Registry::open(&dir.join("watcher.sqlite")).unwrap();
        let schema: String = registry
            .connection
            .query_row(
                "SELECT value FROM metadata WHERE key='schema_version'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(schema, "1");
        drop(registry);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn unknown_schema_version_is_rejected() {
        let dir = test_dir("schema-unknown");
        let path = dir.join("watcher.sqlite");
        {
            let registry = Registry::open(&path).unwrap();
            registry
                .connection
                .execute(
                    "UPDATE metadata SET value='99' WHERE key='schema_version'",
                    [],
                )
                .unwrap();
        }
        let error = match Registry::open(&path) {
            Ok(_) => panic!("expected unknown schema to be rejected"),
            Err(error) => error.to_string(),
        };
        assert!(
            error.contains("unsupported Watcher registry schema version 99"),
            "unexpected error: {error}"
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn config_mutations_bump_revision_but_attempt_outcomes_do_not() {
        let dir = test_dir("revision");
        let mut registry = Registry::open(&dir.join("watcher.sqlite")).unwrap();

        let base = registry.revision().unwrap();
        let watch = registry.add(fixture_spec(&dir)).unwrap();
        let after_add = registry.revision().unwrap();
        assert!(after_add > base, "add must bump the configuration revision");

        // Attempt outcomes must not bump the revision, otherwise every poll would trigger a
        // full sender reconciliation.
        let prior = registry
            .begin_attempt("attempt-idle", &watch, Some("digest"))
            .unwrap();
        assert_eq!(registry.revision().unwrap(), after_add);
        registry
            .commit_idle(
                &watch,
                "attempt-idle",
                serde_json::json!({"cursor": 2}),
                false,
                &serde_json::json!({"outcome": "idle"}),
            )
            .unwrap();
        assert_eq!(
            registry.revision().unwrap(),
            after_add,
            "idle commit must not bump revision"
        );

        let advanced = registry.get(&watch.id).unwrap().unwrap();
        registry
            .begin_attempt("attempt-fail", &advanced, Some("digest"))
            .unwrap();
        registry
            .record_failure(&advanced, "attempt-fail", "degraded", "backoff", None, None)
            .unwrap();
        assert_eq!(
            registry.revision().unwrap(),
            after_add,
            "failure must not bump revision"
        );

        registry
            .begin_attempt("attempt-event", &advanced, Some("digest"))
            .unwrap();
        registry
            .commit_event(
                &advanced,
                "attempt-event",
                "provider:1",
                &prior,
                serde_json::json!({"cursor": 3}),
                "envelope",
                "digest",
                7,
                &serde_json::json!({"receipt": "delivered"}),
                false,
                &serde_json::json!({"outcome": "event"}),
            )
            .unwrap();
        assert_eq!(
            registry.revision().unwrap(),
            after_add,
            "event commit must not bump revision"
        );

        let current = registry.get(&watch.id).unwrap().unwrap();
        registry
            .begin_attempt("attempt-noop", &current, Some("digest"))
            .unwrap();
        registry
            .finish_noop(
                &current,
                "attempt-noop",
                "stale-duplicate",
                None,
                Some("provider:1"),
                Some("envelope"),
                Some("already committed"),
            )
            .unwrap();
        assert_eq!(
            registry.revision().unwrap(),
            after_add,
            "noop must not bump revision"
        );

        // Lifecycle mutations must bump it so the scheduler recomputes the sender set.
        registry.set_status(&watch.id, WatchStatus::Paused).unwrap();
        let after_pause = registry.revision().unwrap();
        assert!(after_pause > after_add, "pause must bump revision");
        registry.set_status(&watch.id, WatchStatus::Active).unwrap();
        assert!(
            registry.revision().unwrap() > after_pause,
            "resume must bump revision"
        );

        drop(registry);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn committed_state_survives_reopen() {
        let dir = test_dir("restart-recovery");
        let path = dir.join("watcher.sqlite");
        let (watch_id, next_due) = {
            let mut registry = Registry::open(&path).unwrap();
            let watch = registry.add(fixture_spec(&dir)).unwrap();
            registry
                .begin_attempt("attempt-1", &watch, Some("digest"))
                .unwrap();
            registry
                .commit_idle(
                    &watch,
                    "attempt-1",
                    serde_json::json!({"cursor": 9}),
                    false,
                    &serde_json::json!({"outcome": "idle"}),
                )
                .unwrap();
            let stored = registry.get(&watch.id).unwrap().unwrap();
            (watch.id, stored.next_due_ms)
        };

        let reopened = Registry::open(&path).unwrap();
        let recovered = reopened.get(&watch_id).unwrap().unwrap();
        assert_eq!(recovered.state, serde_json::json!({"cursor": 9}));
        assert_eq!(recovered.next_due_ms, next_due);
        assert_eq!(recovered.status, WatchStatus::Active.as_str());
        drop(reopened);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn startup_reconciles_interrupted_runtime_and_delays_retry() {
        let dir = test_dir("startup-reconcile");
        let mut registry = Registry::open(&dir.join("watcher.sqlite")).unwrap();
        let watch = registry.add(fixture_spec(&dir)).unwrap();

        // A committed event establishes detector state + a ledger row that must survive untouched.
        let prior = registry
            .begin_attempt("attempt-committed", &watch, Some("digest"))
            .unwrap();
        registry
            .commit_event(
                &watch,
                "attempt-committed",
                "provider:1",
                &prior,
                serde_json::json!({"cursor": 2}),
                "envelope",
                "digest",
                42,
                &serde_json::json!({"receipt": "delivered"}),
                false,
                &serde_json::json!({"outcome": "event"}),
            )
            .unwrap();

        // A second attempt is left unfinished, and a runtime row is left running: the wreckage of
        // an abruptly killed Watcher (PID 59916) that never recorded a clean shutdown.
        let current = registry.get(&watch.id).unwrap().unwrap();
        registry
            .begin_attempt("attempt-stale", &current, Some("digest"))
            .unwrap();
        registry.runtime_started("old-runtime", 59916).unwrap();

        let before = now_ms();
        let reconciled = registry.reconcile_interrupted_runtimes().unwrap();

        assert_eq!(
            reconciled.runtime_session_ids,
            vec!["old-runtime".to_string()]
        );
        assert_eq!(reconciled.attempt_ids, vec!["attempt-stale".to_string()]);
        assert_eq!(reconciled.watch_ids, vec![watch.id.clone()]);

        // The stale runtime row is now interrupted with an end time.
        let (status, ended): (String, Option<i64>) = registry
            .connection
            .query_row(
                "SELECT status, ended_at_ms FROM runtime_sessions WHERE runtime_session_id='old-runtime'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(status, "interrupted");
        assert!(
            ended.is_some(),
            "interrupted runtime must record an end time"
        );

        // The unfinished attempt is a visible runtime-interrupted failure with no commit/receipt.
        let attempts = registry.attempts(&watch.id, 10).unwrap();
        let stale = attempts.iter().find(|a| a.id == "attempt-stale").unwrap();
        assert_eq!(stale.outcome.as_deref(), Some("runtime-interrupted"));
        assert!(stale.finished_at_ms.is_some());
        assert!(!stale.state_committed);
        assert!(stale.receipt_json.is_none());
        assert!(stale.event_id.is_none());
        assert!(stale.diagnostic.is_some());

        // The already-committed attempt is untouched.
        let committed = attempts
            .iter()
            .find(|a| a.id == "attempt-committed")
            .unwrap();
        assert!(committed.state_committed);

        // Detector state and the ledger are preserved.
        let after = registry.get(&watch.id).unwrap().unwrap();
        assert_eq!(after.state, serde_json::json!({"cursor": 2}));
        let events = registry.events(&watch.id, 10).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_id, "provider:1");

        // Failure is counted once and the retry is fenced by at least timeout + normal backoff so a
        // surviving detector descendant cannot immediately overlap the fresh runtime.
        assert_eq!(after.failure_count, 1);
        let fence = (fixture_spec(&dir).timeout_seconds as i64) * 1000 + backoff_ms(1);
        assert!(
            after.next_due_ms - before >= fence,
            "next_due delay {} must be at least the timeout+backoff fence {fence}",
            after.next_due_ms - before
        );

        drop(registry);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn clean_startup_with_no_stale_rows_is_a_noop() {
        let dir = test_dir("startup-clean");
        let mut registry = Registry::open(&dir.join("watcher.sqlite")).unwrap();
        let watch = registry.add(fixture_spec(&dir)).unwrap();

        // A fully finished attempt and a cleanly ended runtime leave nothing to reconcile.
        registry
            .begin_attempt("attempt-1", &watch, Some("digest"))
            .unwrap();
        registry
            .commit_idle(
                &watch,
                "attempt-1",
                serde_json::json!({"cursor": 3}),
                false,
                &serde_json::json!({"outcome": "idle"}),
            )
            .unwrap();
        let record = registry.runtime_started("prior-runtime", 4242).unwrap();
        registry
            .runtime_finished(record, "shutdown", &serde_json::json!({"reason": "clean"}))
            .unwrap();

        let before = registry.get(&watch.id).unwrap().unwrap();
        let reconciled = registry.reconcile_interrupted_runtimes().unwrap();
        assert!(
            reconciled.is_empty(),
            "clean startup must reconcile nothing"
        );

        let after = registry.get(&watch.id).unwrap().unwrap();
        assert_eq!(after.failure_count, before.failure_count);
        assert_eq!(after.next_due_ms, before.next_due_ms);
        assert_eq!(after.state, before.state);

        drop(registry);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn idle_commit_resets_failure_backoff() {
        let dir = test_dir("idle-reset");
        let mut registry = Registry::open(&dir.join("watcher.sqlite")).unwrap();
        let watch = registry.add(fixture_spec(&dir)).unwrap();
        registry
            .begin_attempt("attempt-fail", &watch, Some("digest"))
            .unwrap();
        registry
            .record_failure(&watch, "attempt-fail", "degraded", "backoff", None, None)
            .unwrap();
        let failed = registry.get(&watch.id).unwrap().unwrap();
        assert_eq!(failed.failure_count, 1);

        registry
            .begin_attempt("attempt-idle", &failed, Some("digest"))
            .unwrap();
        registry
            .commit_idle(
                &failed,
                "attempt-idle",
                serde_json::json!({"cursor": 5}),
                false,
                &serde_json::json!({"outcome": "idle"}),
            )
            .unwrap();
        let recovered = registry.get(&watch.id).unwrap().unwrap();
        assert_eq!(recovered.failure_count, 0);
        assert_eq!(recovered.state, serde_json::json!({"cursor": 5}));
        drop(registry);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn terminal_without_event_marks_watch_terminal() {
        let dir = test_dir("terminal-noevent");
        let mut registry = Registry::open(&dir.join("watcher.sqlite")).unwrap();
        let watch = registry.add(fixture_spec(&dir)).unwrap();
        registry
            .begin_attempt("attempt-1", &watch, Some("digest"))
            .unwrap();
        registry
            .commit_idle(
                &watch,
                "attempt-1",
                serde_json::json!({"cursor": 2}),
                true,
                &serde_json::json!({"outcome": "terminal"}),
            )
            .unwrap();
        assert_eq!(
            registry.get(&watch.id).unwrap().unwrap().status,
            WatchStatus::Terminal.as_str()
        );
        drop(registry);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn terminal_event_commit_marks_watch_terminal_with_ledger() {
        let dir = test_dir("terminal-event");
        let mut registry = Registry::open(&dir.join("watcher.sqlite")).unwrap();
        let watch = registry.add(fixture_spec(&dir)).unwrap();
        let prior = registry
            .begin_attempt("attempt-1", &watch, Some("digest"))
            .unwrap();
        registry
            .commit_event(
                &watch,
                "attempt-1",
                "provider:final",
                &prior,
                serde_json::json!({"cursor": 2}),
                "envelope",
                "digest",
                11,
                &serde_json::json!({"receipt": "queued-unoccupied"}),
                true,
                &serde_json::json!({"outcome": "terminal"}),
            )
            .unwrap();
        let stored = registry.get(&watch.id).unwrap().unwrap();
        assert_eq!(stored.status, WatchStatus::Terminal.as_str());
        assert_eq!(registry.events(&watch.id, 10).unwrap().len(), 1);
        drop(registry);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn configured_senders_includes_paused_and_excludes_removed() {
        let dir = test_dir("configured-senders");
        let mut registry = Registry::open(&dir.join("watcher.sqlite")).unwrap();

        let mut active = fixture_spec(&dir);
        active.id = "active-watch".into();
        active.sender = "service:active".into();
        registry.add(active).unwrap();

        let mut paused = fixture_spec(&dir);
        paused.id = "paused-watch".into();
        paused.sender = "service:paused".into();
        registry.add(paused).unwrap();
        registry
            .set_status("paused-watch", WatchStatus::Paused)
            .unwrap();

        let mut removed = fixture_spec(&dir);
        removed.id = "removed-watch".into();
        removed.sender = "service:removed".into();
        registry.add(removed).unwrap();
        registry.remove("removed-watch").unwrap();

        let senders = registry.configured_senders().unwrap();
        assert!(senders.contains(&"service:active".to_string()));
        assert!(
            senders.contains(&"service:paused".to_string()),
            "paused watches must keep their sender attached"
        );
        assert!(
            !senders.contains(&"service:removed".to_string()),
            "removed watches must release their sender"
        );
        drop(registry);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn update_cannot_change_sender_or_target() {
        let dir = test_dir("update-reroute");
        let mut registry = Registry::open(&dir.join("watcher.sqlite")).unwrap();
        let watch = registry.add(fixture_spec(&dir)).unwrap();
        let mut rerouted = fixture_spec(&dir);
        rerouted.id = watch.id.clone();
        rerouted.target = "project:other".into();
        let error = registry
            .update(&watch.id, rerouted)
            .unwrap_err()
            .to_string();
        assert!(error.contains("immutable"), "unexpected error: {error}");
        drop(registry);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn pinned_digest_mismatch_is_rejected_at_add() {
        let dir = test_dir("pinned-mismatch");
        let mut registry = Registry::open(&dir.join("watcher.sqlite")).unwrap();
        let mut spec = fixture_spec(&dir);
        spec.script_mode = ScriptMode::Pinned;
        spec.script_digest = Some("0".repeat(64));
        let error = registry.add(spec).unwrap_err().to_string();
        assert!(
            error.contains("pinned scriptDigest does not match"),
            "unexpected error: {error}"
        );
        drop(registry);
        let _ = fs::remove_dir_all(dir);
    }
}
