//! Marked snapshots cannot become live registries without an explicit offline restore.

use super::*;
use crate::maintenance::{copy_snapshot, BackupReport};
use std::collections::BTreeMap;
use std::path::PathBuf;
use telex::platform_fs;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BackupMetadata {
    pub version: u32,
    pub state: String,
    pub source_path: PathBuf,
    pub source_physical_identity: String,
    pub source_responsibility: String,
    pub host_id: String,
    pub registry_schema: u32,
    pub registry_revision: u64,
    pub created_at: String,
    pub content_hash: String,
    pub high_water: BTreeMap<String, Value>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RestoreReport {
    pub version: u32,
    pub original_evidence: PathBuf,
    pub source_matches_snapshot: bool,
    pub fenced_watches: u64,
}

pub(super) fn check_startup(connection: &Connection) -> Result<()> {
    if let Some(metadata) = metadata(connection)? {
        anyhow::ensure!(metadata.state == "restored", "backup-restore-required");
    }
    Ok(())
}

fn metadata(connection: &Connection) -> Result<Option<BackupMetadata>> {
    let exists: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='watcher_v2_backup_meta')",
        [],
        |row| row.get(0),
    )?;
    if !exists {
        return Ok(None);
    }
    read_one(
        connection,
        "SELECT data FROM watcher_v2_backup_meta WHERE singleton=1",
        [],
    )
}

fn save_metadata(connection: &Connection, metadata: &BackupMetadata) -> Result<()> {
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS watcher_v2_backup_meta (
           singleton INTEGER PRIMARY KEY CHECK(singleton=1), data TEXT NOT NULL);",
    )?;
    connection.execute(
        "INSERT INTO watcher_v2_backup_meta VALUES(1,?1)
         ON CONFLICT(singleton) DO UPDATE SET data=excluded.data",
        [serde_json::to_string(metadata)?],
    )?;
    Ok(())
}

fn content_hash(connection: &Connection) -> Result<String> {
    use sha2::{Digest, Sha256};
    let mut digest = Sha256::new();
    for (table, order) in [
        ("watcher_v2_meta", "singleton"),
        ("watcher_v2_watches", "id"),
        ("watcher_v2_revisions", "watch_id,revision"),
        ("watcher_v2_attempts", "id"),
        ("watcher_v2_operations", "id"),
        ("watcher_v2_audits", "id"),
        ("watcher_v2_operation_evidence", "id"),
        ("watcher_v2_compaction", "watch_id"),
    ] {
        digest.update(table.as_bytes());
        let query = if table == "watcher_v2_meta" {
            "SELECT json_array(version,revision,responsibility) FROM watcher_v2_meta ORDER BY singleton".to_owned()
        } else {
            format!("SELECT data FROM {table} ORDER BY {order}")
        };
        let mut statement = connection.prepare(&query)?;
        let mut rows = statement.query([])?;
        while let Some(row) = rows.next()? {
            let data: String = row.get(0)?;
            digest.update((data.len() as u64).to_be_bytes());
            digest.update(data.as_bytes());
        }
    }
    Ok(format!(
        "sha256:{}",
        digest
            .finalize()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    ))
}

fn integrity(connection: &Connection) -> Result<()> {
    let check: String = connection.query_row("PRAGMA integrity_check", [], |row| row.get(0))?;
    anyhow::ensure!(check == "ok", "backup-integrity-failed");
    Ok(())
}

fn canonical_new_path(path: &Path) -> Result<PathBuf> {
    anyhow::ensure!(path.is_absolute(), "backup-path-must-be-absolute");
    let parent = path
        .parent()
        .context("backup parent missing")?
        .canonicalize()
        .context("backup parent must already exist")?;
    let path = parent.join(path.file_name().context("backup filename missing")?);
    anyhow::ensure!(
        !platform_fs::path_present(&path)?,
        "backup-destination-exists"
    );
    no_sidecars(&path)?;
    Ok(path)
}

fn no_sidecars(path: &Path) -> Result<()> {
    for suffix in ["-wal", "-shm", "-journal"] {
        anyhow::ensure!(
            !platform_fs::path_present(&PathBuf::from(format!("{}{suffix}", path.display())))?,
            "backup-sidecar-present"
        );
    }
    Ok(())
}

/// Publish without replacement, with no overwrite fallback.
fn publish_snapshot(source: &Connection, path: &Path, metadata: &BackupMetadata) -> Result<()> {
    let temporary = path.with_file_name(format!(".watcher-backup-{}.tmp", Uuid::new_v4()));
    platform_fs::write_owner_only_file_exact(&temporary, b"")?;
    let result = (|| -> Result<()> {
        let mut destination = Connection::open(&temporary)?;
        copy_snapshot(source, &mut destination)?;
        destination
            .execute_batch("PRAGMA journal_mode=DELETE; PRAGMA synchronous=FULL; VACUUM;")?;
        save_metadata(&destination, metadata)?;
        integrity(&destination)?;
        destination.close().map_err(|(_, error)| error)?;
        no_sidecars(&temporary)?;
        no_sidecars(path)?;
        platform_fs::stat_owner_only_file(&temporary, u64::MAX)?;
        sync_file(&temporary)?;
        publish_new(&temporary, path)?;
        let published =
            Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        integrity(&published)?;
        published.close().map_err(|(_, error)| error)?;
        no_sidecars(path)?;
        sync_file(path)?;
        #[cfg(unix)]
        std::fs::File::open(path.parent().context("backup parent missing")?)?.sync_all()?;
        Ok(())
    })();
    if temporary.exists() {
        std::fs::remove_file(&temporary).context("remove private backup temporary file")?;
    }
    result
}

fn sync_file(path: &Path) -> Result<()> {
    std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)?
        .sync_all()?;
    Ok(())
}

#[cfg(unix)]
fn publish_new(temporary: &Path, path: &Path) -> Result<()> {
    std::fs::hard_link(temporary, path)
        .context("backup-publication-failed: no-replace hard link required")?;
    Ok(())
}

#[cfg(windows)]
fn publish_new(temporary: &Path, path: &Path) -> Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{MoveFileExW, MOVEFILE_WRITE_THROUGH};
    let from: Vec<u16> = temporary.as_os_str().encode_wide().chain(Some(0)).collect();
    let to: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    anyhow::ensure!(
        unsafe { MoveFileExW(from.as_ptr(), to.as_ptr(), MOVEFILE_WRITE_THROUGH) } != 0,
        "backup-publication-failed: {}",
        std::io::Error::last_os_error()
    );
    Ok(())
}

impl Registry {
    pub fn backup(&self, path: &Path) -> Result<BackupReport> {
        audit(
            &self.connection,
            None,
            "backup",
            &json!({"version":1,"path":path}),
        )?;
        let path = canonical_new_path(path)?;
        for suffix in ["-wal", "-shm", "-journal"] {
            anyhow::ensure!(
                path != Path::new(&format!("{}{suffix}", self._ownership.path.display())),
                "backup-path-is-registry-sidecar"
            );
        }
        let watches = self.list()?;
        let metadata = BackupMetadata {
            version: 1,
            state: "backup".into(),
            source_path: self._ownership.path.clone(),
            source_physical_identity: crate::ownership::physical_identity(&self._ownership.path)?,
            source_responsibility: self.responsibility.clone(),
            host_id: platform_fs::host_id()?,
            registry_schema: 3,
            registry_revision: self.revision()?,
            created_at: timestamp(now_ms()),
            content_hash: content_hash(&self.connection)?,
            high_water: watches
                .iter()
                .map(|w| {
                    (
                        w.id.clone(),
                        json!({
                            "allocated":w.last_allocated_event_sequence,
                            "committed":w.last_committed_event_sequence,
                            "pendingOperationId":w.pending_operation_id,
                            "backendProfile":w.registration.as_ref().map(|r| &r.backend_profile),
                            "logicalStoreId":w.logical_store_id
                        }),
                    )
                })
                .collect(),
        };
        publish_snapshot(&self.connection, &path, &metadata)?;
        Ok(BackupReport {
            version: 1,
            bytes: std::fs::metadata(&path)?.len(),
            path,
            registry_revision: metadata.registry_revision,
        })
    }
}

/// Restore into the original physical file: its ownership lock never changes.
pub fn restore(registry_path: &Path, backup_path: &Path) -> Result<RestoreReport> {
    restore_with_progress(registry_path, backup_path, |_| {})
}

pub(crate) fn restore_with_progress(
    registry_path: &Path,
    backup_path: &Path,
    mut progress: impl FnMut(&str),
) -> Result<RestoreReport> {
    anyhow::ensure!(
        registry_path.is_absolute() && backup_path.is_absolute(),
        "restore-paths-must-be-absolute"
    );
    anyhow::ensure!(
        registry_path.is_file() && backup_path.is_file(),
        "restore-files-must-exist"
    );
    let original_ownership = Ownership::acquire(registry_path)?;
    let backup_ownership = Ownership::acquire(backup_path)?;
    platform_fs::stat_owner_only_file(&backup_ownership.path, u64::MAX)?;
    let mut original = Connection::open(&original_ownership.path)?;
    let source = Connection::open_with_flags(
        &backup_ownership.path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
    no_sidecars(&backup_ownership.path)?;
    integrity(&source)?;
    let source_metadata = metadata(&source)?.context("restore-source-not-marked")?;
    anyhow::ensure!(
        source_metadata.version == 1 && source_metadata.state == "backup",
        "restore-source-not-marked"
    );
    anyhow::ensure!(
        source_metadata.host_id == platform_fs::host_id()?,
        "restore-host-mismatch"
    );
    let (source_version, source_responsibility): (u32, String) = source.query_row(
        "SELECT version,responsibility FROM watcher_v2_meta WHERE singleton=1",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    anyhow::ensure!(
        source_version == source_metadata.registry_schema,
        "restore-schema-mismatch"
    );
    anyhow::ensure!(
        source_responsibility == source_metadata.source_responsibility,
        "restore-responsibility-mismatch"
    );
    anyhow::ensure!(
        content_hash(&source)? == source_metadata.content_hash,
        "restore-content-mismatch"
    );
    anyhow::ensure!(
        source_metadata.source_path == original_ownership.path
            && source_metadata.source_physical_identity
                == crate::ownership::physical_identity(&original_ownership.path)?,
        "restore-source-identity-mismatch"
    );
    let (version, responsibility): (u32, String) = original.query_row(
        "SELECT version,responsibility FROM watcher_v2_meta WHERE singleton=1",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    anyhow::ensure!(
        version == 3 && source_metadata.registry_schema == 3,
        "restore-schema-mismatch"
    );
    anyhow::ensure!(
        responsibility == source_metadata.source_responsibility,
        "restore-responsibility-mismatch"
    );
    let page_size = |connection: &Connection| -> Result<u64> {
        Ok(connection.query_row("PRAGMA page_size", [], |row| row.get(0))?)
    };
    anyhow::ensure!(
        page_size(&original)? == page_size(&source)?,
        "restore-page-size-mismatch"
    );
    let original_watches: Vec<Watch> =
        read_many(&original, "SELECT data FROM watcher_v2_watches", [])?;
    let snapshot_watches: Vec<Watch> =
        read_many(&source, "SELECT data FROM watcher_v2_watches", [])?;
    for old in &snapshot_watches {
        if let Some(current) = original_watches.iter().find(|w| w.id == old.id) {
            anyhow::ensure!(
                current.logical_store_id == old.logical_store_id
                    && current.registration.as_ref().map(|r| (
                        &r.backend_profile,
                        &r.sender,
                        &r.target
                    )) == old.registration.as_ref().map(|r| (
                        &r.backend_profile,
                        &r.sender,
                        &r.target
                    )),
                "restore-backend-route-mismatch"
            );
        }
    }
    let source_matches_snapshot = content_hash(&original)? == source_metadata.content_hash;
    let original_evidence = original_ownership.path.with_file_name(format!(
        "{}.pre-restore-{}",
        original_ownership
            .path
            .file_name()
            .context("registry filename missing")?
            .to_string_lossy(),
        Uuid::new_v4()
    ));
    let mut evidence_metadata = source_metadata.clone();
    evidence_metadata.content_hash = content_hash(&original)?;
    evidence_metadata.created_at = timestamp(now_ms());
    evidence_metadata.registry_revision = original.query_row(
        "SELECT revision FROM watcher_v2_meta WHERE singleton=1",
        [],
        |row| row.get(0),
    )?;
    evidence_metadata.high_water = original_watches
        .iter()
        .map(|w| {
            (
                w.id.clone(),
                json!({
                    "allocated":w.last_allocated_event_sequence,
                    "committed":w.last_committed_event_sequence,
                    "pendingOperationId":w.pending_operation_id,
                    "backendProfile":w.registration.as_ref().map(|r| &r.backend_profile),
                    "logicalStoreId":w.logical_store_id
                }),
            )
        })
        .collect();
    progress("before-evidence");
    publish_snapshot(&original, &original_evidence, &evidence_metadata)?;
    progress("evidence-published");

    // Prepare the fenced image before the atomic SQLite destination transaction.
    let staging = original_ownership
        .path
        .with_file_name(format!(".watcher-restore-{}.tmp", Uuid::new_v4()));
    platform_fs::write_owner_only_file_exact(&staging, b"")?;
    let result = (|| -> Result<RestoreReport> {
        let mut image = Connection::open(&staging)?;
        copy_snapshot(&source, &mut image)?;
        let tx = image.transaction()?;
        let mut fenced_watches = 0;
        for mut watch in snapshot_watches {
            if matches!(watch.lifecycle, Lifecycle::Active | Lifecycle::Paused) {
                watch.restore_review_required = true;
                restore_fence(&mut watch);
                if let Some(id) = &watch.pending_operation_id {
                    let mut operation =
                        get_operation(&tx, id)?.context("restore pending operation missing")?;
                    operation.status = OperationStatus::QueryRequired;
                    operation.next_recovery_ms = now_ms();
                    save_operation(&tx, &operation)?;
                }
                save_watch(&tx, &watch)?;
                fenced_watches += 1;
            }
        }
        let mut restored_metadata = source_metadata;
        restored_metadata.state = "restored".into();
        save_metadata(&tx, &restored_metadata)?;
        audit(
            &tx,
            None,
            "restore",
            &json!({
                "version":1,"sourceMatchesSnapshot":source_matches_snapshot,
                "originalEvidence":original_evidence,"fencedWatches":fenced_watches
            }),
        )?;
        tx.commit()?;
        integrity(&image)?;
        progress("image-prepared");
        original.execute_batch("PRAGMA synchronous=FULL;")?;
        crate::maintenance::copy_snapshot_with_progress(&image, &mut original, || {
            progress("copying")
        })?;
        progress("committed");
        integrity(&original)?;
        image.close().map_err(|(_, error)| error)?;
        Ok(RestoreReport {
            version: 1,
            original_evidence,
            source_matches_snapshot,
            fenced_watches,
        })
    })();
    std::fs::remove_file(&staging).context("remove private restore staging file")?;
    result
}
