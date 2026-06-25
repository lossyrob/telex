//! SQLite backend: the zero-config local substrate. WAL mode plus a busy timeout
//! make multiple processes (holders, senders, waiters) safe on one file. Liveness is
//! TTL-heartbeat; delivery is poll-with-cursor. Validated for multi-process use in the spike.
//!
//! Schema version history:
//!   v0 — original schema (no epoch columns, no telex_schema_version table)
//!   v2 — epoch-aware leases (`lease_epoch INTEGER NOT NULL`, `owner_instance_id TEXT`),
//!         durable `clock_hwm`, `consumed_at_ms` on deliveries, detach tombstones,
//!         `telex_schema_version` table.
//!         The `NOT NULL` constraint on `lease_epoch` is the store-level hard-fail barrier that
//!         prevents old (non-epoch) binaries from writing to a migrated store (§3.4).

use anyhow::{anyhow, bail, Context, Result};
use async_trait::async_trait;
use rusqlite::{params, Connection, OptionalExtension};
use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use super::{Backend, Capabilities};
use crate::model::*;

const CURRENT_SCHEMA_VERSION: i64 = 2;

// ---------------------------------------------------------------------------
// Store advisory lock
// ---------------------------------------------------------------------------

/// RAII holder for the per-store OS advisory lock.  Dropping this releases the lock.
/// The lock is held for the lifetime of `SqliteBackend` when opened via `open_locked`.
///
/// Production daemon code calls `open_locked`; test fixtures use `open` (no lock) so
/// the conformance suite can open the same path multiple times within one process.
///
/// Lock directory (config-root-invariant, per-OS-user):
///   Windows: `%LOCALAPPDATA%\telex\locks\`
///   Unix:    `$XDG_STATE_HOME/telex/locks/`  (or `$HOME/.local/state/telex/locks/`)
///
/// Lock file: `store-<file-id>.lock` where `file-id` is `dev-inode` (Unix) or
/// `volserial-fileidx` (Windows), so two config roots that alias the same physical
/// SQLite file share exactly one lock target.
// The inner File is never read; it's held alive so the OS lock is released on drop.
#[allow(dead_code)]
pub struct StoreLock(std::fs::File);

fn store_lock_dir() -> Result<std::path::PathBuf> {
    #[cfg(windows)]
    let base = std::env::var_os("LOCALAPPDATA")
        .map(std::path::PathBuf::from)
        .or_else(|| dirs::data_local_dir())
        .ok_or_else(|| anyhow!("cannot resolve LOCALAPPDATA for store lock directory"))?;

    #[cfg(not(windows))]
    let base = std::env::var_os("XDG_STATE_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| dirs::home_dir().map(|h| h.join(".local").join("state")))
        .ok_or_else(|| anyhow!("cannot resolve state dir for store lock directory"))?;

    #[cfg(windows)]
    let dir = base.join("telex-locks");
    #[cfg(not(windows))]
    let dir = {
        let telex_dir = base.join("telex");
        std::fs::create_dir_all(&telex_dir)
            .with_context(|| format!("creating store lock parent {:?}", telex_dir))?;
        telex_dir.join("locks")
    };
    ensure_private_local_dir(&dir)
        .with_context(|| format!("validating store lock dir {:?}", dir))?;
    Ok(dir)
}

#[cfg(unix)]
fn ensure_private_local_dir(path: &std::path::Path) -> Result<()> {
    use std::os::unix::fs::{DirBuilderExt, MetadataExt};

    if path.exists() {
        let link_meta = std::fs::symlink_metadata(path)
            .with_context(|| format!("checking store lock directory {:?}", path))?;
        if link_meta.file_type().is_symlink() {
            bail!("store lock directory {:?} is a symlink", path);
        }
        if !link_meta.is_dir() {
            bail!("store lock directory {:?} is not a directory", path);
        }
    } else {
        let mut builder = std::fs::DirBuilder::new();
        builder.mode(0o700);
        builder
            .create(path)
            .with_context(|| format!("creating owner-private store lock directory {:?}", path))?;
    }

    let meta = std::fs::metadata(path)
        .with_context(|| format!("checking store lock directory {:?}", path))?;
    let uid = unsafe { libc::geteuid() };
    if meta.uid() != uid {
        bail!(
            "store lock directory {:?} is owned by uid {}, expected current uid {}",
            path,
            meta.uid(),
            uid
        );
    }
    if meta.mode() & 0o077 != 0 {
        bail!("store lock directory {:?} is group/world accessible", path);
    }
    Ok(())
}

#[cfg(windows)]
fn ensure_private_local_dir(path: &std::path::Path) -> Result<()> {
    use std::os::windows::fs::MetadataExt;
    use std::path::{Component, Prefix};
    use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;

    if path.components().any(|component| {
        matches!(
            component,
            Component::Prefix(prefix)
                if matches!(prefix.kind(), Prefix::UNC(_, _) | Prefix::VerbatimUNC(_, _))
        )
    }) {
        bail!("store lock directory {:?} is not a local path", path);
    }

    if path.exists() {
        let meta = std::fs::symlink_metadata(path)
            .with_context(|| format!("checking store lock directory {:?}", path))?;
        if !meta.is_dir() {
            bail!("store lock directory {:?} is not a directory", path);
        }
        if meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            bail!("store lock directory {:?} is a reparse point", path);
        }
    } else {
        create_windows_owner_only_dir(path)
            .with_context(|| format!("creating owner-private store lock directory {:?}", path))?;
    }

    let sid = windows_current_user_sid()?;
    let sddl = windows_dir_security_sddl(path)?;
    if !windows_owner_private_sddl_is_strict(&sddl, &sid) {
        bail!(
            "store lock directory {:?} is not owner-private for current SID",
            path
        );
    }
    Ok(())
}

#[cfg(not(any(unix, windows)))]
fn ensure_private_local_dir(_path: &std::path::Path) -> Result<()> {
    bail!("owner-private local store lock directories are not supported on this platform")
}

#[cfg(unix)]
fn validate_store_path_not_reparse(path: &std::path::Path) -> Result<()> {
    let meta = std::fs::symlink_metadata(path)
        .with_context(|| format!("checking SQLite store path {:?}", path))?;
    if meta.file_type().is_symlink() {
        bail!("SQLite store path {:?} is a symlink", path);
    }
    Ok(())
}

#[cfg(windows)]
fn validate_store_path_not_reparse(path: &std::path::Path) -> Result<()> {
    use std::os::windows::fs::MetadataExt;
    use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;

    let meta = std::fs::symlink_metadata(path)
        .with_context(|| format!("checking SQLite store path {:?}", path))?;
    if meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        bail!("SQLite store path {:?} is a reparse point", path);
    }
    Ok(())
}

#[cfg(not(any(unix, windows)))]
fn validate_store_path_not_reparse(_path: &std::path::Path) -> Result<()> {
    bail!("SQLite store path validation is not supported on this platform")
}

#[cfg(windows)]
fn create_windows_owner_only_dir(path: &std::path::Path) -> Result<()> {
    use std::ffi::{c_void, OsStr};
    use windows_sys::Win32::Foundation::{GetLastError, LocalFree, ERROR_ALREADY_EXISTS};
    use windows_sys::Win32::Security::Authorization::{
        ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
    };
    use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;
    use windows_sys::Win32::Storage::FileSystem::CreateDirectoryW;

    let sid = windows_current_user_sid()?;
    let sddl = format!("O:{sid}G:{sid}D:P(A;;GA;;;{sid})");
    let wide_sddl = wide_null(OsStr::new(&sddl));
    let mut descriptor: *mut c_void = std::ptr::null_mut();
    let ok = unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            wide_sddl.as_ptr(),
            SDDL_REVISION_1,
            &mut descriptor,
            std::ptr::null_mut(),
        )
    };
    if ok == 0 {
        bail!(
            "building owner-only store lock directory security descriptor: {}",
            std::io::Error::last_os_error()
        );
    }
    let mut attrs = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor,
        bInheritHandle: 0,
    };
    let wide_path = wide_null(path.as_os_str());
    let ok = unsafe { CreateDirectoryW(wide_path.as_ptr(), &mut attrs) };
    unsafe {
        LocalFree(descriptor);
    }
    if ok == 0 {
        let err = unsafe { GetLastError() };
        if err != ERROR_ALREADY_EXISTS {
            bail!(
                "creating owner-private store lock directory {:?}: {}",
                path,
                std::io::Error::last_os_error()
            );
        }
    }
    Ok(())
}

#[cfg(windows)]
fn windows_dir_security_sddl(path: &std::path::Path) -> Result<String> {
    use std::ffi::c_void;
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Authorization::{
        ConvertSecurityDescriptorToStringSecurityDescriptorW, GetNamedSecurityInfoW,
        SDDL_REVISION_1, SE_FILE_OBJECT,
    };
    use windows_sys::Win32::Security::{DACL_SECURITY_INFORMATION, OWNER_SECURITY_INFORMATION};

    let wide_path = wide_null(path.as_os_str());
    let mut sd: *mut c_void = std::ptr::null_mut();
    let rc = unsafe {
        GetNamedSecurityInfoW(
            wide_path.as_ptr(),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut sd,
        )
    };
    if rc != 0 {
        bail!(
            "reading store lock directory security descriptor {:?}: {}",
            path,
            std::io::Error::from_raw_os_error(rc as i32)
        );
    }
    let mut sddl_ptr: *mut u16 = std::ptr::null_mut();
    let ok = unsafe {
        ConvertSecurityDescriptorToStringSecurityDescriptorW(
            sd,
            SDDL_REVISION_1,
            OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            &mut sddl_ptr,
            std::ptr::null_mut(),
        )
    };
    unsafe {
        LocalFree(sd);
    }
    if ok == 0 {
        bail!(
            "converting store lock directory security descriptor {:?}: {}",
            path,
            std::io::Error::last_os_error()
        );
    }
    let sddl = unsafe { wide_ptr_to_string(sddl_ptr) };
    unsafe {
        LocalFree(sddl_ptr as *mut c_void);
    }
    Ok(sddl)
}

#[cfg(windows)]
fn windows_owner_private_sddl_is_strict(sddl: &str, sid: &str) -> bool {
    if !sddl_section(sddl, "O:")
        .is_some_and(|owner| owner == sid || matches!(owner.as_str(), "OW" | "CO" | "BA" | "SY"))
    {
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
        if ace_sid == sid || matches!(ace_sid.as_str(), "OW" | "CO") {
            has_current_sid = true;
            continue;
        }
        if !matches!(ace_sid.as_str(), "SY" | "BA" | "AC" | "S-1-15-2-1") {
            return false;
        }
    }
    has_current_sid
}

#[cfg(windows)]
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

#[cfg(windows)]
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
        let ace_sid = fields[5].trim();
        if ace_sid.is_empty() {
            return Vec::new();
        }
        sids.push(ace_sid.to_string());
        rest = &rest[end + 1..];
    }
    sids
}

#[cfg(windows)]
fn windows_current_user_sid() -> Result<String> {
    use std::ffi::c_void;
    use windows_sys::Win32::Foundation::{CloseHandle, LocalFree, HANDLE, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Security::Authorization::ConvertSidToStringSidW;
    use windows_sys::Win32::Security::{GetTokenInformation, TokenUser, TOKEN_QUERY, TOKEN_USER};
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    let mut token: HANDLE = 0;
    let ok = unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) };
    if ok == 0 {
        bail!(
            "opening current process token: {}",
            std::io::Error::last_os_error()
        );
    }
    struct TokenHandle(HANDLE);
    impl Drop for TokenHandle {
        fn drop(&mut self) {
            if self.0 != 0 && self.0 != INVALID_HANDLE_VALUE {
                unsafe {
                    CloseHandle(self.0);
                }
            }
        }
    }
    let token = TokenHandle(token);

    let mut needed = 0u32;
    unsafe {
        GetTokenInformation(token.0, TokenUser, std::ptr::null_mut(), 0, &mut needed);
    }
    if needed == 0 {
        bail!(
            "sizing current token user information: {}",
            std::io::Error::last_os_error()
        );
    }
    let mut buf = vec![0u8; needed as usize];
    let ok = unsafe {
        GetTokenInformation(
            token.0,
            TokenUser,
            buf.as_mut_ptr() as *mut c_void,
            needed,
            &mut needed,
        )
    };
    if ok == 0 {
        bail!(
            "reading current token user information: {}",
            std::io::Error::last_os_error()
        );
    }
    let token_user = unsafe { &*(buf.as_ptr() as *const TOKEN_USER) };
    let mut sid_ptr: *mut u16 = std::ptr::null_mut();
    let ok = unsafe { ConvertSidToStringSidW(token_user.User.Sid, &mut sid_ptr) };
    if ok == 0 {
        bail!(
            "converting current SID: {}",
            std::io::Error::last_os_error()
        );
    }
    let sid = unsafe { wide_ptr_to_string(sid_ptr) };
    unsafe {
        LocalFree(sid_ptr as *mut c_void);
    }
    Ok(sid)
}

#[cfg(windows)]
fn wide_null(s: &std::ffi::OsStr) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    s.encode_wide().chain(std::iter::once(0)).collect()
}

#[cfg(windows)]
unsafe fn wide_ptr_to_string(ptr: *const u16) -> String {
    let mut len = 0usize;
    while *ptr.add(len) != 0 {
        len += 1;
    }
    String::from_utf16_lossy(std::slice::from_raw_parts(ptr, len))
}

/// Return a stable, per-OS-user-invariant string that identifies the physical store file.
fn store_file_id(path: &std::path::Path) -> Result<String> {
    // Best-effort canonicalise first (resolves symlinks/hardlinks).
    let canonical = std::fs::canonicalize(path)
        .with_context(|| format!("canonicalizing SQLite store for lock identity {:?}", path))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if let Ok(m) = std::fs::metadata(&canonical) {
            return Ok(format!("{}-{}", m.dev(), m.ino()));
        }
    }

    #[cfg(windows)]
    {
        if let Ok(id) = windows_file_id(&canonical) {
            return Ok(id);
        }
    }

    bail!(
        "cannot determine stable physical file identity for SQLite store {:?}; refusing path-hash lock fallback",
        canonical
    )
}

#[cfg(windows)]
fn windows_file_id(path: &std::path::Path) -> Result<String> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Foundation::HANDLE;
    use windows_sys::Win32::Storage::FileSystem::{
        GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
    };

    let file = std::fs::File::open(path)?;
    let handle = file.as_raw_handle() as HANDLE;
    let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
    let ok = unsafe { GetFileInformationByHandle(handle, &mut info) };
    if ok == 0 {
        bail!(
            "GetFileInformationByHandle failed: {}",
            std::io::Error::last_os_error()
        );
    }
    Ok(format!(
        "{}-{}-{}",
        info.dwVolumeSerialNumber, info.nFileIndexHigh, info.nFileIndexLow
    ))
}

/// Acquire an exclusive OS advisory lock on a lock file whose name is derived from the
/// physical store identity.  Fails immediately (does not block) if another process
/// already holds the lock.
fn acquire_store_lock(db_path: &str) -> Result<StoreLock> {
    if db_path == ":memory:" {
        bail!("cannot acquire a canonical store lock for an in-memory SQLite database");
    }
    let db = std::path::Path::new(db_path);
    if let Some(parent) = db.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating SQLite store parent {:?}", parent))?;
        }
    }
    // Ensure the file exists before computing its file-id, otherwise the first opener would
    // fall back to a path-hash lock while later openers use the physical file-id lock.
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(db)
        .with_context(|| format!("creating SQLite store before locking {:?}", db))?;
    validate_store_path_not_reparse(db)?;

    let lock_dir = store_lock_dir()?;
    let file_id = store_file_id(db)?;
    let lock_path = lock_dir.join(format!("store-{}.lock", file_id));

    let lock_file = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .open(&lock_path)
        .map_err(|e| anyhow!("cannot open store lock {:?}: {}", lock_path, e))?;

    try_lock_exclusive(&lock_file).map_err(|e| {
        anyhow!(
            "cannot acquire store lock for {:?} (another instance may be holding it): {}",
            db_path,
            e
        )
    })?;

    Ok(StoreLock(lock_file))
}

#[cfg(unix)]
fn try_lock_exclusive(file: &std::fs::File) -> Result<()> {
    use std::os::unix::io::AsRawFd;
    let ret = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if ret == 0 {
        Ok(())
    } else {
        let e = std::io::Error::last_os_error();
        if e.raw_os_error() == Some(libc::EWOULDBLOCK) {
            bail!("lock already held")
        }
        bail!("flock: {}", e)
    }
}

#[cfg(windows)]
fn try_lock_exclusive(file: &std::fs::File) -> Result<()> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Foundation::HANDLE;
    use windows_sys::Win32::Storage::FileSystem::{
        LockFileEx, LOCKFILE_EXCLUSIVE_LOCK, LOCKFILE_FAIL_IMMEDIATELY,
    };
    use windows_sys::Win32::System::IO::OVERLAPPED;

    let handle = file.as_raw_handle() as HANDLE;
    let mut ov: OVERLAPPED = unsafe { std::mem::zeroed() };
    let ok = unsafe {
        LockFileEx(
            handle,
            LOCKFILE_EXCLUSIVE_LOCK | LOCKFILE_FAIL_IMMEDIATELY,
            0,
            u32::MAX,
            u32::MAX,
            &mut ov,
        )
    };
    if ok != 0 {
        Ok(())
    } else {
        bail!("LockFileEx: {}", std::io::Error::last_os_error())
    }
}

#[cfg(not(any(unix, windows)))]
fn try_lock_exclusive(_file: &std::fs::File) -> Result<()> {
    bail!("SQLite canonical-store advisory locks are not supported on this platform")
}

// ---------------------------------------------------------------------------
// SqliteBackend
// ---------------------------------------------------------------------------

pub struct SqliteBackend {
    conn: Arc<Mutex<Connection>>,
    /// Advisory store lock; Some when opened via `open_locked`, None otherwise.
    _store_lock: Option<StoreLock>,
}

impl SqliteBackend {
    /// Open (or create) the SQLite store at `path` **without** acquiring the store advisory lock.
    /// Suitable for test fixtures that open the same path multiple times in one process.
    /// Production daemon code should call `open_locked` instead.
    pub fn open(path: &str) -> Result<Self> {
        let conn = Self::open_conn(path)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
            _store_lock: None,
        })
    }

    /// Open (or create) the SQLite store at `path` and acquire the per-store OS advisory lock.
    /// Fails immediately if another process holds the lock, ensuring single-writer authority.
    pub fn open_locked(path: &str) -> Result<Self> {
        // Acquire the lock before opening the connection so that if lock acquisition fails
        // the file is never touched.
        let lock = acquire_store_lock(path)?;
        let conn = Self::open_conn(path)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
            _store_lock: Some(lock),
        })
    }

    fn open_conn(path: &str) -> Result<Connection> {
        if let Some(parent) = std::path::Path::new(path).parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent).ok();
            }
        }
        let conn = Connection::open(path)?;
        // Set busy_timeout *before* the journal_mode=WAL switch: that switch briefly takes a
        // write lock, so when several connections open the same fresh database at once
        // (multiple holders/senders starting together) a still-default zero timeout makes the
        // contended opener fail with a spurious "database is locked" instead of waiting. This
        // greatly reduces such startup errors — though it is not an absolute guarantee, since
        // SQLite skips the busy handler on a simultaneous SHARED->EXCLUSIVE WAL promotion to
        // avoid deadlock. The backend conformance concurrency scenario exercises this path.
        conn.execute_batch(
            "PRAGMA busy_timeout=5000; PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;",
        )?;
        Ok(conn)
    }

    async fn run<T, F>(&self, f: F) -> Result<T>
    where
        F: FnOnce(&Connection) -> Result<T> + Send + 'static,
        T: Send + 'static,
    {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let guard = conn.lock().unwrap();
            f(&guard)
        })
        .await?
    }
}

// ---------------------------------------------------------------------------
// Schema helpers
// ---------------------------------------------------------------------------

fn table_exists(c: &Connection, name: &str) -> rusqlite::Result<bool> {
    let n: i64 = c.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
        params![name],
        |r| r.get(0),
    )?;
    Ok(n > 0)
}

fn has_column(c: &Connection, table: &str, col: &str) -> rusqlite::Result<bool> {
    // pragma_table_info is a table-valued function available in SQLite >= 3.16.
    let n: i64 = c.query_row(
        "SELECT COUNT(*) FROM pragma_table_info(?1) WHERE name=?2",
        params![table, col],
        |r| r.get(0),
    )?;
    Ok(n > 0)
}

/// Read (and advance) the durable clock high-water mark.
/// Must be called within a write transaction (BEGIN IMMEDIATE) so the update is atomic
/// with any timestamp written to lease/delivery columns.
fn advance_clock_hwm(c: &Connection) -> Result<i64> {
    let new_hwm = next_clock_hwm(c)?;
    persist_clock_hwm(c, new_hwm)?;
    Ok(new_hwm)
}

fn durable_clock_read(c: &Connection) -> Result<i64> {
    let now_wall = now_ms();
    let hwm: Option<i64> = c
        .query_row("SELECT hwm_ms FROM clock_hwm WHERE id=1", [], |r| r.get(0))
        .optional()?;
    Ok(hwm.map_or(now_wall, |h| std::cmp::max(now_wall, h)))
}

fn next_clock_hwm(c: &Connection) -> Result<i64> {
    let now_wall = now_ms();
    let hwm: Option<i64> = c
        .query_row("SELECT hwm_ms FROM clock_hwm WHERE id=1", [], |r| r.get(0))
        .optional()?;
    Ok(hwm.map_or(now_wall, |h| std::cmp::max(now_wall, h + 1)))
}

fn persist_clock_hwm(c: &Connection, hwm_ms: i64) -> Result<()> {
    c.execute(
        "INSERT INTO clock_hwm (id, hwm_ms) VALUES (1, ?1)
         ON CONFLICT(id) DO UPDATE SET hwm_ms = MAX(clock_hwm.hwm_ms, excluded.hwm_ms)",
        params![hwm_ms],
    )?;
    Ok(())
}

fn with_immediate_transaction<T, F>(c: &Connection, f: F) -> Result<T>
where
    F: FnOnce(&Connection) -> Result<T>,
{
    c.execute_batch("BEGIN IMMEDIATE;")?;
    let result = f(c);
    match &result {
        Ok(_) => c.execute_batch("COMMIT;")?,
        Err(_) => {
            let _ = c.execute_batch("ROLLBACK;");
        }
    }
    result
}

/// Run the full schema initialisation / migration inside a single BEGIN IMMEDIATE transaction
/// so it is crash-safe (re-runnable; a partially-applied migration is rolled back and retried).
fn init_schema_inner(c: &Connection) -> Result<()> {
    with_immediate_transaction(c, do_schema)
}

fn do_schema(c: &Connection) -> Result<()> {
    // ---- Detect current schema version --------------------------------
    let schema_v_exists = table_exists(c, "telex_schema_version")?;
    let current_version: i64 = if schema_v_exists {
        c.query_row(
            "SELECT version FROM telex_schema_version ORDER BY version DESC LIMIT 1",
            [],
            |r| r.get(0),
        )
        .optional()?
        .unwrap_or(0)
    } else {
        0
    };

    if current_version > CURRENT_SCHEMA_VERSION {
        bail!(
            "SQLite schema version {current_version} is newer than supported version {CURRENT_SCHEMA_VERSION}"
        );
    }

    // Already up to date; still verify/add idempotent v2 invariants that older
    // builds may have missed before they wrote the same schema version.
    if current_version == CURRENT_SCHEMA_VERSION {
        ensure_v2_invariants(c)?;
        return Ok(());
    }

    // ---- Create / migrate non-lease tables ----------------------------
    // addresses: unchanged shape; safe to create IF NOT EXISTS.
    c.execute_batch(
        "CREATE TABLE IF NOT EXISTS addresses (
            address       TEXT PRIMARY KEY,
            description   TEXT,
            scope         TEXT,
            tags          TEXT,
            status        TEXT NOT NULL DEFAULT 'active',
            created_at_ms INTEGER NOT NULL
        );",
    )?;

    // messages: unchanged shape.
    c.execute_batch(
        "CREATE TABLE IF NOT EXISTS messages (
            id            INTEGER PRIMARY KEY AUTOINCREMENT,
            thread_id     INTEGER,
            parent_id     INTEGER,
            from_addr     TEXT,
            to_addr       TEXT NOT NULL,
            cc            TEXT,
            kind          TEXT NOT NULL DEFAULT 'note',
            attention     TEXT NOT NULL DEFAULT 'background',
            requires_disposition INTEGER NOT NULL DEFAULT 0,
            subject       TEXT,
            body          TEXT NOT NULL,
            metadata      TEXT,
            sent_at_ms    INTEGER NOT NULL,
            created_at_ms INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS messages_to_id_idx ON messages(to_addr, id);
        CREATE INDEX IF NOT EXISTS messages_thread_idx ON messages(thread_id, id);",
    )?;

    // dispositions: unchanged shape.
    c.execute_batch(
        "CREATE TABLE IF NOT EXISTS dispositions (
            id           INTEGER PRIMARY KEY AUTOINCREMENT,
            message_id   INTEGER NOT NULL,
            recipient    TEXT NOT NULL,
            state        TEXT NOT NULL,
            note         TEXT,
            by_principal TEXT,
            at_ms        INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS dispositions_msg_idx ON dispositions(message_id, id);",
    )?;

    // deliveries: create with full v2 shape including consumed_at_ms.
    // If the table already exists (v0), add the new column.
    let deliveries_exists = table_exists(c, "deliveries")?;
    if !deliveries_exists {
        c.execute_batch(
            "CREATE TABLE deliveries (
                id              INTEGER PRIMARY KEY AUTOINCREMENT,
                message_id      INTEGER NOT NULL,
                recipient       TEXT NOT NULL,
                occupant        TEXT,
                delivered_at_ms INTEGER NOT NULL,
                consumed_at_ms  INTEGER,
                UNIQUE(message_id, recipient)
            );",
        )?;
    } else if !has_column(c, "deliveries", "consumed_at_ms")? {
        // Add consumed_at_ms column, then back-fill: existing rows were delivered under the
        // old semantics so treat them as consumed (preserves do-not-redeliver invariant).
        c.execute_batch("ALTER TABLE deliveries ADD COLUMN consumed_at_ms INTEGER;")?;
        c.execute_batch(
            "UPDATE deliveries SET consumed_at_ms = delivered_at_ms WHERE consumed_at_ms IS NULL;",
        )?;
    }

    // ---- Migrate the leases table ------------------------------------
    // The v2 leases table has `lease_epoch INTEGER NOT NULL` (no default) so any old binary
    // that tries to INSERT without lease_epoch will fail — this is the store-level hard-fail
    // barrier (§3.4 / M10) that prevents non-epoch writers from corrupting the fence.
    let leases_exists = table_exists(c, "leases")?;
    if !leases_exists {
        // Fresh database: create the v2 leases table directly.
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
                lease_epoch       INTEGER NOT NULL,
                owner_instance_id TEXT,
                daemon_fence_token INTEGER NOT NULL DEFAULT 0
            );",
        )?;
    } else if !has_column(c, "leases", "lease_epoch")? {
        // v0 database: rename the old table, create the constrained v2 table, migrate rows.
        // If a previous migration was interrupted after the rename but before creating the
        // new table, leases_v0 will already exist — skip the rename in that case.
        if !table_exists(c, "leases_v0")? {
            c.execute_batch("ALTER TABLE leases RENAME TO leases_v0;")?;
        }
        c.execute_batch(
            "CREATE TABLE IF NOT EXISTS leases (
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
                lease_epoch       INTEGER NOT NULL,
                owner_instance_id TEXT,
                daemon_fence_token INTEGER NOT NULL DEFAULT 0
            );",
        )?;
        // Migrate v0 rows: seed epoch=1, owner=NULL (unowned; must re-claim under new model).
        c.execute_batch(
            "INSERT OR IGNORE INTO leases \
             (address, occupant, host, principal, description, tags, scope, pid, \
              since_ms, heartbeat_at_ms, lease_epoch, owner_instance_id, daemon_fence_token) \
             SELECT address, occupant, host, principal, description, tags, scope, pid, \
                    since_ms, heartbeat_at_ms, 1, NULL, 0 \
             FROM leases_v0;",
        )?;
    }
    // If leases exists and already has lease_epoch, nothing to do.

    // ---- Durable clock high-water table ------------------------------
    c.execute_batch(
        "CREATE TABLE IF NOT EXISTS clock_hwm (
            id     INTEGER PRIMARY KEY CHECK (id = 1),
            hwm_ms INTEGER NOT NULL
        );",
    )?;
    let now = now_ms();
    c.execute(
        "INSERT INTO clock_hwm (id, hwm_ms) VALUES (1, ?1) ON CONFLICT(id) DO NOTHING",
        params![now],
    )?;

    // ---- Schema version record ---------------------------------------
    c.execute_batch(
        "CREATE TABLE IF NOT EXISTS telex_schema_version (
            singleton INTEGER NOT NULL DEFAULT 1 UNIQUE,
            version   INTEGER NOT NULL
        );",
    )?;
    ensure_v2_invariants(c)?;
    c.execute(
        "INSERT INTO telex_schema_version (singleton, version) VALUES (1, ?1)
         ON CONFLICT(singleton) DO UPDATE SET version = MAX(version, ?1)",
        params![CURRENT_SCHEMA_VERSION],
    )?;

    Ok(())
}

fn ensure_v2_invariants(c: &Connection) -> Result<()> {
    if table_exists(c, "leases")? && !has_column(c, "leases", "daemon_fence_token")? {
        c.execute_batch(
            "ALTER TABLE leases ADD COLUMN daemon_fence_token INTEGER NOT NULL DEFAULT 0;",
        )?;
    }
    c.execute_batch(
        "CREATE INDEX IF NOT EXISTS deliveries_recipient_pending_idx
             ON deliveries(recipient, consumed_at_ms, message_id);
         CREATE TABLE IF NOT EXISTS legacy_cutover_claims (
             address       TEXT PRIMARY KEY,
             claimed_at_ms INTEGER NOT NULL
         );
         CREATE TABLE IF NOT EXISTS telex_schema_meta (
             key   TEXT PRIMARY KEY,
             value TEXT NOT NULL
         );
         CREATE TABLE IF NOT EXISTS detach_tombstones (
             session_id TEXT NOT NULL,
             address    TEXT NOT NULL,
             reason     TEXT NOT NULL,
             at_ms      INTEGER NOT NULL,
             PRIMARY KEY(session_id, address)
         );
         CREATE INDEX IF NOT EXISTS detach_tombstones_session_idx
             ON detach_tombstones(session_id);
         CREATE TRIGGER IF NOT EXISTS leases_reject_unfenced_update
         BEFORE UPDATE ON leases
         FOR EACH ROW
         WHEN NEW.daemon_fence_token <= OLD.daemon_fence_token
         BEGIN
             SELECT RAISE(ABORT, 'legacy lease update rejected: missing daemon fence token');
         END;",
    )?;
    backfill_delivery_rows_once(c)?;
    Ok(())
}

fn backfill_delivery_rows_once(c: &Connection) -> Result<()> {
    let complete: bool = c
        .query_row(
            "SELECT value='1' FROM telex_schema_meta WHERE key='delivery_backfill_v2_complete'",
            [],
            |r| r.get(0),
        )
        .optional()?
        .unwrap_or(false);
    if complete {
        return Ok(());
    }
    backfill_delivery_rows(c)?;
    c.execute(
        "INSERT INTO telex_schema_meta(key, value)
         VALUES ('delivery_backfill_v2_complete', '1')
         ON CONFLICT(key) DO UPDATE SET value='1'",
        [],
    )?;
    Ok(())
}

fn backfill_delivery_rows(c: &Connection) -> Result<()> {
    let mut stmt = c.prepare("SELECT id, to_addr, cc, created_at_ms FROM messages ORDER BY id")?;
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<String>>(2)?,
                r.get::<_, i64>(3)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for (message_id, to_addr, cc, created_at_ms) in rows {
        for recipient in fanout_recipients(&to_addr, cc.as_deref()) {
            c.execute(
                "INSERT OR IGNORE INTO deliveries
                 (message_id, recipient, delivered_at_ms, consumed_at_ms)
                 VALUES (?1, ?2, ?3, NULL)",
                params![message_id, recipient, created_at_ms],
            )?;
        }
    }
    Ok(())
}

fn materialize_pending_delivery_rows_for_recipient(c: &Connection, recipient: &str) -> Result<()> {
    let mut stmt = c.prepare(
        "SELECT id, to_addr, cc, created_at_ms
         FROM messages
         WHERE (to_addr=?1 OR cc LIKE '%' || ?1 || '%')
           AND NOT EXISTS (
               SELECT 1 FROM deliveries d
               WHERE d.message_id=messages.id AND d.recipient=?1
           )
         ORDER BY id",
    )?;
    let rows = stmt
        .query_map(params![recipient], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<String>>(2)?,
                r.get::<_, i64>(3)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for (message_id, to_addr, cc, created_at_ms) in rows {
        if fanout_recipients(&to_addr, cc.as_deref())
            .iter()
            .any(|addr| addr == recipient)
        {
            c.execute(
                "INSERT OR IGNORE INTO deliveries
                 (message_id, recipient, delivered_at_ms, consumed_at_ms)
                 VALUES (?1, ?2, ?3, NULL)",
                params![message_id, recipient, created_at_ms],
            )?;
        }
    }
    Ok(())
}

fn legacy_cutover_pending(c: &Connection, addr: &str) -> Result<bool> {
    let already: bool = c.query_row(
        "SELECT EXISTS(SELECT 1 FROM legacy_cutover_claims WHERE address=?1)",
        params![addr],
        |r| r.get(0),
    )?;
    if already {
        return Ok(false);
    }
    if table_exists(c, "leases_v0")? {
        let migrated: bool = c.query_row(
            "SELECT EXISTS(SELECT 1 FROM leases_v0 WHERE address=?1)",
            params![addr],
            |r| r.get(0),
        )?;
        if migrated {
            return Ok(true);
        }
    }
    Ok(false)
}

fn record_legacy_cutover_claim(c: &Connection, addr: &str, now: i64) -> Result<()> {
    c.execute(
        "INSERT OR IGNORE INTO legacy_cutover_claims(address, claimed_at_ms) VALUES (?1, ?2)",
        params![addr, now],
    )?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Message mapping
// ---------------------------------------------------------------------------

/// Column list used by every message SELECT so row mapping stays positional and stable.
const MSG_COLS: &str = "id, thread_id, parent_id, from_addr, to_addr, cc, kind, attention, \
    requires_disposition, subject, body, metadata, sent_at_ms, created_at_ms";
const MSG_COLS_M: &str = "m.id, m.thread_id, m.parent_id, m.from_addr, m.to_addr, m.cc, m.kind, \
    m.attention, m.requires_disposition, m.subject, m.body, m.metadata, m.sent_at_ms, \
    m.created_at_ms";

fn map_message(r: &rusqlite::Row) -> rusqlite::Result<MessageRow> {
    let id: i64 = r.get(0)?;
    let thread_id: Option<i64> = r.get(1)?;
    Ok(MessageRow {
        id,
        thread_id: thread_id.unwrap_or(id),
        parent_id: r.get(2)?,
        from_addr: r.get(3)?,
        to_addr: r.get(4)?,
        cc: r.get(5)?,
        kind: r.get(6)?,
        attention: r.get(7)?,
        requires_disposition: r.get::<_, i64>(8)? != 0,
        subject: r.get(9)?,
        body: r.get(10)?,
        metadata: r.get(11)?,
        sent_at_ms: r.get(12)?,
        created_at_ms: r.get(13)?,
    })
}

fn fanout_recipients(to_addr: &str, cc: Option<&str>) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut recipients = Vec::new();
    for raw in std::iter::once(to_addr).chain(
        cc.into_iter()
            .flat_map(|s| s.split(','))
            .map(str::trim)
            .filter(|s| !s.is_empty()),
    ) {
        if seen.insert(raw.to_string()) {
            recipients.push(raw.to_string());
        }
    }
    recipients
}

// ---------------------------------------------------------------------------
// Lease helpers
// ---------------------------------------------------------------------------

fn read_lease(c: &Connection, address: &str) -> Result<Option<LeaseRow>> {
    let row = c
        .query_row(
            "SELECT address, occupant, host, principal, description, tags, scope, pid, \
             since_ms, heartbeat_at_ms, lease_epoch, owner_instance_id \
             FROM leases WHERE address=?1",
            params![address],
            |r| {
                Ok(LeaseRow {
                    address: r.get(0)?,
                    occupant: r.get(1)?,
                    host: r.get(2)?,
                    principal: r.get(3)?,
                    description: r.get(4)?,
                    tags: r.get(5)?,
                    scope: r.get(6)?,
                    pid: r.get(7)?,
                    since_ms: r.get(8)?,
                    heartbeat_at_ms: r.get(9)?,
                    lease_epoch: r.get(10)?,
                    owner_instance_id: r.get(11)?,
                })
            },
        )
        .optional()
        .map_err(|e| anyhow!(e))?;
    Ok(row)
}

fn placeholder_lease(address: &str, occupant: Option<String>) -> LeaseRow {
    LeaseRow {
        address: address.to_string(),
        occupant,
        host: None,
        principal: None,
        description: None,
        tags: None,
        scope: None,
        pid: None,
        since_ms: 0,
        heartbeat_at_ms: 0,
        lease_epoch: None,
        owner_instance_id: None,
    }
}

/// Inner logic for `claim_epoch_lease`, called within a `BEGIN IMMEDIATE` transaction.
fn claim_epoch_inner(
    c: &Connection,
    addr: &str,
    owner: &str,
    liveness_window_secs: i64,
) -> Result<EpochClaimResult> {
    let durable_now = durable_clock_read(c)?;
    let stale_cutoff_ms =
        durable_now.saturating_sub(liveness_window_secs.max(0).saturating_mul(1000));

    // Row existence is decided inside BEGIN IMMEDIATE, so no other writer can race between this
    // read and a first-row insert. Avoid advancing the durable clock on blocked claims.
    let existing: Option<(Option<i64>, Option<String>, i64)> = c
        .query_row(
            "SELECT lease_epoch, owner_instance_id, heartbeat_at_ms FROM leases WHERE address=?1",
            params![addr],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;

    let Some((cur_epoch, cur_owner, cur_hb)) = existing else {
        let now = next_clock_hwm(c)?;
        let inserted = c.execute(
            "INSERT INTO leases \
             (address, since_ms, heartbeat_at_ms, lease_epoch, owner_instance_id, daemon_fence_token)
             VALUES (?1, ?2, ?2, 1, ?3, 1)",
            params![addr, now, owner],
        )?;
        if inserted == 0 {
            let row =
                read_lease(c, addr)?.ok_or_else(|| anyhow!("lease row vanished during claim"))?;
            return Ok(EpochClaimResult::AlreadyOwned {
                lease_epoch: row.lease_epoch.unwrap_or(0),
                owner_instance_id: row.owner_instance_id.clone().unwrap_or_default(),
                lease_row: row,
            });
        }
        persist_clock_hwm(c, now)?;
        return Ok(EpochClaimResult::Claimed(EpochClaimed {
            lease_epoch: 1,
            owner_instance_id: owner.to_string(),
            legacy_cutover: false,
        }));
    };

    if cur_epoch.is_none() {
        let legacy_cutover = true;
        let now = next_clock_hwm(c)?;
        let updated = c.execute(
            "UPDATE leases
                SET owner_instance_id = ?1,
                    lease_epoch       = 1,
                    heartbeat_at_ms   = ?2,
                    daemon_fence_token = daemon_fence_token + 1
              WHERE address = ?3
                AND lease_epoch IS NULL",
            params![owner, now, addr],
        )?;
        if updated > 0 {
            persist_clock_hwm(c, now)?;
            record_legacy_cutover_claim(c, addr, now)?;
            return Ok(EpochClaimResult::Claimed(EpochClaimed {
                lease_epoch: 1,
                owner_instance_id: owner.to_string(),
                legacy_cutover,
            }));
        }
        let row = read_lease(c, addr)?.ok_or_else(|| anyhow!("lease row vanished during claim"))?;
        return Ok(EpochClaimResult::AlreadyOwned {
            lease_epoch: row.lease_epoch.unwrap_or(0),
            owner_instance_id: row.owner_instance_id.clone().unwrap_or_default(),
            lease_row: row,
        });
    }
    let cur_epoch = cur_epoch.unwrap();

    let legacy_cutover = legacy_cutover_pending(c, addr)?;
    let can_claim = cur_owner.is_none() || cur_hb < stale_cutoff_ms;
    if !can_claim {
        let row =
            read_lease(c, addr)?.unwrap_or_else(|| placeholder_lease(addr, cur_owner.clone()));
        return Ok(EpochClaimResult::AlreadyOwned {
            lease_epoch: cur_epoch,
            owner_instance_id: cur_owner.unwrap_or_default(),
            lease_row: row,
        });
    }

    // CAS update: increment epoch atomically inside the backend.
    let now = next_clock_hwm(c)?;
    let updated = c.execute(
        "UPDATE leases
            SET owner_instance_id = ?1,
                lease_epoch        = lease_epoch + 1,
                heartbeat_at_ms    = ?2,
                daemon_fence_token = daemon_fence_token + 1
          WHERE address = ?3
            AND lease_epoch = ?4
            AND owner_instance_id IS NOT DISTINCT FROM ?5
            AND (owner_instance_id IS NULL OR heartbeat_at_ms < ?6)",
        params![owner, now, addr, cur_epoch, cur_owner, stale_cutoff_ms],
    )?;

    if updated > 0 {
        persist_clock_hwm(c, now)?;
        if legacy_cutover {
            record_legacy_cutover_claim(c, addr, now)?;
        }
        Ok(EpochClaimResult::Claimed(EpochClaimed {
            lease_epoch: cur_epoch + 1,
            owner_instance_id: owner.to_string(),
            legacy_cutover,
        }))
    } else {
        // Lost the race — re-read and return AlreadyOwned.
        let row = read_lease(c, addr)?.ok_or_else(|| anyhow!("lease row vanished during claim"))?;
        Ok(EpochClaimResult::AlreadyOwned {
            lease_epoch: row.lease_epoch.unwrap_or(cur_epoch),
            owner_instance_id: row.owner_instance_id.clone().unwrap_or_default(),
            lease_row: row,
        })
    }
}

// ---------------------------------------------------------------------------
// Backend trait implementation
// ---------------------------------------------------------------------------

#[async_trait]
impl Backend for SqliteBackend {
    fn kind(&self) -> &'static str {
        "sqlite"
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            durable: true,
            push: "poll",
            lease: "ttl",
        }
    }

    async fn init_schema(&self) -> Result<()> {
        self.run(|c| init_schema_inner(c)).await
    }

    async fn ensure_address(
        &self,
        address: &str,
        description: Option<&str>,
        scope: Option<&str>,
        tags: Option<&str>,
    ) -> Result<()> {
        let (a, d, s, t) = (
            address.to_string(),
            description.map(str::to_string),
            scope.map(str::to_string),
            tags.map(str::to_string),
        );
        let now = now_ms();
        self.run(move |c| {
            c.execute(
                "INSERT INTO addresses(address, description, scope, tags, status, created_at_ms) \
                 VALUES (?1,?2,?3,?4,'active',?5) \
                 ON CONFLICT(address) DO UPDATE SET \
                    description=COALESCE(excluded.description, addresses.description), \
                    scope=COALESCE(excluded.scope, addresses.scope), \
                    tags=COALESCE(excluded.tags, addresses.tags)",
                params![a, d, s, t, now],
            )?;
            Ok(())
        })
        .await
    }

    async fn get_address(&self, address: &str) -> Result<Option<AddressRow>> {
        let a = address.to_string();
        self.run(move |c| {
            let row = c
                .query_row(
                    "SELECT address, description, scope, tags, status, created_at_ms \
                     FROM addresses WHERE address=?1",
                    params![a],
                    |r| {
                        Ok(AddressRow {
                            address: r.get(0)?,
                            description: r.get(1)?,
                            scope: r.get(2)?,
                            tags: r.get(3)?,
                            status: r.get(4)?,
                            created_at_ms: r.get(5)?,
                        })
                    },
                )
                .optional()?;
            Ok(row)
        })
        .await
    }

    async fn set_address_status(&self, address: &str, status: &str) -> Result<bool> {
        let (a, s) = (address.to_string(), status.to_string());
        self.run(move |c| {
            let n = c.execute(
                "UPDATE addresses SET status=?2 WHERE address=?1",
                params![a, s],
            )?;
            Ok(n > 0)
        })
        .await
    }

    async fn list_addresses(
        &self,
        scope: Option<&str>,
        include_retired: bool,
    ) -> Result<Vec<AddressRow>> {
        let scope = scope.map(str::to_string);
        self.run(move |c| {
            let mut sql = String::from(
                "SELECT address, description, scope, tags, status, created_at_ms \
                 FROM addresses WHERE 1=1",
            );
            if !include_retired {
                sql.push_str(" AND status='active'");
            }
            if scope.is_some() {
                sql.push_str(" AND scope=?1");
            }
            sql.push_str(" ORDER BY address");
            let mut stmt = c.prepare(&sql)?;
            let map = |r: &rusqlite::Row| {
                Ok(AddressRow {
                    address: r.get(0)?,
                    description: r.get(1)?,
                    scope: r.get(2)?,
                    tags: r.get(3)?,
                    status: r.get(4)?,
                    created_at_ms: r.get(5)?,
                })
            };
            let rows: Vec<AddressRow> = if let Some(s) = scope {
                stmt.query_map(params![s], map)?
                    .collect::<rusqlite::Result<_>>()?
            } else {
                stmt.query_map([], map)?.collect::<rusqlite::Result<_>>()?
            };
            Ok(rows)
        })
        .await
    }

    // ---- leases / liveness -------------------------------------------

    async fn claim_lease(&self, claim: &LeaseClaim, window_secs: i64) -> Result<LeaseOutcome> {
        let claim = claim.clone();
        self.run(move |c| {
            with_immediate_transaction(c, |c| {
                let now = now_ms();
                let stale_cutoff = now - window_secs * 1000;

                // Read current row (need epoch + owner for new schema).
                let existing: Option<(Option<String>, Option<String>, i64, i64, Option<i64>)> = c
                    .query_row(
                        "SELECT occupant, owner_instance_id, since_ms, heartbeat_at_ms, lease_epoch \
                         FROM leases WHERE address=?1",
                        params![claim.address],
                        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
                    )
                    .optional()?;

                if let Some((occ, owner, _since, hb, _epoch)) = &existing {
                    // Live = owner IS NOT NULL and heartbeat fresh.
                    let live = owner.is_some() && now - *hb < window_secs * 1000;
                    let same = occ.as_deref() == Some(claim.occupant.as_str());
                    if live && !same {
                        let row = read_lease(c, &claim.address)?
                            .unwrap_or_else(|| placeholder_lease(&claim.address, occ.clone()));
                        return Ok(LeaseOutcome::AlreadyOccupied(row));
                    }
                }

                // Determine since_ms: stable across same-occupant refreshes.
                let since = match &existing {
                    Some((occ, _, since, _, _)) if occ.as_deref() == Some(claim.occupant.as_str()) => *since,
                    _ => now,
                };

                // Determine new epoch.
                let new_epoch = match &existing {
                    None => 1, // New row
                    Some((occ, _, _, _, epoch)) => {
                        if occ.as_deref() == Some(claim.occupant.as_str()) {
                            // Same occupant: heartbeat refresh, keep epoch.
                            epoch.unwrap_or(1)
                        } else {
                            // New or stale occupant: advance epoch.
                            epoch.unwrap_or(0) + 1
                        }
                    }
                };

                // Stale-owner guard: if a different live owner exists that is NOT stale,
                // we already returned AlreadyOccupied above; here the owner IS NULL or stale.
                // For the "different occupant, stale" case, we need to pass the stale_cutoff check.
                // Since we already checked live && !same above, we're safe to proceed.
                let _ = stale_cutoff; // used implicitly via the live check

                c.execute(
                    "INSERT INTO leases(address, occupant, host, principal, description, tags, \
                     scope, pid, since_ms, heartbeat_at_ms, lease_epoch, owner_instance_id, daemon_fence_token) \
                     VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,1) \
                     ON CONFLICT(address) DO UPDATE SET \
                       occupant=excluded.occupant, host=excluded.host, \
                       principal=excluded.principal, description=excluded.description, \
                       tags=excluded.tags, scope=excluded.scope, pid=excluded.pid, \
                       since_ms=excluded.since_ms, heartbeat_at_ms=excluded.heartbeat_at_ms, \
                       lease_epoch=excluded.lease_epoch, \
                       owner_instance_id=excluded.owner_instance_id, \
                       daemon_fence_token=leases.daemon_fence_token + 1",
                    params![
                        claim.address, claim.occupant, claim.host, claim.principal,
                        claim.description, claim.tags, claim.scope, claim.pid,
                        since, now, new_epoch, claim.occupant  // use occupant as synthetic owner
                    ],
                )?;
                Ok(LeaseOutcome::Claimed)
            })
        })
        .await
    }

    async fn heartbeat(&self, address: &str) -> Result<()> {
        let a = address.to_string();
        let now = now_ms();
        self.run(move |c| {
            c.execute(
                "UPDATE leases
                    SET heartbeat_at_ms=?2,
                        daemon_fence_token=daemon_fence_token + 1
                  WHERE address=?1",
                params![a, now],
            )?;
            Ok(())
        })
        .await
    }

    /// Non-deleting release: clear ownership fields and set heartbeat to 0 so the row is
    /// immediately reclaimable (stale check passes).  The `lease_epoch` high-water is
    /// preserved so the next claimant continues the monotonic sequence (§11.2).
    async fn release_lease(&self, address: &str, occupant: &str) -> Result<bool> {
        let (a, o) = (address.to_string(), occupant.to_string());
        self.run(move |c| {
            let n = c.execute(
                "UPDATE leases \
                 SET owner_instance_id = NULL, occupant = NULL, heartbeat_at_ms = 0, \
                     daemon_fence_token = daemon_fence_token + 1 \
                 WHERE address = ?1 AND occupant = ?2",
                params![a, o],
            )?;
            Ok(n > 0)
        })
        .await
    }

    async fn get_lease(&self, address: &str) -> Result<Option<LeaseRow>> {
        let a = address.to_string();
        self.run(move |c| read_lease(c, &a)).await
    }

    async fn occupancy(&self, address: &str, window_secs: i64) -> Result<Occupancy> {
        let a = address.to_string();
        let now = now_ms();
        self.run(move |c| {
            let row = c
                .query_row(
                    "SELECT occupant, heartbeat_at_ms, owner_instance_id \
                     FROM leases WHERE address=?1",
                    params![a],
                    |r| {
                        Ok((
                            r.get::<_, Option<String>>(0)?,
                            r.get::<_, i64>(1)?,
                            r.get::<_, Option<String>>(2)?,
                        ))
                    },
                )
                .optional()?;
            Ok(match row {
                None => Occupancy {
                    occupied: false,
                    age_secs: 0.0,
                    occupant: None,
                },
                Some((occupant, hb, owner)) => {
                    let age_ms = now - hb;
                    // Occupied iff owner_instance_id IS NOT NULL (epoch-aware) and heartbeat fresh.
                    let occupied = owner.is_some() && age_ms < window_secs * 1000;
                    Occupancy {
                        occupied,
                        age_secs: age_ms as f64 / 1000.0,
                        occupant,
                    }
                }
            })
        })
        .await
    }

    // ---- epoch-aware lease / delivery fence --------------------------

    async fn claim_epoch_lease(
        &self,
        address: &str,
        owner_instance_id: &str,
        liveness_window_secs: i64,
    ) -> Result<EpochClaimResult> {
        let (addr, owner, window) = (
            address.to_owned(),
            owner_instance_id.to_owned(),
            liveness_window_secs,
        );
        self.run(move |c| {
            with_immediate_transaction(c, |c| claim_epoch_inner(c, &addr, &owner, window))
        })
        .await
    }

    async fn heartbeat_epoch(
        &self,
        address: &str,
        owner_instance_id: &str,
        lease_epoch: i64,
    ) -> Result<bool> {
        let (addr, owner, epoch) = (
            address.to_owned(),
            owner_instance_id.to_owned(),
            lease_epoch,
        );
        self.run(move |c| {
            with_immediate_transaction(c, |c| {
                let now = next_clock_hwm(c)?;
                let n = c.execute(
                    "UPDATE leases
                        SET heartbeat_at_ms=?1,
                            daemon_fence_token=daemon_fence_token + 1 \
                     WHERE address=?2 AND lease_epoch=?3 AND owner_instance_id=?4",
                    params![now, addr, epoch, owner],
                )?;
                if n > 0 {
                    persist_clock_hwm(c, now)?;
                }
                Ok(n > 0)
            })
        })
        .await
    }

    async fn release_epoch_lease(
        &self,
        address: &str,
        owner_instance_id: &str,
        lease_epoch: i64,
    ) -> Result<bool> {
        let (addr, owner, epoch) = (
            address.to_owned(),
            owner_instance_id.to_owned(),
            lease_epoch,
        );
        self.run(move |c| {
            let n = c.execute(
                "UPDATE leases
                    SET owner_instance_id = NULL,
                        daemon_fence_token = daemon_fence_token + 1 \
                 WHERE address=?1 AND lease_epoch=?2 AND owner_instance_id=?3",
                params![addr, epoch, owner],
            )?;
            Ok(n > 0)
        })
        .await
    }

    async fn release_epoch_lease_for_detach(
        &self,
        address: &str,
        owner_instance_id: &str,
        lease_epoch: i64,
        session_id: &str,
        reason: &str,
    ) -> Result<bool> {
        let (addr, owner, session, reason) = (
            address.to_owned(),
            owner_instance_id.to_owned(),
            session_id.to_owned(),
            reason.to_owned(),
        );
        self.run(move |c| {
            with_immediate_transaction(c, |c| {
                let n = c.execute(
                    "UPDATE leases
                        SET owner_instance_id = NULL,
                            daemon_fence_token = daemon_fence_token + 1
                      WHERE address=?1 AND lease_epoch=?2 AND owner_instance_id=?3",
                    params![addr, lease_epoch, owner],
                )?;
                if n > 0 {
                    let at_ms = next_clock_hwm(c)?;
                    c.execute(
                        "INSERT INTO detach_tombstones(session_id, address, reason, at_ms)
                         VALUES (?1, ?2, ?3, ?4)
                         ON CONFLICT(session_id, address) DO UPDATE SET
                            reason=excluded.reason,
                            at_ms=excluded.at_ms",
                        params![session, addr, reason, at_ms],
                    )?;
                    persist_clock_hwm(c, at_ms)?;
                }
                Ok(n > 0)
            })
        })
        .await
    }

    async fn reset_epoch_lease(&self, address: &str) -> Result<Option<i64>> {
        let addr = address.to_owned();
        self.run(move |c| {
            with_immediate_transaction(c, |c| {
                let epoch: Option<i64> = c
                    .query_row(
                        "SELECT lease_epoch FROM leases WHERE address=?1",
                        params![addr],
                        |r| r.get(0),
                    )
                    .optional()?;
                if let Some(epoch) = epoch {
                    c.execute(
                        "UPDATE leases
                            SET owner_instance_id = NULL,
                                heartbeat_at_ms = 0,
                                daemon_fence_token = daemon_fence_token + 1 \
                         WHERE address=?1",
                        params![addr],
                    )?;
                    Ok(Some(epoch))
                } else {
                    Ok(None)
                }
            })
        })
        .await
    }

    async fn mark_consumed_if_current_owner(
        &self,
        recipient: &str,
        owner_instance_id: &str,
        lease_epoch: i64,
        message_id: i64,
    ) -> Result<DeliveryOutcome> {
        let (rec, owner, epoch, mid) = (
            recipient.to_owned(),
            owner_instance_id.to_owned(),
            lease_epoch,
            message_id,
        );
        self.run(move |c| {
            with_immediate_transaction(c, |c| {
                // Step 1: Check ownership (NotOwner has strict precedence over all other outcomes).
                let lease_state: Option<(i64, Option<String>)> = c
                    .query_row(
                        "SELECT lease_epoch, owner_instance_id FROM leases WHERE address=?1",
                        params![rec],
                        |r| Ok((r.get(0)?, r.get(1)?)),
                    )
                    .optional()?;

                let is_owner = match &lease_state {
                    Some((le, oi)) => *le == epoch && oi.as_deref() == Some(owner.as_str()),
                    None => false,
                };
                if !is_owner {
                    return Ok(DeliveryOutcome::NotOwner);
                }

                // Step 2: Ensure legacy/pre-v2 messages have explicit pending delivery rows,
                // then check the row.
                materialize_pending_delivery_rows_for_recipient(c, &rec)?;
                let consumed: Option<Option<i64>> = c
                    .query_row(
                        "SELECT consumed_at_ms FROM deliveries WHERE message_id=?1 AND recipient=?2",
                        params![mid, rec],
                        |r| r.get(0),
                    )
                    .optional()?;

                match consumed {
                    None => Ok(DeliveryOutcome::AckNoOp),
                    Some(Some(_)) => {
                        // Row exists and already consumed — idempotent success.
                        Ok(DeliveryOutcome::AlreadyConsumed)
                    }
                    Some(None) => {
                        // Row exists but not yet consumed — mark it.
                        let now = next_clock_hwm(c)?;
                        let updated = c.execute(
                            "UPDATE deliveries SET consumed_at_ms=?1 \
                             WHERE message_id=?2 AND recipient=?3 AND consumed_at_ms IS NULL",
                            params![now, mid, rec],
                        )?;
                        if updated > 0 {
                            persist_clock_hwm(c, now)?;
                            Ok(DeliveryOutcome::Marked)
                        } else {
                            Ok(DeliveryOutcome::AlreadyConsumed)
                        }
                    }
                }
            })
        })
        .await
    }

    async fn durable_clock_now_ms(&self) -> Result<i64> {
        self.run(|c| with_immediate_transaction(c, advance_clock_hwm))
            .await
    }

    async fn delivery_retention_count(&self) -> Result<i64> {
        self.run(|c| {
            let n = c.query_row("SELECT COUNT(*) FROM deliveries", [], |r| r.get(0))?;
            Ok(n)
        })
        .await
    }

    async fn record_detach_tombstone(
        &self,
        session_id: &str,
        address: &str,
        reason: &str,
    ) -> Result<()> {
        let (session, addr, reason) =
            (session_id.to_owned(), address.to_owned(), reason.to_owned());
        self.run(move |c| {
            with_immediate_transaction(c, |c| {
                let at_ms = advance_clock_hwm(c)?;
                c.execute(
                    "INSERT INTO detach_tombstones(session_id, address, reason, at_ms)
                     VALUES (?1, ?2, ?3, ?4)
                     ON CONFLICT(session_id, address) DO UPDATE SET
                        reason=excluded.reason,
                        at_ms=excluded.at_ms",
                    params![session, addr, reason, at_ms],
                )?;
                Ok(())
            })
        })
        .await
    }

    async fn clear_detach_tombstone(&self, session_id: &str, address: &str) -> Result<()> {
        let (session, addr) = (session_id.to_owned(), address.to_owned());
        self.run(move |c| {
            c.execute(
                "DELETE FROM detach_tombstones WHERE session_id=?1 AND address=?2",
                params![session, addr],
            )?;
            Ok(())
        })
        .await
    }

    async fn detach_tombstone(
        &self,
        session_id: &str,
        address: &str,
    ) -> Result<Option<DetachTombstone>> {
        let (session, addr) = (session_id.to_owned(), address.to_owned());
        self.run(move |c| {
            let row = c
                .query_row(
                    "SELECT session_id, address, reason, at_ms
                     FROM detach_tombstones
                     WHERE session_id=?1 AND address=?2",
                    params![session, addr],
                    |r| {
                        Ok(DetachTombstone {
                            session_id: r.get(0)?,
                            address: r.get(1)?,
                            reason: r.get(2)?,
                            at_ms: r.get(3)?,
                        })
                    },
                )
                .optional()?;
            Ok(row)
        })
        .await
    }

    // ---- messages ----------------------------------------------------

    async fn mark_delivered(
        &self,
        message_id: i64,
        recipient: &str,
        occupant: Option<&str>,
    ) -> Result<()> {
        let (r, o) = (recipient.to_string(), occupant.map(str::to_string));
        let now = now_ms();
        self.run(move |c| {
            // Backward-compat: mark_delivered also sets consumed_at_ms so old callers still
            // suppress re-delivery.  If a fan-out row already exists (consumed_at_ms=NULL),
            // update it to consumed; otherwise insert a fully-consumed row.
            c.execute(
                "INSERT INTO deliveries(message_id, recipient, occupant, delivered_at_ms, consumed_at_ms) \
                 VALUES (?1,?2,?3,?4,?4) \
                 ON CONFLICT(message_id, recipient) DO UPDATE SET consumed_at_ms = excluded.consumed_at_ms",
                params![message_id, r, o, now],
            )?;
            Ok(())
        })
        .await
    }

    async fn fetch_undelivered(&self, address: &str) -> Result<Vec<MessageRow>> {
        let a = address.to_string();
        self.run(move |c| {
            materialize_pending_delivery_rows_for_recipient(c, &a)?;
            // Recipient-first: pending delivery rows are the source of delivery work. Legacy
            // no-row messages are materialized above before any emission.
            let sql = format!(
                "SELECT {MSG_COLS_M} FROM deliveries d \
                 JOIN messages m ON m.id=d.message_id \
                 WHERE d.recipient=?1 \
                  AND d.consumed_at_ms IS NULL \
                  AND COALESCE((SELECT disp.state FROM dispositions disp \
                                 WHERE disp.message_id=m.id AND disp.recipient=?1 \
                                 ORDER BY disp.id DESC LIMIT 1), '') NOT IN ({}) \
                 ORDER BY d.message_id",
                terminal_dispositions_sql_list()
            );
            let mut stmt = c.prepare(&sql)?;
            let rows = stmt
                .query_map(params![a], map_message)?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
        .await
    }

    async fn has_delivery_for_recipient(&self, message_id: i64, recipient: &str) -> Result<bool> {
        let r = recipient.to_string();
        self.run(move |c| {
            materialize_pending_delivery_rows_for_recipient(c, &r)?;
            let exists: Option<i64> = c
                .query_row(
                    "SELECT 1 FROM deliveries WHERE message_id=?1 AND recipient=?2 LIMIT 1",
                    params![message_id, r],
                    |row| row.get(0),
                )
                .optional()?;
            Ok(exists.is_some())
        })
        .await
    }

    async fn insert_message(&self, m: &NewMessage) -> Result<MessageRow> {
        let m = m.clone();
        self.run(move |c| {
            with_immediate_transaction(c, |c| {
                let now = now_ms();
                c.execute(
                    "INSERT INTO messages(thread_id, parent_id, from_addr, to_addr, cc, kind, \
                     attention, requires_disposition, subject, body, metadata, sent_at_ms, \
                     created_at_ms) \
                     VALUES (NULL,?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
                    params![
                        m.parent_id,
                        m.from_addr,
                        m.to_addr,
                        m.cc,
                        m.kind,
                        m.attention.as_str(),
                        m.requires_disposition as i64,
                        m.subject,
                        m.body,
                        m.metadata,
                        m.sent_at_ms,
                        now
                    ],
                )?;
                let id = c.last_insert_rowid();

                // Resolve thread: inherit the parent's thread, else root on self.
                let thread_id: i64 = match m.parent_id {
                    Some(pid) => c
                        .query_row(
                            "SELECT COALESCE(thread_id, id) FROM messages WHERE id=?1",
                            params![pid],
                            |r| r.get(0),
                        )
                        .optional()?
                        .unwrap_or(id),
                    None => id,
                };
                c.execute(
                    "UPDATE messages SET thread_id=?2 WHERE id=?1",
                    params![id, thread_id],
                )?;

                // Fan-out: create a pending delivery row for each addressed recipient so
                // `fetch_undelivered` and `mark_consumed_if_current_owner` are per-recipient.
                for recipient in fanout_recipients(&m.to_addr, m.cc.as_deref()) {
                    c.execute(
                        "INSERT OR IGNORE INTO deliveries \
                         (message_id, recipient, delivered_at_ms, consumed_at_ms) \
                         VALUES (?1, ?2, ?3, NULL)",
                        params![id, recipient, now],
                    )?;
                }

                let row = c.query_row(
                    &format!("SELECT {MSG_COLS} FROM messages WHERE id=?1"),
                    params![id],
                    map_message,
                )?;
                Ok(row)
            })
        })
        .await
    }

    async fn get_message(&self, id: i64) -> Result<Option<MessageRow>> {
        self.run(move |c| {
            let row = c
                .query_row(
                    &format!("SELECT {MSG_COLS} FROM messages WHERE id=?1"),
                    params![id],
                    map_message,
                )
                .optional()?;
            Ok(row)
        })
        .await
    }

    async fn thread_messages(&self, thread_id: i64) -> Result<Vec<MessageRow>> {
        self.run(move |c| {
            let sql =
                format!("SELECT {MSG_COLS} FROM messages WHERE thread_id=?1 OR id=?1 ORDER BY id");
            let mut stmt = c.prepare(&sql)?;
            let rows = stmt
                .query_map(params![thread_id], map_message)?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
        .await
    }

    async fn inbox(&self, address: &str, include_all: bool, limit: i64) -> Result<Vec<InboxItem>> {
        let a = address.to_string();
        self.run(move |c| {
            let sql = format!(
                "SELECT {MSG_COLS}, \
                    (SELECT d.state FROM dispositions d WHERE d.message_id=messages.id \
                       AND d.recipient=?1 ORDER BY d.id DESC LIMIT 1) AS latest_disp \
                 FROM messages WHERE to_addr=?1 ORDER BY id DESC LIMIT ?2"
            );
            let mut stmt = c.prepare(&sql)?;
            let rows = stmt
                .query_map(params![a, limit], |r| {
                    let msg = map_message(r)?;
                    let latest: Option<String> = r.get(14)?;
                    Ok((msg, latest))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            let items: Vec<InboxItem> = rows
                .into_iter()
                .map(|(message, latest)| {
                    let terminal = latest
                        .as_deref()
                        .map(Disposition::is_terminal_str)
                        .unwrap_or(false);
                    let actionable = message.requires_disposition && !terminal;
                    InboxItem {
                        message,
                        latest_disposition: latest,
                        actionable,
                    }
                })
                .filter(|it| include_all || it.actionable)
                .collect();
            Ok(items)
        })
        .await
    }

    async fn export(
        &self,
        address: Option<&str>,
        thread: Option<i64>,
        since: i64,
    ) -> Result<Vec<MessageRow>> {
        let a = address.map(str::to_string);
        self.run(move |c| {
            let mut sql = format!("SELECT {MSG_COLS} FROM messages WHERE id>?1");
            if a.is_some() {
                sql.push_str(" AND (to_addr=?2 OR from_addr=?2)");
            }
            if let Some(t) = thread {
                sql.push_str(&format!(" AND (thread_id={t} OR id={t})"));
            }
            sql.push_str(" ORDER BY id");
            let mut stmt = c.prepare(&sql)?;
            let rows = if let Some(addr) = a {
                stmt.query_map(params![since, addr], map_message)?
                    .collect::<rusqlite::Result<Vec<_>>>()?
            } else {
                stmt.query_map(params![since], map_message)?
                    .collect::<rusqlite::Result<Vec<_>>>()?
            };
            Ok(rows)
        })
        .await
    }

    async fn insert_disposition(
        &self,
        message_id: i64,
        recipient: &str,
        state: &str,
        note: Option<&str>,
        by: Option<&str>,
    ) -> Result<DispositionRow> {
        let (r, s, n, b) = (
            recipient.to_string(),
            state.to_string(),
            note.map(str::to_string),
            by.map(str::to_string),
        );
        self.run(move |c| {
            let now = now_ms();
            c.execute(
                "INSERT INTO dispositions(message_id, recipient, state, note, by_principal, at_ms) \
                 VALUES (?1,?2,?3,?4,?5,?6)",
                params![message_id, r, s, n, b, now],
            )?;
            let id = c.last_insert_rowid();
            Ok(DispositionRow {
                id,
                message_id,
                recipient: r,
                state: s,
                note: n,
                by_principal: b,
                at_ms: now,
            })
        })
        .await
    }

    async fn dispositions_for(&self, message_id: i64) -> Result<Vec<DispositionRow>> {
        self.run(move |c| {
            let mut stmt = c.prepare(
                "SELECT id, message_id, recipient, state, note, by_principal, at_ms \
                 FROM dispositions WHERE message_id=?1 ORDER BY id",
            )?;
            let rows = stmt
                .query_map(params![message_id], |r| {
                    Ok(DispositionRow {
                        id: r.get(0)?,
                        message_id: r.get(1)?,
                        recipient: r.get(2)?,
                        state: r.get(3)?,
                        note: r.get(4)?,
                        by_principal: r.get(5)?,
                        at_ms: r.get(6)?,
                    })
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
        .await
    }

    async fn notify_new(&self, _address: &str, _id: i64, _sent_at_ms: i64) -> Result<()> {
        Ok(()) // no native push; poll covers it
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEST_SEQ: AtomicU64 = AtomicU64::new(1);

    fn test_db_path(label: &str) -> std::path::PathBuf {
        let seq = TEST_SEQ.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::current_dir()
            .unwrap()
            .join("target")
            .join("sqlite-p6-tests");
        std::fs::create_dir_all(&dir).unwrap();
        dir.join(format!("{label}-{}-{seq}.db", std::process::id()))
    }

    #[tokio::test]
    async fn v0_lease_row_migrates_without_delete_and_next_claim_advances_epoch() {
        let path = test_db_path("v0-migrate");
        {
            let c = Connection::open(&path).unwrap();
            c.execute_batch(
                "CREATE TABLE leases (
                    address         TEXT PRIMARY KEY,
                    occupant        TEXT,
                    host            TEXT,
                    principal       TEXT,
                    description     TEXT,
                    tags            TEXT,
                    scope           TEXT,
                    pid             INTEGER,
                    since_ms        INTEGER NOT NULL,
                    heartbeat_at_ms INTEGER NOT NULL
                );
                INSERT INTO leases(address, occupant, host, principal, since_ms, heartbeat_at_ms)
                VALUES ('addr:legacy', 'legacy-holder', 'host', 'principal', 10, 20);",
            )
            .unwrap();
        }

        let backend = SqliteBackend::open(&path.to_string_lossy()).unwrap();
        backend.init_schema().await.unwrap();
        let migrated = backend.get_lease("addr:legacy").await.unwrap().unwrap();
        assert_eq!(migrated.lease_epoch, Some(1));
        assert_eq!(migrated.owner_instance_id, None);

        let old_rows: i64 = backend
            .run(|c| {
                Ok(c.query_row(
                    "SELECT COUNT(*) FROM leases_v0 WHERE address='addr:legacy'",
                    [],
                    |r| r.get(0),
                )?)
            })
            .await
            .unwrap();
        assert_eq!(old_rows, 1, "migration must not delete the legacy row copy");
        let old_writer = backend
            .run(|c| {
                Ok(c.execute(
                    "INSERT INTO leases(address, occupant, since_ms, heartbeat_at_ms)
                     VALUES ('addr:old-writer', 'old', 1, 1)",
                    [],
                ))
            })
            .await
            .unwrap();
        assert!(
            old_writer.is_err(),
            "non-epoch legacy writes must fail after migration"
        );
        // Raw SQL simulates the stale binary's non-epoch heartbeat path; a separate old-binary
        // process harness is still the remaining end-to-end cutover axis.
        let old_update = backend
            .run(|c| {
                Ok(c.execute(
                    "UPDATE leases SET heartbeat_at_ms=999 WHERE address='addr:legacy'",
                    [],
                ))
            })
            .await
            .unwrap();
        assert!(
            old_update.is_err(),
            "non-epoch legacy UPDATE heartbeat must fail after migration"
        );
        assert!(backend
            .heartbeat_epoch("addr:legacy", "missing-owner", 1)
            .await
            .is_ok());

        let claimed = backend
            .claim_epoch_lease("addr:legacy", "daemon-new", 15)
            .await
            .unwrap();
        match claimed {
            EpochClaimResult::Claimed(claimed) => {
                assert_eq!(claimed.lease_epoch, 2);
                assert!(claimed.legacy_cutover);
            }
            other => panic!("expected claim, got {other:?}"),
        }
        assert!(backend
            .heartbeat_epoch("addr:legacy", "daemon-new", 2)
            .await
            .unwrap());
        assert!(backend
            .release_epoch_lease("addr:legacy", "daemon-new", 2)
            .await
            .unwrap());
    }

    #[tokio::test]
    async fn nullable_non_epoch_row_claims_explicitly_without_delete_then_advances() {
        let path = test_db_path("null-epoch");
        {
            let c = Connection::open(&path).unwrap();
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
                VALUES ('addr:null', 'legacy-holder', 'host', 'principal', 10, 20, NULL, 'legacy-owner');",
            )
            .unwrap();
        }

        let backend = SqliteBackend::open(&path.to_string_lossy()).unwrap();
        backend.init_schema().await.unwrap();
        let claimed = backend
            .claim_epoch_lease("addr:null", "daemon-new", 15)
            .await
            .unwrap();
        match claimed {
            EpochClaimResult::Claimed(claimed) => {
                assert_eq!(claimed.lease_epoch, 1);
                assert!(claimed.legacy_cutover);
            }
            other => panic!("expected legacy claim, got {other:?}"),
        }
        let rows: i64 = backend
            .run(|c| {
                Ok(c.query_row(
                    "SELECT COUNT(*) FROM leases WHERE address='addr:null'",
                    [],
                    |r| r.get(0),
                )?)
            })
            .await
            .unwrap();
        assert_eq!(
            rows, 1,
            "explicit NULL->1 cutover must update, not delete/reinsert"
        );

        assert!(backend
            .release_epoch_lease("addr:null", "daemon-new", 1)
            .await
            .unwrap());
        let next = backend
            .claim_epoch_lease("addr:null", "daemon-next", 15)
            .await
            .unwrap();
        match next {
            EpochClaimResult::Claimed(claimed) => {
                assert_eq!(claimed.lease_epoch, 2);
                assert!(!claimed.legacy_cutover);
            }
            other => panic!("expected monotonic next claim, got {other:?}"),
        }
    }

    #[test]
    fn missing_store_file_id_rejects_path_hash_fallback() {
        let path = test_db_path("missing-file-id");
        let err = store_file_id(&path).expect_err("missing file must not hash the path");
        assert!(
            err.to_string().contains("canonicalizing")
                || err.to_string().contains("physical file identity"),
            "unexpected error: {err:#}"
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_store_lock_sddl_rejects_broad_aces() {
        let sid = windows_current_user_sid().expect("current SID");
        let private = format!("O:{sid}G:{sid}D:(A;;GA;;;SY)(A;;GA;;;BA)(A;;GA;;;{sid})");
        assert!(windows_owner_private_sddl_is_strict(&private, &sid));
        let broad = format!("O:{sid}G:{sid}D:(A;;GA;;;{sid})(A;;GR;;;WD)");
        assert!(!windows_owner_private_sddl_is_strict(&broad, &sid));
        // Deterministic parser coverage for the foreign-SID ACE; a true different-principal
        // directory pre-create still needs a two-token Windows integration harness.
        let foreign =
            format!("O:{sid}G:{sid}D:(A;;GA;;;SY)(A;;GA;;;{sid})(A;;GR;;;S-1-5-21-1-2-3-444)");
        assert!(!windows_owner_private_sddl_is_strict(&foreign, &sid));
    }

    #[tokio::test]
    async fn future_schema_version_fails_closed_before_mutation() {
        let path = test_db_path("future-schema");
        {
            let c = Connection::open(&path).unwrap();
            c.execute_batch(
                "CREATE TABLE telex_schema_version (
                    singleton INTEGER NOT NULL DEFAULT 1 UNIQUE,
                    version   INTEGER NOT NULL
                );
                INSERT INTO telex_schema_version(singleton, version) VALUES (1, 999);",
            )
            .unwrap();
        }
        let backend = SqliteBackend::open(&path.to_string_lossy()).unwrap();
        let err = backend
            .init_schema()
            .await
            .expect_err("future schema must be rejected");
        assert!(err.to_string().contains("newer than supported"));
    }

    #[tokio::test]
    async fn claim_uses_durable_clock_for_stale_cutoff_after_wall_clock_skew() {
        let path = test_db_path("durable-cutoff");
        let backend = SqliteBackend::open(&path.to_string_lossy()).unwrap();
        backend.init_schema().await.unwrap();
        let future_hwm = now_ms() + 120_000;
        backend
            .run(move |c| {
                c.execute("UPDATE clock_hwm SET hwm_ms=?1 WHERE id=1", params![future_hwm])?;
                c.execute(
                    "INSERT INTO leases(address, occupant, since_ms, heartbeat_at_ms, lease_epoch, owner_instance_id)
                     VALUES ('addr:skew', 'old', ?1, ?2, 4, 'old-owner')",
                    params![future_hwm - 30_000, future_hwm - 20_000],
                )?;
                Ok(())
            })
            .await
            .unwrap();

        let claimed = backend
            .claim_epoch_lease("addr:skew", "new-owner", 15)
            .await
            .unwrap();
        match claimed {
            EpochClaimResult::Claimed(claimed) => {
                assert_eq!(claimed.lease_epoch, 5);
                assert_eq!(claimed.owner_instance_id, "new-owner");
            }
            other => panic!("durable clock cutoff should reclaim stale owner, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn failed_claims_do_not_advance_durable_clock_or_make_fresh_owner_stale() {
        let path = test_db_path("failed-claim-clock");
        let backend = SqliteBackend::open(&path.to_string_lossy()).unwrap();
        backend.init_schema().await.unwrap();
        let future_hwm = now_ms() + 120_000;
        backend
            .run(move |c| {
                c.execute("UPDATE clock_hwm SET hwm_ms=?1 WHERE id=1", params![future_hwm])?;
                c.execute(
                    "INSERT INTO leases(address, occupant, since_ms, heartbeat_at_ms, lease_epoch, owner_instance_id, daemon_fence_token)
                     VALUES ('addr:fresh', 'fresh', ?1, ?1, 7, 'fresh-owner', 1)",
                    params![future_hwm],
                )?;
                Ok(())
            })
            .await
            .unwrap();

        for _ in 0..10 {
            match backend
                .claim_epoch_lease("addr:fresh", "blocked-owner", 1)
                .await
                .unwrap()
            {
                EpochClaimResult::AlreadyOwned {
                    lease_epoch,
                    owner_instance_id,
                    ..
                } => {
                    assert_eq!(lease_epoch, 7);
                    assert_eq!(owner_instance_id, "fresh-owner");
                }
                other => panic!("fresh owner must not become stale, got {other:?}"),
            }
        }
        let hwm_after: i64 = backend
            .run(|c| Ok(c.query_row("SELECT hwm_ms FROM clock_hwm WHERE id=1", [], |r| r.get(0))?))
            .await
            .unwrap();
        assert_eq!(hwm_after, future_hwm);
    }

    #[tokio::test]
    async fn delivery_backfill_marker_prevents_repeat_on_already_v2_reopen() {
        let path = test_db_path("backfill-once");
        let backend = SqliteBackend::open(&path.to_string_lossy()).unwrap();
        backend.init_schema().await.unwrap();
        backend
            .run(|c| {
                c.execute_batch(
                    "INSERT INTO messages(id, thread_id, from_addr, to_addr, kind, attention, body, sent_at_ms, created_at_ms)
                     VALUES (10, 10, 'sender', 'addr:once', 'note', 'background', 'body', 10, 10);
                     DELETE FROM telex_schema_meta WHERE key='delivery_backfill_v2_complete';",
                )?;
                Ok(())
            })
            .await
            .unwrap();

        backend.init_schema().await.unwrap();
        let first_count: i64 = backend
            .run(|c| {
                Ok(c.query_row(
                    "SELECT COUNT(*) FROM deliveries WHERE message_id=10 AND recipient='addr:once'",
                    [],
                    |r| r.get(0),
                )?)
            })
            .await
            .unwrap();
        assert_eq!(first_count, 1);
        let marker: String = backend
            .run(|c| {
                Ok(c.query_row(
                    "SELECT value FROM telex_schema_meta WHERE key='delivery_backfill_v2_complete'",
                    [],
                    |r| r.get(0),
                )?)
            })
            .await
            .unwrap();
        assert_eq!(marker, "1");

        backend
            .run(|c| {
                c.execute("DELETE FROM deliveries WHERE message_id=10", [])?;
                Ok(())
            })
            .await
            .unwrap();
        backend.init_schema().await.unwrap();
        let second_count: i64 = backend
            .run(|c| {
                Ok(c.query_row(
                    "SELECT COUNT(*) FROM deliveries WHERE message_id=10 AND recipient='addr:once'",
                    [],
                    |r| r.get(0),
                )?)
            })
            .await
            .unwrap();
        assert_eq!(second_count, 0, "marker should skip repeated full backfill");
    }

    #[tokio::test]
    async fn legacy_message_without_delivery_row_materializes_and_acks_consumed() {
        let path = test_db_path("legacy-no-delivery");
        {
            let c = Connection::open(&path).unwrap();
            c.execute_batch(
                "CREATE TABLE messages (
                    id            INTEGER PRIMARY KEY AUTOINCREMENT,
                    thread_id     INTEGER,
                    parent_id     INTEGER,
                    from_addr     TEXT,
                    to_addr       TEXT NOT NULL,
                    cc            TEXT,
                    kind          TEXT NOT NULL DEFAULT 'note',
                    attention     TEXT NOT NULL DEFAULT 'background',
                    requires_disposition INTEGER NOT NULL DEFAULT 0,
                    subject       TEXT,
                    body          TEXT NOT NULL,
                    metadata      TEXT,
                    sent_at_ms    INTEGER NOT NULL,
                    created_at_ms INTEGER NOT NULL
                );
                INSERT INTO messages(id, thread_id, from_addr, to_addr, kind, attention, body, sent_at_ms, created_at_ms)
                VALUES (1, 1, 'sender', 'addr:legacy-msg', 'note', 'background', 'body', 10, 10);",
            )
            .unwrap();
        }
        let backend = SqliteBackend::open(&path.to_string_lossy()).unwrap();
        backend.init_schema().await.unwrap();
        match backend
            .claim_epoch_lease("addr:legacy-msg", "owner", 15)
            .await
            .unwrap()
        {
            EpochClaimResult::Claimed(_) => {}
            other => panic!("expected claim, got {other:?}"),
        }
        let rows = backend.fetch_undelivered("addr:legacy-msg").await.unwrap();
        assert_eq!(rows.iter().map(|m| m.id).collect::<Vec<_>>(), vec![1]);
        assert_eq!(
            backend
                .mark_consumed_if_current_owner("addr:legacy-msg", "owner", 1, 1)
                .await
                .unwrap(),
            DeliveryOutcome::Marked
        );
        assert!(backend
            .fetch_undelivered("addr:legacy-msg")
            .await
            .unwrap()
            .is_empty());
    }
}
