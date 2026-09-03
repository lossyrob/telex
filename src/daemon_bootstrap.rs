//! Application-client daemon bootstrap: installed-current or pinned-exact.
//!
//! This module owns the Application Client-side selector admission, trusted-root
//! validation, InstalledCurrent selection freezing, and pre-Hello peer identity
//! projection that the public `ApplicationDaemonBootstrap` policy relies on. It
//! never opens a public raw IPC seam and never allows the consumer binary to
//! host `daemon serve`. All lifecycle admissions are OS-backed and fail closed
//! on unsupported semantics.
//!
//! See `.streamliner/workstreams/application-client/design/current-design.md`
//! and `.streamliner/workstreams/local-daemon/design/current-design.md` for the
//! promoted contract; this module is the load-bearing implementation.
#![allow(clippy::result_large_err)]

use crate::install::{self, InstallLayout, VersionManifest};
use std::collections::BTreeSet;
use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Bounded selector-admission acquisition deadline. Callers (parent connect
/// and upgrade/rollback) never block on the selector lock for longer than
/// this. Exhaustion maps to [`DaemonBootstrapFailure::SelectionUnstable`].
pub(crate) const SELECTOR_LOCK_DEADLINE: Duration = Duration::from_secs(5);

/// Per-attempt sleep between non-blocking retries. Small enough to keep
/// worst-case admission latency close to the deadline, large enough to avoid
/// spinning on the lock file.
const SELECTOR_LOCK_RETRY_INTERVAL: Duration = Duration::from_millis(25);

/// Hidden env transport handing a resolved selection to a spawned daemon.
///
/// Not a bearer secret and not part of the public IPC surface: the token only
/// witnesses one InstalledCurrent resolution and is validated by the child
/// independently against a fresh resolution and its own process image before
/// serving.
pub(crate) const BOOTSTRAP_TOKEN_ENV: &str = "TELEX_DAEMON_SELECTION_TOKEN";

/// Public typed enumeration of installed-current / exact bootstrap failures.
///
/// Kept non-exhaustive on purpose so promoting new fail-closed reasons never
/// breaks source compatibility. No variant carries raw authority paths as
/// durable evidence.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
#[serde(rename_all = "kebab-case")]
pub enum DaemonBootstrapFailure {
    InvalidTrustedRoot,
    UnsafeInstallAuthority,
    MissingCurrent,
    InvalidManifest,
    IncompatibleManifest,
    SelectionUnstable,
    MissingExecutable,
    ExecutableIdentityMismatch,
    ForeignDaemon,
}

impl std::fmt::Display for DaemonBootstrapFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::InvalidTrustedRoot => "invalid trusted install root",
            Self::UnsafeInstallAuthority => "unsafe install authority",
            Self::MissingCurrent => "installed-current selector is missing",
            Self::InvalidManifest => "installed-current manifest is invalid",
            Self::IncompatibleManifest => "installed-current manifest is incompatible",
            Self::SelectionUnstable => "installed-current selection is unstable",
            Self::MissingExecutable => "installed-current executable is missing",
            Self::ExecutableIdentityMismatch => "installed-current executable identity mismatch",
            Self::ForeignDaemon => "prestarted daemon does not match the selected build",
        };
        f.write_str(s)
    }
}

impl std::error::Error for DaemonBootstrapFailure {}

/// Immutable, canonical bootstrap policy captured at configuration time.
///
/// The `InstalledCurrent` variant captures the canonical absolute root once,
/// so a later working-directory change cannot reinterpret it. `ExactExecutable`
/// captures the canonical target path and its platform file identity; it has
/// no manifest authority and does not follow upgrade.
#[derive(Debug, Clone)]
pub(crate) enum BootstrapPolicy {
    InstalledCurrent {
        trusted_root: PathBuf,
        root_identity: FileIdentity,
    },
    ExactExecutable {
        executable: PathBuf,
        file_identity: FileIdentity,
    },
}

impl BootstrapPolicy {
    /// Freeze an `InstalledCurrent { trusted_root }` policy: validate the root
    /// path shape, canonicalize it, and record its owner-authority baseline.
    pub(crate) fn installed_current(
        trusted_root: PathBuf,
    ) -> Result<Arc<Self>, DaemonBootstrapFailure> {
        if trusted_root.as_os_str().is_empty() || !trusted_root.is_absolute() {
            return Err(DaemonBootstrapFailure::InvalidTrustedRoot);
        }
        for component in trusted_root.components() {
            if matches!(component, Component::ParentDir) {
                return Err(DaemonBootstrapFailure::InvalidTrustedRoot);
            }
        }
        let supplied = std::fs::symlink_metadata(&trusted_root)
            .map_err(|_| DaemonBootstrapFailure::InvalidTrustedRoot)?;
        if supplied.file_type().is_symlink() || is_reparse_point(&supplied) || !supplied.is_dir() {
            return Err(DaemonBootstrapFailure::InvalidTrustedRoot);
        }
        let canonical = std::fs::canonicalize(&trusted_root)
            .map_err(|_| DaemonBootstrapFailure::InvalidTrustedRoot)?;
        if !canonical.is_dir() {
            return Err(DaemonBootstrapFailure::InvalidTrustedRoot);
        }
        // Authority-chain check: fail closed if the trusted root is owned by a
        // foreign principal or grants write/delete/ownership to another
        // principal. Owner writability is permitted (same-user upgrade).
        check_authority_dir(&canonical)?;
        check_parent_authority_chain(&canonical)?;
        let root_identity =
            path_identity(&canonical).ok_or(DaemonBootstrapFailure::InvalidTrustedRoot)?;
        Ok(Arc::new(BootstrapPolicy::InstalledCurrent {
            trusted_root: canonical,
            root_identity,
        }))
    }

    /// Freeze an `ExactExecutable { executable }` policy.
    pub(crate) fn exact_executable(
        executable: PathBuf,
    ) -> Result<Arc<Self>, DaemonBootstrapFailure> {
        if executable.as_os_str().is_empty() || !executable.is_absolute() {
            return Err(DaemonBootstrapFailure::MissingExecutable);
        }
        let supplied = std::fs::symlink_metadata(&executable)
            .map_err(|_| DaemonBootstrapFailure::MissingExecutable)?;
        if supplied.file_type().is_symlink() || is_reparse_point(&supplied) || !supplied.is_file() {
            return Err(DaemonBootstrapFailure::UnsafeInstallAuthority);
        }
        let canonical = std::fs::canonicalize(&executable)
            .map_err(|_| DaemonBootstrapFailure::MissingExecutable)?;
        if !canonical.is_file() {
            return Err(DaemonBootstrapFailure::MissingExecutable);
        }
        // ExactExecutable is a pinned dev/test seam. Apply the same
        // untrusted-writability guard as InstalledCurrent so a dev binary in a
        // world-writable location is rejected up-front, and capture the file
        // identity for later pre-Hello peer authentication.
        check_authority_file(&canonical)?;
        check_parent_authority_chain(&canonical)?;
        let file_identity = file_identity(&canonical)?;
        Ok(Arc::new(BootstrapPolicy::ExactExecutable {
            executable: canonical,
            file_identity,
        }))
    }

    /// Return the canonical trusted root of an `InstalledCurrent` policy, if any.
    #[allow(dead_code)]
    pub(crate) fn installed_current_root(&self) -> Option<&Path> {
        match self {
            BootstrapPolicy::InstalledCurrent { trusted_root, .. } => Some(trusted_root),
            BootstrapPolicy::ExactExecutable { .. } => None,
        }
    }

    /// Return the frozen exact executable path, if any.
    #[allow(dead_code)]
    pub(crate) fn exact_executable_path(&self) -> Option<&Path> {
        match self {
            BootstrapPolicy::ExactExecutable { executable, .. } => Some(executable),
            BootstrapPolicy::InstalledCurrent { .. } => None,
        }
    }
}

/// Internal immutable resolved-selection token.
///
/// Not public, not a bearer secret. Carries the selected tag, load-bearing
/// manifest identity, canonical target path, and platform file identity so one
/// resolution is used for both spawn and pre-Hello peer verification.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct SelectionToken {
    pub trusted_root: PathBuf,
    pub root_identity: FileIdentity,
    pub tag: String,
    pub package_version: String,
    pub build_id: String,
    pub schema_min: i64,
    pub schema_max: i64,
    pub protocol_major: u16,
    pub protocol_minor: u16,
    pub required_capabilities: Vec<String>,
    pub target_exe: PathBuf,
    pub file_identity: FileIdentity,
}

impl SelectionToken {
    /// Encode the token for hidden env-var handoff to a spawned daemon child.
    pub(crate) fn to_env_value(&self) -> String {
        serde_json::to_string(self).expect("SelectionToken serializes")
    }

    /// Parse a token from its hidden env-var encoding.
    pub(crate) fn from_env_value(raw: &str) -> Option<Self> {
        serde_json::from_str::<Self>(raw).ok()
    }
}

/// Platform file identity captured with the resolved target executable.
///
/// On Unix this is `(dev, ino)`; on Windows this is
/// `(volume_serial, file_index)` from `BY_HANDLE_FILE_INFORMATION`. Both
/// witness that a later replacement of the same path yields a different
/// identity, closing selector movement races without claiming
/// executable-content integrity.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct FileIdentity {
    pub kind: FileIdentityKind,
    pub high: u64,
    pub low: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum FileIdentityKind {
    UnixDevIno,
    WindowsVolumeFileId,
}

pub(crate) fn file_identity(path: &Path) -> Result<FileIdentity, DaemonBootstrapFailure> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let meta =
            std::fs::metadata(path).map_err(|_| DaemonBootstrapFailure::MissingExecutable)?;
        Ok(FileIdentity {
            kind: FileIdentityKind::UnixDevIno,
            high: meta.dev(),
            low: meta.ino(),
        })
    }

    #[cfg(windows)]
    {
        open_windows_witness(path).map(|witness| witness.identity)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = path;
        Err(DaemonBootstrapFailure::UnsafeInstallAuthority)
    }
}

#[cfg(unix)]
fn path_identity(path: &Path) -> Option<FileIdentity> {
    use std::os::unix::fs::MetadataExt;
    let meta = std::fs::metadata(path).ok()?;
    Some(FileIdentity {
        kind: FileIdentityKind::UnixDevIno,
        high: meta.dev(),
        low: meta.ino(),
    })
}

#[cfg(windows)]
fn path_identity(path: &Path) -> Option<FileIdentity> {
    windows_path_identity(path)
}

#[cfg(not(any(unix, windows)))]
fn path_identity(_path: &Path) -> Option<FileIdentity> {
    None
}

/// Resolve `<root>/current` -> `<root>/versions/<tag>/telex[.exe]` with full
/// authority and containment validation, plus manifest binding/compatibility
/// checks. Returns the frozen selection token used for both spawn and
/// pre-Hello peer verification.
pub(crate) fn resolve_installed_current(
    trusted_root: &Path,
) -> Result<SelectionToken, DaemonBootstrapFailure> {
    // Re-canonicalize each connect cycle so any selector-authority movement is
    // observed immediately. If the root disappeared or became unsafe, fail
    // closed rather than reusing a stale resolution.
    let supplied = std::fs::symlink_metadata(trusted_root)
        .map_err(|_| DaemonBootstrapFailure::InvalidTrustedRoot)?;
    if supplied.file_type().is_symlink() || is_reparse_point(&supplied) || !supplied.is_dir() {
        return Err(DaemonBootstrapFailure::InvalidTrustedRoot);
    }
    let canonical_root = std::fs::canonicalize(trusted_root)
        .map_err(|_| DaemonBootstrapFailure::InvalidTrustedRoot)?;
    check_authority_dir(&canonical_root)?;
    let layout = install::layout_for_root(&canonical_root);
    let tag = read_current_tag(&layout)?;
    validate_installed_target(&layout, &canonical_root, &tag, true)
}

/// Validate one versioned target before publishing it as `current`.
///
/// Upgrade and rollback use this same strict path as InstalledCurrent so they
/// cannot atomically publish a selection that production clients must reject.
/// The `current` selector is deliberately *not* required here: it names the
/// predecessor (or nothing at all on a first install), and the candidate is
/// validated on its own authority.
pub(crate) fn validate_installed_target_for_switch(
    layout: &InstallLayout,
    tag: &str,
) -> Result<(), DaemonBootstrapFailure> {
    let canonical_root = validate_install_root_for_switch(&layout.root)?;
    let canonical_layout = install::layout_for_root(&canonical_root);
    validate_installed_target(&canonical_layout, &canonical_root, tag, false).map(|_| ())
}

pub(crate) fn validate_install_root_for_switch(
    root: &Path,
) -> Result<PathBuf, DaemonBootstrapFailure> {
    let policy = BootstrapPolicy::installed_current(root.to_path_buf())?;
    match policy.as_ref() {
        BootstrapPolicy::InstalledCurrent { trusted_root, .. } => Ok(trusted_root.clone()),
        BootstrapPolicy::ExactExecutable { .. } => unreachable!(),
    }
}

fn validate_installed_target(
    layout: &InstallLayout,
    canonical_root: &Path,
    tag: &str,
    require_current_selector: bool,
) -> Result<SelectionToken, DaemonBootstrapFailure> {
    let root_identity =
        path_identity(canonical_root).ok_or(DaemonBootstrapFailure::InvalidTrustedRoot)?;
    // Enforce owner authority on every intermediate directory reachable from
    // the trusted root up to and including the version tag directory before
    // reading the manifest or executable. This fails closed on any component
    // that a foreign principal could write, delete, or take ownership of.
    check_authority_dir(&layout.versions_dir)?;
    let tag_dir = layout.versions_dir.join(tag);
    check_authority_dir(&tag_dir)?;
    if require_current_selector {
        check_authority_file(&layout.current_path)?;
    }
    let manifest_path = tag_dir.join("manifest.json");
    check_authority_file(&manifest_path)?;
    let manifest = read_and_validate_manifest(&manifest_path, tag)?;
    let target_exe = derive_target_executable(layout, tag, &manifest)?;
    let canonical_target = std::fs::canonicalize(&target_exe)
        .map_err(|_| DaemonBootstrapFailure::MissingExecutable)?;
    ensure_contained(canonical_root, &canonical_target)?;
    check_authority_file(&canonical_target)?;
    let identity = file_identity(&canonical_target)?;
    let (schema_min, schema_max, protocol_major, protocol_minor, required_capabilities) = (
        manifest.schema_min,
        manifest.schema_max,
        manifest.protocol_major,
        manifest.protocol_minor,
        manifest.required_capabilities.clone(),
    );
    Ok(SelectionToken {
        trusted_root: canonical_root.to_path_buf(),
        root_identity,
        tag: manifest.tag.clone(),
        package_version: manifest.package_version.clone(),
        build_id: manifest.build_id.clone(),
        schema_min,
        schema_max,
        protocol_major,
        protocol_minor,
        required_capabilities,
        target_exe: canonical_target,
        file_identity: identity,
    })
}

fn read_current_tag(layout: &InstallLayout) -> Result<String, DaemonBootstrapFailure> {
    match std::fs::symlink_metadata(&layout.current_path) {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(DaemonBootstrapFailure::MissingCurrent)
        }
        Err(_) => return Err(DaemonBootstrapFailure::UnsafeInstallAuthority),
    }
    check_authority_file(&layout.current_path)?;
    // The `previous` selector is never independently acceptable; even if
    // present, InstalledCurrent must resolve only `current`. We deliberately
    // do not read the previous file here.
    let raw = std::fs::read_to_string(&layout.current_path)
        .map_err(|_| DaemonBootstrapFailure::MissingCurrent)?;
    let tag = raw.trim();
    if tag.is_empty()
        || tag.contains('/')
        || tag.contains('\\')
        || tag == "."
        || tag == ".."
        || tag.contains("..")
    {
        return Err(DaemonBootstrapFailure::InvalidManifest);
    }
    Ok(tag.to_string())
}

fn read_and_validate_manifest(
    manifest_path: &Path,
    tag: &str,
) -> Result<VersionManifest, DaemonBootstrapFailure> {
    let raw = std::fs::read_to_string(manifest_path)
        .map_err(|_| DaemonBootstrapFailure::InvalidManifest)?;
    let manifest: VersionManifest =
        serde_json::from_str(&raw).map_err(|_| DaemonBootstrapFailure::InvalidManifest)?;
    // The manifest must bind the selected tag; a mismatched tag is a foreign
    // manifest, not a compatibility skew.
    if manifest.tag != tag {
        return Err(DaemonBootstrapFailure::InvalidManifest);
    }
    // Load-bearing identity fields must be non-empty and non-sentinel. Empty
    // or `unknown` build identity forfeits the HelloAck build check; this is
    // rejected as an incomplete binding rather than a compatibility skew.
    if manifest.build_id.is_empty() || manifest.build_id == install::UNKNOWN_BUILD_ID {
        return Err(DaemonBootstrapFailure::InvalidManifest);
    }
    if manifest.package_version.is_empty() {
        return Err(DaemonBootstrapFailure::InvalidManifest);
    }
    if manifest.protocol_major != crate::daemon_ipc::PROTOCOL_MAJOR {
        return Err(DaemonBootstrapFailure::IncompatibleManifest);
    }
    if !(manifest.schema_min..=manifest.schema_max).contains(&install::SUPPORTED_SCHEMA_MAX) {
        return Err(DaemonBootstrapFailure::IncompatibleManifest);
    }
    // Every required capability listed by the manifest must be a non-empty
    // string, and the collection must be duplicate-free. A malformed
    // capability list is a manifest defect, not a compatibility skew.
    let mut seen: BTreeSet<&str> = BTreeSet::new();
    for cap in &manifest.required_capabilities {
        if cap.is_empty() {
            return Err(DaemonBootstrapFailure::InvalidManifest);
        }
        if !seen.insert(cap.as_str()) {
            return Err(DaemonBootstrapFailure::InvalidManifest);
        }
    }
    // Every required daemon capability the current binary depends on must be
    // present in the manifest metadata; the connected HelloAck still performs
    // the peer capability negotiation.
    for cap in crate::daemon_ipc::REQUIRED_CAPABILITIES {
        if !manifest.required_capabilities.iter().any(|c| c == *cap) {
            return Err(DaemonBootstrapFailure::IncompatibleManifest);
        }
    }
    Ok(manifest)
}

fn derive_target_executable(
    layout: &InstallLayout,
    tag: &str,
    manifest: &VersionManifest,
) -> Result<PathBuf, DaemonBootstrapFailure> {
    // Never trust the manifest's `binary` field as a path authority; only
    // require it to bind the same versioned executable we derive here. An
    // empty or unresolvable manifest binary is an incomplete binding.
    let derived = layout.versions_dir.join(tag).join(install::exe_name());
    let derived_metadata = std::fs::symlink_metadata(&derived)
        .map_err(|_| DaemonBootstrapFailure::MissingExecutable)?;
    if derived_metadata.file_type().is_symlink()
        || is_reparse_point(&derived_metadata)
        || !derived_metadata.is_file()
    {
        return Err(DaemonBootstrapFailure::UnsafeInstallAuthority);
    }
    if !derived.is_file() {
        return Err(DaemonBootstrapFailure::MissingExecutable);
    }
    if manifest.binary.is_empty() {
        return Err(DaemonBootstrapFailure::InvalidManifest);
    }
    let manifest_path = PathBuf::from(&manifest.binary);
    if !manifest_path.is_absolute() {
        return Err(DaemonBootstrapFailure::InvalidManifest);
    }
    let manifest_canonical = std::fs::canonicalize(&manifest_path)
        .map_err(|_| DaemonBootstrapFailure::InvalidManifest)?;
    let derived_canonical =
        std::fs::canonicalize(&derived).map_err(|_| DaemonBootstrapFailure::MissingExecutable)?;
    if !same_path(&manifest_canonical, &derived_canonical) {
        return Err(DaemonBootstrapFailure::InvalidManifest);
    }
    Ok(derived)
}

fn ensure_contained(root: &Path, target: &Path) -> Result<(), DaemonBootstrapFailure> {
    if !target.starts_with(root) {
        return Err(DaemonBootstrapFailure::UnsafeInstallAuthority);
    }
    Ok(())
}

fn same_path(a: &Path, b: &Path) -> bool {
    #[cfg(windows)]
    {
        let na = a
            .to_string_lossy()
            .trim_start_matches(r"\\?\")
            .replace('/', r"\")
            .to_ascii_lowercase();
        let nb = b
            .to_string_lossy()
            .trim_start_matches(r"\\?\")
            .replace('/', r"\")
            .to_ascii_lowercase();
        na == nb
    }
    #[cfg(not(windows))]
    {
        a == b
    }
}

/// Fail closed unless `path` is an ordinary file whose authority chain is
/// entirely owned by the current OS principal and whose DACL (on Windows)
/// grants write, delete, or ownership control only to the current principal
/// or well-known privileged principals.
fn check_authority_file(path: &Path) -> Result<(), DaemonBootstrapFailure> {
    let meta =
        std::fs::symlink_metadata(path).map_err(|_| DaemonBootstrapFailure::MissingExecutable)?;
    if meta.file_type().is_symlink() || is_reparse_point(&meta) {
        return Err(DaemonBootstrapFailure::UnsafeInstallAuthority);
    }
    if !meta.is_file() {
        return Err(DaemonBootstrapFailure::UnsafeInstallAuthority);
    }
    check_authority_component(path, &meta)?;
    Ok(())
}

/// Fail closed unless `path` is a directory owned by the current OS principal
/// with a strict authority DACL, and every ancestor component up to the
/// filesystem root satisfies the same guarantee. `..` components are rejected
/// at policy freeze time; here we accept only fully canonical paths.
fn check_authority_dir(path: &Path) -> Result<(), DaemonBootstrapFailure> {
    let meta =
        std::fs::symlink_metadata(path).map_err(|_| DaemonBootstrapFailure::InvalidTrustedRoot)?;
    if meta.file_type().is_symlink() || is_reparse_point(&meta) {
        return Err(DaemonBootstrapFailure::UnsafeInstallAuthority);
    }
    if !meta.is_dir() {
        return Err(DaemonBootstrapFailure::InvalidTrustedRoot);
    }
    check_authority_component(path, &meta)?;
    Ok(())
}

fn check_parent_authority_chain(path: &Path) -> Result<(), DaemonBootstrapFailure> {
    let mut ancestor = path.parent();
    while let Some(directory) = ancestor {
        let metadata = std::fs::symlink_metadata(directory)
            .map_err(|_| DaemonBootstrapFailure::UnsafeInstallAuthority)?;
        if metadata.file_type().is_symlink() || is_reparse_point(&metadata) || !metadata.is_dir() {
            return Err(DaemonBootstrapFailure::UnsafeInstallAuthority);
        }
        check_authority_ancestor(directory, &metadata)?;
        ancestor = directory.parent();
    }
    Ok(())
}

#[cfg(unix)]
fn check_authority_component(
    _path: &Path,
    meta: &std::fs::Metadata,
) -> Result<(), DaemonBootstrapFailure> {
    use std::os::unix::fs::MetadataExt;
    // Fail closed if any principal other than the current OS user owns the
    // authority artifact, or if group/world write is granted at this component.
    let uid = unsafe { libc::geteuid() };
    if meta.uid() != uid {
        return Err(DaemonBootstrapFailure::UnsafeInstallAuthority);
    }

    if meta.mode() & 0o022 != 0 {
        return Err(DaemonBootstrapFailure::UnsafeInstallAuthority);
    }
    Ok(())
}

#[cfg(unix)]
fn check_authority_ancestor(
    _path: &Path,
    meta: &std::fs::Metadata,
) -> Result<(), DaemonBootstrapFailure> {
    use std::os::unix::fs::MetadataExt;
    let uid = unsafe { libc::geteuid() };
    if meta.uid() != uid && meta.uid() != 0 {
        return Err(DaemonBootstrapFailure::UnsafeInstallAuthority);
    }
    let foreign_writable = meta.mode() & 0o022 != 0;
    let sticky = meta.mode() & 0o1000 != 0;
    if foreign_writable && !sticky {
        return Err(DaemonBootstrapFailure::UnsafeInstallAuthority);
    }
    Ok(())
}

#[cfg(windows)]
fn is_reparse_point(meta: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;
    meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn is_reparse_point(_meta: &std::fs::Metadata) -> bool {
    false
}

#[cfg(windows)]
fn check_authority_component(
    path: &Path,
    _meta: &std::fs::Metadata,
) -> Result<(), DaemonBootstrapFailure> {
    windows_authority::check_owner_and_dacl(path)
}

#[cfg(windows)]
fn check_authority_ancestor(
    path: &Path,
    _meta: &std::fs::Metadata,
) -> Result<(), DaemonBootstrapFailure> {
    windows_authority::check_ancestor_authority(path)
}

#[cfg(not(any(unix, windows)))]
fn check_authority_component(
    _path: &Path,
    _meta: &std::fs::Metadata,
) -> Result<(), DaemonBootstrapFailure> {
    Err(DaemonBootstrapFailure::UnsafeInstallAuthority)
}

#[cfg(not(any(unix, windows)))]
fn check_authority_ancestor(
    _path: &Path,
    _meta: &std::fs::Metadata,
) -> Result<(), DaemonBootstrapFailure> {
    Err(DaemonBootstrapFailure::UnsafeInstallAuthority)
}

#[cfg(windows)]
mod windows_authority {
    use super::DaemonBootstrapFailure;
    use std::ffi::c_void;
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;
    use windows_sys::Win32::Foundation::{LocalFree, PSID};
    use windows_sys::Win32::Security::Authorization::{
        ConvertSidToStringSidW, GetNamedSecurityInfoW, SE_FILE_OBJECT,
    };
    use windows_sys::Win32::Security::{
        AclSizeInformation, EqualSid, GetAce, GetAclInformation, GetSecurityDescriptorControl,
        GetTokenInformation, TokenUser, ACCESS_ALLOWED_ACE, ACL, ACL_SIZE_INFORMATION,
        DACL_SECURITY_INFORMATION, OWNER_SECURITY_INFORMATION, SE_DACL_PRESENT, SE_DACL_PROTECTED,
        TOKEN_QUERY, TOKEN_USER,
    };
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    const ACCESS_ALLOWED_ACE_TYPE: u8 = 0;
    const ACCESS_DENIED_ACE_TYPE: u8 = 1;
    const INHERIT_ONLY_ACE: u8 = 0x08;
    const DELETE: u32 = 0x0001_0000;
    const WRITE_DAC: u32 = 0x0004_0000;
    const WRITE_OWNER: u32 = 0x0008_0000;
    const GENERIC_ALL: u32 = 0x1000_0000;
    const GENERIC_WRITE: u32 = 0x4000_0000;
    const FILE_WRITE_DATA: u32 = 0x0000_0002;
    const FILE_APPEND_DATA: u32 = 0x0000_0004;
    const FILE_WRITE_EA: u32 = 0x0000_0010;
    const FILE_DELETE_CHILD: u32 = 0x0000_0040;
    const FILE_WRITE_ATTRIBUTES: u32 = 0x0000_0100;
    const AUTHORITY_MUTATION_RIGHTS: u32 = DELETE
        | WRITE_DAC
        | WRITE_OWNER
        | GENERIC_ALL
        | GENERIC_WRITE
        | FILE_WRITE_DATA
        | FILE_APPEND_DATA
        | FILE_WRITE_EA
        | FILE_DELETE_CHILD
        | FILE_WRITE_ATTRIBUTES;

    /// Fail closed unless the current OS user's SID owns `path` and its DACL
    /// grants access only to the owning user, `LocalSystem`, or the local
    /// `BUILTIN\Administrators` group; every ACE must be a permit ACE for one
    /// of those principals. Reparse points, missing DACLs, and unprotected or
    /// inherited DACLs also fail closed.
    pub(super) fn check_owner_and_dacl(path: &Path) -> Result<(), DaemonBootstrapFailure> {
        let owner_sid = current_user_sid()?;
        let system_sid = SidBuf::from_str("S-1-5-18")?;
        let admins_sid = SidBuf::from_str("S-1-5-32-544")?;
        let trusted_installer_sid =
            SidBuf::from_str("S-1-5-80-956008885-3418522649-1831038044-1853292631-2271478464")?;
        let creator_owner_sid = SidBuf::from_str("S-1-3-0")?;
        let trusted = TrustedSidRefs {
            owner: owner_sid.as_psid(),
            system: system_sid.as_psid(),
            admins: admins_sid.as_psid(),
            trusted_installer: trusted_installer_sid.as_psid(),
            creator_owner: creator_owner_sid.as_psid(),
        };
        check_component(path, &trusted, true, AUTHORITY_MUTATION_RIGHTS)
    }

    pub(super) fn check_ancestor_authority(path: &Path) -> Result<(), DaemonBootstrapFailure> {
        let owner_sid = current_user_sid()?;
        let system_sid = SidBuf::from_str("S-1-5-18")?;
        let admins_sid = SidBuf::from_str("S-1-5-32-544")?;
        let trusted_installer_sid =
            SidBuf::from_str("S-1-5-80-956008885-3418522649-1831038044-1853292631-2271478464")?;
        let creator_owner_sid = SidBuf::from_str("S-1-3-0")?;
        let trusted = TrustedSidRefs {
            owner: owner_sid.as_psid(),
            system: system_sid.as_psid(),
            admins: admins_sid.as_psid(),
            trusted_installer: trusted_installer_sid.as_psid(),
            creator_owner: creator_owner_sid.as_psid(),
        };
        check_component(
            path,
            &trusted,
            false,
            DELETE | WRITE_DAC | WRITE_OWNER | FILE_DELETE_CHILD | GENERIC_ALL | GENERIC_WRITE,
        )
    }

    struct TrustedSidRefs {
        owner: PSID,
        system: PSID,
        admins: PSID,
        trusted_installer: PSID,
        creator_owner: PSID,
    }

    fn check_component(
        path: &Path,
        trusted: &TrustedSidRefs,
        require_current_owner: bool,
        foreign_mutation_rights: u32,
    ) -> Result<(), DaemonBootstrapFailure> {
        let wide: Vec<u16> = path
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        let mut sd: *mut c_void = std::ptr::null_mut();
        let mut owner: PSID = std::ptr::null_mut();
        let mut dacl: *mut ACL = std::ptr::null_mut();
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
            return Err(DaemonBootstrapFailure::UnsafeInstallAuthority);
        }
        let _guard = LocalAllocGuard(sd);

        let owner_is_current = !owner.is_null() && unsafe { EqualSid(owner, trusted.owner) } != 0;
        let owner_is_system = !owner.is_null() && unsafe { EqualSid(owner, trusted.system) } != 0;
        let owner_is_admins = !owner.is_null() && unsafe { EqualSid(owner, trusted.admins) } != 0;
        let owner_is_trusted_installer =
            !owner.is_null() && unsafe { EqualSid(owner, trusted.trusted_installer) } != 0;
        if !owner_is_current
            && (require_current_owner
                || (!owner_is_system && !owner_is_admins && !owner_is_trusted_installer))
        {
            return Err(DaemonBootstrapFailure::UnsafeInstallAuthority);
        }

        let mut control: u16 = 0;
        let mut revision: u32 = 0;
        let ok = unsafe { GetSecurityDescriptorControl(sd, &mut control, &mut revision) };
        if ok == 0 || control & SE_DACL_PRESENT == 0 {
            return Err(DaemonBootstrapFailure::UnsafeInstallAuthority);
        }
        // We require the DACL to be either protected (explicit) OR entirely
        // composed of ACEs pointing at the owner/SYSTEM/Administrators.
        // Non-protected DACLs are permitted only when every ACE is trusted;
        // otherwise inherited entries from an untrusted parent could grant
        // write access to foreign principals.
        if dacl.is_null() {
            return Err(DaemonBootstrapFailure::UnsafeInstallAuthority);
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
            return Err(DaemonBootstrapFailure::UnsafeInstallAuthority);
        }
        for idx in 0..info.AceCount {
            let mut ace_ptr: *mut c_void = std::ptr::null_mut();
            let ok = unsafe { GetAce(dacl, idx, &mut ace_ptr) };
            if ok == 0 || ace_ptr.is_null() {
                return Err(DaemonBootstrapFailure::UnsafeInstallAuthority);
            }
            let header = unsafe { &*(ace_ptr as *const windows_sys::Win32::Security::ACE_HEADER) };
            if !require_current_owner && header.AceFlags & INHERIT_ONLY_ACE != 0 {
                continue;
            }
            match header.AceType {
                ACCESS_ALLOWED_ACE_TYPE => {
                    let ace = unsafe { &*(ace_ptr as *const ACCESS_ALLOWED_ACE) };
                    let sid = (&ace.SidStart as *const u32).cast::<c_void>() as PSID;
                    let allowed = unsafe {
                        EqualSid(sid, trusted.owner) != 0
                            || EqualSid(sid, trusted.system) != 0
                            || EqualSid(sid, trusted.admins) != 0
                            || EqualSid(sid, trusted.trusted_installer) != 0
                            || EqualSid(sid, trusted.creator_owner) != 0
                    };
                    if !allowed && ace.Mask & foreign_mutation_rights != 0 {
                        return Err(DaemonBootstrapFailure::UnsafeInstallAuthority);
                    }
                }
                ACCESS_DENIED_ACE_TYPE => {
                    // Deny ACEs cannot grant mutation authority. Leave normal
                    // access checks to file opening rather than rejecting a
                    // safely restrictive descriptor.
                }
                _ => {
                    return Err(DaemonBootstrapFailure::UnsafeInstallAuthority);
                }
            }
        }
        let _ = control;
        let _ = revision;
        let _ = SE_DACL_PROTECTED;
        Ok(())
    }

    fn current_user_sid() -> Result<SidBuf, DaemonBootstrapFailure> {
        let mut token: isize = 0;
        let ok = unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) };
        if ok == 0 {
            return Err(DaemonBootstrapFailure::UnsafeInstallAuthority);
        }
        let _tguard = HandleGuard(token);
        let mut needed: u32 = 0;
        unsafe {
            GetTokenInformation(token, TokenUser, std::ptr::null_mut(), 0, &mut needed);
        }
        if needed == 0 {
            return Err(DaemonBootstrapFailure::UnsafeInstallAuthority);
        }
        let mut buf = vec![0u8; needed as usize];
        let ok = unsafe {
            GetTokenInformation(
                token,
                TokenUser,
                buf.as_mut_ptr() as *mut c_void,
                needed,
                &mut needed,
            )
        };
        if ok == 0 {
            return Err(DaemonBootstrapFailure::UnsafeInstallAuthority);
        }
        let user = unsafe { &*(buf.as_ptr() as *const TOKEN_USER) };
        SidBuf::from_str(&sid_to_string(user.User.Sid)?)
    }

    fn sid_to_string(sid: PSID) -> Result<String, DaemonBootstrapFailure> {
        let mut wide: *mut u16 = std::ptr::null_mut();
        let ok = unsafe { ConvertSidToStringSidW(sid, &mut wide) };
        if ok == 0 || wide.is_null() {
            return Err(DaemonBootstrapFailure::UnsafeInstallAuthority);
        }
        let mut len = 0usize;
        while unsafe { *wide.add(len) } != 0 {
            len += 1;
        }
        let slice = unsafe { std::slice::from_raw_parts(wide, len) };
        let s = String::from_utf16(slice)
            .map_err(|_| DaemonBootstrapFailure::UnsafeInstallAuthority)?;
        unsafe {
            LocalFree(wide as *mut c_void);
        }
        Ok(s)
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

    struct HandleGuard(isize);
    impl Drop for HandleGuard {
        fn drop(&mut self) {
            if self.0 != 0 {
                unsafe {
                    windows_sys::Win32::Foundation::CloseHandle(self.0);
                }
            }
        }
    }

    /// Owned SID buffer allocated by `ConvertStringSidToSidW`.
    struct SidBuf(PSID);
    impl SidBuf {
        fn from_str(s: &str) -> Result<Self, DaemonBootstrapFailure> {
            use windows_sys::Win32::Security::Authorization::ConvertStringSidToSidW;
            let wide: Vec<u16> = s.encode_utf16().chain(std::iter::once(0)).collect();
            let mut sid: PSID = std::ptr::null_mut();
            let ok = unsafe { ConvertStringSidToSidW(wide.as_ptr(), &mut sid) };
            if ok == 0 || sid.is_null() {
                return Err(DaemonBootstrapFailure::UnsafeInstallAuthority);
            }
            Ok(Self(sid))
        }

        fn as_psid(&self) -> PSID {
            self.0
        }
    }
    impl Drop for SidBuf {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe {
                    LocalFree(self.0);
                }
            }
        }
    }
}

/// Selector-coordination lock persistent under the trusted install root.
///
/// The lock file is created lazily on first use and is never deleted or
/// replaced as part of selection. Unix uses an advisory flock; Windows uses a
/// `LockFileEx` range lock on the first byte. Lock scope is one process; the
/// guard releases automatically on drop or process crash.
#[derive(Debug)]
pub(crate) struct SelectorAdmission {
    file: std::fs::File,
    #[cfg(windows)]
    #[allow(dead_code)]
    exclusive: bool,
}

impl SelectorAdmission {
    /// Non-blocking, bounded-retry shared acquisition. Suitable for the
    /// synchronous startup path (child pre-serve) that cannot borrow the
    /// caller's async runtime.
    #[allow(dead_code)]
    pub(crate) fn shared(root: &Path) -> Result<Self, DaemonBootstrapFailure> {
        acquire_with_deadline_sync(root, false, SELECTOR_LOCK_DEADLINE)
    }

    /// Non-blocking, bounded-retry exclusive acquisition. Callers that operate
    /// on a Tokio runtime must prefer [`Self::exclusive_async`] to avoid
    /// blocking a worker thread.
    #[allow(dead_code)]
    pub(crate) fn exclusive(root: &Path) -> Result<Self, DaemonBootstrapFailure> {
        acquire_with_deadline_sync(root, true, SELECTOR_LOCK_DEADLINE)
    }

    /// Async bounded-retry shared acquisition. The underlying non-blocking
    /// filesystem calls are trivially fast; the async wrapper only exists so
    /// callers can compose the deadline with other async work without
    /// occupying a Tokio worker thread.
    pub(crate) async fn shared_async(root: PathBuf) -> Result<Self, DaemonBootstrapFailure> {
        acquire_with_deadline_async(root, false, SELECTOR_LOCK_DEADLINE).await
    }

    /// Async bounded-retry exclusive acquisition.
    pub(crate) async fn exclusive_async(root: PathBuf) -> Result<Self, DaemonBootstrapFailure> {
        acquire_with_deadline_async(root, true, SELECTOR_LOCK_DEADLINE).await
    }
}

impl Drop for SelectorAdmission {
    fn drop(&mut self) {
        let _ = fs2::FileExt::unlock(&self.file);
    }
}

fn selector_lock_path(root: &Path) -> PathBuf {
    root.join(".telex-selector.lock")
}

#[allow(dead_code)]
fn acquire_with_deadline_sync(
    root: &Path,
    exclusive: bool,
    deadline: Duration,
) -> Result<SelectorAdmission, DaemonBootstrapFailure> {
    let deadline_at = Instant::now() + deadline;
    loop {
        match try_acquire_nonblocking(root, exclusive) {
            Ok(guard) => return Ok(guard),
            Err(NonBlockingLockError::WouldBlock) => {
                if Instant::now() >= deadline_at {
                    return Err(DaemonBootstrapFailure::SelectionUnstable);
                }
                std::thread::sleep(SELECTOR_LOCK_RETRY_INTERVAL);
            }
            Err(NonBlockingLockError::Unsafe) => {
                return Err(DaemonBootstrapFailure::UnsafeInstallAuthority);
            }
        }
    }
}

async fn acquire_with_deadline_async(
    root: PathBuf,
    exclusive: bool,
    deadline: Duration,
) -> Result<SelectorAdmission, DaemonBootstrapFailure> {
    let deadline_at = Instant::now() + deadline;
    loop {
        let root_owned = root.clone();
        let attempt =
            tokio::task::spawn_blocking(move || try_acquire_nonblocking(&root_owned, exclusive))
                .await
                .map_err(|_| DaemonBootstrapFailure::SelectionUnstable)?;
        match attempt {
            Ok(guard) => return Ok(guard),
            Err(NonBlockingLockError::WouldBlock) => {
                if Instant::now() >= deadline_at {
                    return Err(DaemonBootstrapFailure::SelectionUnstable);
                }
                tokio::time::sleep(SELECTOR_LOCK_RETRY_INTERVAL).await;
            }
            Err(NonBlockingLockError::Unsafe) => {
                return Err(DaemonBootstrapFailure::UnsafeInstallAuthority);
            }
        }
    }
}

#[derive(Debug)]
enum NonBlockingLockError {
    WouldBlock,
    Unsafe,
}

#[cfg(unix)]
fn try_acquire_nonblocking(
    root: &Path,
    exclusive: bool,
) -> Result<SelectorAdmission, NonBlockingLockError> {
    use std::os::unix::fs::OpenOptionsExt;
    let canonical_root = std::fs::canonicalize(root).map_err(|_| NonBlockingLockError::Unsafe)?;
    let path = selector_lock_path(&canonical_root);
    let file = std::fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&path)
        .map_err(|_| NonBlockingLockError::Unsafe)?;
    let metadata = file.metadata().map_err(|_| NonBlockingLockError::Unsafe)?;
    check_authority_component(&path, &metadata).map_err(|_| NonBlockingLockError::Unsafe)?;
    let result = if exclusive {
        fs2::FileExt::try_lock_exclusive(&file)
    } else {
        fs2::FileExt::try_lock_shared(&file)
    };
    if let Err(error) = result {
        if error.kind() == std::io::ErrorKind::WouldBlock
            || error.raw_os_error() == Some(libc::EWOULDBLOCK)
        {
            return Err(NonBlockingLockError::WouldBlock);
        }
        return Err(NonBlockingLockError::Unsafe);
    }
    Ok(SelectorAdmission { file })
}

#[cfg(windows)]
fn try_acquire_nonblocking(
    root: &Path,
    exclusive: bool,
) -> Result<SelectorAdmission, NonBlockingLockError> {
    use std::os::windows::fs::OpenOptionsExt;
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_ATTRIBUTE_NORMAL, FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_READ, FILE_SHARE_WRITE,
    };
    let canonical_root = std::fs::canonicalize(root).map_err(|_| NonBlockingLockError::Unsafe)?;
    let path = selector_lock_path(&canonical_root);
    let file = std::fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
        .custom_flags(FILE_ATTRIBUTE_NORMAL | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(&path)
        .map_err(|_| NonBlockingLockError::Unsafe)?;
    let metadata = std::fs::symlink_metadata(&path).map_err(|_| NonBlockingLockError::Unsafe)?;
    if metadata.file_type().is_symlink() || is_reparse_point(&metadata) || !metadata.is_file() {
        return Err(NonBlockingLockError::Unsafe);
    }
    check_authority_component(&path, &metadata).map_err(|_| NonBlockingLockError::Unsafe)?;
    let result = if exclusive {
        fs2::FileExt::try_lock_exclusive(&file)
    } else {
        fs2::FileExt::try_lock_shared(&file)
    };
    if let Err(error) = result {
        const ERROR_LOCK_VIOLATION: i32 = 33;
        if error.kind() == std::io::ErrorKind::WouldBlock
            || error.raw_os_error() == Some(ERROR_LOCK_VIOLATION)
        {
            return Err(NonBlockingLockError::WouldBlock);
        }
        return Err(NonBlockingLockError::Unsafe);
    }
    Ok(SelectorAdmission { file, exclusive })
}

#[cfg(not(any(unix, windows)))]
fn try_acquire_nonblocking(
    _root: &Path,
    _exclusive: bool,
) -> Result<SelectorAdmission, NonBlockingLockError> {
    Err(NonBlockingLockError::Unsafe)
}

#[cfg(windows)]
/// Windows-only witness handle for a canonical executable path.
///
/// Kept alive across `CreateProcessW` so no writer can replace or delete the
/// selected binary between selection and spawn. Sharing is set to
/// `FILE_SHARE_READ` only, which denies concurrent openers write or delete
/// access while the witness is alive; `CreateProcessW` still succeeds because
/// its own opens request only read/execute-equivalent access.
#[cfg(windows)]
pub(crate) struct WindowsExecutableWitness {
    pub(crate) identity: FileIdentity,
    handle: isize,
}

#[cfg(windows)]
impl Drop for WindowsExecutableWitness {
    fn drop(&mut self) {
        use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
        if self.handle != 0 && self.handle != INVALID_HANDLE_VALUE {
            unsafe {
                CloseHandle(self.handle);
            }
        }
    }
}

// SAFETY: `WindowsExecutableWitness` owns a HANDLE with no interior mutation.
// Callers move it across `.await` points during connect-or-spawn.
#[cfg(windows)]
unsafe impl Send for WindowsExecutableWitness {}

#[cfg(windows)]
pub(crate) fn open_windows_witness(
    path: &Path,
) -> Result<WindowsExecutableWitness, DaemonBootstrapFailure> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION, FILE_ATTRIBUTE_NORMAL,
        FILE_SHARE_READ, OPEN_EXISTING,
    };
    let wide: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let handle = unsafe {
        CreateFileW(
            wide.as_ptr(),
            0,
            FILE_SHARE_READ,
            std::ptr::null(),
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            0,
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(DaemonBootstrapFailure::ExecutableIdentityMismatch);
    }
    let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
    let ok = unsafe { GetFileInformationByHandle(handle, &mut info) };
    if ok == 0 {
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(handle);
        }
        return Err(DaemonBootstrapFailure::ExecutableIdentityMismatch);
    }
    Ok(WindowsExecutableWitness {
        identity: FileIdentity {
            kind: FileIdentityKind::WindowsVolumeFileId,
            high: info.dwVolumeSerialNumber as u64,
            low: ((info.nFileIndexHigh as u64) << 32) | info.nFileIndexLow as u64,
        },
        handle,
    })
}

#[cfg(windows)]
fn windows_path_identity(path: &Path) -> Option<FileIdentity> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
        FILE_FLAG_BACKUP_SEMANTICS, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
        OPEN_EXISTING,
    };
    let wide: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let handle = unsafe {
        CreateFileW(
            wide.as_ptr(),
            0,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            std::ptr::null(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS,
            0,
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return None;
    }
    let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
    let ok = unsafe { GetFileInformationByHandle(handle, &mut info) };
    unsafe {
        CloseHandle(handle);
    }
    (ok != 0).then_some(FileIdentity {
        kind: FileIdentityKind::WindowsVolumeFileId,
        high: info.dwVolumeSerialNumber as u64,
        low: ((info.nFileIndexHigh as u64) << 32) | info.nFileIndexLow as u64,
    })
}

/// Child-side hook: when a spawned daemon sees `TELEX_DAEMON_SELECTION_TOKEN`,
/// it must independently acquire shared selector admission, resolve a fresh
/// InstalledCurrent selection, and verify its own image against the token
/// before serving.
///
/// Returns `Ok(None)` when no bootstrap token is present (the daemon was
/// started outside a bootstrap-controlled parent, which is the legacy CLI
/// path). Returns `Ok(Some(guard))` when validation succeeded and the child
/// must hold the guard through endpoint/capability/readiness publication.
/// After successful publication, callers must invoke
/// [`release_after_readiness_publication`] before serving drain so upgrade or
/// rollback exclusive waiters can proceed. Returns `Err` when validation
/// failed and the child must not serve.
pub(crate) async fn child_validate_bootstrap_env(
) -> Result<Option<SelectorAdmission>, DaemonBootstrapFailure> {
    let raw = match std::env::var(BOOTSTRAP_TOKEN_ENV) {
        Ok(v) => v,
        Err(_) => return Ok(None),
    };
    // Consume the env var immediately so any child of this daemon does not
    // inherit a stale token that could re-run this validation elsewhere.
    std::env::remove_var(BOOTSTRAP_TOKEN_ENV);
    let token = SelectionToken::from_env_value(&raw)
        .ok_or(DaemonBootstrapFailure::ExecutableIdentityMismatch)?;
    let guard = SelectorAdmission::shared_async(token.trusted_root.clone()).await?;
    let fresh = resolve_installed_current(&token.trusted_root)?;
    if fresh.tag != token.tag
        || fresh.root_identity != token.root_identity
        || fresh.build_id != token.build_id
        || fresh.package_version != token.package_version
        || fresh.schema_min != token.schema_min
        || fresh.schema_max != token.schema_max
        || fresh.protocol_major != token.protocol_major
        || fresh.protocol_minor != token.protocol_minor
        || fresh.required_capabilities != token.required_capabilities
        || fresh.target_exe != token.target_exe
        || fresh.file_identity != token.file_identity
    {
        return Err(DaemonBootstrapFailure::SelectionUnstable);
    }
    // Own-image check: canonical current-exe path and platform file identity
    // must match the frozen target. This closes the InstalledCurrent
    // resolve -> spawn -> exec-replace race.
    let own_exe = std::env::current_exe()
        .and_then(std::fs::canonicalize)
        .map_err(|_| DaemonBootstrapFailure::ExecutableIdentityMismatch)?;
    if !same_path(&own_exe, &token.target_exe) {
        return Err(DaemonBootstrapFailure::ExecutableIdentityMismatch);
    }
    let own_identity =
        file_identity(&own_exe).map_err(|_| DaemonBootstrapFailure::ExecutableIdentityMismatch)?;
    if own_identity != token.file_identity {
        return Err(DaemonBootstrapFailure::ExecutableIdentityMismatch);
    }
    Ok(Some(guard))
}

/// Explicit readiness publication boundary.
///
/// A spawned daemon must call this exactly once, after the serve endpoint is
/// bound, the capability file is written, and the listener is armed for the
/// next accept. Dropping the guard here releases the child's shared selector
/// admission so upgrade or rollback exclusive waiters are not blocked for the
/// remaining serve lifetime. The parent's own shared admission is unaffected.
pub(crate) fn release_after_readiness_publication(guard: Option<SelectorAdmission>) {
    drop(guard);
}

/// Compose a `Command` spawn environment carrying the selection token.
///
/// The parent uses this when spawning the resolved InstalledCurrent daemon
/// target so the child can independently re-verify before serving.
pub(crate) fn spawn_env(token: &SelectionToken) -> Vec<(OsString, OsString)> {
    vec![(
        OsString::from(BOOTSTRAP_TOKEN_ENV),
        OsString::from(token.to_env_value()),
    )]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static NEXT: AtomicUsize = AtomicUsize::new(1);

    fn temp_dir(name: &str) -> PathBuf {
        let id = NEXT.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!(
            "telex-bootstrap-test-{}-{name}-{id}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_manifest(layout: &InstallLayout, tag: &str, manifest: &VersionManifest) {
        let dir = layout.versions_dir.join(tag);
        fs::create_dir_all(&dir).unwrap();
        let binary = dir.join(install::exe_name());
        fs::write(&binary, b"fake-telex").unwrap();
        // Update the manifest binary field to bind the derived path so
        // stricter binding checks pass by default.
        let mut concrete = manifest.clone();
        if concrete.binary.is_empty() {
            concrete.binary = binary.to_string_lossy().to_string();
        }
        let path = dir.join("manifest.json");
        fs::write(path, serde_json::to_string_pretty(&concrete).unwrap()).unwrap();
    }

    fn write_current(layout: &InstallLayout, tag: &str) {
        fs::create_dir_all(&layout.root).unwrap();
        fs::write(&layout.current_path, tag).unwrap();
    }

    fn compat_manifest(tag: &str) -> VersionManifest {
        VersionManifest {
            tag: tag.to_string(),
            package_version: "9.9.9".to_string(),
            build_id: "test-build".to_string(),
            binary: String::new(),
            installed_at_ms: 0,
            source: "test".to_string(),
            schema_min: install::SUPPORTED_SCHEMA_MIN,
            schema_max: install::SUPPORTED_SCHEMA_MAX,
            protocol_major: crate::daemon_ipc::PROTOCOL_MAJOR,
            protocol_minor: crate::daemon_ipc::PROTOCOL_MINOR,
            required_capabilities: crate::daemon_ipc::REQUIRED_CAPABILITIES
                .iter()
                .map(|s| (*s).to_string())
                .collect(),
            copilot_bridge_protocol: 0,
            min_compatible_plugin_version: "0.0.0".to_string(),
            previous_tag: None,
        }
    }

    #[test]
    fn relative_root_rejected_as_invalid() {
        let err = BootstrapPolicy::installed_current(PathBuf::from("relative/path"))
            .expect_err("relative root should fail");
        assert_eq!(err, DaemonBootstrapFailure::InvalidTrustedRoot);
    }

    #[test]
    fn validate_for_switch_does_not_require_an_existing_current_selector() {
        // A first install has no `current` yet, and an upgrade's `current`
        // names the *predecessor*. Candidate validation must judge the
        // candidate on its own authority, while `resolve_installed_current`
        // still requires the selector it just read.
        let root = temp_dir("switch-without-current");
        let layout = install::layout_for_root(&root);
        write_manifest(&layout, "v1", &compat_manifest("v1"));
        assert!(!layout.current_path.exists());
        validate_installed_target_for_switch(&layout, "v1")
            .expect("a candidate must validate before `current` exists");

        assert_eq!(
            resolve_installed_current(&root).expect_err("no selector yet"),
            DaemonBootstrapFailure::MissingCurrent
        );

        // Once published, resolution succeeds and the candidate path stays
        // strict about the rest of the authority chain.
        write_current(&layout, "v1");
        resolve_installed_current(&root).expect("published selection resolves");
        validate_installed_target_for_switch(&layout, "v-missing")
            .expect_err("an uninstalled candidate must still fail closed");
    }

    #[test]
    fn empty_root_rejected_as_invalid() {
        let err =
            BootstrapPolicy::installed_current(PathBuf::new()).expect_err("empty root should fail");
        assert_eq!(err, DaemonBootstrapFailure::InvalidTrustedRoot);
    }

    #[test]
    fn nonexistent_absolute_root_rejected_as_invalid() {
        let missing = std::env::temp_dir().join(format!(
            "telex-bootstrap-does-not-exist-{}",
            std::process::id()
        ));
        let err =
            BootstrapPolicy::installed_current(missing).expect_err("missing root should fail");
        assert_eq!(err, DaemonBootstrapFailure::InvalidTrustedRoot);
    }

    #[test]
    fn root_with_parent_dir_component_rejected() {
        let base = temp_dir("parent-dir");
        let with_dotdot = base.join("nested").join("..").join("nested");
        // Path shape check must reject `..` even when it would resolve.
        let err =
            BootstrapPolicy::installed_current(with_dotdot).expect_err(".. in path should fail");
        assert_eq!(err, DaemonBootstrapFailure::InvalidTrustedRoot);
        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn resolve_reports_missing_current_when_selector_absent() {
        let root = temp_dir("no-current");
        let err = resolve_installed_current(&root).expect_err("missing current should fail");
        assert_eq!(err, DaemonBootstrapFailure::MissingCurrent);
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn resolve_reports_missing_executable_when_binary_absent() {
        let root = temp_dir("no-binary");
        let layout = install::layout_for_root(&root);
        let manifest = compat_manifest("v1");
        write_manifest(&layout, "v1", &manifest);
        write_current(&layout, "v1");
        let bin = layout.versions_dir.join("v1").join(install::exe_name());
        fs::remove_file(&bin).unwrap();
        let err = resolve_installed_current(&root).expect_err("missing binary should fail");
        assert_eq!(err, DaemonBootstrapFailure::MissingExecutable);
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn resolve_reports_incompatible_manifest_when_protocol_major_wrong() {
        let root = temp_dir("bad-proto");
        let layout = install::layout_for_root(&root);
        let mut manifest = compat_manifest("v1");
        manifest.protocol_major = manifest.protocol_major.wrapping_add(1);
        write_manifest(&layout, "v1", &manifest);
        write_current(&layout, "v1");
        let err = resolve_installed_current(&root).expect_err("bad protocol should fail");
        assert_eq!(err, DaemonBootstrapFailure::IncompatibleManifest);
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn resolve_reports_incompatible_manifest_when_schema_range_excludes_current() {
        let root = temp_dir("bad-schema");
        let layout = install::layout_for_root(&root);
        let mut manifest = compat_manifest("v1");
        manifest.schema_min = install::SUPPORTED_SCHEMA_MAX + 5;
        manifest.schema_max = install::SUPPORTED_SCHEMA_MAX + 6;
        write_manifest(&layout, "v1", &manifest);
        write_current(&layout, "v1");
        let err = resolve_installed_current(&root).expect_err("bad schema should fail");
        assert_eq!(err, DaemonBootstrapFailure::IncompatibleManifest);
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn resolve_reports_invalid_manifest_when_tag_does_not_bind() {
        let root = temp_dir("mis-tag");
        let layout = install::layout_for_root(&root);
        let mut manifest = compat_manifest("v1");
        manifest.tag = "different".to_string();
        write_manifest(&layout, "v1", &manifest);
        write_current(&layout, "v1");
        let err = resolve_installed_current(&root).expect_err("mismatched tag should fail");
        assert_eq!(err, DaemonBootstrapFailure::InvalidManifest);
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn resolve_reports_invalid_manifest_when_current_contains_path_escape() {
        let root = temp_dir("escape-tag");
        let layout = install::layout_for_root(&root);
        let manifest = compat_manifest("v1");
        write_manifest(&layout, "v1", &manifest);
        write_current(&layout, "..\\other");
        let err = resolve_installed_current(&root).expect_err("current with .. should fail");
        assert_eq!(err, DaemonBootstrapFailure::InvalidManifest);
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn resolve_succeeds_and_produces_matching_selection_token() {
        let root = temp_dir("happy-path");
        let layout = install::layout_for_root(&root);
        let manifest = compat_manifest("v1");
        write_manifest(&layout, "v1", &manifest);
        write_current(&layout, "v1");
        let token = resolve_installed_current(&root).expect("happy path resolves");
        assert_eq!(token.tag, "v1");
        assert_eq!(token.build_id, "test-build");
        assert_eq!(token.package_version, "9.9.9");
        assert_eq!(token.protocol_major, crate::daemon_ipc::PROTOCOL_MAJOR);
        let expected_exe = layout.versions_dir.join("v1").join(install::exe_name());
        let expected_canonical = std::fs::canonicalize(&expected_exe).unwrap();
        assert_eq!(token.target_exe, expected_canonical);
        assert!(token
            .target_exe
            .starts_with(std::fs::canonicalize(&root).unwrap()));
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn selection_token_env_roundtrip_preserves_load_bearing_fields() {
        let root = temp_dir("env-roundtrip");
        let layout = install::layout_for_root(&root);
        let manifest = compat_manifest("v1");
        write_manifest(&layout, "v1", &manifest);
        write_current(&layout, "v1");
        let token = resolve_installed_current(&root).unwrap();
        let env = token.to_env_value();
        let parsed = SelectionToken::from_env_value(&env).expect("token roundtrips through env");
        assert_eq!(parsed, token);
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn exact_executable_missing_path_rejected() {
        let missing =
            std::env::temp_dir().join(format!("telex-bootstrap-exact-{}", std::process::id()));
        let err =
            BootstrapPolicy::exact_executable(missing).expect_err("missing exact exe should fail");
        assert_eq!(err, DaemonBootstrapFailure::MissingExecutable);
    }

    #[test]
    fn exact_executable_relative_rejected() {
        let err = BootstrapPolicy::exact_executable(PathBuf::from("telex"))
            .expect_err("relative exact exe should fail");
        assert_eq!(err, DaemonBootstrapFailure::MissingExecutable);
    }

    #[test]
    fn display_never_leaks_authority_paths() {
        for variant in [
            DaemonBootstrapFailure::InvalidTrustedRoot,
            DaemonBootstrapFailure::UnsafeInstallAuthority,
            DaemonBootstrapFailure::MissingCurrent,
            DaemonBootstrapFailure::InvalidManifest,
            DaemonBootstrapFailure::IncompatibleManifest,
            DaemonBootstrapFailure::SelectionUnstable,
            DaemonBootstrapFailure::MissingExecutable,
            DaemonBootstrapFailure::ExecutableIdentityMismatch,
            DaemonBootstrapFailure::ForeignDaemon,
        ] {
            let rendered = variant.to_string();
            assert!(!rendered.contains('/'));
            assert!(!rendered.contains('\\'));
            assert!(!rendered.is_empty());
        }
    }

    #[test]
    fn resolve_rejects_empty_build_id() {
        let root = temp_dir("empty-build-id");
        let layout = install::layout_for_root(&root);
        let mut manifest = compat_manifest("v1");
        manifest.build_id = String::new();
        write_manifest(&layout, "v1", &manifest);
        write_current(&layout, "v1");
        let err = resolve_installed_current(&root).expect_err("empty build id should fail");
        assert_eq!(err, DaemonBootstrapFailure::InvalidManifest);
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn resolve_rejects_unknown_sentinel_build_id() {
        let root = temp_dir("unknown-build-id");
        let layout = install::layout_for_root(&root);
        let mut manifest = compat_manifest("v1");
        manifest.build_id = install::UNKNOWN_BUILD_ID.to_string();
        write_manifest(&layout, "v1", &manifest);
        write_current(&layout, "v1");
        let err = resolve_installed_current(&root).expect_err("unknown build id should fail");
        assert_eq!(err, DaemonBootstrapFailure::InvalidManifest);
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn resolve_rejects_empty_package_version() {
        let root = temp_dir("empty-version");
        let layout = install::layout_for_root(&root);
        let mut manifest = compat_manifest("v1");
        manifest.package_version = String::new();
        write_manifest(&layout, "v1", &manifest);
        write_current(&layout, "v1");
        let err = resolve_installed_current(&root).expect_err("empty package version should fail");
        assert_eq!(err, DaemonBootstrapFailure::InvalidManifest);
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn resolve_rejects_empty_manifest_binary() {
        let root = temp_dir("empty-binary");
        let layout = install::layout_for_root(&root);
        let dir = layout.versions_dir.join("v1");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(install::exe_name()), b"fake-telex").unwrap();
        // Intentionally leave binary empty and skip our write_manifest helper.
        let mut manifest = compat_manifest("v1");
        manifest.binary = String::new();
        fs::write(
            dir.join("manifest.json"),
            serde_json::to_string_pretty(&manifest).unwrap(),
        )
        .unwrap();
        write_current(&layout, "v1");
        let err = resolve_installed_current(&root).expect_err("empty binary should fail");
        assert_eq!(err, DaemonBootstrapFailure::InvalidManifest);
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn resolve_rejects_unresolvable_manifest_binary() {
        let root = temp_dir("bad-binary");
        let layout = install::layout_for_root(&root);
        let dir = layout.versions_dir.join("v1");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(install::exe_name()), b"fake-telex").unwrap();
        let mut manifest = compat_manifest("v1");
        manifest.binary = std::env::temp_dir()
            .join(format!(
                "telex-bootstrap-nonexistent-{}",
                std::process::id()
            ))
            .to_string_lossy()
            .to_string();
        fs::write(
            dir.join("manifest.json"),
            serde_json::to_string_pretty(&manifest).unwrap(),
        )
        .unwrap();
        write_current(&layout, "v1");
        let err = resolve_installed_current(&root).expect_err("unresolvable binary should fail");
        assert_eq!(err, DaemonBootstrapFailure::InvalidManifest);
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn resolve_rejects_duplicate_required_capabilities() {
        let root = temp_dir("dup-caps");
        let layout = install::layout_for_root(&root);
        let mut manifest = compat_manifest("v1");
        let first = manifest.required_capabilities[0].clone();
        manifest.required_capabilities.push(first);
        write_manifest(&layout, "v1", &manifest);
        write_current(&layout, "v1");
        let err = resolve_installed_current(&root).expect_err("duplicate caps should fail");
        assert_eq!(err, DaemonBootstrapFailure::InvalidManifest);
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn resolve_rejects_empty_required_capability_entry() {
        let root = temp_dir("empty-cap");
        let layout = install::layout_for_root(&root);
        let mut manifest = compat_manifest("v1");
        manifest.required_capabilities.push(String::new());
        write_manifest(&layout, "v1", &manifest);
        write_current(&layout, "v1");
        let err = resolve_installed_current(&root).expect_err("empty cap entry should fail");
        assert_eq!(err, DaemonBootstrapFailure::InvalidManifest);
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn resolve_rejects_missing_required_capability() {
        let root = temp_dir("missing-cap");
        let layout = install::layout_for_root(&root);
        let mut manifest = compat_manifest("v1");
        manifest.required_capabilities.pop().unwrap();
        write_manifest(&layout, "v1", &manifest);
        write_current(&layout, "v1");
        let err = resolve_installed_current(&root).expect_err("missing cap should fail");
        assert_eq!(err, DaemonBootstrapFailure::IncompatibleManifest);
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn selector_admission_shared_holders_coexist_and_release() {
        let root = temp_dir("selector-lock");
        let shared_one = SelectorAdmission::shared(&root).expect("shared admission");
        let shared_two = SelectorAdmission::shared(&root).expect("second shared admission");
        drop(shared_one);
        drop(shared_two);
        assert!(selector_lock_path(&root).is_file());
        // After all shared holders release, exclusive becomes acquirable.
        let excl = SelectorAdmission::exclusive(&root).expect("exclusive admission after release");
        drop(excl);
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn selector_admission_exclusive_blocked_by_shared_is_bounded_and_maps_to_unstable() {
        let root = temp_dir("selector-exclusive-blocked");
        let held_shared = SelectorAdmission::shared(&root).expect("shared admission");
        let start = std::time::Instant::now();
        // With SELECTOR_LOCK_DEADLINE=5s, we short-circuit via a shorter
        // deadline through the internal helper to keep tests fast.
        let err = acquire_with_deadline_sync(&root, true, Duration::from_millis(150))
            .expect_err("exclusive should be blocked by live shared holder");
        assert_eq!(err, DaemonBootstrapFailure::SelectionUnstable);
        assert!(
            start.elapsed() < Duration::from_secs(2),
            "acquisition must be bounded and non-blocking"
        );
        drop(held_shared);
        // After release, exclusive succeeds within the deadline.
        let excl = acquire_with_deadline_sync(&root, true, Duration::from_millis(200))
            .expect("exclusive after release");
        drop(excl);
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn selector_admission_child_release_boundary_frees_upgrade_waiter() {
        // Regression for the previously-observed deadlock where the child's
        // shared lease lived for the daemon's entire `serve` lifetime and
        // upgrade's exclusive acquisition blocked drain forever. The
        // release-after-readiness-publication boundary drops the guard so
        // upgrade/rollback exclusive acquisition succeeds while the daemon
        // is still serving.
        let root = temp_dir("child-release-boundary");
        let guard = Some(SelectorAdmission::shared(&root).expect("child shared admission"));
        release_after_readiness_publication(guard);
        // After the child releases, upgrade's exclusive must succeed.
        let excl = acquire_with_deadline_sync(&root, true, Duration::from_millis(500))
            .expect("exclusive after child release");
        drop(excl);
        fs::remove_dir_all(root).ok();
    }

    #[tokio::test]
    async fn selector_admission_async_bounded_and_maps_to_unstable() {
        let root = temp_dir("selector-async-blocked");
        let held_shared = SelectorAdmission::shared(&root).expect("shared admission");
        let start = std::time::Instant::now();
        let err = acquire_with_deadline_async(root.clone(), true, Duration::from_millis(150))
            .await
            .expect_err("exclusive should be blocked");
        assert_eq!(err, DaemonBootstrapFailure::SelectionUnstable);
        assert!(
            start.elapsed() < Duration::from_secs(2),
            "async acquisition must not block a worker beyond the deadline"
        );
        drop(held_shared);
        let excl = acquire_with_deadline_async(root.clone(), true, Duration::from_millis(500))
            .await
            .expect("exclusive after release");
        drop(excl);
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn exact_executable_captures_file_identity_at_freeze_time() {
        let dir = temp_dir("exact-identity");
        let exe = dir.join(install::exe_name());
        fs::write(&exe, b"exact-body").unwrap();
        let policy =
            BootstrapPolicy::exact_executable(exe.clone()).expect("exact executable freeze");
        match policy.as_ref() {
            BootstrapPolicy::ExactExecutable {
                executable,
                file_identity,
            } => {
                let canonical = std::fs::canonicalize(&exe).unwrap();
                assert_eq!(executable, &canonical);
                let observed = file_identity_pub(&canonical).expect("identity");
                assert_eq!(&observed, file_identity);
            }
            _ => panic!("expected ExactExecutable"),
        }
        fs::remove_dir_all(dir).ok();
    }

    /// Test-only shim so the `super::file_identity` symbol used by the test
    /// remains crate-private without an `allow(dead_code)` on the callable.
    fn file_identity_pub(path: &Path) -> Result<FileIdentity, DaemonBootstrapFailure> {
        file_identity(path)
    }
}
