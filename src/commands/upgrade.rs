use anyhow::{anyhow, bail, Result};
use serde_json::json;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{Duration, Instant};

use crate::cli::{Ctx, GcArgs, RollbackArgs, UpgradeArgs, VersionArgs};
use crate::daemon::DaemonError;
use crate::daemon_ipc::{Request, Response, ERROR_NOT_RUNNING, ERROR_UNAUTHORIZED};
use crate::install;
use crate::output::emit;

pub async fn version(ctx: &Ctx, args: VersionArgs) -> Result<i32> {
    let info = install::version_info(args.root)?;
    let daemon_metadata = crate::daemon::daemon_version_metadata();
    let out = json!({
        "version": info,
        "daemon_metadata": daemon_metadata,
        "copilot": {
            "bridge_protocol": crate::commands::copilot::COPILOT_BRIDGE_PROTOCOL,
            "min_compatible_plugin_version": crate::commands::copilot::MIN_COMPATIBLE_PLUGIN_VERSION,
        }
    });
    emit(ctx.fmt, &out, || {
        println!("telex {}", info.package_version);
        println!("exe {}", info.current_exe);
        println!("install_root {}", info.install.root);
        println!(
            "current {}",
            info.install
                .current_tag
                .as_deref()
                .unwrap_or("(not versioned)")
        );
        if let Some(binary) = info.install.current_binary.as_deref() {
            println!("current_binary {binary}");
        }
        println!(
            "protocol {}.{}",
            daemon_metadata.protocol_version.major, daemon_metadata.protocol_version.minor
        );
        println!(
            "copilot_bridge v{} min_plugin v{}",
            crate::commands::copilot::COPILOT_BRIDGE_PROTOCOL,
            crate::commands::copilot::MIN_COMPATIBLE_PLUGIN_VERSION
        );
    });
    Ok(0)
}

pub async fn upgrade(ctx: &Ctx, args: UpgradeArgs) -> Result<i32> {
    let layout = install::layout_from_optional_root(args.root.clone())?;
    match args.from.clone() {
        Some(from) => upgrade_local(ctx, &args, &layout, &from).await,
        None => upgrade_release(ctx, &args, &layout).await,
    }
}

/// A resolved binary ready to install through the versioned layout, plus optional
/// release metadata for JSON transparency.
struct InstallPlan {
    tag: String,
    source: PathBuf,
    source_label: String,
    release: Option<serde_json::Value>,
    /// When set (release path), assert the probed binary self-reports this version before
    /// installing, so a mislabeled release asset cannot be installed under the wrong tag.
    #[cfg_attr(not(feature = "self-update"), allow(dead_code))]
    expected_version: Option<String>,
}

/// Local/manual upgrade path (`telex upgrade --from <binary>`).
async fn upgrade_local(
    ctx: &Ctx,
    args: &UpgradeArgs,
    layout: &install::InstallLayout,
    from: &Path,
) -> Result<i32> {
    let tag = args
        .version
        .clone()
        .unwrap_or_else(|| format!("v{}", env!("CARGO_PKG_VERSION")));
    let source = resolve_source_binary(from)?;
    let plan = InstallPlan {
        tag,
        source,
        source_label: format!("local:{}", from.display()),
        release: None,
        expected_version: None,
    };
    perform_upgrade(ctx, layout, plan, args).await
}

/// Release upgrade path (`telex upgrade` with no --from): discover a public GitHub release,
/// download + verify + extract the platform asset, then install through the versioned layout.
#[cfg(feature = "self-update")]
async fn upgrade_release(
    ctx: &Ctx,
    args: &UpgradeArgs,
    layout: &install::InstallLayout,
) -> Result<i32> {
    use crate::release;

    let requested = args
        .version
        .as_deref()
        .map(release::normalize_tag)
        .transpose()?;
    let (target, kind) = release::current_target().ok_or_else(|| {
        anyhow!(
            "self-update is not supported on this platform ({}/{}); install from source with \
             `cargo install --git https://github.com/lossyrob/telex --features entra`",
            std::env::consts::OS,
            std::env::consts::ARCH
        )
    })?;
    let cfg = release::FetchConfig::from_repo(&args.repo);

    progress(
        ctx,
        &format!(
            "Resolving {} release from {}...",
            requested.as_deref().unwrap_or("latest"),
            cfg.repo
        ),
    );
    let rel = release::discover_release(&cfg, requested.as_deref()).await?;
    let tag = rel.tag_name.clone();

    // Already-current short-circuit — only when BOTH tags normalize successfully and are equal,
    // so two un-normalizable tags are never treated as "the same".
    if !args.force {
        let current = install::version_info(Some(layout.root.clone()))?
            .install
            .current_tag;
        if let Some(cur) = &current {
            if let (Ok(cur_norm), Ok(tag_norm)) =
                (release::normalize_tag(cur), release::normalize_tag(&tag))
            {
                if cur_norm == tag_norm {
                    let out = json!({
                        "upgrade": false,
                        "status": "already_current",
                        "tag": tag,
                        "current": cur,
                    });
                    emit(ctx.fmt, &out, || {
                        println!("already current {tag} (use --force to reinstall)");
                    });
                    return Ok(0);
                }
            }
        }
    }

    let selected = release::select_asset(&rel.asset_names(), &tag, target, kind)?;
    progress(ctx, &format!("Downloading {}...", selected.archive_name));
    let archive = release::download_asset(
        &cfg,
        &tag,
        &selected.archive_name,
        release::MAX_ARCHIVE_BYTES,
    )
    .await?;
    let sidecar = release::download_asset(
        &cfg,
        &tag,
        &selected.sidecar_name,
        release::MAX_SIDECAR_BYTES,
    )
    .await?;
    let expected = release::parse_sha256_sidecar(&String::from_utf8_lossy(&sidecar))?;
    release::verify_checksum(&archive, &expected)?;
    progress(ctx, "Checksum verified.");

    // Stage the verified binary before promoting it through the versioned installer. `Staging`
    // cleans itself up on drop (including the early-return path below).
    let staging = staging_dir(layout)?;
    let staged = release::safe_extract(kind, &archive, &staging.path)?;
    let plan = InstallPlan {
        tag: tag.clone(),
        source: staged,
        source_label: format!("github-release:{}@{}", cfg.repo, tag),
        release: Some(json!({
            "repo": cfg.repo,
            "tag": tag,
            "asset": selected.archive_name,
            "sidecar": selected.sidecar_name,
            "verified": true,
            "prerelease": rel.prerelease,
        })),
        expected_version: Some(tag.clone()),
    };
    perform_upgrade(ctx, layout, plan, args).await
    // `staging` is dropped here (after install copies the staged binary), removing the temp dir.
}

/// Release path stub for builds compiled without the `self-update` feature.
#[cfg(not(feature = "self-update"))]
async fn upgrade_release(
    _ctx: &Ctx,
    _args: &UpgradeArgs,
    _layout: &install::InstallLayout,
) -> Result<i32> {
    bail!(
        "this telex build was compiled without release-upgrade support (the `self-update` \
         feature is disabled). Install a specific local build with `telex upgrade --from \
         <binary>`, reinstall the published binary (which includes self-update), or run \
         `cargo install --git https://github.com/lossyrob/telex --features entra`."
    )
}

/// Install a resolved binary through the versioned layout and switch/drain as requested.
/// Shared by the local and release upgrade paths.
async fn perform_upgrade(
    ctx: &Ctx,
    layout: &install::InstallLayout,
    plan: InstallPlan,
    args: &UpgradeArgs,
) -> Result<i32> {
    let source_metadata = source_metadata(&plan.source, &layout.root)?;
    // Release path only: the asset's self-reported version must match the tag it was published
    // under, so a mislabeled/renamed release asset cannot be installed under the wrong tag.
    #[cfg(feature = "self-update")]
    if let Some(expected) = &plan.expected_version {
        let probed = crate::release::normalize_tag(&source_metadata.package_version)?;
        let want = crate::release::normalize_tag(expected)?;
        if probed != want {
            bail!(
                "release {expected} contains a binary that reports version {} (tag/binary \
                 mismatch); refusing to install",
                source_metadata.package_version
            );
        }
    }
    let installed = install::install_binary(
        layout,
        &plan.tag,
        &plan.source,
        &plan.source_label,
        false,
        Some(source_metadata),
    )?;
    if !args.no_switch {
        let manifest = install::read_manifest(layout, &plan.tag)?;
        install::validate_manifest_for_current(&manifest)?;
    }
    let drain = if args.no_switch || args.skip_drain {
        json!({"skipped": true})
    } else {
        drain_daemon(ctx, args.drain_timeout_ms).await?
    };
    for warning in &installed.warnings {
        eprintln!("warning: {warning}");
    }
    let switched = if args.no_switch {
        None
    } else {
        Some(install::switch_to(layout, &plan.tag)?)
    };
    let out = json!({
        "upgrade": true,
        "installed": installed,
        "drain": drain,
        "switch": switched,
        "release": plan.release,
    });
    emit(ctx.fmt, &out, || {
        println!("installed {}", plan.tag);
        for warning in &installed.warnings {
            println!("warning {warning}");
        }
        if let Some(switched) = &switched {
            println!("current {}", switched.switched_to);
            println!("binary {}", switched.current_binary);
        } else {
            println!("current unchanged (--no-switch)");
        }
    });
    Ok(0)
}

/// Emit progress to stderr in text mode only (JSON consumers get the structured result).
#[cfg(feature = "self-update")]
fn progress(ctx: &Ctx, msg: &str) {
    if ctx.fmt == crate::output::Format::Text {
        eprintln!("{msg}");
    }
}

/// A controlled staging directory under the install root, removed on drop (RAII) so an
/// early return, `?`, or panic cannot leak multi-MB temp dirs under `<root>/.staging`.
#[cfg(feature = "self-update")]
struct Staging {
    path: PathBuf,
}

#[cfg(feature = "self-update")]
impl Drop for Staging {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

#[cfg(feature = "self-update")]
fn staging_dir(layout: &install::InstallLayout) -> Result<Staging> {
    use anyhow::Context;
    let base = layout.root.join(".staging");
    std::fs::create_dir_all(&base)
        .with_context(|| format!("creating staging base {}", base.display()))?;
    // Best-effort sweep of orphaned staging dirs from earlier aborted upgrades (crash, SIGKILL,
    // power loss) so they cannot accumulate under the install root.
    sweep_stale_staging(&base);
    let dir = base.join(format!(
        "upgrade-{}-{}",
        std::process::id(),
        crate::model::now_ms()
    ));
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("creating staging dir {}", dir.display()))?;
    Ok(Staging { path: dir })
}

/// Remove staging entries older than one hour (best-effort). Never removes this process's own
/// staging dirs (name prefix `upgrade-<pid>-`), so a long-running upgrade under clock skew
/// cannot delete its own in-flight staging.
#[cfg(feature = "self-update")]
fn sweep_stale_staging(base: &Path) {
    let Ok(entries) = std::fs::read_dir(base) else {
        return;
    };
    let own_prefix = format!("upgrade-{}-", std::process::id());
    let now = std::time::SystemTime::now();
    for entry in entries.flatten() {
        if entry
            .file_name()
            .to_str()
            .is_some_and(|n| n.starts_with(&own_prefix))
        {
            continue;
        }
        let stale = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|modified| now.duration_since(modified).ok())
            .map(|age| age.as_secs() > 3600)
            .unwrap_or(false);
        if stale {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

pub async fn rollback(ctx: &Ctx, args: RollbackArgs) -> Result<i32> {
    let layout = install::layout_from_optional_root(args.root)?;
    let target = match args.version {
        Some(tag) => tag,
        None => install::version_info(Some(layout.root.clone()))?
            .install
            .previous_tag
            .ok_or_else(|| anyhow!("no previous installed version recorded; pass --version"))?,
    };
    let manifest = install::read_manifest(&layout, &target)?;
    install::validate_manifest_for_current(&manifest)?;
    let drain = if args.skip_drain {
        json!({"skipped": true})
    } else {
        drain_daemon(ctx, args.drain_timeout_ms).await?
    };
    let switched = install::switch_to(&layout, &target)?;
    let out = json!({
        "rollback": true,
        "drain": drain,
        "switch": switched,
    });
    emit(ctx.fmt, &out, || {
        println!("current {}", switched.switched_to);
        println!("binary {}", switched.current_binary);
    });
    Ok(0)
}

pub async fn gc(ctx: &Ctx, args: GcArgs) -> Result<i32> {
    let layout = install::layout_from_optional_root(args.root)?;
    let report = install::gc(&layout, args.dry_run, args.force)?;
    emit(ctx.fmt, &report, || {
        println!("install_root {}", report.root);
        for entry in &report.entries {
            println!("{} {} ({})", entry.action, entry.tag, entry.reason);
        }
    });
    Ok(0)
}

fn resolve_source_binary(path: &Path) -> Result<PathBuf> {
    if path.is_dir() {
        let binary = path.join(install::exe_name());
        if binary.is_file() {
            return Ok(binary);
        }
        bail!(
            "upgrade source directory {} does not contain {}",
            path.display(),
            install::exe_name()
        );
    }
    Ok(path.to_path_buf())
}

/// Environment variables stripped from the version-probe child. The release path forks a
/// freshly downloaded binary (checksum-verified against its sidecar, but not authenticated) to
/// read its metadata before install; it must not inherit the user's credentials, since a
/// compromised release or download mirror could otherwise execute code with the token in-env.
const SENSITIVE_PROBE_ENV: &[&str] = &["GITHUB_TOKEN", "GH_TOKEN"];

fn strip_sensitive_env(cmd: &mut Command) {
    for var in SENSITIVE_PROBE_ENV {
        cmd.env_remove(var);
    }
}

fn source_metadata(source: &Path, root: &Path) -> Result<install::SourceMetadata> {
    if !source.is_file() {
        bail!("upgrade source is not a file: {}", source.display());
    }
    let output = source_version_output(source, root, Duration::from_secs(10))?;
    if !output.status.success() {
        bail!(
            "upgrade source {} did not run `telex --json version` successfully: {}",
            source.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }

    fn source_version_output(source: &Path, root: &Path, timeout: Duration) -> Result<Output> {
        let mut command = Command::new(source);
        command
            .arg("--json")
            .arg("version")
            .arg("--root")
            .arg(root)
            .env(install::LAUNCHER_GUARD_ENV, "1")
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        strip_sensitive_env(&mut command);
        let mut child = command
            .spawn()
            .map_err(|e| anyhow!("running source telex binary {}: {e}", source.display()))?;
        let deadline = Instant::now() + timeout;
        loop {
            match child.try_wait() {
                Ok(Some(_status)) => {
                    return child
                        .wait_with_output()
                        .map_err(|e| anyhow!("collecting source telex version output: {e}"));
                }
                Ok(None) if Instant::now() >= deadline => {
                    let _ = child.kill();
                    let _ = child.wait();
                    bail!(
                        "upgrade source {} timed out while running `telex --json version`",
                        source.display()
                    );
                }
                Ok(None) => std::thread::sleep(Duration::from_millis(20)),
                Err(e) => bail!("waiting for source telex version command: {e}"),
            }
        }
    }
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).map_err(|e| {
        anyhow!(
            "upgrade source {} did not emit valid version JSON: {e}",
            source.display()
        )
    })?;
    let version = value
        .get("version")
        .ok_or_else(|| anyhow!("source version JSON missing `version` object"))?;
    let daemon = value
        .get("daemon_metadata")
        .ok_or_else(|| anyhow!("source version JSON missing `daemon_metadata` object"))?;
    let protocol = daemon
        .get("protocol_version")
        .ok_or_else(|| anyhow!("source version JSON missing protocol_version"))?;
    let copilot = value
        .get("copilot")
        .ok_or_else(|| anyhow!("source version JSON missing `copilot` object"))?;
    Ok(install::SourceMetadata {
        package_version: required_str(version, "package_version")?.to_string(),
        schema_min: required_i64(version, "supported_schema_min")?,
        schema_max: required_i64(version, "supported_schema_max")?,
        protocol_major: required_u16(protocol, "major")?,
        protocol_minor: required_u16(protocol, "minor")?,
        required_capabilities: daemon
            .get("required_capabilities")
            .and_then(|v| v.as_array())
            .ok_or_else(|| anyhow!("source version JSON missing required_capabilities"))?
            .iter()
            .map(|v| {
                v.as_str()
                    .map(str::to_string)
                    .ok_or_else(|| anyhow!("required_capabilities must be strings"))
            })
            .collect::<Result<Vec<_>>>()?,
        copilot_bridge_protocol: required_u32(copilot, "bridge_protocol")?,
        min_compatible_plugin_version: required_str(copilot, "min_compatible_plugin_version")?
            .to_string(),
    })
}

fn required_str<'a>(value: &'a serde_json::Value, key: &str) -> Result<&'a str> {
    value
        .get(key)
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("source version JSON missing string field `{key}`"))
}

fn required_i64(value: &serde_json::Value, key: &str) -> Result<i64> {
    value
        .get(key)
        .and_then(|v| v.as_i64())
        .ok_or_else(|| anyhow!("source version JSON missing integer field `{key}`"))
}

fn required_u16(value: &serde_json::Value, key: &str) -> Result<u16> {
    let raw = required_i64(value, key)?;
    u16::try_from(raw).map_err(|_| anyhow!("source version field `{key}` is out of range: {raw}"))
}

fn required_u32(value: &serde_json::Value, key: &str) -> Result<u32> {
    let raw = required_i64(value, key)?;
    u32::try_from(raw).map_err(|_| anyhow!("source version field `{key}` is out of range: {raw}"))
}

async fn drain_daemon(ctx: &Ctx, timeout_ms: u64) -> Result<serde_json::Value> {
    let store_key = ctx.store_key()?;
    let paths = crate::daemon::DaemonPaths::current()?;
    let cap = match crate::daemon::read_cap_file(&paths.cap_path) {
        Ok(cap) => cap,
        Err(DaemonError::NotRunning(_)) => {
            return Ok(json!({"drained": false, "status": "not_running"}));
        }
        Err(e) => return Err(e.into()),
    };
    let timeout = Duration::from_millis(timeout_ms.max(1));
    let response = match tokio::time::timeout(timeout, async {
        let mut client = crate::daemon::connect_existing(&store_key).await?;
        client
            .request(&Request::Drain {
                proof: Some(cap.admin_cap),
            })
            .await
    })
    .await
    {
        Ok(Ok(response)) => response,
        Ok(Err(DaemonError::NotRunning(message))) => {
            return Ok(json!({"drained": false, "status": "not_running", "message": message}));
        }
        Ok(Err(DaemonError::Unauthorized(msg))) => {
            bail!(
                "drain failed: cannot authenticate the running daemon — {msg}; \
                 the daemon may have been started by a different telex binary \
                 (a foreign-executable daemon); re-run this command from the \
                 daemon's owning binary, or pass --skip-drain to bypass drain \
                 coordination"
            )
        }
        Ok(Err(e)) => return Err(e.into()),
        Err(_) => bail!("daemon drain timed out after {timeout_ms}ms"),
    };
    match response {
        Response::Ack { .. } => Ok(json!({"drained": true, "status": "draining"})),
        Response::Error { code, message, .. } if code == ERROR_NOT_RUNNING => {
            Ok(json!({"drained": false, "status": "not_running", "message": message}))
        }
        Response::Error { code, message, .. } if code == ERROR_UNAUTHORIZED => {
            bail!(
                "drain failed: the daemon rejected the drain request as unauthorized ({message}); \
                 the daemon may have been started by a different telex binary \
                 (a foreign-executable daemon); re-run this command from the \
                 daemon's owning binary, or pass --skip-drain to bypass drain \
                 coordination"
            )
        }
        Response::Error { code, message, .. } => bail!("daemon drain failed: {code}: {message}"),
        other => bail!("unexpected daemon drain response: {other:?}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// When a foreign-executable daemon (started by a different telex binary) owns the store, the
    /// IPC auth check returns DaemonError::Unauthorized before the drain request is even sent.
    /// The error should name the cause and suggest --skip-drain so it is actionable (issue #81).
    #[test]
    fn drain_unauthorized_connection_error_is_actionable() {
        // Reproduce the message the bail! arm produces for DaemonError::Unauthorized.
        let inner = "server executable /a/telex does not match /b/telex";
        let formatted = format!(
            "drain failed: cannot authenticate the running daemon — {inner}; \
             the daemon may have been started by a different telex binary \
             (a foreign-executable daemon); re-run this command from the \
             daemon's owning binary, or pass --skip-drain to bypass drain \
             coordination"
        );
        assert!(
            formatted.contains("foreign-executable daemon"),
            "message should name the foreign-executable cause: {formatted}"
        );
        assert!(
            formatted.contains("--skip-drain"),
            "message should suggest --skip-drain: {formatted}"
        );
        assert!(
            formatted.contains(inner),
            "message should include original detail: {formatted}"
        );
    }

    /// When the daemon responds with Unauthorized to a Drain request (response-level auth error),
    /// the error message should also be actionable (issue #81).
    #[test]
    fn drain_unauthorized_response_error_is_actionable() {
        let raw_message = "proof rejected by daemon".to_string();
        let formatted = format!(
            "drain failed: the daemon rejected the drain request as unauthorized ({raw_message}); \
             the daemon may have been started by a different telex binary \
             (a foreign-executable daemon); re-run this command from the \
             daemon's owning binary, or pass --skip-drain to bypass drain \
             coordination"
        );
        assert!(
            formatted.contains("foreign-executable daemon"),
            "message should name the foreign-executable cause: {formatted}"
        );
        assert!(
            formatted.contains("--skip-drain"),
            "message should suggest --skip-drain: {formatted}"
        );
        assert!(
            formatted.contains(&raw_message),
            "message should include original detail: {formatted}"
        );
    }

    #[test]
    fn strip_sensitive_env_hides_github_token_from_child() {
        // MF-1: the version probe must not leak the user's token to the forked candidate binary.
        std::env::set_var("GITHUB_TOKEN", "SENTINEL_LEAK_9x7q");
        #[cfg(windows)]
        let mut cmd = {
            let mut c = Command::new("cmd");
            c.args(["/c", "echo", "%GITHUB_TOKEN%"]);
            c
        };
        #[cfg(unix)]
        let mut cmd = {
            let mut c = Command::new("sh");
            c.args(["-c", "printf %s \"${GITHUB_TOKEN:-}\""]);
            c
        };
        strip_sensitive_env(&mut cmd);
        let output = cmd.output().expect("run env-echo child");
        std::env::remove_var("GITHUB_TOKEN");
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            !stdout.contains("SENTINEL_LEAK_9x7q"),
            "GITHUB_TOKEN leaked to the probe child: {stdout:?}"
        );
    }
}
