//! Versioned, conservative maintenance of the registry's disposable history.

use super::*;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

pub const COMPACTION_VERSION: u32 = 1;
pub const COMPLETED_ATTEMPTS_PER_WATCH: u32 = 100;
pub const COMPACTION_BATCH_LIMIT: u32 = 256;
pub const RUNTIME_HISTORY_LIMIT: u32 = 10;

pub fn failure_code(error: &anyhow::Error) -> Option<&'static str> {
    const CODES: &[&str] = &[
        "backup-restore-required",
        "backup-publication-failed",
        "backup-integrity-failed",
        "backup-sidecar-present",
        "backup-path-must-be-absolute",
        "backup-destination-exists",
        "backup-path-is-registry-sidecar",
        "restore-paths-must-be-absolute",
        "restore-files-must-exist",
        "restore-source-not-marked",
        "restore-host-mismatch",
        "restore-source-identity-mismatch",
        "restore-schema-mismatch",
        "restore-responsibility-mismatch",
        "restore-page-size-mismatch",
        "restore-backend-route-mismatch",
        "restore-content-mismatch",
        "registry-already-owned",
        "unsupported-registry-version",
        "unsupported-compaction-version",
        "backup-failed",
        "compaction-failed",
    ];
    for cause in error.chain() {
        let message = cause.to_string();
        if let Some(code) = CODES.iter().copied().find(|code| {
            message == *code
                || message
                    .strip_prefix(code)
                    .is_some_and(|tail| tail.starts_with(':'))
        }) {
            // More specific causes take precedence over operation context.
            if !matches!(code, "backup-failed" | "compaction-failed") {
                return Some(code);
            }
        }
    }
    CODES
        .iter()
        .copied()
        .find(|code| error.chain().any(|cause| cause.to_string() == *code))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompactionStats {
    pub version: u32,
    pub completed_attempts_per_watch: u32,
    pub batch_limit: u32,
    pub runtime_history_limit: u32,
    pub attempts: u64,
    pub runtimes: u64,
    pub bytes: u64,
    pub outcomes: BTreeMap<String, u64>,
    pub diagnostic_categories: BTreeMap<String, u64>,
}

impl Default for CompactionStats {
    fn default() -> Self {
        Self {
            version: COMPACTION_VERSION,
            completed_attempts_per_watch: COMPLETED_ATTEMPTS_PER_WATCH,
            batch_limit: COMPACTION_BATCH_LIMIT,
            runtime_history_limit: RUNTIME_HISTORY_LIMIT,
            attempts: 0,
            runtimes: 0,
            bytes: 0,
            outcomes: BTreeMap::new(),
            diagnostic_categories: BTreeMap::new(),
        }
    }
}

impl CompactionStats {
    fn add(&mut self, other: Self) -> Result<()> {
        anyhow::ensure!(
            other.version == COMPACTION_VERSION,
            "unsupported-compaction-version"
        );
        self.attempts = self
            .attempts
            .checked_add(other.attempts)
            .context("attempt counter exhausted")?;
        self.runtimes = self
            .runtimes
            .checked_add(other.runtimes)
            .context("runtime counter exhausted")?;
        self.bytes = self
            .bytes
            .checked_add(other.bytes)
            .context("byte counter exhausted")?;
        for (key, value) in other.outcomes {
            let count = self.outcomes.entry(key).or_default();
            *count = count
                .checked_add(value)
                .context("outcome counter exhausted")?;
        }
        for (key, value) in other.diagnostic_categories {
            let count = self.diagnostic_categories.entry(key).or_default();
            *count = count
                .checked_add(value)
                .context("diagnostic counter exhausted")?;
        }
        Ok(())
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompactionReport {
    pub version: u32,
    pub attempts_deleted: u64,
    pub runtimes_deleted: u64,
    pub bytes_deleted: u64,
    pub totals: CompactionStats,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BackupReport {
    pub version: u32,
    pub path: PathBuf,
    pub bytes: u64,
    pub registry_revision: u64,
}

impl Registry {
    pub(super) fn initialize_maintenance(&self) -> Result<()> {
        self.connection.execute_batch(
            "CREATE TABLE IF NOT EXISTS watcher_v2_compaction (
                watch_id TEXT PRIMARY KEY, version INTEGER NOT NULL, data TEXT NOT NULL);
             CREATE INDEX IF NOT EXISTS watcher_v2_operation_attempt
                ON watcher_v2_operations(json_extract(data,'$.attemptId'));
             CREATE INDEX IF NOT EXISTS watcher_v2_attempt_containment_runtime
                ON watcher_v2_attempts(json_extract(data,'$.runtimeId'))
                WHERE json_extract(data,'$.containment') IS NOT NULL;",
        )?;
        let unsupported: bool = self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM watcher_v2_compaction WHERE version<>?1)",
            [COMPACTION_VERSION],
            |row| row.get(0),
        )?;
        anyhow::ensure!(!unsupported, "unsupported-compaction-version");
        Ok(())
    }

    pub fn compaction_stats(&self, watch_id: Option<&str>) -> Result<CompactionStats> {
        let mut total = CompactionStats::default();
        let values: Vec<CompactionStats> = read_many(
            &self.connection,
            "SELECT data FROM watcher_v2_compaction WHERE ?1 IS NULL OR watch_id=?1",
            [watch_id],
        )?;
        for value in values {
            total.add(value)?;
        }
        Ok(total)
    }

    /// One bounded transaction; proof-bearing records are never compacted.
    pub fn compact(&mut self) -> Result<CompactionReport> {
        self.compact_with_audit(true)
    }

    pub(crate) fn compact_automatic(&mut self) -> Result<CompactionReport> {
        self.compact_with_audit(false)
    }

    fn compact_with_audit(&mut self, manual: bool) -> Result<CompactionReport> {
        let started = Instant::now();
        self.connection.progress_handler(
            1000,
            Some(move || started.elapsed() > Duration::from_secs(2)),
        );
        let result = self.compact_batch(manual);
        self.connection.progress_handler(0, None::<fn() -> bool>);
        result
    }

    fn compact_batch(&mut self, manual: bool) -> Result<CompactionReport> {
        let tx = self.connection.transaction()?;
        let candidates = {
            let mut query = tx.prepare(
                "WITH completed AS (
                    SELECT id,watch_id,started,data,
                           row_number() OVER (PARTITION BY watch_id ORDER BY started DESC,id DESC) AS position
                    FROM watcher_v2_attempts
                    WHERE json_extract(data,'$.finishedAtMs') IS NOT NULL
                      AND json_extract(data,'$.outcome')<>'running'
                )
                SELECT a.id,a.watch_id,json_extract(a.data,'$.outcome'),
                       json_extract(a.data,'$.diagnosticCategory'),length(CAST(a.data AS BLOB))
                FROM completed a JOIN watcher_v2_watches w ON w.id=a.watch_id
                WHERE a.position>?1
                  AND a.id IS NOT json_extract(w.data,'$.lastAttemptId')
                  AND a.id IS NOT json_extract(w.data,'$.runningAttemptId')
                  AND a.id IS NOT json_extract(w.data,'$.latestFailure.attemptId')
                  AND NOT EXISTS (
                    SELECT 1 FROM watcher_v2_operations o
                    WHERE o.watch_id=a.watch_id AND json_extract(o.data,'$.attemptId')=a.id
                      AND json_extract(o.data,'$.status') NOT IN
                        ('accepted','closed-not-accepted','removed-not-recorded','removed-rejected'))
                ORDER BY a.started,a.id LIMIT ?2",
            )?;
            let rows = query
                .query_map(
                    params![COMPLETED_ATTEMPTS_PER_WATCH, COMPACTION_BATCH_LIMIT],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, Option<String>>(3)?,
                            row.get::<_, u64>(4)?,
                        ))
                    },
                )?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            rows
        };
        let mut deltas: BTreeMap<String, CompactionStats> = BTreeMap::new();
        let mut attempts_deleted = 0;
        let mut bytes_deleted = 0;
        for (id, watch, outcome, category, bytes) in candidates {
            let delta = deltas.entry(watch).or_default();
            delta.attempts += 1;
            delta.bytes += bytes;
            *delta.outcomes.entry(outcome).or_default() += 1;
            if let Some(category) = category {
                *delta.diagnostic_categories.entry(category).or_default() += 1;
            }
            tx.execute(
                "UPDATE watcher_v2_operations SET data=json_set(data,'$.attemptCompacted',json('true'))
                 WHERE json_extract(data,'$.attemptId')=?1",
                [&id],
            )?;
            tx.execute("DELETE FROM watcher_v2_attempts WHERE id=?1", [id])?;
            attempts_deleted += 1;
            bytes_deleted += bytes;
        }
        let old_runtimes = {
            let mut query = tx.prepare(
                "SELECT id,length(CAST(data AS BLOB)) FROM watcher_v2_runtimes
                 WHERE json_extract(data,'$.status')<>'running'
                   AND NOT EXISTS (
                     SELECT 1 FROM watcher_v2_attempts a
                     WHERE json_extract(a.data,'$.runtimeId')=watcher_v2_runtimes.id
                       AND json_extract(a.data,'$.containment') IS NOT NULL)
                 ORDER BY json_extract(data,'$.startedAt') DESC,id DESC LIMIT ?1 OFFSET ?2",
            )?;
            let rows = query
                .query_map(
                    params![
                        u64::from(COMPACTION_BATCH_LIMIT) - attempts_deleted,
                        RUNTIME_HISTORY_LIMIT
                    ],
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, u64>(1)?)),
                )?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            rows
        };
        let runtimes_deleted = old_runtimes.len() as u64;
        for (id, bytes) in old_runtimes {
            let delta = deltas.entry(String::new()).or_default();
            delta.runtimes += 1;
            delta.bytes += bytes;
            bytes_deleted += bytes;
            tx.execute("DELETE FROM watcher_v2_runtimes WHERE id=?1", [id])?;
        }
        for (watch, delta) in deltas {
            let mut total: CompactionStats = read_one(
                &tx,
                "SELECT data FROM watcher_v2_compaction WHERE watch_id=?1",
                [&watch],
            )?
            .unwrap_or_default();
            total.add(delta)?;
            tx.execute(
                "INSERT INTO watcher_v2_compaction(watch_id,version,data) VALUES(?1,?2,?3)
                 ON CONFLICT(watch_id) DO UPDATE SET data=excluded.data",
                params![watch, COMPACTION_VERSION, serde_json::to_string(&total)?],
            )?;
        }
        if manual || attempts_deleted > 0 || runtimes_deleted > 0 {
            audit(
                &tx,
                None,
                "compact",
                &json!({
                    "version":COMPACTION_VERSION, "attemptsDeleted":attempts_deleted,
                    "runtimesDeleted":runtimes_deleted,"bytesDeleted":bytes_deleted
                }),
            )?;
        }
        tx.commit()?;
        Ok(CompactionReport {
            version: COMPACTION_VERSION,
            attempts_deleted,
            runtimes_deleted,
            bytes_deleted,
            totals: self.compaction_stats(None)?,
        })
    }
}

pub(super) fn copy_snapshot(source: &Connection, destination: &mut Connection) -> Result<()> {
    copy_snapshot_with_progress(source, destination, || {})
}

pub(super) fn copy_snapshot_with_progress(
    source: &Connection,
    destination: &mut Connection,
    mut progress: impl FnMut(),
) -> Result<()> {
    let backup = rusqlite::backup::Backup::new(source, destination)?;
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        match backup.step(128)? {
            rusqlite::backup::StepResult::Done => break,
            rusqlite::backup::StepResult::More => progress(),
            rusqlite::backup::StepResult::Busy | rusqlite::backup::StepResult::Locked => {
                std::thread::sleep(Duration::from_millis(5));
            }
            _ => bail!("unsupported SQLite backup state"),
        }
        anyhow::ensure!(Instant::now() < deadline, "registry backup timed out");
    }
    Ok(())
}
