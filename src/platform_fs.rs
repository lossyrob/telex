//! Shared owner-private filesystem and process-identity primitives.
//!
//! These were previously private inside `daemon::platform`. They are promoted here verbatim so a
//! second authority-bearing consumer — the durable station-intent store (ADR 0052) — inherits the
//! *same* fail-closed owner-private checks the daemon cap file and endpoint already rely on,
//! rather than growing a parallel, weaker implementation. `daemon::platform` re-exports every
//! promoted function, so daemon behavior is byte-for-byte unchanged.
//!
//! Three rules hold everywhere in this module:
//!
//! 1. **Fail closed.** Anything that cannot be positively verified is an error, never a silent
//!    "assume fine". There are no broad catches and no permissive fallbacks.
//! 2. **Check the open handle, not the path.** Every read-side check is made against the handle the
//!    read will use, so no path can be swapped between the check and the read.
//! 3. **Both platforms, same posture.** Windows gets a real DACL/owner/reparse-point check
//!    (`validate_owner_private_file_security`), not just "the file lives in a directory we trust".

use std::path::{Component, Path, PathBuf};
use std::sync::OnceLock;

pub type Result<T> = std::result::Result<T, FsError>;

/// Error surface of the shared primitives. Mirrors the two `DaemonError` variants these functions
/// used to produce so `daemon::platform`'s re-exports keep their exact error text.
#[derive(Debug)]
pub enum FsError {
    Io {
        action: &'static str,
        source: std::io::Error,
    },
    Unsupported {
        capability: &'static str,
        message: String,
    },
}

impl std::fmt::Display for FsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FsError::Io { action, source } => write!(f, "{action}: {source}"),
            FsError::Unsupported {
                capability,
                message,
            } => write!(f, "{capability} is unsupported on this platform: {message}"),
        }
    }
}

impl std::error::Error for FsError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            FsError::Io { source, .. } => Some(source),
            FsError::Unsupported { .. } => None,
        }
    }
}

pub(crate) fn io_err(action: &'static str, source: std::io::Error) -> FsError {
    FsError::Io { action, source }
}

fn unsupported(capability: &'static str, message: impl Into<String>) -> FsError {
    FsError::Unsupported {
        capability: capability_static(capability),
        message: message.into(),
    }
}

/// `capability` is already `'static`; this exists only so callers read symmetrically.
const fn capability_static(capability: &'static str) -> &'static str {
    capability
}

/// Metadata captured from the same open handle the content was read through, so an age or size
/// decision can never be made against a different file than the one that was read.
#[derive(Debug, Clone, Copy)]
pub struct OwnerOnlyFileMeta {
    pub len: u64,
    /// Modification time in unix epoch milliseconds, or `None` when the platform cannot report one.
    pub modified_ms: Option<i64>,
}

/// Read an owner-private file fail-closed, per-file, on both platforms.
///
/// Rejects (never "warns and continues"): a non-regular file, a symlink or reparse point, a file
/// larger than `max_bytes`, a foreign owner, and — on Unix — any group/world permission bit. On
/// Windows the file's own owner SID and DACL are validated (see
/// `validate_owner_private_file_security`), so a credential file living *outside* the intent scope
/// is still checkable rather than trusted because of where it happens to sit.
pub fn read_owner_only_file(path: &Path, max_bytes: u64) -> Result<Vec<u8>> {
    read_owner_only_file_with_meta(path, max_bytes).map(|(bytes, _)| bytes)
}

/// `read_owner_only_file` plus the handle's own metadata (size, mtime) for age-bounded reads.
pub fn read_owner_only_file_with_meta(
    path: &Path,
    max_bytes: u64,
) -> Result<(Vec<u8>, OwnerOnlyFileMeta)> {
    imp::read_owner_only_file_with_meta(path, max_bytes)
}

/// Run every owner-private security and shape check and return the handle's metadata **without
/// reading the contents**.
///
/// This exists so an age-bounded credential read can decide "too old" before the secret is ever
/// brought into memory: a stale credential must produce no read, no connection, and no probe.
pub fn stat_owner_only_file(path: &Path, max_bytes: u64) -> Result<OwnerOnlyFileMeta> {
    imp::open_owner_only_file(path, max_bytes).map(|(_, meta)| meta)
}

/// Write `bytes` to a fresh owner-only file and atomically move it into place.
///
/// `CREATE_NEW` semantics on a randomized sibling temp name plus `rename` means a reader never sees
/// a partial manifest and an attacker cannot pre-create the target to capture the write.
///
/// The retry loop exists for Windows: replacing a file another handle has open without
/// `FILE_SHARE_DELETE` fails with `ERROR_ACCESS_DENIED` (`PermissionDenied`), not
/// `AlreadyExists`, so the fallback branch never ran and a concurrent reader turned an ordinary
/// write into a hard error. Telex's own readers now open with `FILE_SHARE_DELETE`; the retry
/// covers everything else (antivirus, an indexer, an older telex process).
pub fn write_owner_only_file_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    const RENAME_ATTEMPTS: u32 = 10;
    const RENAME_RETRY: std::time::Duration = std::time::Duration::from_millis(20);
    let tmp = sibling_tmp_path(path);
    write_owner_only_file_exact(&tmp, bytes)?;
    let mut last: Option<std::io::Error> = None;
    for attempt in 0..RENAME_ATTEMPTS {
        match std::fs::rename(&tmp, path) {
            Ok(()) => return Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                if let Err(e) = std::fs::remove_file(path) {
                    let _ = std::fs::remove_file(&tmp);
                    return Err(io_err("replacing owner-only file", e));
                }
                last = Some(e);
            }
            Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => last = Some(e),
            Err(e) => {
                let _ = std::fs::remove_file(&tmp);
                return Err(io_err("installing owner-only file", e));
            }
        }
        if attempt + 1 < RENAME_ATTEMPTS {
            std::thread::sleep(RENAME_RETRY);
        }
    }
    let _ = std::fs::remove_file(&tmp);
    Err(io_err(
        "installing owner-only file",
        last.unwrap_or_else(|| std::io::Error::other("rename did not complete")),
    ))
}

fn sibling_tmp_path(path: &Path) -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(1);
    let file_name = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("owner-only");
    path.with_file_name(format!(
        "{file_name}.{}.{}.tmp",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ))
}

/// Does this path exist — with "could not tell" kept as its own answer.
///
/// [`Path::exists`] collapses *every* failure into `false`: an ACL that denies metadata, a
/// directory the process may not traverse, a volume that went away, a name the platform rejects.
/// That is precisely rule 1 of this module inverted — it turns "unverifiable" into a confident
/// negative — and every authority-bearing decision that asks "is there a record here?" is a
/// decision where the confident negative is the *unsafe* direction: an existing record that cannot
/// be stat'd reads as no record at all, and the caller then does whatever it does when a binding is
/// genuinely new.
///
/// So `Ok(false)` here is only ever a **positive** `NotFound` from the platform. Anything else is
/// `Err`, and the caller decides which of its typed failure states that is. Callers that legitimately
/// want a best-effort answer say so explicitly at the call site (`matches!(.., Ok(false))`,
/// `unwrap_or(true)`) rather than inheriting it from the probe.
pub fn path_present(path: &Path) -> Result<bool> {
    #[cfg(test)]
    if let Some(error) = stat_faults::injected(path) {
        return Err(io_err("checking whether a path exists", error));
    }
    match path.try_exists() {
        Ok(present) => Ok(present),
        // `ENOTDIR`: a component of the path is not a directory, so nothing can exist at this path
        // — a *proof* of absence exactly like `NotFound`, not an inability to tell. The two
        // platforms simply disagree about which they report: asking Windows about
        // `some-file\child` answers `Ok(false)`, while Unix raises `ENOTDIR`. Without this, a run
        // directory with debris where a scope belongs read as "undecidable" on Unix and as "empty"
        // on Windows, and the same registration was refused on one platform and admitted on the
        // other.
        Err(e) if e.kind() == std::io::ErrorKind::NotADirectory => Ok(false),
        Err(e) => Err(io_err("checking whether a path exists", e)),
    }
}

/// Test seam for the one condition a test cannot portably *produce*: a filesystem that answers
/// "I cannot tell you whether this exists".
///
/// Every real way to induce it is platform-specific and flaky in CI — a Unix `chmod 000` on the
/// parent directory is a no-op when the suite runs as root and has no Windows equivalent, a Windows
/// deny-ACE has no Unix equivalent, and a path the platform rejects outright is a different error on
/// each. The behavior under test is not "how does this platform deny metadata"; it is "what does
/// telex do when the answer is an error", so the error is injected at the single function that asks.
///
/// **Isolation.** The registry is global (not thread-local) because it has to survive a
/// multi-threaded runtime, and `cargo test` runs the whole module in one process with many tests in
/// flight at once. Three things keep one test's fault out of its neighbours':
///
/// 1. It applies to **one exact path**. Any other path — including a sibling under the same
///    directory — is answered by the real filesystem.
/// 2. Guards are **individually identified**, so overlapping guards stack instead of clobbering
///    each other. Keying by path alone meant a second guard for the same path silently replaced the
///    first, and whichever dropped first removed the fault both were relying on.
/// 3. A guard removes **only its own** entry on drop, including when the drop happens while a
///    panic unwinds, so a failing test cannot leave a fault armed for whatever runs next.
#[cfg(test)]
pub(crate) mod stat_faults {
    use std::collections::HashMap;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Mutex, OnceLock};

    struct Fault {
        /// Identifies the guard that installed this fault, so drop removes exactly one entry.
        id: u64,
        kind: std::io::ErrorKind,
        /// Probes to answer truthfully before the fault starts applying. Exists because some rules
        /// probe the same path twice — an entry check and a re-check after a failed load — and the
        /// second one is a distinct decision that has to be pinned on its own.
        skip: usize,
    }

    fn registry() -> &'static Mutex<HashMap<PathBuf, Vec<Fault>>> {
        static REGISTRY: OnceLock<Mutex<HashMap<PathBuf, Vec<Fault>>>> = OnceLock::new();
        REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
    }

    fn next_id() -> u64 {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        NEXT.fetch_add(1, Ordering::Relaxed)
    }

    /// While this guard is alive, [`super::path_present`] fails for this exact path instead of
    /// answering. Dropping it restores the real filesystem.
    #[must_use = "the fault is only active while the guard is alive"]
    #[derive(Debug)]
    pub(crate) struct Unstatable {
        path: PathBuf,
        id: u64,
    }

    impl Unstatable {
        /// The common shape: an existing path whose metadata the platform refuses to hand over.
        pub(crate) fn new(path: impl Into<PathBuf>) -> Self {
            Self::install(path, std::io::ErrorKind::PermissionDenied, 0)
        }

        /// The same, but only from the `skip + 1`-th probe of this path onward.
        pub(crate) fn after(path: impl Into<PathBuf>, skip: usize) -> Self {
            Self::install(path, std::io::ErrorKind::PermissionDenied, skip)
        }

        fn install(path: impl Into<PathBuf>, kind: std::io::ErrorKind, skip: usize) -> Self {
            let path = path.into();
            let id = next_id();
            registry()
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .entry(path.clone())
                .or_default()
                .push(Fault { id, kind, skip });
            Self { path, id }
        }
    }

    impl Drop for Unstatable {
        fn drop(&mut self) {
            // `unwrap_or_else(into_inner)`: a test panicking while holding this lock must not
            // poison the seam for every test that runs afterwards.
            let mut registry = registry()
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if let Some(faults) = registry.get_mut(&self.path) {
                faults.retain(|fault| fault.id != self.id);
                if faults.is_empty() {
                    registry.remove(&self.path);
                }
            }
        }
    }

    pub(super) fn injected(path: &Path) -> Option<std::io::Error> {
        let mut registry = registry()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        // Innermost guard wins, so a nested fault shadows the outer one for as long as it lives.
        let fault = registry.get_mut(path)?.last_mut()?;
        if fault.skip > 0 {
            fault.skip -= 1;
            return None;
        }
        Some(std::io::Error::new(
            fault.kind,
            format!("injected stat fault for {}", path.display()),
        ))
    }
}

/// Resolve `path` and prove it is strictly contained under `root`.
///
/// Containment is decided on canonicalized paths (never a string prefix), `..` is rejected in the
/// input, and every component of the **caller's literal path** at or below `root` is checked for a
/// symlink/reparse point, so a link planted inside the root cannot redirect a read out of it — and
/// a link is refused even when its target happens to land back inside the root.
pub fn contained_under(root: &Path, path: &Path) -> Result<PathBuf> {
    if path.components().any(|c| matches!(c, Component::ParentDir)) {
        return Err(unsupported(
            "owner-private path containment",
            format!("{} contains a parent-directory component", path.display()),
        ));
    }
    let canonical_root = std::fs::canonicalize(root)
        .map_err(|e| io_err("canonicalizing owner-private containment root", e))?;
    let canonical_path = std::fs::canonicalize(path)
        .map_err(|e| io_err("canonicalizing owner-private contained path", e))?;
    if !canonical_path.starts_with(&canonical_root) || canonical_path == canonical_root {
        return Err(unsupported(
            "owner-private path containment",
            format!(
                "{} does not resolve strictly under {}",
                path.display(),
                root.display()
            ),
        ));
    }
    // Walk root -> path and reject any link on the chain.
    //
    // The walk is over the **caller's literal path**, not the canonicalized one. Walking the
    // canonical path was a no-op check: `canonicalize` resolves every link away, so the chain it
    // produces is symlink-free by construction and this loop could never fire — `contained_under`
    // accepted a symlink placed directly inside the root as long as its target also landed inside
    // the root. Canonicalization proves *where the path ends up*; only the literal chain can prove
    // *how it got there*.
    //
    // Only the segment at or below the root is telex's to police, and "at the root" is decided by
    // resolving each prefix rather than by string matching, so a caller that spells the root
    // differently (a manifest supplies these paths verbatim) is still covered. The root's own
    // ancestors are the operator's business — `/tmp` and `/var` are symlinks on macOS — and the
    // root itself was already resolved above.
    let mut walked = PathBuf::new();
    let mut below_root = false;
    for component in path.components() {
        walked.push(component);
        if !below_root {
            below_root = std::fs::canonicalize(&walked)
                .map(|resolved| resolved == canonical_root)
                .unwrap_or(false);
            continue;
        }
        let meta = std::fs::symlink_metadata(&walked)
            .map_err(|e| io_err("checking owner-private containment chain", e))?;
        if meta.file_type().is_symlink() {
            return Err(unsupported(
                "owner-private path containment",
                format!("{} traverses a symlink", walked.display()),
            ));
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if meta.file_attributes() & imp::FILE_ATTRIBUTE_REPARSE_POINT_BIT != 0 {
                return Err(unsupported(
                    "owner-private path containment",
                    format!("{} traverses a reparse point", walked.display()),
                ));
            }
        }
    }
    if !below_root {
        // The literal chain never passed through the root, so nothing on it was checked. The
        // resolved path is inside the root, which means it got there through a link above or at
        // the root — exactly the redirection this function exists to refuse.
        return Err(unsupported(
            "owner-private path containment",
            format!(
                "{} reaches {} only by traversing a link",
                path.display(),
                root.display()
            ),
        ));
    }
    Ok(canonical_path)
}

/// Create (or repair) an owner-private directory and return its canonical path.
pub fn ensure_owner_private_dir(path: &Path) -> Result<PathBuf> {
    imp::ensure_owner_private_dir(path)
}

/// Ensure a **producer root** — a directory telex shares with an external same-user producer — is
/// owner-private, and return its canonical path.
///
/// This is deliberately *not* `ensure_owner_private_dir`. That function rewrites the directory's
/// DACL to the daemon's protected, non-inheritable owner-only descriptor, which is correct for a
/// directory telex owns outright but destructive for one it shares: on Windows, protecting a
/// directory's DACL re-propagates inheritance to its children, and any file whose access came
/// purely from inherited ACEs — every file the producer wrote before telex touched the directory —
/// is left with an empty DACL and becomes unreadable *to everyone, including its own author*.
/// Hardening the bridge root that way would break the very producer this feature exists to keep
/// alive.
///
/// So the rule here is create-strict, validate-existing:
/// * A directory that does not exist yet is created with the owner-only descriptor, so it is
///   owner-private from birth and has no children to strip.
/// * A directory that already exists is **validated, never rewritten**: its owner must be a SID
///   this process may own objects as (see `imp::self_owner_sids` — the token user, the token's
///   default owner, and any `SE_GROUP_OWNER` group; on an elevated token that includes
///   `Administrators`, which is what Windows actually stamps on the directories such a process
///   creates), a DACL must be present, and every allowed ACE must name the current user, `SYSTEM`,
///   or local `Administrators`. `Everyone`, `Authenticated Users`, `Users`, or any foreign SID
///   fails closed.
///
/// The posture is therefore unchanged — a non-owner-private root is still refused — while the
/// producer's own files keep working. Per-file checks (`read_owner_only_file`) still apply to the
/// credential itself, so containment in this directory is never the only thing being trusted.
pub fn ensure_owner_private_producer_root(path: &Path) -> Result<PathBuf> {
    imp::ensure_owner_private_producer_root(path)
}

/// Create a brand-new owner-only file containing `bytes` followed by a newline.
///
/// Preserved verbatim from `daemon::platform` — the daemon cap file's on-disk shape depends on the
/// trailing newline, so the exact-bytes writer is a separate function.
pub fn write_owner_only_file(path: &Path, bytes: &[u8]) -> Result<()> {
    imp::write_owner_only_file(path, bytes, true)
}

/// Create a brand-new owner-only file containing exactly `bytes`.
pub fn write_owner_only_file_exact(path: &Path, bytes: &[u8]) -> Result<()> {
    imp::write_owner_only_file(path, bytes, false)
}

/// Absolute, canonical executable path of a live process. Fails closed when it cannot be resolved.
pub fn process_exe_path(pid: u32) -> Result<PathBuf> {
    imp::process_exe_path(pid)
}

/// Stable machine identity, hashed before it is returned so no raw machine identifier is ever
/// persisted into an intent manifest, status projection, or event log.
pub fn host_id() -> Result<String> {
    imp::raw_host_id().map(|raw| hashed_identity("host", &raw))
}

/// Boot-session identity, hashed like `host_id`. Distinguishes a reused `(pid, start_time)` pair
/// across a reboot — the Linux boot-relative start-time reproducibility hole.
///
/// Resolved once per process. The value is compared for *exact equality* across processes (the
/// attaching CLI writes it into a station intent, the daemon recomputes it), and a disagreement
/// terminates every intent as `foreign_host_or_boot`, so a single stable answer per process is
/// part of the contract rather than an optimization.
pub fn boot_id() -> Result<String> {
    static CACHE: OnceLock<std::result::Result<String, String>> = OnceLock::new();
    match CACHE.get_or_init(|| {
        imp::raw_boot_id(None)
            .map(|raw| hashed_identity("boot", &raw))
            .map_err(|e| e.to_string())
    }) {
        Ok(id) => Ok(id.clone()),
        Err(message) => Err(FsError::Unsupported {
            capability: "boot session identity",
            message: message.clone(),
        }),
    }
}

/// [`boot_id`] with the per-process memoization bypassed.
///
/// `#[doc(hidden)]`: a test seam, not API. Every consumer wants the cached value; a test that
/// asserts "two independent processes agree" must not be able to satisfy itself from a `OnceLock`
/// that was populated once, because that asserts nothing about the resolver at all. This calls the
/// platform resolver each time, which on Windows means exercising the persistence and read-back
/// path on every call.
#[doc(hidden)]
pub fn boot_id_uncached() -> Result<String> {
    imp::raw_boot_id(None).map(|raw| hashed_identity("boot", &raw))
}

/// The test-only scope handed to the platform resolver: an isolated storage namespace, and
/// optionally an observation point inside the cold-start path to synchronize on.
///
/// Private, with private fields, and it appears in no public signature. That is the enforcement:
/// the production resolvers ([`boot_id`], [`boot_id_uncached`]) pass `None`, so there is no value
/// of this type in existence on the production path and therefore no way for the barrier hook to
/// be reached without a caller that first named — and had validated — a test namespace.
struct TestBootIdScope<'a> {
    namespace: &'a str,
    /// Invoked at most once, and only when the pre-lock read found the record missing or invalid:
    /// after that observation, before the mint lock is acquired and the record re-checked. That is
    /// the exact instant a lost-update race is decided, so a test that parks every participant here
    /// makes the race happen by construction rather than by scheduling luck.
    at_cold_start: Option<&'a dyn Fn()>,
}

/// [`boot_id_uncached`] resolved inside an isolated, test-only storage namespace.
///
/// `#[doc(hidden)]`: a test seam, not API, and deliberately **not** an environment variable. The
/// production resolver reads exactly one location (`HKCU\Software\telex` on Windows, the kernel on
/// Unix) and nothing a parent process sets can repoint it — that environment-independence is the
/// whole reason the identity lives in the registry rather than under `%LOCALAPPDATA%`, because two
/// processes that disagree turn every station intent into `foreign_host_or_boot`.
///
/// A cold-start test still has to be able to *delete* the record before racing several processes at
/// it, and doing that to the real per-user record would knock over any daemon running on the
/// developer's machine. So the namespace is passed explicitly by the caller: the library never
/// reads it from the environment, and the concurrency test propagates its own variable to its own
/// child test binaries.
///
/// `namespace` is validated (ASCII alphanumeric, `-`, `_`; 1..=48 bytes) so it can never escape the
/// test container into the production key, and the result carries the same 32-character contract as
/// [`boot_id`].
#[doc(hidden)]
pub fn boot_id_uncached_in_test_namespace(namespace: &str) -> Result<String> {
    validate_test_boot_id_namespace(namespace)?;
    imp::raw_boot_id(Some(TestBootIdScope {
        namespace,
        at_cold_start: None,
    }))
    .map(|raw| hashed_identity("boot", &raw))
}

/// [`boot_id_uncached_in_test_namespace`], with `at_cold_start` invoked at the instant the caller
/// has observed a missing or invalid record and has not yet taken the mint lock.
///
/// `#[doc(hidden)]`: the determinism seam for the cross-process cold-start regression. Without it
/// the test asserts on a race it merely *hopes* occurred — a launch barrier releases twelve
/// processes together, but nothing stops the first one from finishing its whole mint before the
/// twelfth has read the key, in which case the eleven others take the uncontended
/// already-a-record path and a resolver with no serialization at all still passes. Parking every
/// participant here and releasing them together makes "all twelve saw an empty record" a
/// precondition of the assertion rather than a hope.
///
/// It cannot be reached from [`boot_id`] or [`boot_id_uncached`]: the hook travels only inside
/// [`TestBootIdScope`], which the production resolvers never construct, and reaching it at all
/// requires a validated test namespace, so the parked window can never be opened over the real
/// per-user record.
///
/// On hosts whose boot identity comes from the kernel (Linux, macOS) there is no record and no
/// mint, so the resolver has no such instant and the hook is never invoked. That is the parity
/// statement, not an omission: there is no race to make deterministic there.
#[doc(hidden)]
pub fn boot_id_uncached_in_test_namespace_at_cold_start(
    namespace: &str,
    at_cold_start: &dyn Fn(),
) -> Result<String> {
    validate_test_boot_id_namespace(namespace)?;
    imp::raw_boot_id(Some(TestBootIdScope {
        namespace,
        at_cold_start: Some(at_cold_start),
    }))
    .map(|raw| hashed_identity("boot", &raw))
}

/// Delete the persisted record of a test namespace, so the next resolution in it is a cold start.
///
/// `#[doc(hidden)]`: the other half of [`boot_id_uncached_in_test_namespace`]. It can only ever
/// address the validated test container, never the production record. On platforms whose boot
/// identity comes from the kernel (Linux, macOS) nothing is persisted, so this is a no-op — which
/// is the parity statement: there is no mint to race there.
///
/// Deliberately per-namespace, with no "remove every test namespace" companion. A sweep is
/// unscoped by definition, and `cargo test` runs test binaries — and developers run several
/// checkouts — concurrently: one run's cleanup would delete a namespace another run was mid-race
/// in, turning an unrelated suite red for reasons invisible in its own output. Each run owns
/// exactly the namespace it named, and removes exactly that, including on panic.
#[doc(hidden)]
pub fn clear_test_boot_id_namespace(namespace: &str) -> Result<()> {
    validate_test_boot_id_namespace(namespace)?;
    imp::clear_boot_id_namespace(namespace)
}

fn validate_test_boot_id_namespace(namespace: &str) -> Result<()> {
    let acceptable = (1..=48).contains(&namespace.len())
        && namespace
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
    if !acceptable {
        return Err(FsError::Unsupported {
            capability: "boot session identity",
            message: format!(
                "{namespace:?} is not a usable boot-identity test namespace; it must be 1..=48 \
                 bytes of ASCII alphanumerics, '-', or '_'"
            ),
        });
    }
    Ok(())
}

fn hashed_identity(domain: &str, raw: &str) -> String {
    let mut material = Vec::with_capacity(domain.len() + raw.len() + 1);
    material.extend_from_slice(domain.as_bytes());
    material.push(0x1f);
    material.extend_from_slice(raw.as_bytes());
    sha256_hex(&material)[..32].to_string()
}

// ---------------------------------------------------------------------------------------------
// SHA-256
// ---------------------------------------------------------------------------------------------

/// SHA-256 as lowercase hex.
///
/// Implemented here rather than pulled from `sha2` because `sha2` is only in the dependency graph
/// behind the optional `self-update` feature, and intent identity must be identical in every
/// feature combination (including `--no-default-features --features sqlite`). Adding a mandatory
/// crypto dependency for one hash would be a larger change than 60 lines of a fully specified
/// algorithm, and the identity must never differ between builds.
pub fn sha256_hex(bytes: &[u8]) -> String {
    let digest = sha256(bytes);
    let mut out = String::with_capacity(64);
    for byte in digest {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

const SHA256_K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

fn sha256(bytes: &[u8]) -> [u8; 32] {
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];
    let bit_len = (bytes.len() as u64).wrapping_mul(8);
    let mut padded = bytes.to_vec();
    padded.push(0x80);
    while padded.len() % 64 != 56 {
        padded.push(0);
    }
    padded.extend_from_slice(&bit_len.to_be_bytes());

    for chunk in padded.as_chunks::<64>().0 {
        let mut w = [0u32; 64];
        for (i, word) in chunk.as_chunks::<4>().0.iter().enumerate() {
            w[i] = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let (mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh) =
            (h[0], h[1], h[2], h[3], h[4], h[5], h[6], h[7]);
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let temp1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(SHA256_K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }
        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
        h[5] = h[5].wrapping_add(f);
        h[6] = h[6].wrapping_add(g);
        h[7] = h[7].wrapping_add(hh);
    }

    let mut out = [0u8; 32];
    for (i, word) in h.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
    }
    out
}

fn system_time_to_ms(time: std::time::SystemTime) -> Option<i64> {
    time.duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_millis()).ok())
}

// ---------------------------------------------------------------------------------------------
// Unix
// ---------------------------------------------------------------------------------------------

#[cfg(unix)]
mod imp {
    use super::{io_err, system_time_to_ms, FsError, OwnerOnlyFileMeta, Result};
    use std::io::Read;
    use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
    use std::path::{Path, PathBuf};

    pub(super) fn ensure_owner_private_dir(path: &Path) -> Result<PathBuf> {
        match std::fs::symlink_metadata(path) {
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let mut builder = std::fs::DirBuilder::new();
                builder.recursive(true).mode(0o700);
                let mut create_error = None;
                for _ in 0..25 {
                    match builder.create(path) {
                        Ok(()) => {
                            create_error = None;
                            break;
                        }
                        Err(error) => {
                            create_error = Some(error);
                            if std::fs::symlink_metadata(path).is_ok() {
                                create_error = None;
                                break;
                            }
                            std::thread::sleep(std::time::Duration::from_millis(10));
                        }
                    }
                }
                if let Some(error) = create_error {
                    return Err(io_err("creating owner-private daemon directory", error));
                }
            }
            Err(error) => {
                return Err(io_err("checking owner-private daemon directory", error));
            }
        }
        let link_meta = std::fs::symlink_metadata(path)
            .map_err(|e| io_err("checking owner-private daemon directory", e))?;
        if link_meta.file_type().is_symlink() {
            return Err(FsError::Unsupported {
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
            return Err(FsError::Unsupported {
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
            return Err(FsError::Unsupported {
                capability: "owner-private daemon directory",
                message: format!("{} is group/world accessible", path.display()),
            });
        }
        std::fs::canonicalize(path).map_err(|e| io_err("canonicalizing daemon directory", e))
    }

    /// On Unix the owner-private rule is `chmod 0700` plus a uid check, and POSIX permissions are
    /// not inherited by children, so hardening a shared directory cannot strip access from files
    /// already inside it. The producer-root rule is therefore identical to the plain one here; the
    /// two only diverge on Windows.
    pub(super) fn ensure_owner_private_producer_root(path: &Path) -> Result<PathBuf> {
        ensure_owner_private_dir(path)
    }

    pub(super) fn write_owner_only_file(
        path: &Path,
        bytes: &[u8],
        trailing_newline: bool,
    ) -> Result<()> {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)
            .map_err(|e| io_err("creating owner-only daemon capability file", e))?;
        use std::io::Write;
        file.write_all(bytes)
            .map_err(|e| io_err("writing daemon capability file", e))?;
        if trailing_newline {
            file.write_all(b"\n")
                .map_err(|e| io_err("writing daemon capability file", e))?;
        }
        file.sync_all()
            .map_err(|e| io_err("syncing daemon capability file", e))?;
        Ok(())
    }

    pub(super) fn open_owner_only_file(
        path: &Path,
        max_bytes: u64,
    ) -> Result<(std::fs::File, OwnerOnlyFileMeta)> {
        // O_NOFOLLOW: a symlink at the final component fails the open outright, so the checks below
        // and the read that follows are guaranteed to be about the same inode.
        let file = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(path)
            .map_err(|e| io_err("opening owner-only file", e))?;
        let meta = file
            .metadata()
            .map_err(|e| io_err("inspecting owner-only file", e))?;
        if !meta.file_type().is_file() {
            return Err(FsError::Unsupported {
                capability: "owner-only file read",
                message: format!("{} is not a regular file", path.display()),
            });
        }
        let uid = unsafe { libc::geteuid() };
        if meta.uid() != uid {
            return Err(FsError::Unsupported {
                capability: "owner-only file read",
                message: format!(
                    "{} is owned by uid {}, expected uid {}",
                    path.display(),
                    meta.uid(),
                    uid
                ),
            });
        }
        if meta.mode() & 0o077 != 0 {
            return Err(FsError::Unsupported {
                capability: "owner-only file read",
                message: format!("{} is group/world accessible", path.display()),
            });
        }
        if meta.len() > max_bytes {
            return Err(FsError::Unsupported {
                capability: "owner-only file read",
                message: format!(
                    "{} is {} bytes, over the {max_bytes} byte cap",
                    path.display(),
                    meta.len()
                ),
            });
        }
        let modified_ms = meta.modified().ok().and_then(system_time_to_ms);
        Ok((
            file,
            OwnerOnlyFileMeta {
                len: meta.len(),
                modified_ms,
            },
        ))
    }

    pub(super) fn read_owner_only_file_with_meta(
        path: &Path,
        max_bytes: u64,
    ) -> Result<(Vec<u8>, OwnerOnlyFileMeta)> {
        let (file, meta) = open_owner_only_file(path, max_bytes)?;
        let mut buf = Vec::with_capacity(meta.len as usize);
        file.take(max_bytes + 1)
            .read_to_end(&mut buf)
            .map_err(|e| io_err("reading owner-only file", e))?;
        if buf.len() as u64 > max_bytes {
            return Err(FsError::Unsupported {
                capability: "owner-only file read",
                message: format!("{} grew past the {max_bytes} byte cap", path.display()),
            });
        }
        Ok((buf, meta))
    }

    #[cfg(target_os = "linux")]
    pub(super) fn process_exe_path(pid: u32) -> Result<PathBuf> {
        std::fs::canonicalize(format!("/proc/{pid}/exe")).map_err(|e| FsError::Unsupported {
            capability: "process executable resolution",
            message: format!("cannot verify /proc/{pid}/exe: {e}"),
        })
    }

    #[cfg(target_os = "macos")]
    pub(super) fn process_exe_path(pid: u32) -> Result<PathBuf> {
        use std::ffi::OsString;
        use std::os::unix::ffi::OsStringExt;

        let mut buffer = vec![0u8; libc::PROC_PIDPATHINFO_MAXSIZE as usize];
        let bytes = unsafe {
            libc::proc_pidpath(
                pid as libc::c_int,
                buffer.as_mut_ptr() as *mut libc::c_void,
                buffer.len() as u32,
            )
        };
        if bytes <= 0 {
            return Err(FsError::Unsupported {
                capability: "process executable resolution",
                message: format!(
                    "cannot resolve executable path for pid {pid}: {}",
                    std::io::Error::last_os_error()
                ),
            });
        }
        buffer.truncate(bytes as usize);
        if buffer.last() == Some(&0) {
            buffer.pop();
        }
        let path = PathBuf::from(OsString::from_vec(buffer));
        std::fs::canonicalize(&path).map_err(|e| FsError::Unsupported {
            capability: "process executable resolution",
            message: format!("cannot canonicalize {} for pid {pid}: {e}", path.display()),
        })
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    pub(super) fn process_exe_path(_pid: u32) -> Result<PathBuf> {
        Err(FsError::Unsupported {
            capability: "process executable resolution",
            message: "process executable resolution is only wired for Linux and macOS".into(),
        })
    }

    #[cfg(target_os = "linux")]
    pub(super) fn raw_host_id() -> Result<String> {
        for candidate in ["/etc/machine-id", "/var/lib/dbus/machine-id"] {
            if let Ok(raw) = std::fs::read_to_string(candidate) {
                let trimmed = raw.trim();
                if !trimmed.is_empty() {
                    return Ok(trimmed.to_string());
                }
            }
        }
        Err(FsError::Unsupported {
            capability: "stable host identity",
            message: "neither /etc/machine-id nor /var/lib/dbus/machine-id is readable".into(),
        })
    }

    #[cfg(target_os = "macos")]
    pub(super) fn raw_host_id() -> Result<String> {
        sysctl_string("kern.uuid").ok_or(FsError::Unsupported {
            capability: "stable host identity",
            message: "sysctl kern.uuid is unavailable".into(),
        })
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    pub(super) fn raw_host_id() -> Result<String> {
        Err(FsError::Unsupported {
            capability: "stable host identity",
            message: "host identity is only wired for Linux and macOS".into(),
        })
    }

    /// Linux takes the boot identity straight from the kernel, so there is nothing to mint, nothing
    /// to persist, and therefore no first-writer race for a test scope to isolate or synchronize.
    /// Every process on the host reads the same bytes; the parameter exists only so the
    /// cross-platform seam has one shape.
    #[cfg(target_os = "linux")]
    pub(super) fn raw_boot_id(_scope: Option<super::TestBootIdScope<'_>>) -> Result<String> {
        let raw = std::fs::read_to_string("/proc/sys/kernel/random/boot_id").map_err(|e| {
            FsError::Unsupported {
                capability: "boot session identity",
                message: format!("cannot read /proc/sys/kernel/random/boot_id: {e}"),
            }
        })?;
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Err(FsError::Unsupported {
                capability: "boot session identity",
                message: "/proc/sys/kernel/random/boot_id is empty".into(),
            });
        }
        Ok(trimmed.to_string())
    }

    /// macOS derives the identity from `kern.boottime`, which the kernel fixes at boot and every
    /// process reads identically. Same as Linux: no mint, no persistence, no race.
    #[cfg(target_os = "macos")]
    pub(super) fn raw_boot_id(_scope: Option<super::TestBootIdScope<'_>>) -> Result<String> {
        let mut boottime: libc::timeval = unsafe { std::mem::zeroed() };
        let mut size = std::mem::size_of::<libc::timeval>();
        let name = std::ffi::CString::new("kern.boottime").expect("static sysctl name");
        let rc = unsafe {
            libc::sysctlbyname(
                name.as_ptr(),
                &mut boottime as *mut _ as *mut libc::c_void,
                &mut size,
                std::ptr::null_mut(),
                0,
            )
        };
        if rc != 0 || boottime.tv_sec == 0 {
            return Err(FsError::Unsupported {
                capability: "boot session identity",
                message: "sysctl kern.boottime is unavailable".into(),
            });
        }
        Ok(format!("{}.{}", boottime.tv_sec, boottime.tv_usec))
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    pub(super) fn raw_boot_id(_scope: Option<super::TestBootIdScope<'_>>) -> Result<String> {
        Err(FsError::Unsupported {
            capability: "boot session identity",
            message: "boot identity is only wired for Linux and macOS".into(),
        })
    }

    /// Nothing is persisted on Unix — the identity is kernel state — so there is no test record to
    /// remove and no cold start to arrange.
    pub(super) fn clear_boot_id_namespace(_namespace: &str) -> Result<()> {
        Ok(())
    }

    #[cfg(target_os = "macos")]
    fn sysctl_string(name: &str) -> Option<String> {
        let cname = std::ffi::CString::new(name).ok()?;
        let mut buf = vec![0u8; 256];
        let mut size = buf.len();
        let rc = unsafe {
            libc::sysctlbyname(
                cname.as_ptr(),
                buf.as_mut_ptr() as *mut libc::c_void,
                &mut size,
                std::ptr::null_mut(),
                0,
            )
        };
        if rc != 0 || size == 0 {
            return None;
        }
        buf.truncate(size);
        while buf.last() == Some(&0) {
            buf.pop();
        }
        String::from_utf8(buf).ok().filter(|s| !s.is_empty())
    }
}

// ---------------------------------------------------------------------------------------------
// Windows
// ---------------------------------------------------------------------------------------------

#[cfg(windows)]
mod imp {
    use super::{io_err, system_time_to_ms, FsError, OwnerOnlyFileMeta, Result};
    use std::ffi::{c_void, OsStr, OsString};
    use std::io::Read;
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    use std::os::windows::fs::MetadataExt;
    use std::os::windows::io::{AsRawHandle, FromRawHandle};
    use std::path::{Component, Path, PathBuf, Prefix};
    use std::sync::OnceLock;
    use windows_sys::Win32::Foundation::{
        CloseHandle, GetLastError, LocalFree, ERROR_ALREADY_EXISTS, ERROR_FILE_NOT_FOUND, HANDLE,
        INVALID_HANDLE_VALUE, PSID, WAIT_ABANDONED, WAIT_OBJECT_0, WAIT_TIMEOUT,
    };
    use windows_sys::Win32::Security::Authorization::{
        ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
        ConvertStringSidToSidW, GetNamedSecurityInfoW, GetSecurityInfo, SetNamedSecurityInfoW,
        SDDL_REVISION_1, SE_FILE_OBJECT,
    };
    use windows_sys::Win32::Security::{
        AclSizeInformation, EqualSid, GetAce, GetAclInformation, GetSecurityDescriptorControl,
        GetSecurityDescriptorDacl, GetSecurityDescriptorOwner, GetTokenInformation, TokenGroups,
        TokenOwner, TokenUser, ACCESS_ALLOWED_ACE, ACL, ACL_SIZE_INFORMATION,
        DACL_SECURITY_INFORMATION, OWNER_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION,
        SECURITY_ATTRIBUTES, SE_DACL_PRESENT, TOKEN_GROUPS, TOKEN_INFORMATION_CLASS, TOKEN_OWNER,
        TOKEN_QUERY, TOKEN_USER,
    };
    use windows_sys::Win32::Storage::FileSystem::{
        CreateDirectoryW, CreateFileW, CREATE_NEW, FILE_ATTRIBUTE_NORMAL,
        FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_OPEN_REPARSE_POINT, FILE_GENERIC_READ,
        FILE_GENERIC_WRITE, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
    };
    use windows_sys::Win32::System::Registry::{
        RegDeleteKeyExW, RegDeleteTreeW, RegGetValueW, RegSetKeyValueW, HKEY_CURRENT_USER,
        HKEY_LOCAL_MACHINE, REG_SZ, RRF_RT_REG_SZ,
    };
    use windows_sys::Win32::System::SystemInformation::GetTickCount64;
    use windows_sys::Win32::System::Threading::{
        CreateMutexW, GetCurrentProcess, OpenProcess, OpenProcessToken, QueryFullProcessImageNameW,
        ReleaseMutex, WaitForSingleObject, PROCESS_QUERY_LIMITED_INFORMATION,
    };

    pub(super) const FILE_ATTRIBUTE_REPARSE_POINT_BIT: u32 = FILE_ATTRIBUTE_REPARSE_POINT;

    const ACCESS_ALLOWED_ACE_TYPE: u8 = 0;
    const ACCESS_DENIED_ACE_TYPE: u8 = 1;

    /// `SE_GROUP_OWNER` / `SE_GROUP_USE_FOR_DENY_ONLY` from `winnt.h`. `windows-sys` 0.52 binds the
    /// `TOKEN_GROUPS` struct but not these attribute bits, and they are fixed ABI values.
    const SE_GROUP_OWNER: u32 = 0x0000_0008;
    const SE_GROUP_USE_FOR_DENY_ONLY: u32 = 0x0000_0010;

    /// Whether an ACE trustee is one telex considers safe on an owner-private object.
    ///
    /// The set matches the enforced runtime notion of a strict owner-private descriptor: the
    /// current user, `SYSTEM`, local `Administrators`, and the per-logon-session SID
    /// (`S-1-5-5-X-Y`, which Windows puts in a token's default DACL and which is scoped to this
    /// logon). Everything else — notably `Everyone` (`S-1-1-0`), `Authenticated Users`
    /// (`S-1-5-11`), `Users` (`S-1-5-32-545`), and the broad AppContainer groups
    /// `ALL APPLICATION PACKAGES` (`S-1-15-2-1`) / `ALL RESTRICTED APPLICATION PACKAGES`
    /// (`S-1-15-2-2`) — is refused, so a broadened DACL fails closed rather than being quietly
    /// accepted.
    ///
    /// The `S-1-15-*` prefixes are deliberately **not** here. They existed only in a `#[cfg(test)]`
    /// SDDL helper before this module was promoted, never in the enforced path, and the two
    /// validate-only callers this feature added (`validate_owner_private_file_security` for the
    /// producer credential, and `ensure_owner_private_producer_root` for an existing bridge root)
    /// are exactly the places where a load-bearing allowlist must not include a group with the
    /// reach of `Users`.
    fn ace_trustee_is_allowlisted(
        sid: PSID,
        self_sids: &[String],
        system: PSID,
        admins: PSID,
    ) -> (bool, bool) {
        let text = sid_to_string(sid);
        let is_self = text
            .as_deref()
            .is_some_and(|text| self_sids.iter().any(|known| known == text));
        if is_self {
            return (true, true);
        }
        let privileged = unsafe { EqualSid(sid, system) != 0 || EqualSid(sid, admins) != 0 };
        if privileged {
            return (true, false);
        }
        match text {
            Some(text) => (text.starts_with("S-1-5-5-"), false),
            None => (false, false),
        }
    }

    fn sid_to_string(sid: PSID) -> Option<String> {
        let mut raw: *mut u16 = std::ptr::null_mut();
        let ok = unsafe { ConvertSidToStringSidW(sid, &mut raw) };
        if ok == 0 || raw.is_null() {
            return None;
        }
        let text = unsafe { wide_ptr_to_string(raw) };
        unsafe {
            LocalFree(raw as *mut c_void);
        }
        Some(text)
    }

    /// The SIDs this process may legitimately be the **owner** of an object as.
    ///
    /// This is deliberately not "the token user SID". Windows does not stamp the *user* on objects
    /// a process creates; it stamps the token's **default owner**, and on an elevated
    /// administrator token that is `BUILTIN\Administrators` (S-1-5-32-544), not the user. Every
    /// GitHub Actions Windows runner is such a token, which is why a directory the process had
    /// just created with `create_dir_all` — and a credential file it had just written — were both
    /// refused as "not owned by the current SID". That is the check misreading its own artifacts,
    /// not a real foreign owner.
    ///
    /// The set is therefore taken from the token itself, which is the only authority on the
    /// question:
    ///
    /// * `TokenUser` — the user this process runs as.
    /// * `TokenOwner` — the SID Windows actually stamps on objects this process creates.
    /// * every `TokenGroups` entry carrying `SE_GROUP_OWNER` — Windows' own definition of "a SID
    ///   this token is allowed to assign as an object's owner". Deny-only groups are skipped: on a
    ///   filtered (non-elevated) token `BUILTIN\Administrators` is present but marked
    ///   `SE_GROUP_USE_FOR_DENY_ONLY`, and such a token may *not* own objects as Administrators.
    ///
    /// This does not loosen the posture. A standard user's token yields exactly `{user SID}`, so
    /// the rule is byte-for-byte what it was. An administrator's token additionally admits
    /// `Administrators` — a principal the DACL allowlist (`ace_trustee_is_allowlisted`) already
    /// trusts unconditionally, and one that can take ownership of any object on the machine
    /// regardless. Anything the token does not name is still refused, so a genuinely foreign owner
    /// still fails closed.
    ///
    /// Cached: a process token's identity does not change for the life of the process, and this is
    /// consulted on every owner-private read.
    fn self_owner_sids() -> Result<&'static [String]> {
        static CACHE: OnceLock<Vec<String>> = OnceLock::new();
        if let Some(cached) = CACHE.get() {
            return Ok(cached.as_slice());
        }
        let computed = read_self_owner_sids()?;
        Ok(CACHE.get_or_init(|| computed).as_slice())
    }

    fn read_self_owner_sids() -> Result<Vec<String>> {
        let token = current_process_token()?;
        let mut sids: Vec<String> = Vec::new();

        let user = token_information(token.0, TokenUser, "reading token user information")?;
        let token_user = unsafe { &*(user.as_ptr() as *const TOKEN_USER) };
        push_sid(&mut sids, token_user.User.Sid);

        let owner = token_information(token.0, TokenOwner, "reading token owner information")?;
        let token_owner = unsafe { &*(owner.as_ptr() as *const TOKEN_OWNER) };
        push_sid(&mut sids, token_owner.Owner);

        let groups = token_information(token.0, TokenGroups, "reading token groups")?;
        let token_groups = unsafe { &*(groups.as_ptr() as *const TOKEN_GROUPS) };
        let entries = unsafe {
            std::slice::from_raw_parts(
                token_groups.Groups.as_ptr(),
                token_groups.GroupCount as usize,
            )
        };
        for entry in entries {
            if entry.Attributes & SE_GROUP_USE_FOR_DENY_ONLY != 0 {
                continue;
            }
            if entry.Attributes & SE_GROUP_OWNER == 0 {
                continue;
            }
            push_sid(&mut sids, entry.Sid);
        }

        if sids.is_empty() {
            // Fail closed: with no known self SID every owner comparison would be a guess.
            return Err(FsError::Unsupported {
                capability: "owner-only file read",
                message: "the process token named no SID it can own objects as".into(),
            });
        }
        Ok(sids)
    }

    fn push_sid(into: &mut Vec<String>, sid: PSID) {
        if sid.is_null() {
            return;
        }
        if let Some(text) = sid_to_string(sid) {
            if !into.contains(&text) {
                into.push(text);
            }
        }
    }

    /// `GetTokenInformation` with the standard size-then-read dance.
    ///
    /// The buffer is a `Vec<u64>` rather than a `Vec<u8>` so the returned bytes are guaranteed
    /// pointer-aligned: every structure read out of it (`TOKEN_USER`, `TOKEN_OWNER`,
    /// `TOKEN_GROUPS`) starts with a `PSID`, and reading those through a byte-aligned pointer is
    /// undefined behavior.
    fn token_information(
        token: HANDLE,
        class: TOKEN_INFORMATION_CLASS,
        action: &'static str,
    ) -> Result<Vec<u64>> {
        let mut needed = 0u32;
        unsafe {
            GetTokenInformation(token, class, std::ptr::null_mut(), 0, &mut needed);
        }
        if needed == 0 {
            return Err(io_err(action, std::io::Error::last_os_error()));
        }
        let mut buf = vec![0u64; needed as usize / std::mem::size_of::<u64>() + 1];
        let ok = unsafe {
            GetTokenInformation(
                token,
                class,
                buf.as_mut_ptr() as *mut c_void,
                needed,
                &mut needed,
            )
        };
        if ok == 0 {
            return Err(io_err(action, std::io::Error::last_os_error()));
        }
        Ok(buf)
    }

    /// Is `owner` a SID this process is allowed to own objects as? See [`self_owner_sids`].
    fn owner_is_self(owner: PSID, self_sids: &[String]) -> bool {
        if owner.is_null() {
            return false;
        }
        match sid_to_string(owner) {
            Some(text) => self_sids.contains(&text),
            None => false,
        }
    }

    /// Owner rejection text that names the SID actually found, so a real foreign-owner refusal is
    /// diagnosable instead of reading as "the check is broken".
    fn foreign_owner_message(path: &Path, owner: PSID) -> String {
        let found = sid_to_string(owner).unwrap_or_else(|| "an unreadable SID".to_string());
        format!(
            "{} is owned by {found}, which is not a SID this process can own objects as",
            path.display()
        )
    }

    /// The owner SID a path actually carries, as a string. Test-only: the enforced paths compare
    /// the raw `PSID` they already hold rather than round-tripping through a `String`.
    #[cfg(test)]
    fn owner_sid_of_path(path: &Path) -> Result<String> {
        let mut sd: *mut c_void = std::ptr::null_mut();
        let mut owner: PSID = std::ptr::null_mut();
        let wide = wide_null(path.as_os_str());
        let rc = unsafe {
            GetNamedSecurityInfoW(
                wide.as_ptr(),
                SE_FILE_OBJECT,
                OWNER_SECURITY_INFORMATION,
                &mut owner,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &mut sd,
            )
        };
        if rc != 0 {
            return Err(FsError::Unsupported {
                capability: "owner-private daemon directory",
                message: format!(
                    "cannot read owner for {}: {}",
                    path.display(),
                    std::io::Error::from_raw_os_error(rc as i32)
                ),
            });
        }
        let _sd_guard = LocalAllocGuard(sd);
        sid_to_string(owner).ok_or_else(|| FsError::Unsupported {
            capability: "owner-private daemon directory",
            message: format!("owner SID for {} is unreadable", path.display()),
        })
    }

    pub(super) fn ensure_owner_private_dir(path: &Path) -> Result<PathBuf> {
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
        validate_owner_private_dir_security(&canonical, true)?;
        Ok(canonical)
    }

    /// Create-strict, validate-existing. See the doc comment on
    /// `platform_fs::ensure_owner_private_producer_root` for why an existing shared directory is
    /// validated rather than rewritten.
    pub(super) fn ensure_owner_private_producer_root(path: &Path) -> Result<PathBuf> {
        let created = if path.exists() {
            false
        } else {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| io_err("creating producer root parent", e))?;
            }
            create_owner_only_dir(path)?;
            true
        };
        validate_owner_private_dir_shape(path)?;
        let canonical =
            std::fs::canonicalize(path).map_err(|e| io_err("canonicalizing producer root", e))?;
        validate_owner_private_dir_shape(&canonical)?;
        // A directory telex just created carries the protected owner-only descriptor and must
        // still look like one; an existing directory only has to be *safe*, not telex-shaped.
        validate_owner_private_dir_security(&canonical, created)?;
        Ok(canonical)
    }

    pub(super) fn write_owner_only_file(
        path: &Path,
        bytes: &[u8],
        trailing_newline: bool,
    ) -> Result<()> {
        use std::io::Write;
        let sa = owner_only_security_attributes()?;
        let wide = wide_null(path.as_os_str());
        let handle = unsafe {
            CreateFileW(
                wide.as_ptr(),
                FILE_GENERIC_WRITE,
                0,
                &sa.attrs,
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
        if trailing_newline {
            file.write_all(b"\n")
                .map_err(|e| io_err("writing daemon capability file", e))?;
        }
        file.sync_all()
            .map_err(|e| io_err("syncing daemon capability file", e))?;
        Ok(())
    }

    pub(super) fn open_owner_only_file(
        path: &Path,
        max_bytes: u64,
    ) -> Result<(std::fs::File, OwnerOnlyFileMeta)> {
        let wide = wide_null(path.as_os_str());
        // FILE_FLAG_OPEN_REPARSE_POINT: open the link itself rather than its target, so a reparse
        // point is *detected* below instead of silently followed.
        //
        // `FILE_SHARE_DELETE | FILE_SHARE_WRITE` alongside `FILE_SHARE_READ`: without
        // `FILE_SHARE_DELETE`, a concurrent `rename`-into-place over a file this handle has open
        // fails with `ERROR_ACCESS_DENIED` on Windows. The colliding pairs are real and
        // cross-process — the daemon scans every manifest in the scope while a CLI `attach` or
        // `finalize` rewrites one, and vice versa — so the missing share flag turned an ordinary
        // race into a hard error on the attach path and a dropped evidence write on the daemon
        // path. Sharing does not weaken the security check: ownership, DACL, size, and reparse
        // status are all validated on this handle.
        let handle = unsafe {
            CreateFileW(
                wide.as_ptr(),
                FILE_GENERIC_READ,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                std::ptr::null_mut(),
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL | FILE_FLAG_OPEN_REPARSE_POINT,
                0,
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            return Err(io_err(
                "opening owner-only file",
                std::io::Error::last_os_error(),
            ));
        }
        let file = unsafe { std::fs::File::from_raw_handle(handle as _) };
        let meta = file
            .metadata()
            .map_err(|e| io_err("inspecting owner-only file", e))?;
        if meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err(FsError::Unsupported {
                capability: "owner-only file read",
                message: format!("{} is a reparse point", path.display()),
            });
        }
        if !meta.is_file() {
            return Err(FsError::Unsupported {
                capability: "owner-only file read",
                message: format!("{} is not a regular file", path.display()),
            });
        }
        if meta.len() > max_bytes {
            return Err(FsError::Unsupported {
                capability: "owner-only file read",
                message: format!(
                    "{} is {} bytes, over the {max_bytes} byte cap",
                    path.display(),
                    meta.len()
                ),
            });
        }
        validate_owner_private_file_security(file.as_raw_handle() as HANDLE, path)?;
        let modified_ms = meta.modified().ok().and_then(system_time_to_ms);
        Ok((
            file,
            OwnerOnlyFileMeta {
                len: meta.len(),
                modified_ms,
            },
        ))
    }

    pub(super) fn read_owner_only_file_with_meta(
        path: &Path,
        max_bytes: u64,
    ) -> Result<(Vec<u8>, OwnerOnlyFileMeta)> {
        let (file, meta) = open_owner_only_file(path, max_bytes)?;
        let mut buf = Vec::with_capacity(meta.len as usize);
        file.take(max_bytes + 1)
            .read_to_end(&mut buf)
            .map_err(|e| io_err("reading owner-only file", e))?;
        if buf.len() as u64 > max_bytes {
            return Err(FsError::Unsupported {
                capability: "owner-only file read",
                message: format!("{} grew past the {max_bytes} byte cap", path.display()),
            });
        }
        Ok((buf, meta))
    }

    /// Per-file Windows owner/DACL validator, modelled on `validate_owner_private_dir_security`.
    ///
    /// Unlike the directory validator this does **not** require a protected (non-inheritable) DACL:
    /// a credential file written by an unrelated same-user producer legitimately carries the
    /// process token's normal current-user/SYSTEM/Administrators inherited DACL. What it does
    /// require is that every ACE names a principal in that allowlist, so `Everyone`,
    /// `Authenticated Users`, `Users`, or any foreign SID makes the read fail closed.
    pub(super) fn validate_owner_private_file_security(handle: HANDLE, path: &Path) -> Result<()> {
        let mut sd: *mut c_void = std::ptr::null_mut();
        let mut owner: PSID = std::ptr::null_mut();
        let mut dacl: *mut ACL = std::ptr::null_mut();
        let rc = unsafe {
            GetSecurityInfo(
                handle,
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
            return Err(FsError::Unsupported {
                capability: "owner-only file read",
                message: format!(
                    "cannot read security descriptor for {}: {}",
                    path.display(),
                    std::io::Error::from_raw_os_error(rc as i32)
                ),
            });
        }
        let _sd_guard = LocalAllocGuard(sd);

        let self_sids = self_owner_sids()?;
        let system_sid = sid_from_string("S-1-5-18")?;
        let admins_sid = sid_from_string("S-1-5-32-544")?;

        if !owner_is_self(owner, self_sids) {
            return Err(FsError::Unsupported {
                capability: "owner-only file read",
                message: foreign_owner_message(path, owner),
            });
        }

        let mut control = 0u16;
        let mut revision = 0u32;
        let ok = unsafe { GetSecurityDescriptorControl(sd, &mut control, &mut revision) };
        if ok == 0 || control & SE_DACL_PRESENT == 0 || dacl.is_null() {
            return Err(FsError::Unsupported {
                capability: "owner-only file read",
                message: format!("{} has no DACL", path.display()),
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
            return Err(FsError::Unsupported {
                capability: "owner-only file read",
                message: format!("{} has an empty or unreadable DACL", path.display()),
            });
        }

        let mut grants_self = false;
        for idx in 0..info.AceCount {
            let mut ace_ptr: *mut c_void = std::ptr::null_mut();
            let ok = unsafe { GetAce(dacl, idx, &mut ace_ptr) };
            if ok == 0 || ace_ptr.is_null() {
                return Err(io_err(
                    "reading owner-only file ACE",
                    std::io::Error::last_os_error(),
                ));
            }
            let header = unsafe { &*(ace_ptr as *const windows_sys::Win32::Security::ACE_HEADER) };
            match header.AceType {
                ACCESS_ALLOWED_ACE_TYPE => {
                    let ace = unsafe { &*(ace_ptr as *const ACCESS_ALLOWED_ACE) };
                    let sid = (&ace.SidStart as *const u32).cast::<c_void>() as PSID;
                    let (allowed, is_self) =
                        ace_trustee_is_allowlisted(sid, self_sids, system_sid.0, admins_sid.0);
                    if !allowed {
                        return Err(FsError::Unsupported {
                            capability: "owner-only file read",
                            message: format!(
                                "{} grants access to a principal outside the owner allowlist",
                                path.display()
                            ),
                        });
                    }
                    grants_self |= is_self;
                }
                ACCESS_DENIED_ACE_TYPE => {
                    return Err(FsError::Unsupported {
                        capability: "owner-only file read",
                        message: format!("{} contains a deny ACE", path.display()),
                    });
                }
                other => {
                    return Err(FsError::Unsupported {
                        capability: "owner-only file read",
                        message: format!(
                            "{} contains unsupported ACE type {other}",
                            path.display()
                        ),
                    });
                }
            }
        }
        if !grants_self {
            return Err(FsError::Unsupported {
                capability: "owner-only file read",
                message: format!(
                    "{} grants no access to any SID this process owns objects as",
                    path.display()
                ),
            });
        }
        Ok(())
    }

    fn create_owner_only_dir(path: &Path) -> Result<()> {
        let sa = owner_only_security_attributes()?;
        let wide = wide_null(path.as_os_str());
        let ok = unsafe { CreateDirectoryW(wide.as_ptr(), &sa.attrs) };
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
            return Err(FsError::Unsupported {
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
        if path.components().any(|component| {
            matches!(
                component,
                Component::Prefix(prefix)
                    if matches!(prefix.kind(), Prefix::UNC(_, _) | Prefix::VerbatimUNC(_, _))
            )
        }) {
            return Err(FsError::Unsupported {
                capability: "owner-private daemon directory",
                message: format!("{} is not a local path", path.display()),
            });
        }
        let meta = std::fs::symlink_metadata(path)
            .map_err(|e| io_err("checking owner-private daemon directory", e))?;
        if !meta.is_dir() {
            return Err(FsError::Unsupported {
                capability: "owner-private daemon directory",
                message: format!("{} is not a directory", path.display()),
            });
        }
        if meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err(FsError::Unsupported {
                capability: "owner-private daemon directory",
                message: format!("{} is a reparse point", path.display()),
            });
        }
        Ok(())
    }

    /// Validate a directory's owner and DACL.
    ///
    /// `require_protected` distinguishes the two callers: a directory telex owns outright must
    /// carry an explicit, non-inheriting (`SE_DACL_PROTECTED`) owner-only DACL, while a producer
    /// root telex merely shares only has to be *safe* — owner is the current user and every
    /// allowed ACE names the current user, `SYSTEM`, or local `Administrators`. Both reject
    /// `Everyone`, `Authenticated Users`, `Users`, any foreign SID, and any deny ACE.
    fn validate_owner_private_dir_security(path: &Path, require_protected: bool) -> Result<()> {
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
            return Err(FsError::Unsupported {
                capability: "owner-private daemon directory",
                message: format!(
                    "cannot read security descriptor for {}: {}",
                    path.display(),
                    std::io::Error::from_raw_os_error(rc as i32)
                ),
            });
        }
        let _sd_guard = LocalAllocGuard(sd);

        let self_sids = self_owner_sids()?;
        let system_sid = sid_from_string("S-1-5-18")?;
        let admins_sid = sid_from_string("S-1-5-32-544")?;

        if !owner_is_self(owner, self_sids) {
            return Err(FsError::Unsupported {
                capability: "owner-private daemon directory",
                message: foreign_owner_message(path, owner),
            });
        }

        let mut control = 0u16;
        let mut revision = 0u32;
        let ok = unsafe { GetSecurityDescriptorControl(sd, &mut control, &mut revision) };
        if ok == 0
            || control & SE_DACL_PRESENT == 0
            || (require_protected && control & windows_sys::Win32::Security::SE_DACL_PROTECTED == 0)
        {
            return Err(FsError::Unsupported {
                capability: "owner-private daemon directory",
                message: format!("{} does not have a protected explicit DACL", path.display()),
            });
        }
        if dacl.is_null() {
            return Err(FsError::Unsupported {
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
                    let (allowed, _) =
                        ace_trustee_is_allowlisted(sid, self_sids, system_sid.0, admins_sid.0);
                    if !allowed {
                        return Err(FsError::Unsupported {
                            capability: "owner-private daemon directory",
                            message: format!("{} grants access to a non-owner SID", path.display()),
                        });
                    }
                }
                ACCESS_DENIED_ACE_TYPE => {
                    return Err(FsError::Unsupported {
                        capability: "owner-private daemon directory",
                        message: format!("{} contains a deny ACE", path.display()),
                    });
                }
                other => {
                    return Err(FsError::Unsupported {
                        capability: "owner-private daemon directory",
                        message: format!(
                            "{} contains unsupported ACE type {other}",
                            path.display()
                        ),
                    });
                }
            }
        }

        Ok(())
    }

    pub(super) fn process_exe_path(pid: u32) -> Result<PathBuf> {
        let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
        if process == 0 {
            return Err(FsError::Unsupported {
                capability: "process executable resolution",
                message: format!(
                    "cannot open process {pid}: {}",
                    std::io::Error::last_os_error()
                ),
            });
        }
        let process = Handle(process);
        let mut buf = vec![0u16; 32768];
        let mut len = buf.len() as u32;
        let ok = unsafe { QueryFullProcessImageNameW(process.0, 0, buf.as_mut_ptr(), &mut len) };
        if ok == 0 {
            return Err(FsError::Unsupported {
                capability: "process executable resolution",
                message: format!(
                    "cannot resolve executable for pid {pid}: {}",
                    std::io::Error::last_os_error()
                ),
            });
        }
        let raw = PathBuf::from(OsString::from_wide(&buf[..len as usize]));
        std::fs::canonicalize(&raw).map_err(|e| FsError::Unsupported {
            capability: "process executable resolution",
            message: format!("cannot canonicalize {} for pid {pid}: {e}", raw.display()),
        })
    }

    pub(super) fn raw_host_id() -> Result<String> {
        let subkey = wide_null(OsStr::new(r"SOFTWARE\Microsoft\Cryptography"));
        let value = wide_null(OsStr::new("MachineGuid"));
        let mut buf = vec![0u16; 128];
        let mut size = (buf.len() * 2) as u32;
        let rc = unsafe {
            RegGetValueW(
                HKEY_LOCAL_MACHINE,
                subkey.as_ptr(),
                value.as_ptr(),
                RRF_RT_REG_SZ,
                std::ptr::null_mut(),
                buf.as_mut_ptr() as *mut c_void,
                &mut size,
            )
        };
        if rc != 0 {
            return Err(FsError::Unsupported {
                capability: "stable host identity",
                message: format!(
                    "cannot read HKLM\\SOFTWARE\\Microsoft\\Cryptography\\MachineGuid: {}",
                    std::io::Error::from_raw_os_error(rc as i32)
                ),
            });
        }
        let chars = (size as usize / 2).min(buf.len());
        let mut text = String::from_utf16_lossy(&buf[..chars]);
        text.retain(|c| c != '\0');
        let text = text.trim().to_string();
        if text.is_empty() {
            return Err(FsError::Unsupported {
                capability: "stable host identity",
                message: "MachineGuid is empty".into(),
            });
        }
        Ok(text)
    }

    pub(super) fn raw_boot_id(scope: Option<super::TestBootIdScope<'_>>) -> Result<String> {
        // Windows has no kernel-provided boot identifier, and the obvious derivation
        // (`SystemTime::now() - GetTickCount64()`) is **not stable within one boot**:
        // `GetTickCount64` advances in ~15.6 ms steps while the wall clock does not, so the
        // derived instant jitters across a second boundary a few percent of the time, and any
        // wall-clock step (NTP resync, VM resume, manual change) shifts it outright. Two processes
        // computing it independently — the attaching CLI and the daemon — then disagree, every
        // intent terminates as `foreign_host_or_boot`, and the anti-downgrade guard turns that
        // into a hard refusal of an unrelated `telex attach`.
        //
        // So the identifier is *minted once per boot and persisted*, and the derived instant is
        // used only to decide whether the persisted record still belongs to this boot. Two
        // independent checks have to agree for that:
        //
        // * monotonic uptime must not have gone backwards (a reboot resets it to ~0), and
        // * the derived boot instant must still match within `BOOT_INSTANT_TOLERANCE_MS`, which
        //   absorbs both the tick granularity and an ordinary NTP correction.
        //
        // Persisting is only half the answer, though: the **first** mint of a boot is a
        // read-modify-write across processes, and an unserialized one loses. Cold start is exactly
        // when several telex processes appear at once — a `telex copilot attach` that spawns the
        // daemon, a watcher, a console — and with a plain read/mint/overwrite each of them sees an
        // absent record, mints its own, and overwrites whatever the others wrote. The read-back
        // does not save it: writer A can write and read back its own value *before* writer B
        // overwrites the key, so A returns `A`, B returns `B`, and every process started after
        // them returns `B`. A's station intent is then permanently `foreign_host_or_boot` — the
        // precise failure the persisted record exists to prevent, reintroduced by the race.
        //
        // So the mint is serialized on a user-scoped named mutex and re-checked under it: the
        // first writer of a boot wins, and everyone else adopts the record it left rather than
        // minting a competing one. The uncontended path (a record already exists) never touches
        // the lock. See `BootIdMintLock`.
        const BOOT_INSTANT_TOLERANCE_MS: i64 = 60_000;

        let namespace = scope.as_ref().map(|scope| scope.namespace);
        let key = boot_id_key(namespace);

        let sample_clock = || -> Result<(u64, i64)> {
            let uptime_ms = unsafe { GetTickCount64() };
            let now_ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .map_err(|_| FsError::Unsupported {
                    capability: "boot session identity",
                    message: "system clock is before the unix epoch".into(),
                })?;
            if uptime_ms == 0 || now_ms < uptime_ms {
                return Err(FsError::Unsupported {
                    capability: "boot session identity",
                    message: "system uptime is unavailable".into(),
                });
            }
            Ok((uptime_ms, (now_ms - uptime_ms) as i64))
        };

        let read_valid = |uptime_ms: u64, boot_instant_ms: i64| -> Option<String> {
            let record: BootIdRecord = serde_json::from_str(&read_boot_id_record(&key)?).ok()?;
            // Monotonic uptime, with the same tolerance as the instant check: a reboot resets
            // uptime to ~0 against a stored value of hours or days, so the slack costs nothing —
            // and without it two processes sampling `GetTickCount64` milliseconds apart disagree
            // about whose record is newer and each mint their own id, which is the disagreement
            // this whole mechanism exists to remove.
            let uptime_monotonic =
                uptime_ms.saturating_add(BOOT_INSTANT_TOLERANCE_MS as u64) >= record.uptime_ms;
            let same_instant =
                (boot_instant_ms - record.boot_instant_ms).abs() <= BOOT_INSTANT_TOLERANCE_MS;
            (uptime_monotonic && same_instant && !record.id.is_empty()).then_some(record.id)
        };

        let (uptime_ms, boot_instant_ms) = sample_clock()?;
        if let Some(id) = read_valid(uptime_ms, boot_instant_ms) {
            return Ok(id);
        }

        // No usable record: this process is a candidate first writer. Everything from here — the
        // re-check, the mint, the write, the read-back — happens under the lock, so exactly one
        // process per boot can reach the mint with the key still empty. A lock that cannot be
        // taken is a hard failure, not a licence to race: see `BootIdMintLock`.
        //
        // The observation above is the instant a lost-update race is decided, so it is also where
        // the cross-process regression test parks every participant before releasing them into the
        // lock together. `at_cold_start` is `None` on every production path — it can only travel
        // inside a `TestBootIdScope`, which `boot_id`/`boot_id_uncached` never construct.
        if let Some(at_cold_start) = scope.as_ref().and_then(|scope| scope.at_cold_start) {
            at_cold_start();
        }

        let _mint = BootIdMintLock::acquire(namespace)?;

        // Re-sample rather than reuse the pre-lock reading: the wait is bounded but not
        // instantaneous, and the record we are about to judge may have been written during it.
        let (uptime_ms, boot_instant_ms) = sample_clock()?;
        if let Some(id) = read_valid(uptime_ms, boot_instant_ms) {
            return Ok(id);
        }

        let mut bytes = [0u8; 16];
        getrandom::getrandom(&mut bytes).map_err(|e| FsError::Unsupported {
            capability: "boot session identity",
            message: format!("generating a boot session id: {e}"),
        })?;
        let id: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
        let record = BootIdRecord {
            id: id.clone(),
            boot_instant_ms,
            uptime_ms,
        };
        let encoded = serde_json::to_string(&record).map_err(|e| FsError::Unsupported {
            capability: "boot session identity",
            message: format!("serializing the boot session id: {e}"),
        })?;
        // Fail **explicitly** when the record cannot be persisted or read back, rather than
        // returning a per-process value. See `resolve_minted_boot_id`. The read-back still matters
        // under the lock: it is what proves the write actually landed in the key the next process
        // will read.
        resolve_minted_boot_id(
            write_boot_id_record(&key, &encoded),
            read_valid(uptime_ms, boot_instant_ms),
        )
    }

    /// Decide the outcome of a mint: persisted and read back, or an explicit failure.
    ///
    /// Extracted so the failure branch is reachable from a test on any host — denying a write to
    /// `HKCU\Software\telex` for real would mean changing the machine the suite runs on, so the
    /// decision is tested rather than the ACL.
    ///
    /// The tempting degradation this replaces — "keep a usable id for *this* process" — is not a
    /// degradation at all. The value is compared for **exact equality** across processes: the
    /// attaching CLI writes it into a station intent, the daemon recomputes it. A per-process id
    /// therefore makes every intent `foreign_host_or_boot` the instant anyone else reads it, which
    /// is terminal; GC then removes the record as a foreign identity with a dead producer; and the
    /// anti-downgrade guard turns the same condition into a hard refusal of an unrelated
    /// `telex attach`. Silently minting an identity that is *guaranteed* to disagree is strictly
    /// worse than saying so — an explicit error surfaces at `copilot attach`, naming the cause,
    /// instead of as an unexplained recovery failure hours later. `boot_id()` memoizes the error
    /// too, so the answer is at least consistent for the life of the process.
    fn resolve_minted_boot_id(written: Result<()>, read_back: Option<String>) -> Result<String> {
        written?;
        // Re-read rather than trusting our own write: two processes minting at the same instant
        // both write, and the read-back is what makes them converge on one value instead of each
        // keeping its own — which is precisely the cross-process disagreement being removed. A
        // read-back that does not come back is the same failure as a write that did not land.
        read_back.ok_or(FsError::Unsupported {
            capability: "boot session identity",
            message: "the persisted boot session id could not be read back".into(),
        })
    }

    #[derive(serde::Serialize, serde::Deserialize)]
    struct BootIdRecord {
        id: String,
        boot_instant_ms: i64,
        uptime_ms: u64,
    }

    /// Where the per-boot identifier is persisted: `HKCU\Software\telex`.
    ///
    /// The per-user registry rather than a file under `%LOCALAPPDATA%` **because it is
    /// environment-independent**. Two processes must agree on this value or every station intent
    /// fails closed as `foreign_host_or_boot`, and any file location is reachable only through an
    /// environment variable that a parent process (or telex's own test harness) can repoint,
    /// which would reintroduce the disagreement by a different route.
    const BOOT_ID_KEY: &str = "Software\\telex";
    const BOOT_ID_VALUE: &str = "BootSessionId";

    /// Container for the isolated namespaces the concurrency test resolves in. Production never
    /// writes here, and a test namespace can never address `BOOT_ID_KEY` itself: the namespace is
    /// validated to ASCII alphanumerics, `-`, and `_` before it reaches this function, so it can
    /// contribute neither a `\` nor a `..` to the path.
    const BOOT_ID_TEST_KEY: &str = "Software\\telex\\TestBootSessions";

    fn boot_id_key(namespace: Option<&str>) -> String {
        match namespace {
            None => BOOT_ID_KEY.to_string(),
            Some(namespace) => format!("{BOOT_ID_TEST_KEY}\\{namespace}"),
        }
    }

    /// Serializes the **first** mint of a boot across processes of one user.
    ///
    /// A named mutex, not a registry convention, because the registry has no atomic
    /// compare-and-set on a value: `RegSetKeyValueW` is an unconditional overwrite, so
    /// read/mint/write is a lost-update race no ordering of those three calls can close.
    ///
    /// The lock has to cover exactly the set of processes that can write the record, and that set
    /// is defined by `HKCU`: one per *user*, shared by every Terminal Services session that user is
    /// logged into. So the name is `Global\` — the machine-wide object namespace — with the token
    /// user's SID appended. Two users never contend (different SIDs, different `HKCU` hives, so
    /// blocking each other would be pure interference), while the console session and a concurrent
    /// RDP session of the *same* user do contend, which is correct: they share one `HKCU`, so
    /// cold-starting together is exactly the lost update this exists to prevent. Elevated and
    /// non-elevated processes of one user share the SID too, so a UAC split token also serializes.
    ///
    /// `Local\` — the per-session namespace — was the earlier scope and is wrong for this reason:
    /// it is narrower than the resource it guards. Two logon sessions of one user would serialize
    /// on two different objects while writing the same key.
    ///
    /// `Global\` costs nothing here. `SeCreateGlobalPrivilege` — which an ordinary interactive user
    /// does not hold — gates only *section* (file-mapping) and *symbolic link* objects in that
    /// namespace; events, semaphores and mutexes are exempt, so a standard non-elevated user in an
    /// RDP session creates this mutex successfully. Should some hardened configuration refuse it
    /// anyway, `CreateMutexW` fails and the mint fails with it — closed, not unserialized.
    ///
    /// The mutex is created with the same owner-only descriptor as every other object telex
    /// creates, so it cannot be squatted: another user's process that guesses the name gets
    /// `ERROR_ACCESS_DENIED` from `CreateMutexW`, and telex's own mint fails closed rather than
    /// proceeding unserialized if someone squats the name first.
    ///
    /// Every failure here is fatal to the mint. The alternative — "the lock did not work, mint
    /// anyway" — is precisely the unserialized write this exists to remove, and it produces an
    /// identity guaranteed to disagree with the one another process persisted, which is terminal
    /// for every station intent that carries it. An error naming the lock is strictly better.
    ///
    /// `WAIT_ABANDONED` counts as acquired: it means a previous holder died mid-mint, and the
    /// caller re-reads and re-validates the record under the lock before doing anything with it,
    /// so a partially finished mint is judged on its merits like any other stored record.
    struct BootIdMintLock(Handle);

    /// The mint lock's object name: machine-wide namespace, user-scoped identity, test namespace
    /// suffix when there is one.
    ///
    /// Extracted so the scope is assertable — `the_mint_lock_is_global_and_user_scoped` pins both
    /// halves, because either one silently narrowing (back to `Local\`, or dropping the SID)
    /// reintroduces a lost-update window that no in-process test can observe.
    fn mint_lock_name(namespace: Option<&str>) -> Result<String> {
        let sid = current_user_sid()?;
        Ok(match namespace {
            None => format!("Global\\telex-boot-session-mint-{sid}"),
            Some(namespace) => format!("Global\\telex-boot-session-mint-{sid}-{namespace}"),
        })
    }

    impl BootIdMintLock {
        /// How long to wait for another process's mint. A mint is a few registry calls, so this is
        /// orders of magnitude more than it needs; it exists so a wedged holder surfaces as a named
        /// error instead of hanging `telex copilot attach` forever.
        const TIMEOUT_MS: u32 = 30_000;

        fn acquire(namespace: Option<&str>) -> Result<Self> {
            let name = mint_lock_name(namespace)?;
            let sa = owner_only_security_attributes()?;
            let wide = wide_null(OsStr::new(&name));
            // `bInitialOwner = FALSE`: take ownership only through the wait below, so the
            // created-vs-opened distinction never has to be interpreted.
            let raw = unsafe { CreateMutexW(&sa.attrs, 0, wide.as_ptr()) };
            if raw == 0 {
                return Err(FsError::Unsupported {
                    capability: "boot session identity",
                    message: format!(
                        "cannot open the boot session mint lock: {}",
                        std::io::Error::last_os_error()
                    ),
                });
            }
            let handle = Handle(raw);
            match unsafe { WaitForSingleObject(handle.0, Self::TIMEOUT_MS) } {
                WAIT_OBJECT_0 | WAIT_ABANDONED => Ok(Self(handle)),
                WAIT_TIMEOUT => Err(FsError::Unsupported {
                    capability: "boot session identity",
                    message: format!(
                        "another process held the boot session mint lock for more than {} ms",
                        Self::TIMEOUT_MS
                    ),
                }),
                _ => Err(FsError::Unsupported {
                    capability: "boot session identity",
                    message: format!(
                        "waiting for the boot session mint lock: {}",
                        std::io::Error::last_os_error()
                    ),
                }),
            }
        }
    }

    impl Drop for BootIdMintLock {
        fn drop(&mut self) {
            unsafe {
                ReleaseMutex(self.0 .0);
            }
        }
    }

    /// Remove a test namespace's persisted record so the next resolution in it is a cold start.
    ///
    /// Only reachable through `platform_fs::clear_test_boot_id_namespace`, which validates the
    /// namespace, so this addresses a subkey of `BOOT_ID_TEST_KEY` and never the production record.
    ///
    /// Scoped to the one namespace, never the whole container: `cargo test` runs test binaries
    /// concurrently and developers run several checkouts at once, so a container-wide sweep would
    /// delete a namespace another run was mid-race in. The container key itself is removed
    /// best-effort afterwards, which is safe precisely because `RegDeleteKeyExW` refuses a key that
    /// still has subkeys — a concurrent run's namespace keeps it alive.
    pub(super) fn clear_boot_id_namespace(namespace: &str) -> Result<()> {
        let key = boot_id_key(Some(namespace));
        debug_assert_ne!(key, BOOT_ID_KEY);
        let wide = wide_null(OsStr::new(&key));
        // `RegDeleteTreeW` clears the values and subkeys; the now-empty key itself is removed
        // best-effort so a long-lived profile does not accumulate one per test run.
        let rc = unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, wide.as_ptr()) };
        if rc != 0 && rc != ERROR_FILE_NOT_FOUND {
            return Err(FsError::Unsupported {
                capability: "boot session identity",
                message: format!(
                    "clearing the boot session test namespace failed with status {rc}"
                ),
            });
        }
        let container = wide_null(OsStr::new(BOOT_ID_TEST_KEY));
        unsafe {
            RegDeleteKeyExW(HKEY_CURRENT_USER, wide.as_ptr(), 0, 0);
            RegDeleteKeyExW(HKEY_CURRENT_USER, container.as_ptr(), 0, 0);
        }
        Ok(())
    }

    fn read_boot_id_record(key: &str) -> Option<String> {
        let key = wide_null(std::ffi::OsStr::new(key));
        let value = wide_null(std::ffi::OsStr::new(BOOT_ID_VALUE));
        let mut size: u32 = 0;
        let rc = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                key.as_ptr(),
                value.as_ptr(),
                RRF_RT_REG_SZ,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &mut size,
            )
        };
        if rc != 0 || size == 0 {
            return None;
        }
        let mut buf = vec![0u16; (size as usize).div_ceil(2)];
        let mut size_out = size;
        let rc = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                key.as_ptr(),
                value.as_ptr(),
                RRF_RT_REG_SZ,
                std::ptr::null_mut(),
                buf.as_mut_ptr() as *mut c_void,
                &mut size_out,
            )
        };
        if rc != 0 {
            return None;
        }
        let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        Some(String::from_utf16_lossy(&buf[..len]))
    }

    fn write_boot_id_record(key: &str, json: &str) -> Result<()> {
        let key = wide_null(std::ffi::OsStr::new(key));
        let value = wide_null(std::ffi::OsStr::new(BOOT_ID_VALUE));
        let data = wide_null(std::ffi::OsStr::new(json));
        let bytes = (data.len() * 2) as u32;
        let rc = unsafe {
            RegSetKeyValueW(
                HKEY_CURRENT_USER,
                key.as_ptr(),
                value.as_ptr(),
                REG_SZ,
                data.as_ptr() as *const c_void,
                bytes,
            )
        };
        if rc != 0 {
            return Err(FsError::Unsupported {
                capability: "boot session identity",
                message: format!("persisting the boot session id failed with status {rc}"),
            });
        }
        Ok(())
    }

    fn current_user_sid() -> Result<String> {
        let token = current_process_token()?;
        sid_string_from_token(token.0)
    }

    fn current_process_token() -> Result<Handle> {
        let mut token = 0isize;
        let ok = unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) };
        if ok == 0 {
            return Err(io_err(
                "opening process token",
                std::io::Error::last_os_error(),
            ));
        }
        Ok(Handle(token))
    }

    fn sid_string_from_token(token: HANDLE) -> Result<String> {
        let buf = token_information(token, TokenUser, "reading token user information")?;
        let token_user = unsafe { &*(buf.as_ptr() as *const TOKEN_USER) };
        sid_to_string(token_user.User.Sid)
            .ok_or_else(|| io_err("converting SID to string", std::io::Error::last_os_error()))
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
        let sid = current_user_sid()?;
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
            return Err(FsError::Unsupported {
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

    #[cfg(test)]
    mod boot_id_tests {
        use super::*;

        /// The owner rule admits exactly what this process's token says it may own — and nothing
        /// else.
        ///
        /// The regression: the rule compared an object's owner against `TokenUser` alone. Windows
        /// does not stamp the user on objects a process creates; it stamps the token's *default
        /// owner*, which on an elevated administrator token (every GitHub Actions Windows runner,
        /// and any developer running from an elevated shell) is `BUILTIN\Administrators`. So a
        /// directory the process had just created and a credential file it had just written were
        /// both refused as foreign-owned, and `telex copilot attach` could not secure its own
        /// bridge producer root.
        ///
        /// Both halves are asserted together on purpose. The first alone would be satisfied by
        /// deleting the owner check; the second alone would be satisfied by keeping the broken one.
        #[test]
        fn the_owner_rule_admits_this_processs_own_objects_and_no_foreign_sid() {
            let dir = std::env::temp_dir().join(format!(
                "telex-owner-rule-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_nanos())
                    .unwrap_or_default()
            ));
            std::fs::create_dir_all(&dir).expect("temp dir");
            let file = dir.join("registry.json");
            std::fs::write(&file, b"{}").expect("write");

            let selves = self_owner_sids().expect("token owner SIDs");
            assert!(
                selves.contains(&current_user_sid().expect("token user SID")),
                "the token user is always a SID this process may own objects as"
            );

            // Whatever SID Windows stamped on the two objects this process just created has to be
            // admissible, or the rule rejects its own artifacts.
            for made_here in [dir.as_path(), file.as_path()] {
                let owner = owner_sid_of_path(made_here).expect("owner SID");
                assert!(
                    selves.contains(&owner),
                    "{} was created by this process and is owned by {owner}, which the owner rule \
                     refused; on an elevated token that SID is BUILTIN\\Administrators",
                    made_here.display()
                );
            }

            // Not a blanket accept. None of these can ever carry `SE_GROUP_OWNER`, so they must
            // stay outside the set on every host, elevated or not.
            for foreign in [
                "S-1-1-0",                                        // Everyone
                "S-1-5-11",                                       // Authenticated Users
                "S-1-5-21-1111111111-2222222222-3333333333-1001", // a foreign user
            ] {
                assert!(
                    !selves.iter().any(|known| known == foreign),
                    "{foreign} must never be treated as a SID this process owns objects as"
                );
                let sid = sid_from_string(foreign).expect("parse SID");
                assert!(
                    !owner_is_self(sid.0, selves),
                    "an object owned by {foreign} must still fail closed"
                );
            }

            let _ = std::fs::remove_dir_all(&dir);
        }

        /// A filtered (non-elevated) token carries `BUILTIN\Administrators` as
        /// `SE_GROUP_USE_FOR_DENY_ONLY`, and such a token may not own objects as Administrators.
        /// Admitting a deny-only group would be a real widening, so the deny-only filter is pinned
        /// against the token this test actually runs under.
        #[test]
        fn a_deny_only_group_is_never_a_self_owner() {
            let token = current_process_token().expect("process token");
            let groups =
                token_information(token.0, TokenGroups, "reading token groups").expect("groups");
            let token_groups = unsafe { &*(groups.as_ptr() as *const TOKEN_GROUPS) };
            let entries = unsafe {
                std::slice::from_raw_parts(
                    token_groups.Groups.as_ptr(),
                    token_groups.GroupCount as usize,
                )
            };
            let selves = self_owner_sids().expect("token owner SIDs");
            for entry in entries {
                if entry.Attributes & SE_GROUP_USE_FOR_DENY_ONLY == 0 {
                    continue;
                }
                let Some(text) = sid_to_string(entry.Sid) else {
                    continue;
                };
                assert!(
                    !selves.contains(&text),
                    "{text} is deny-only on this token and must not be an admissible owner"
                );
            }
        }

        /// The test container is a container: it can never resolve to the production record, and the
        /// production record never gains a namespace suffix.
        #[test]
        fn a_test_namespace_is_contained_and_distinct_from_the_production_record() {
            assert_eq!(boot_id_key(None), BOOT_ID_KEY);
            let scoped = boot_id_key(Some("ns"));
            assert_eq!(scoped, format!("{BOOT_ID_TEST_KEY}\\ns"));
            assert_ne!(scoped, BOOT_ID_KEY);
            assert!(
                scoped.starts_with(&format!("{BOOT_ID_KEY}\\")),
                "the container stays under the product's own key, got {scoped}"
            );
        }

        /// The mint lock's scope: machine-wide namespace, per-user identity.
        ///
        /// Both halves matter and neither is observable from a passing functional test, because a
        /// too-narrow lock only loses when two logon sessions of one user cold-start together.
        /// `Global\` because the record it guards is `HKCU` — one hive per *user*, shared across
        /// every Terminal Services session that user is signed into, so a `Local\` (per-session)
        /// object is narrower than the resource. The SID because two users must **not** contend:
        /// they have different hives, and blocking each other would be interference, not safety.
        #[test]
        fn the_mint_lock_is_global_and_user_scoped() {
            let sid = current_user_sid().expect("the token user SID");
            let production = mint_lock_name(None).expect("the production mint lock name");
            assert!(
                production.starts_with("Global\\"),
                "the mint lock must live in the machine-wide namespace so two logon sessions of \
                 one user serialize on the same object, got {production}"
            );
            assert!(
                production.contains(&sid),
                "the mint lock must be scoped to the token user, so users with different HKCU \
                 hives never block each other, got {production}"
            );

            let scoped = mint_lock_name(Some("ns")).expect("a namespaced mint lock name");
            assert!(scoped.starts_with("Global\\"), "got {scoped}");
            assert!(scoped.contains(&sid), "got {scoped}");
            assert_ne!(
                scoped, production,
                "a test namespace must never contend with the production mint"
            );
            assert!(scoped.ends_with("-ns"), "got {scoped}");
        }

        /// `Global\` is reachable without `SeCreateGlobalPrivilege`.
        ///
        /// The privilege — which an ordinary interactive user does not hold — gates *section* and
        /// *symbolic link* objects in the global namespace, not mutexes. This asserts that on the
        /// host actually running the suite, so the scope widening above cannot turn the mint into a
        /// hard failure for exactly the non-elevated, RDP-session users it is meant to protect. A
        /// failure here is the fail-closed path working as designed and reported as such.
        #[test]
        fn the_global_mint_lock_is_creatable_without_extra_privilege() {
            let namespace = format!("unit-global-{}", std::process::id());
            let held = BootIdMintLock::acquire(Some(&namespace)).unwrap_or_else(|e| {
                panic!(
                    "creating the Global\\ mint lock failed on this host ({e}); the boot identity \
                     mint fails closed rather than racing, but every cold start on this host now \
                     errors"
                )
            });
            drop(held);
        }

        /// The mint lock actually excludes a second holder.
        ///
        /// Asserted across threads because a Windows mutex is owned by a *thread*: a second
        /// acquisition on the same thread would be granted by recursion and prove nothing. This is
        /// the in-process half of the guarantee; the cross-process half — several processes
        /// cold-starting at once and resolving one identity — is
        /// `tests/boot_identity.rs::a_concurrent_cold_start_resolves_exactly_one_boot_identity`.
        #[test]
        fn the_mint_lock_excludes_a_second_holder() {
            let namespace = format!("unit-lock-{}", std::process::id());
            let held = BootIdMintLock::acquire(Some(&namespace)).expect("first holder");
            let (tx, rx) = std::sync::mpsc::channel();
            let contender = std::thread::spawn(move || {
                let _second = BootIdMintLock::acquire(Some(&namespace)).expect("second holder");
                let _ = tx.send(());
            });
            assert!(
                rx.recv_timeout(std::time::Duration::from_millis(250))
                    .is_err(),
                "a second holder was admitted while the mint lock was held, so two processes can \
                 mint competing boot identities at once"
            );
            drop(held);
            rx.recv_timeout(std::time::Duration::from_secs(30))
                .expect("the contender must be admitted once the lock is released");
            contender.join().expect("contender thread");
        }

        /// A host where the per-boot record cannot be persisted, or cannot be read back, must
        /// produce an **explicit** failure rather than a per-process value.
        ///
        /// Denying a write to `HKCU\Software\telex` for real would mean mutating the machine the
        /// suite runs on, so the decision is driven directly. The regression this pins is the old
        /// `if write_boot_id_record(&encoded).is_err() { return Ok(id) }`: every process then
        /// minted its own identity, every station intent written by one and read by another
        /// terminated as `foreign_host_or_boot`, GC removed those records as foreign identities
        /// with dead producers, and the anti-downgrade guard turned the same condition into a hard
        /// refusal of unrelated attaches — all with no error anywhere naming the cause.
        #[test]
        fn a_boot_id_that_cannot_be_persisted_fails_explicitly() {
            let denied = || {
                Err(FsError::Unsupported {
                    capability: "boot session identity",
                    message: "persisting the boot session id failed with status 5".into(),
                })
            };
            let err = resolve_minted_boot_id(denied(), Some("would-have-been-used".to_string()))
                .expect_err("a denied persist must not yield a usable per-process identity");
            let message = err.to_string();
            assert!(
                message.contains("persisting the boot session id"),
                "the failure must name its cause, got {message}"
            );

            let err = resolve_minted_boot_id(Ok(()), None)
                .expect_err("a write that cannot be read back is the same failure");
            assert!(
                err.to_string().contains("could not be read back"),
                "got {err}"
            );

            // The success path is unchanged: the value that comes back is the *persisted* one, not
            // the one this process happened to mint.
            assert_eq!(
                resolve_minted_boot_id(Ok(()), Some("persisted".to_string()))
                    .expect("a persisted, readable identity resolves"),
                "persisted"
            );
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Other platforms
// ---------------------------------------------------------------------------------------------

#[cfg(not(any(unix, windows)))]
mod imp {
    use super::{FsError, OwnerOnlyFileMeta, Result};
    use std::path::{Path, PathBuf};

    pub(super) fn ensure_owner_private_dir(_path: &Path) -> Result<PathBuf> {
        Err(FsError::Unsupported {
            capability: "owner-private daemon directory",
            message: "no owner-only permission implementation for this platform".into(),
        })
    }

    pub(super) fn ensure_owner_private_producer_root(_path: &Path) -> Result<PathBuf> {
        Err(FsError::Unsupported {
            capability: "owner-private producer root",
            message: "no owner-only permission implementation for this platform".into(),
        })
    }

    pub(super) fn write_owner_only_file(
        _path: &Path,
        _bytes: &[u8],
        _trailing_newline: bool,
    ) -> Result<()> {
        Err(FsError::Unsupported {
            capability: "owner-only daemon capability file",
            message: "no owner-only permission implementation for this platform".into(),
        })
    }

    pub(super) fn open_owner_only_file(
        _path: &Path,
        _max_bytes: u64,
    ) -> Result<(std::fs::File, OwnerOnlyFileMeta)> {
        Err(FsError::Unsupported {
            capability: "owner-only file read",
            message: "no owner-only permission implementation for this platform".into(),
        })
    }

    pub(super) fn read_owner_only_file_with_meta(
        _path: &Path,
        _max_bytes: u64,
    ) -> Result<(Vec<u8>, OwnerOnlyFileMeta)> {
        Err(FsError::Unsupported {
            capability: "owner-only file read",
            message: "no owner-only permission implementation for this platform".into(),
        })
    }

    pub(super) fn process_exe_path(_pid: u32) -> Result<PathBuf> {
        Err(FsError::Unsupported {
            capability: "process executable resolution",
            message: "no process identity implementation for this platform".into(),
        })
    }

    pub(super) fn raw_host_id() -> Result<String> {
        Err(FsError::Unsupported {
            capability: "stable host identity",
            message: "no host identity implementation for this platform".into(),
        })
    }

    pub(super) fn raw_boot_id(_scope: Option<super::TestBootIdScope<'_>>) -> Result<String> {
        Err(FsError::Unsupported {
            capability: "boot session identity",
            message: "no boot identity implementation for this platform".into(),
        })
    }

    pub(super) fn clear_boot_id_namespace(_namespace: &str) -> Result<()> {
        Err(FsError::Unsupported {
            capability: "boot session identity",
            message: "no boot identity implementation for this platform".into(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "telex-platform-fs-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or_default()
        ));
        std::fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    /// Write a file the way the real producer does on this platform.
    ///
    /// The bridge extension creates its registry file with mode `0600` (`extension.mjs` passes
    /// `{ mode: 0o600 }` and then `chmod`s it), because the Unix half of the owner-only read
    /// rejects any group or world bit. A bare `std::fs::write` here would produce `0644` under the
    /// default umask and model a producer telex does not have — the assertion would then be about
    /// the test's own sloppiness rather than about hardening. On Windows the process token's
    /// ordinary DACL is already the shape the read accepts, so there is nothing to adjust.
    fn write_producer_file(path: &Path, bytes: &[u8]) {
        std::fs::write(path, bytes).expect("write producer file");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
                .expect("owner-only producer file");
        }
    }

    /// The existence probe's whole contract: `Ok(false)` is a proof of absence and nothing else.
    ///
    /// `Path::exists()` collapses every failure into that same `false`, which is why every
    /// authority-bearing existence check in telex goes through `path_present` instead. The fault is
    /// injected because the real causes (a deny-ACE, an untraversable parent, a mount that went
    /// away) are each platform-specific and none of them is what is being tested — the contract is.
    #[test]
    fn path_present_reports_absence_only_when_it_can_prove_it() {
        let dir = temp_dir("path-present");
        let present = dir.join("here.json");
        std::fs::write(&present, b"{}").expect("write");
        let absent = dir.join("not-here.json");

        assert!(path_present(&present).expect("present"));
        assert!(!path_present(&absent).expect("absent"));

        // The condition `exists()` cannot report: the platform refused to answer.
        let fault = stat_faults::Unstatable::new(&present);
        assert!(
            path_present(&present).is_err(),
            "an undecidable answer is an error, never a confident 'no'"
        );
        drop(fault);
        assert!(
            path_present(&present).expect("restored"),
            "and the fault is scoped: dropping the guard restores the real filesystem"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The seam is shared process-wide state, so its scoping is a property the suite depends on:
    /// `cargo test` runs every one of these in one process with many in flight at once, and a fault
    /// that escaped its guard would surface as an unrelated test failing somewhere else entirely.
    ///
    /// Three escapes are ruled out here — a fault reaching a *different* path, a fault reaching a
    /// *different thread's* probe of a different path, and one guard's drop disarming another
    /// guard's fault on the same path.
    #[test]
    fn an_injected_stat_fault_reaches_nothing_but_its_own_path() {
        let dir = temp_dir("fault-isolation");
        let faulted = dir.join("faulted.json");
        let neighbour = dir.join("neighbour.json");
        std::fs::write(&faulted, b"{}").expect("write faulted");
        std::fs::write(&neighbour, b"{}").expect("write neighbour");

        let outer = stat_faults::Unstatable::new(&faulted);
        assert!(path_present(&faulted).is_err(), "the faulted path fails");
        assert!(
            path_present(&neighbour).expect("a sibling is answered by the real filesystem"),
            "a fault must not spread to another path in the same directory"
        );

        // Concurrent probes on other threads see the real filesystem for their own paths, and the
        // fault for the faulted one. This is the shape the daemon suite actually runs in.
        std::thread::scope(|scope| {
            for _ in 0..4 {
                scope.spawn(|| {
                    for _ in 0..50 {
                        assert!(
                            path_present(&neighbour).expect("neighbour stays statable"),
                            "a concurrent probe of an unfaulted path must not inherit the fault"
                        );
                        assert!(path_present(&faulted).is_err());
                    }
                });
            }
        });

        // Overlapping guards on the same path are independent: dropping the inner one must not
        // disarm the outer one's fault.
        {
            let _inner = stat_faults::Unstatable::new(&faulted);
            assert!(path_present(&faulted).is_err());
        }
        assert!(
            path_present(&faulted).is_err(),
            "an inner guard's drop must not disarm the guard that outlives it"
        );

        drop(outer);
        assert!(
            path_present(&faulted).expect("restored"),
            "the last guard's drop restores the real filesystem"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn sha256_matches_known_vectors() {
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            sha256_hex(b"The quick brown fox jumps over the lazy dog"),
            "d7a8fbb307d7809469ca9abcb0082e4f8d5651e46d3cdb762d02d0bf37c9e592"
        );
    }

    #[test]
    fn owner_only_round_trip_and_size_cap() {
        let dir = temp_dir("roundtrip");
        let scope = ensure_owner_private_dir(&dir).expect("scope");
        let path = scope.join("payload.json");
        write_owner_only_file_atomic(&path, b"{\"a\":1}").expect("write");
        let (bytes, meta) = read_owner_only_file_with_meta(&path, 4096).expect("read");
        assert_eq!(bytes, b"{\"a\":1}");
        assert_eq!(meta.len, 7);
        assert!(meta.modified_ms.is_some());

        // Over-cap reads fail closed rather than truncating.
        assert!(read_owner_only_file(&path, 3).is_err());

        // Atomic rewrite replaces in place and leaves no temp files behind.
        write_owner_only_file_atomic(&path, b"{\"a\":2}").expect("rewrite");
        assert_eq!(
            read_owner_only_file(&path, 4096).expect("reread"),
            b"{\"a\":2}"
        );
        let leftovers = std::fs::read_dir(&scope)
            .expect("scan")
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
            .count();
        assert_eq!(leftovers, 0, "atomic write left a temp file behind");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn concurrent_owner_private_directory_creation_is_idempotent() {
        let base = temp_dir("concurrent-create");
        let dir = base.join("scope");
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(8));
        let threads = (0..8)
            .map(|_| {
                let dir = dir.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    ensure_owner_private_dir(&dir)
                })
            })
            .collect::<Vec<_>>();

        for thread in threads {
            assert_eq!(thread.join().expect("creator thread").expect("scope"), dir);
        }
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn read_rejects_a_directory() {
        let dir = temp_dir("dir-reject");
        let scope = ensure_owner_private_dir(&dir).expect("scope");
        let nested = scope.join("nested");
        std::fs::create_dir(&nested).expect("nested");
        assert!(read_owner_only_file(&nested, 4096).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn containment_accepts_inside_and_rejects_escape() {
        let dir = temp_dir("containment");
        let root = ensure_owner_private_dir(&dir).expect("root");
        let inside = root.join("inside.json");
        write_owner_only_file_atomic(&inside, b"{}").expect("write inside");
        assert!(contained_under(&root, &inside).is_ok());

        let outside_dir = temp_dir("containment-outside");
        let outside = outside_dir.join("outside.json");
        std::fs::write(&outside, b"{}").expect("write outside");
        assert!(contained_under(&root, &outside).is_err());

        // A `..` component is refused before any filesystem access.
        assert!(contained_under(&root, &root.join("..").join("escape.json")).is_err());

        // The root itself is not "strictly under" the root.
        assert!(contained_under(&root, &root).is_err());

        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&outside_dir);
    }

    #[cfg(unix)]
    #[test]
    fn unix_read_rejects_group_readable_and_symlinks() {
        use std::os::unix::fs::PermissionsExt;
        let dir = temp_dir("unix-perms");
        let scope = ensure_owner_private_dir(&dir).expect("scope");
        let path = scope.join("secret.json");
        write_owner_only_file_atomic(&path, b"{}").expect("write");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o640)).expect("relax");
        assert!(
            read_owner_only_file(&path, 4096).is_err(),
            "a group-readable credential file must fail closed"
        );
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).expect("restore");
        assert!(read_owner_only_file(&path, 4096).is_ok());

        let link = scope.join("link.json");
        std::os::unix::fs::symlink(&path, &link).expect("symlink");
        assert!(
            read_owner_only_file(&link, 4096).is_err(),
            "a symlinked credential path must fail closed"
        );
        assert!(contained_under(&scope, &link).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(windows)]
    #[test]
    fn windows_read_accepts_a_normal_same_user_file_outside_any_scope() {
        // The producer writes its credential file with the process token's ordinary DACL, outside
        // the intent scope. That safe shape must be accepted; anything broader must not.
        let dir = temp_dir("win-outside");
        let path = dir.join("registry.json");
        std::fs::write(&path, b"{\"secret\":\"x\"}").expect("write");
        let read = read_owner_only_file(&path, 4096);
        assert!(read.is_ok(), "unexpected rejection: {read:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(windows)]
    #[test]
    fn windows_read_rejects_a_world_accessible_file() {
        let dir = temp_dir("win-broad");
        let path = dir.join("broad.json");
        std::fs::write(&path, b"{}").expect("write");
        // Grant Everyone read via icacls; if the grant does not take, skip rather than assert a
        // false negative (some CI images disallow the change).
        let granted = std::process::Command::new("icacls")
            .arg(&path)
            .arg("/grant")
            .arg("*S-1-1-0:(R)")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        if granted {
            assert!(
                read_owner_only_file(&path, 4096).is_err(),
                "a world-readable credential file must fail closed"
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn producer_root_hardening_never_strips_an_existing_producer_file() {
        // Regression guard for the hazard that made `ensure_owner_private_dir` unusable on a
        // shared directory: on Windows, protecting a directory's DACL re-propagates inheritance and
        // leaves pre-existing children with an empty DACL — unreadable even to their own author.
        // The producer-root rule must validate an existing directory, never rewrite it.
        let dir = temp_dir("producer-root");
        let existing = dir.join("registry.json");
        write_producer_file(&existing, b"{\"secret\":\"x\"}");
        let root = ensure_owner_private_producer_root(&dir).expect("harden producer root");
        assert!(root.is_absolute());
        assert_eq!(
            std::fs::read_to_string(&existing).expect("the producer's own file stays readable"),
            "{\"secret\":\"x\"}"
        );
        // Files the producer writes afterwards are still readable and still pass the per-file
        // owner-private check, which is what the credential read actually relies on.
        let later = dir.join("later.json");
        write_producer_file(&later, b"{}");
        assert!(read_owner_only_file(&later, 4096).is_ok());
        // Idempotent.
        assert!(ensure_owner_private_producer_root(&dir).is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_freshly_created_producer_root_is_owner_private_from_birth() {
        let base = temp_dir("producer-root-fresh");
        let root = base.join("nested").join("telex-bridge");
        assert!(!root.exists());
        let secured = ensure_owner_private_producer_root(&root).expect("create producer root");
        assert!(secured.exists());
        // A file written into a freshly created root is readable and owner-private.
        let file = secured.join("registry.json");
        write_producer_file(&file, b"{}");
        assert!(read_owner_only_file(&file, 4096).is_ok());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn process_identity_primitives_resolve_or_fail_closed() {
        let exe = process_exe_path(std::process::id()).expect("own exe path");
        assert!(exe.is_absolute());
        let expected = std::fs::canonicalize(std::env::current_exe().expect("current exe"))
            .expect("canonical current exe");
        assert_eq!(exe, expected);

        // A pid that cannot exist must fail closed, never return a value.
        assert!(process_exe_path(0).is_err());

        let host = host_id().expect("host id");
        let boot = boot_id().expect("boot id");
        assert_eq!(host.len(), 32);
        assert_eq!(boot.len(), 32);
        assert_ne!(host, boot);
        // Stable within a boot.
        assert_eq!(host, host_id().expect("host id again"));
        assert_eq!(boot, boot_id().expect("boot id again"));
    }

    /// The boot identity is compared for *exact equality* across two independent processes (the
    /// attaching CLI and the daemon), and a mismatch terminates every intent as
    /// `foreign_host_or_boot` — which the anti-downgrade guard then turns into a hard refusal of
    /// an unrelated `telex attach`. On Windows it used to be derived as
    /// `SystemTime::now() - GetTickCount64()`, which jitters across a second boundary a few
    /// percent of the time and shifts outright on any clock step; it is now minted once and
    /// persisted.
    ///
    /// Hammering the **uncached** resolver is what makes that stability a checked property rather
    /// than a claim in a comment. The previous version of this test called `boot_id()`, which is
    /// memoized in a `OnceLock`: after the first call it compared a clone of a cached `String` to
    /// itself two hundred times, so it would have passed unchanged against the jittering
    /// implementation it exists to rule out. Each iteration here re-enters the platform resolver,
    /// which on Windows means a full registry read-back (and, on the first call of the boot, a
    /// mint plus persist plus read-back).
    #[test]
    fn boot_identity_is_stable_across_repeated_independent_resolutions() {
        let first = boot_id_uncached().expect("boot id");
        assert_eq!(first.len(), 32);
        for _ in 0..200 {
            assert_eq!(
                first,
                boot_id_uncached().expect("boot id again"),
                "the boot identity must not jitter: two processes resolving it independently \
                 must agree, or every station intent fails closed"
            );
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        // And the cached accessor must agree with the resolver it caches.
        assert_eq!(first, boot_id().expect("cached boot id"));
    }
}
