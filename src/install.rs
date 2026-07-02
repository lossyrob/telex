//! Versioned install layout, stable launcher dispatch, and local upgrade helpers.

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

pub const LAUNCHER_GUARD_ENV: &str = "TELEX_LAUNCHER_ACTIVE";
pub const INSTALL_ROOT_ENV: &str = "TELEX_INSTALL_ROOT";
pub const SUPPORTED_SCHEMA_MIN: i64 = 2;
pub const SUPPORTED_SCHEMA_MAX: i64 = 2;

#[derive(Debug, Clone, Serialize)]
pub struct InstallLayout {
    pub root: PathBuf,
    pub bin_dir: PathBuf,
    pub versions_dir: PathBuf,
    pub current_path: PathBuf,
    pub previous_path: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionManifest {
    pub tag: String,
    pub package_version: String,
    pub binary: String,
    pub installed_at_ms: i64,
    pub source: String,
    pub schema_min: i64,
    pub schema_max: i64,
    pub protocol_major: u16,
    pub protocol_minor: u16,
    pub required_capabilities: Vec<String>,
    pub copilot_bridge_protocol: u32,
    pub min_compatible_plugin_version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_tag: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct VersionInfo {
    pub package_version: &'static str,
    pub current_exe: String,
    pub launcher_guard_env: &'static str,
    pub supported_schema_min: i64,
    pub supported_schema_max: i64,
    pub install: InstallInfo,
}

#[derive(Debug, Clone, Serialize)]
pub struct InstallInfo {
    pub root: String,
    pub bin: String,
    pub current_tag: Option<String>,
    pub previous_tag: Option<String>,
    pub active_tag: Option<String>,
    pub current_binary: Option<String>,
    pub current_manifest: Option<VersionManifest>,
    pub layout_detected: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct InstallResult {
    pub installed: String,
    pub binary: String,
    pub launcher: String,
    pub root: String,
    pub current_tag: Option<String>,
    pub previous_tag: Option<String>,
    pub switched: bool,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SwitchResult {
    pub switched_to: String,
    pub previous_tag: Option<String>,
    pub current_binary: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct GcEntry {
    pub tag: String,
    pub path: String,
    pub action: &'static str,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct GcReport {
    pub root: String,
    pub dry_run: bool,
    pub force: bool,
    pub entries: Vec<GcEntry>,
}

pub fn exe_name() -> &'static str {
    if cfg!(windows) {
        "telex.exe"
    } else {
        "telex"
    }
}

pub fn default_install_root() -> Result<PathBuf> {
    if let Some(root) = std::env::var_os(INSTALL_ROOT_ENV) {
        return Ok(PathBuf::from(root));
    }
    #[cfg(windows)]
    {
        let base = std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .or_else(dirs::data_local_dir)
            .ok_or_else(|| anyhow!("cannot resolve LOCALAPPDATA for telex install root"))?;
        Ok(base.join("telex"))
    }
    #[cfg(not(windows))]
    {
        let home = dirs::home_dir().ok_or_else(|| anyhow!("could not determine home directory"))?;
        Ok(home.join(".local").join("share").join("telex"))
    }
}

pub fn layout_for_root(root: impl Into<PathBuf>) -> InstallLayout {
    let root = root.into();
    InstallLayout {
        bin_dir: root.join("bin"),
        versions_dir: root.join("versions"),
        current_path: root.join("current"),
        previous_path: root.join("previous"),
        root,
    }
}

pub fn current_layout() -> Result<InstallLayout> {
    let exe = std::env::current_exe().context("resolving current executable")?;
    Ok(layout_for_root(
        infer_install_root_from_exe(&exe).unwrap_or(default_install_root()?),
    ))
}

pub fn layout_from_optional_root(root: Option<PathBuf>) -> Result<InstallLayout> {
    Ok(layout_for_root(match root {
        Some(root) => root,
        None => current_layout()?.root,
    }))
}

pub fn infer_install_root_from_exe(exe: &Path) -> Option<PathBuf> {
    let parent = exe.parent()?;
    if parent.file_name().and_then(|n| n.to_str()) == Some("bin") {
        return parent.parent().map(Path::to_path_buf);
    }
    let tag_dir = parent;
    let versions = tag_dir.parent()?;
    if versions.file_name().and_then(|n| n.to_str()) == Some("versions") {
        return versions.parent().map(Path::to_path_buf);
    }
    None
}

pub fn maybe_dispatch_launcher() -> Result<Option<i32>> {
    if std::env::var_os(LAUNCHER_GUARD_ENV).is_some() {
        return Ok(None);
    }
    let exe = std::env::current_exe().context("resolving current executable")?;
    let Some(root) = infer_install_root_from_exe(&exe) else {
        return Ok(None);
    };
    let parent_is_bin = exe
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        == Some("bin");
    if !parent_is_bin {
        return Ok(None);
    }
    let layout = layout_for_root(root);
    let Some(target) = current_binary(&layout)? else {
        return Ok(None);
    };
    let exe_canon = std::fs::canonicalize(&exe).unwrap_or(exe);
    let target_canon = std::fs::canonicalize(&target).unwrap_or(target.clone());
    if exe_canon == target_canon {
        return Ok(None);
    }

    let status = Command::new(&target)
        .args(std::env::args_os().skip(1))
        .env(LAUNCHER_GUARD_ENV, "1")
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .with_context(|| format!("launching selected telex binary {}", target.display()))?;
    Ok(Some(status.code().unwrap_or(1)))
}

pub fn version_info(root: Option<PathBuf>) -> Result<VersionInfo> {
    let exe = std::env::current_exe().context("resolving current executable")?;
    let layout = layout_from_optional_root(root)?;
    let current_tag = read_tag_file(&layout.current_path)?;
    let previous_tag = read_tag_file(&layout.previous_path)?;
    let active_tag = active_tag_from_exe(&exe);
    let current_binary = current_binary(&layout)?;
    let current_manifest = current_tag
        .as_deref()
        .and_then(|tag| read_manifest(&layout, tag).ok());
    Ok(VersionInfo {
        package_version: env!("CARGO_PKG_VERSION"),
        current_exe: exe.to_string_lossy().into_owned(),
        launcher_guard_env: LAUNCHER_GUARD_ENV,
        supported_schema_min: SUPPORTED_SCHEMA_MIN,
        supported_schema_max: SUPPORTED_SCHEMA_MAX,
        install: InstallInfo {
            root: layout.root.to_string_lossy().into_owned(),
            bin: layout
                .bin_dir
                .join(exe_name())
                .to_string_lossy()
                .into_owned(),
            current_tag,
            previous_tag,
            active_tag,
            current_binary: current_binary.map(|p| p.to_string_lossy().into_owned()),
            current_manifest,
            layout_detected: layout.current_path.exists() && layout.versions_dir.exists(),
        },
    })
}

pub fn install_binary(
    layout: &InstallLayout,
    tag: &str,
    source_binary: &Path,
    source_label: &str,
    switch_current: bool,
) -> Result<InstallResult> {
    validate_tag(tag)?;
    if !source_binary.is_file() {
        bail!("upgrade source is not a file: {}", source_binary.display());
    }
    let previous_tag = read_tag_file(&layout.current_path)?;
    let version_dir = layout.versions_dir.join(tag);
    let version_binary = version_dir.join(exe_name());
    std::fs::create_dir_all(&version_dir)
        .with_context(|| format!("creating version dir {}", version_dir.display()))?;
    copy_if_different(source_binary, &version_binary)
        .with_context(|| format!("installing binary into {}", version_binary.display()))?;

    let mut warnings = Vec::new();
    let launcher = layout.bin_dir.join(exe_name());
    std::fs::create_dir_all(&layout.bin_dir)
        .with_context(|| format!("creating bin dir {}", layout.bin_dir.display()))?;
    if !launcher.exists() {
        copy_if_different(source_binary, &launcher)
            .with_context(|| format!("creating launcher {}", launcher.display()))?;
    } else if std::fs::canonicalize(&launcher).ok() != std::fs::canonicalize(source_binary).ok() {
        if let Err(e) = copy_if_different(source_binary, &launcher) {
            warnings.push(format!(
                "could not refresh launcher {} ({e}); existing PATH binary may predate launcher mode, so rerun the installer after old processes exit",
                launcher.display()
            ));
        }
    }

    let manifest = current_manifest(tag, &version_binary, source_label, previous_tag.clone());
    write_manifest(layout, tag, &manifest)?;
    if switch_current {
        switch_to(layout, tag)?;
    }

    Ok(InstallResult {
        installed: tag.to_string(),
        binary: version_binary.to_string_lossy().into_owned(),
        launcher: launcher.to_string_lossy().into_owned(),
        root: layout.root.to_string_lossy().into_owned(),
        current_tag: read_tag_file(&layout.current_path)?,
        previous_tag,
        switched: switch_current,
        warnings,
    })
}

pub fn switch_to(layout: &InstallLayout, tag: &str) -> Result<SwitchResult> {
    validate_tag(tag)?;
    let manifest = read_manifest(layout, tag)?;
    validate_manifest_for_current(&manifest)?;
    let binary = layout.versions_dir.join(tag).join(exe_name());
    if !binary.is_file() {
        bail!(
            "installed version {tag} is missing binary {}",
            binary.display()
        );
    }
    let previous_tag = read_tag_file(&layout.current_path)?;
    if let Some(previous) = previous_tag.as_deref().filter(|p| *p != tag) {
        atomic_write(&layout.previous_path, previous)?;
    }
    atomic_write(&layout.current_path, tag)?;
    Ok(SwitchResult {
        switched_to: tag.to_string(),
        previous_tag,
        current_binary: binary.to_string_lossy().into_owned(),
    })
}

pub fn read_manifest(layout: &InstallLayout, tag: &str) -> Result<VersionManifest> {
    validate_tag(tag)?;
    let path = layout.versions_dir.join(tag).join("manifest.json");
    let raw = std::fs::read_to_string(&path)
        .with_context(|| format!("reading manifest {}", path.display()))?;
    serde_json::from_str(&raw).with_context(|| format!("parsing manifest {}", path.display()))
}

pub fn validate_manifest_for_current(manifest: &VersionManifest) -> Result<()> {
    if manifest.protocol_major != crate::daemon_ipc::PROTOCOL_MAJOR {
        bail!(
            "version {} uses protocol major {}, current binary requires {}",
            manifest.tag,
            manifest.protocol_major,
            crate::daemon_ipc::PROTOCOL_MAJOR
        );
    }
    if !(manifest.schema_min..=manifest.schema_max).contains(&SUPPORTED_SCHEMA_MAX) {
        bail!(
            "version {} supports schema {}..{}, current store schema is {}",
            manifest.tag,
            manifest.schema_min,
            manifest.schema_max,
            SUPPORTED_SCHEMA_MAX
        );
    }
    Ok(())
}

pub fn gc(layout: &InstallLayout, dry_run: bool, force: bool) -> Result<GcReport> {
    let current = read_tag_file(&layout.current_path)?;
    let previous = read_tag_file(&layout.previous_path)?;
    let active = std::env::current_exe()
        .ok()
        .and_then(|p| active_tag_from_exe(&p));
    let protected: BTreeSet<String> = [current.clone(), previous.clone(), active.clone()]
        .into_iter()
        .flatten()
        .collect();
    let mut entries = Vec::new();
    if !layout.versions_dir.exists() {
        return Ok(GcReport {
            root: layout.root.to_string_lossy().into_owned(),
            dry_run,
            force,
            entries,
        });
    }
    for entry in std::fs::read_dir(&layout.versions_dir)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let tag = entry.file_name().to_string_lossy().into_owned();
        let path = entry.path();
        if protected.contains(&tag) {
            entries.push(GcEntry {
                tag,
                path: path.to_string_lossy().into_owned(),
                action: "keep",
                reason: "current, previous, or active process version".to_string(),
            });
            continue;
        }
        if dry_run {
            entries.push(GcEntry {
                tag,
                path: path.to_string_lossy().into_owned(),
                action: "would_remove",
                reason: "stale installed version".to_string(),
            });
            continue;
        }
        match std::fs::remove_dir_all(&path) {
            Ok(()) => entries.push(GcEntry {
                tag,
                path: path.to_string_lossy().into_owned(),
                action: "removed",
                reason: "stale installed version".to_string(),
            }),
            Err(e) if !force => entries.push(GcEntry {
                tag,
                path: path.to_string_lossy().into_owned(),
                action: "keep",
                reason: format!("remove failed, treating as possibly in use: {e}"),
            }),
            Err(e) => entries.push(GcEntry {
                tag,
                path: path.to_string_lossy().into_owned(),
                action: "keep",
                reason: format!("forced removal still refused by OS/filesystem: {e}"),
            }),
        }
    }
    Ok(GcReport {
        root: layout.root.to_string_lossy().into_owned(),
        dry_run,
        force,
        entries,
    })
}

pub fn active_tag_from_exe(exe: &Path) -> Option<String> {
    let tag_dir = exe.parent()?;
    let versions = tag_dir.parent()?;
    (versions.file_name().and_then(|n| n.to_str()) == Some("versions"))
        .then(|| tag_dir.file_name()?.to_str().map(str::to_string))
        .flatten()
}

pub fn current_binary(layout: &InstallLayout) -> Result<Option<PathBuf>> {
    let Some(tag) = read_tag_file(&layout.current_path)? else {
        return Ok(None);
    };
    Ok(Some(layout.versions_dir.join(tag).join(exe_name())))
}

fn current_manifest(
    tag: &str,
    binary: &Path,
    source: &str,
    previous_tag: Option<String>,
) -> VersionManifest {
    VersionManifest {
        tag: tag.to_string(),
        package_version: env!("CARGO_PKG_VERSION").to_string(),
        binary: binary.to_string_lossy().into_owned(),
        installed_at_ms: crate::model::now_ms(),
        source: source.to_string(),
        schema_min: SUPPORTED_SCHEMA_MIN,
        schema_max: SUPPORTED_SCHEMA_MAX,
        protocol_major: crate::daemon_ipc::PROTOCOL_MAJOR,
        protocol_minor: crate::daemon_ipc::PROTOCOL_MINOR,
        required_capabilities: crate::daemon_ipc::REQUIRED_CAPABILITIES
            .iter()
            .map(|cap| (*cap).to_string())
            .collect(),
        copilot_bridge_protocol: crate::commands::copilot::COPILOT_BRIDGE_PROTOCOL,
        min_compatible_plugin_version: crate::commands::copilot::MIN_COMPATIBLE_PLUGIN_VERSION
            .to_string(),
        previous_tag,
    }
}

fn write_manifest(layout: &InstallLayout, tag: &str, manifest: &VersionManifest) -> Result<()> {
    let path = layout.versions_dir.join(tag).join("manifest.json");
    let raw = serde_json::to_string_pretty(manifest)?;
    atomic_write(&path, &raw)
}

fn read_tag_file(path: &Path) -> Result<Option<String>> {
    match std::fs::read_to_string(path) {
        Ok(raw) => {
            let tag = raw.trim();
            Ok((!tag.is_empty()).then(|| tag.to_string()))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
    }
}

fn atomic_write(path: &Path, value: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension(format!("{}.tmp", std::process::id()));
    std::fs::write(&tmp, value)?;
    match std::fs::rename(&tmp, path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            std::fs::remove_file(path)?;
            std::fs::rename(&tmp, path)?;
            Ok(())
        }
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(e.into())
        }
    }
}

fn copy_if_different(source: &Path, dest: &Path) -> Result<()> {
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if std::fs::canonicalize(source).ok() == std::fs::canonicalize(dest).ok() {
        return Ok(());
    }
    std::fs::copy(source, dest)?;
    Ok(())
}

fn validate_tag(tag: &str) -> Result<()> {
    if tag.trim().is_empty()
        || tag.contains(['/', '\\'])
        || tag == "."
        || tag == ".."
        || tag.contains("..")
    {
        bail!("invalid version tag {tag:?}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static NEXT: AtomicUsize = AtomicUsize::new(1);

    fn temp_root(name: &str) -> PathBuf {
        let id = NEXT.fetch_add(1, Ordering::SeqCst);
        std::env::temp_dir().join(format!(
            "telex-install-test-{}-{name}-{id}",
            std::process::id()
        ))
    }

    fn source_binary(root: &Path, name: &str) -> PathBuf {
        let path = root.join(name).join(exe_name());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"fake-telex").unwrap();
        path
    }

    #[test]
    fn install_switch_and_gc_preserve_current_and_previous() {
        let root = temp_root("switch-gc");
        let layout = layout_for_root(&root);
        let src1 = source_binary(&root, "src1");
        let src2 = source_binary(&root, "src2");

        install_binary(&layout, "v1", &src1, "test", true).unwrap();
        assert_eq!(
            std::fs::read_to_string(&layout.current_path)
                .unwrap()
                .trim(),
            "v1"
        );
        install_binary(&layout, "v2", &src2, "test", true).unwrap();
        assert_eq!(
            std::fs::read_to_string(&layout.current_path)
                .unwrap()
                .trim(),
            "v2"
        );
        assert_eq!(
            std::fs::read_to_string(&layout.previous_path)
                .unwrap()
                .trim(),
            "v1"
        );

        let report = gc(&layout, true, false).unwrap();
        let kept = report
            .entries
            .iter()
            .filter(|e| e.action == "keep")
            .map(|e| e.tag.as_str())
            .collect::<BTreeSet<_>>();
        assert!(kept.contains("v1"));
        assert!(kept.contains("v2"));

        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn switch_rejects_manifest_with_wrong_protocol_major() {
        let root = temp_root("bad-protocol");
        let layout = layout_for_root(&root);
        let src = source_binary(&root, "src");
        install_binary(&layout, "v1", &src, "test", false).unwrap();
        let manifest_path = layout.versions_dir.join("v1").join("manifest.json");
        let mut manifest = read_manifest(&layout, "v1").unwrap();
        manifest.protocol_major = manifest.protocol_major.saturating_add(1);
        atomic_write(
            &manifest_path,
            &serde_json::to_string_pretty(&manifest).unwrap(),
        )
        .unwrap();

        let err = switch_to(&layout, "v1").unwrap_err();
        assert!(err.to_string().contains("protocol major"));

        std::fs::remove_dir_all(root).ok();
    }
}
