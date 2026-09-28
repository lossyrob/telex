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
        println!("build {}", info.build_id);
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
    // Post-switch successor (ADR 0052 decision 14c): spawn the daemon this switch just installed
    // and wait, bounded, for a reconcile pass, so an idle attached session regains push without
    // the user running anything. Skipped when nothing was drained or nothing is recoverable.
    let reconcile = match &switched {
        Some(switched) => {
            verify_successor_reconcile(ctx, &drain, Path::new(&switched.current_binary)).await
        }
        None => json!({
            "attempted": false,
            "successor_binary": serde_json::Value::Null,
            "reason": "no switch performed",
        }),
    };
    let out = json!({
        "upgrade": true,
        "installed": installed,
        "drain": drain,
        "switch": switched,
        "release": plan.release,
        "station_intent_reconcile": reconcile,
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
        print_station_intent_summary(&drain, &reconcile);
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
    // Rollback gets the same pre-flight report as upgrade, plus an explicit warning: a target
    // binary that predates station-intent reconciliation cannot restore these intents, and the
    // documented consequence is a return to manual `telex copilot resume`. Intents are never
    // deleted by a rollback — an older daemon simply ignores a directory it does not know, the
    // singleton-hash namespacing plus the schema range keep it inert with respect to them, and GC
    // deliberately never removes a manifest it cannot read because of its schema version.
    //
    // The successor is the binary the rollback just selected, invoked as a child. Calling
    // `connect_or_spawn` here would have spawned the *new* binary this rollback is moving away
    // from, resurrecting exactly what the operator asked to roll back.
    let reconcile =
        verify_successor_reconcile(ctx, &drain, Path::new(&switched.current_binary)).await;
    let out = json!({
        "rollback": true,
        "drain": drain,
        "switch": switched,
        "station_intent_reconcile": reconcile,
    });
    emit(ctx.fmt, &out, || {
        println!("current {}", switched.switched_to);
        println!("binary {}", switched.current_binary);
        print_station_intent_summary(&drain, &reconcile);
        if recoverable_intent_count(&drain).unwrap_or(0) > 0 {
            println!(
                "station intents  WARNING: if {} predates station-intent reconciliation it cannot restore these bindings; \
                 run `telex --address <station> copilot resume` per station after rolling back",
                switched.switched_to
            );
        }
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
    parse_source_metadata(&value)
}

fn parse_source_metadata(value: &serde_json::Value) -> Result<install::SourceMetadata> {
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
        build_id: version
            .get("build_id")
            .and_then(|value| value.as_str())
            .filter(|value| !value.trim().is_empty())
            .unwrap_or(install::UNKNOWN_BUILD_ID)
            .to_string(),
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

fn unauthorized_drain_message(message: &str, response_rejected: bool) -> String {
    let reason = if response_rejected {
        format!("the daemon rejected the drain request as unauthorized ({message})")
    } else {
        format!("cannot authenticate the running daemon — {message}")
    };
    format!(
        "drain failed: {reason}; \
         the daemon may have been started by a different telex binary \
         (a foreign-executable daemon); re-run this command from the \
         daemon's owning binary, or pass --skip-drain to bypass drain \
         coordination"
    )
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
            bail!(unauthorized_drain_message(&msg, false))
        }
        Ok(Err(e)) => return Err(e.into()),
        Err(_) => bail!("daemon drain timed out after {timeout_ms}ms"),
    };
    match response {
        Response::Ack { drain_intents, .. } => Ok(json!({
            "drained": true,
            "status": "draining",
            // The pre-drain station-intent signal (issue #106), carried through so `upgrade` and
            // `rollback` can report — and, for upgrade, verify — what a successor must restore.
            // `null` means the drained daemon predates station-intent reporting.
            "station_intents": drain_intents,
        })),
        Response::Error { code, message, .. } if code == ERROR_NOT_RUNNING => {
            Ok(json!({"drained": false, "status": "not_running", "message": message}))
        }
        Response::Error { code, message, .. } if code == ERROR_UNAUTHORIZED => {
            bail!(unauthorized_drain_message(&message, true))
        }
        Response::Error { code, message, .. } => bail!("daemon drain failed: {code}: {message}"),
        other => bail!("unexpected daemon drain response: {other:?}"),
    }
}

/// How long a post-switch successor is given to report a completed reconciliation pass.
///
/// Derived from the published graceful bound plus the successor's own spawn/readiness time, so it
/// is generous enough not to be flaky and still bounded — `upgrade`/`rollback` must never hang.
const SUCCESSOR_RECONCILE_TIMEOUT: Duration = Duration::from_secs(30);

/// Wall-clock bound on the successor *process*: its own reconcile bound plus spawn/exit slack.
const SUCCESSOR_WAIT_TIMEOUT: Duration =
    Duration::from_secs(SUCCESSOR_RECONCILE_TIMEOUT.as_secs() + 10);

/// How long a killed (or already exited) successor gets to close its pipes before we give up on
/// its diagnostics. Never unbounded: a grandchild that inherited a pipe can hold it open forever.
const SUCCESSOR_PIPE_DRAIN_GRACE: Duration = Duration::from_secs(2);

/// How long we wait to *reap* a successor we just killed, so the child never outlives this
/// process as a zombie (Unix) or an unclosed handle (Windows).
const SUCCESSOR_REAP_GRACE: Duration = Duration::from_secs(5);

/// Byte caps on what we capture from the successor's pipes. Follows the watcher's
/// `MAX_STDOUT_BYTES` / `MAX_STDERR_BYTES` split: stdout carries a JSON report, stderr a message.
const SUCCESSOR_STDOUT_CAP: usize = 64 * 1024;
const SUCCESSOR_STDERR_CAP: usize = 16 * 1024;

/// Character cap on any successor-provided text that reaches the result JSON.
const SUCCESSOR_DIAGNOSTIC_CHARS: usize = 512;

/// Number of recoverable intents at drain time, or `None` when the daemon did not report any.
fn recoverable_intent_count(drain: &serde_json::Value) -> Option<u64> {
    drain.get("station_intents")?.get("recoverable")?.as_u64()
}

/// Spawn the successor daemon the switch just installed and wait, bounded, for one reconcile pass.
///
/// This is a deliberate, bounded extension of "only `attach` auto-spawns" (ADR 0028), recorded in
/// ADR 0052: it is what makes the issue's motivating scenario — `telex upgrade` with an idle
/// Copilot session — recover without the user typing anything.
///
/// It runs the pass by **invoking the newly selected binary**, not by calling
/// `connect_or_spawn` in this process. Two reasons, both load-bearing:
///
/// * `connect_or_spawn` spawns `current_exe()`, which during an upgrade is the *pre-switch*
///   binary (the launcher execs `versions/<current>/telex`). That left the old binary running as
///   the daemon, and because `connect_existing` requires the server executable to match the
///   client's, every subsequent client — all of which are the new binary — got `Unauthorized`
///   from a daemon that has no idle shutdown. `telex daemon stop` failed the same way.
/// * The same executable-match rule means this process cannot request a pass from a
///   correctly-spawned successor either. The child does both, and its own
///   `telex daemon reconcile` retries a draining predecessor and a pass that did not run.
///
/// It never fails the upgrade: a successor that cannot be reached is reported, not fatal, because
/// the binary is already switched and the next client operation will spawn one anyway.
async fn verify_successor_reconcile(
    ctx: &Ctx,
    drain: &serde_json::Value,
    successor_binary: &Path,
) -> serde_json::Value {
    let Some(recoverable) = recoverable_intent_count(drain) else {
        return json!({
            "attempted": false,
            "successor_binary": successor_binary.to_string_lossy(),
            "reason": "no station-intent report from the drained daemon",
        });
    };
    if recoverable == 0 {
        return json!({
            "attempted": false,
            "successor_binary": successor_binary.to_string_lossy(),
            "recoverable_at_drain": 0,
            "reason": "no recoverable station intents",
        });
    }
    if !successor_binary.is_file() {
        return json!({
            "attempted": false,
            "successor_binary": successor_binary.to_string_lossy(),
            "recoverable_at_drain": recoverable,
            "reason": format!("successor binary {} is missing", successor_binary.display()),
        });
    }
    let mut command = tokio::process::Command::new(successor_binary);
    command
        .arg("--json")
        .arg("daemon")
        .arg("reconcile")
        .arg("--timeout-ms")
        .arg(SUCCESSOR_RECONCILE_TIMEOUT.as_millis().to_string())
        // The successor binary lives under `versions/`, not `bin/`, so it would not re-dispatch
        // anyway; the guard makes that explicit rather than incidental.
        .env(crate::install::LAUNCHER_GUARD_ENV, "1");
    if let Some(db) = ctx.cfg.db_override.as_deref() {
        command.arg("--db").arg(db);
    }
    if let Some(backend) = ctx.cfg.backend_selector.as_deref() {
        command.arg("--backend").arg(backend);
    }
    run_successor_reconcile(
        command,
        successor_binary,
        recoverable,
        SUCCESSOR_WAIT_TIMEOUT,
    )
    .await
}

/// Run one successor `daemon reconcile` child to completion (or to a kill), bounded end to end.
///
/// Three properties this owes the caller, none of which `Command::output()` under a timeout gives:
///
/// * **No pipe deadlock.** Both pipes are drained concurrently in their own tasks and keep reading
///   past the capture cap, so a child that fills one stream while we read the other cannot wedge.
/// * **No direct-child survivor.** A child that overruns the bound is explicitly killed *and
///   reaped*: dropping a `tokio` child neither signals nor collects it. A daemon the child
///   successfully spawned is a separate detached service and remains governed by daemon lifecycle.
/// * **A result on every path.** Every branch reports `successor_binary`, so a consumer can always
///   tell which binary the report is about.
async fn run_successor_reconcile(
    mut command: tokio::process::Command,
    successor_binary: &Path,
    recoverable: u64,
    wait_timeout: Duration,
) -> serde_json::Value {
    let binary = successor_binary.to_string_lossy().to_string();
    command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(e) => {
            return json!({
                "attempted": true,
                "successor_binary": binary,
                "recoverable_at_drain": recoverable,
                "error": format!("spawning the successor: {e}"),
            })
        }
    };
    let pid = child.id();
    let stdout_capture = child
        .stdout
        .take()
        .map(|pipe| tokio::spawn(drain_capped(pipe, SUCCESSOR_STDOUT_CAP)));
    let stderr_capture = child
        .stderr
        .take()
        .map(|pipe| tokio::spawn(drain_capped(pipe, SUCCESSOR_STDERR_CAP)));
    let status = match tokio::time::timeout(wait_timeout, child.wait()).await {
        Ok(Ok(status)) => status,
        Ok(Err(e)) => {
            // A failed `wait` leaves the child's fate unknown, so it gets the same kill-and-reap
            // treatment as an overrun rather than being abandoned.
            let terminated = child.start_kill().is_ok();
            let reaped = matches!(
                tokio::time::timeout(SUCCESSOR_REAP_GRACE, child.wait()).await,
                Ok(Ok(_))
            );
            let stderr = collect_capture(stderr_capture).await;
            let _ = collect_capture(stdout_capture).await;
            return json!({
                "attempted": true,
                "successor_binary": binary,
                "recoverable_at_drain": recoverable,
                "terminated": terminated,
                "reaped": reaped,
                "successor_pid": pid,
                "stderr": bounded_diagnostic(&stderr),
                "error": format!("waiting for the successor: {e}"),
            });
        }
        Err(_) => {
            let terminated = child.start_kill().is_ok();
            let reaped = matches!(
                tokio::time::timeout(SUCCESSOR_REAP_GRACE, child.wait()).await,
                Ok(Ok(_))
            );
            let stderr = collect_capture(stderr_capture).await;
            let _ = collect_capture(stdout_capture).await;
            return json!({
                "attempted": true,
                "successor_binary": binary,
                "recoverable_at_drain": recoverable,
                "timed_out": true,
                "terminated": terminated,
                "reaped": reaped,
                "successor_pid": pid,
                "stderr": bounded_diagnostic(&stderr),
                "error": format!(
                    "successor did not report a reconcile pass within {}",
                    render_bound(wait_timeout)
                ),
            });
        }
    };
    let stdout = collect_capture(stdout_capture).await;
    let stderr = collect_capture(stderr_capture).await;
    successor_reconcile_result(
        recoverable,
        status.success(),
        status.code(),
        &stdout,
        &stderr,
        successor_binary,
    )
}

/// Drain a child pipe to EOF, keeping at most `cap` bytes.
///
/// It keeps reading after the cap instead of stopping: the point is that the child never blocks on
/// a full pipe, which is exactly what a capped-then-abandoned reader would cause.
async fn drain_capped<R: tokio::io::AsyncRead + Unpin>(mut reader: R, cap: usize) -> Vec<u8> {
    use tokio::io::AsyncReadExt;
    let mut captured = Vec::new();
    let mut buffer = [0u8; 8192];
    loop {
        match reader.read(&mut buffer).await {
            Ok(0) | Err(_) => return captured,
            Ok(count) => {
                if captured.len() < cap {
                    let take = count.min(cap - captured.len());
                    captured.extend_from_slice(&buffer[..take]);
                }
            }
        }
    }
}

/// Collect a pipe-drain task, bounded. A reader still blocked after the grace is aborted rather
/// than awaited: a grandchild holding the inherited pipe open must not extend `upgrade`.
async fn collect_capture(capture: Option<tokio::task::JoinHandle<Vec<u8>>>) -> Vec<u8> {
    let Some(mut capture) = capture else {
        return Vec::new();
    };
    match tokio::time::timeout(SUCCESSOR_PIPE_DRAIN_GRACE, &mut capture).await {
        Ok(Ok(bytes)) => bytes,
        Ok(Err(_)) => Vec::new(),
        Err(_) => {
            capture.abort();
            Vec::new()
        }
    }
}

/// Trimmed, character-bounded rendering of captured child output for a diagnostic JSON field.
fn bounded_diagnostic(bytes: &[u8]) -> String {
    bounded_text(&String::from_utf8_lossy(bytes))
}

fn bounded_text(text: &str) -> String {
    text.trim()
        .chars()
        .take(SUCCESSOR_DIAGNOSTIC_CHARS)
        .collect()
}

/// Render a bound for the operator-facing message: whole seconds normally, milliseconds for a
/// sub-second bound, which would otherwise read as "within 0s".
fn render_bound(bound: Duration) -> String {
    if bound.subsec_millis() == 0 {
        format!("{}s", bound.as_secs())
    } else {
        format!("{}ms", bound.as_millis())
    }
}

/// Classify one finished successor run into the `station_intent_reconcile` result.
///
/// stdout is parsed **before** the exit status is consulted. `telex daemon reconcile` exits
/// non-zero precisely when it reports `reconciled: false` — a structured, actionable answer
/// ("no reconcile pass completed before the timeout", "the daemon did not answer...") — so keying
/// on the status first threw away the only diagnosis the successor produced and replaced it with a
/// generic "rejected" plus a stderr tail the daemon never writes.
///
/// Every branch carries `successor_binary`, and every non-zero exit still carries `exit_code` and
/// `min_daemon_minor`, so existing consumers see nothing removed.
fn successor_reconcile_result(
    recoverable: u64,
    success: bool,
    exit_code: Option<i32>,
    stdout: &[u8],
    stderr: &[u8],
    successor_binary: &Path,
) -> serde_json::Value {
    let binary = successor_binary.to_string_lossy().to_string();
    let stderr_tail = bounded_diagnostic(stderr);
    let min_daemon_minor = crate::daemon_reconcile::RECONCILE_MIN_DAEMON_MINOR;
    let parsed: serde_json::Value = match serde_json::from_slice(stdout) {
        Ok(parsed) => parsed,
        Err(e) => {
            // No structured report to preserve. A non-zero exit with unparseable stdout is the
            // "successor is too old to understand `daemon reconcile`" shape (a clap usage error),
            // which is what `min_daemon_minor` is a hint for.
            let mut result = json!({
                "attempted": true,
                "successor_binary": binary,
                "recoverable_at_drain": recoverable,
                "exit_code": exit_code,
                "stderr": stderr_tail,
                "stdout": bounded_diagnostic(stdout),
                "error": if success {
                    format!("successor reconcile output was not JSON ({e})")
                } else {
                    "the successor rejected `daemon reconcile`".to_string()
                },
            });
            if !success {
                result["min_daemon_minor"] = json!(min_daemon_minor);
            }
            return result;
        }
    };
    if parsed
        .get("reconciled")
        .and_then(serde_json::Value::as_bool)
        != Some(true)
    {
        let mut result = json!({
            "attempted": true,
            "successor_binary": binary,
            "recoverable_at_drain": recoverable,
            "exit_code": exit_code,
            "reconciled": false,
            "stderr": stderr_tail,
            "error": parsed
                .get("error")
                .and_then(serde_json::Value::as_str)
                .map(bounded_text)
                .unwrap_or_else(|| "successor reported no completed reconcile pass".to_string()),
        });
        if let Some(store_key) = parsed.get("store_key").and_then(serde_json::Value::as_str) {
            result["store_key"] = json!(bounded_text(store_key));
        }
        if !success {
            result["min_daemon_minor"] = json!(min_daemon_minor);
        }
        return result;
    }
    let report = parsed.get("report").cloned().unwrap_or(json!({}));
    let count = |key: &str| {
        report
            .get(key)
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0)
    };
    json!({
        "attempted": true,
        "reconciled": true,
        "recoverable_at_drain": recoverable,
        "exit_code": exit_code,
        "restored": count("restored"),
        "refreshed_no_op": count("refreshed_no_op"),
        "deferred_lease": count("deferred_lease"),
        "failed": count("failed"),
        "pass_seq": count("pass_seq"),
        "successor_binary": binary,
    })
}

/// Render the station-intent part of an upgrade/rollback result in text mode.
fn print_station_intent_summary(drain: &serde_json::Value, reconcile: &serde_json::Value) {
    match drain.get("station_intents") {
        Some(serde_json::Value::Object(report)) => {
            let get = |key: &str| {
                report
                    .get(key)
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(0)
            };
            println!(
                "station intents  recoverable {} pending {} degraded {} incompatible {} unknown {}",
                get("recoverable"),
                get("pending"),
                get("degraded"),
                get("incompatible"),
                get("unknown")
            );
            if get("pending") > 0 {
                println!(
                    "station intents  {} pending intent(s) are not finalized; a successor cannot restore them until the next Copilot turn boundary",
                    get("pending")
                );
            }
            if get("degraded") + get("incompatible") > 0 {
                println!(
                    "station intents  {} intent(s) need `telex --address <station> copilot resume` after this switch",
                    get("degraded") + get("incompatible")
                );
            }
        }
        _ => println!("station intents  unavailable (no report from the drained daemon)"),
    }
    if reconcile
        .get("attempted")
        .and_then(serde_json::Value::as_bool)
        == Some(true)
    {
        match reconcile.get("error").and_then(serde_json::Value::as_str) {
            Some(error) => println!("station intents  successor reconcile incomplete: {error}"),
            None => println!(
                "station intents  successor restored {} / deferred {} / failed {}",
                reconcile
                    .get("restored")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(0),
                reconcile
                    .get("deferred_lease")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(0),
                reconcile
                    .get("failed")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(0)
            ),
        }
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
        let inner = "server executable /a/telex does not match /b/telex";
        let formatted = unauthorized_drain_message(inner, false);
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
        let formatted = unauthorized_drain_message(&raw_message, true);
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

    #[test]
    fn source_metadata_intentionally_maps_legacy_missing_build_id_to_unknown() {
        let value = serde_json::json!({
            "version": {
                "package_version": "0.1.0",
                "supported_schema_min": 2,
                "supported_schema_max": 2
            },
            "daemon_metadata": {
                "protocol_version": {
                    "major": 1,
                    "minor": 0
                },
                "required_capabilities": ["send"]
            },
            "copilot": {
                "bridge_protocol": 1,
                "min_compatible_plugin_version": "0.1.0"
            }
        });

        // An older candidate cannot report a field it predates. Preserve that
        // first-hop uncertainty explicitly rather than inventing build identity.
        let metadata = parse_source_metadata(&value).unwrap();
        assert_eq!(metadata.build_id, install::UNKNOWN_BUILD_ID);

        let mut current = value;
        current["version"]["build_id"] = serde_json::json!("candidate-build");
        let metadata = parse_source_metadata(&current).unwrap();
        assert_eq!(metadata.build_id, "candidate-build");
    }

    #[test]
    fn successor_reconcile_reports_rejection_with_bounded_stderr() {
        let stderr = format!("unsupported argument {}", "x".repeat(700));
        let result = successor_reconcile_result(
            2,
            false,
            Some(2),
            b"",
            stderr.as_bytes(),
            Path::new("old-telex"),
        );
        assert_eq!(result["attempted"], true);
        assert_eq!(result["recoverable_at_drain"], 2);
        assert_eq!(result["exit_code"], 2);
        assert_eq!(result["error"], "the successor rejected `daemon reconcile`");
        assert!(
            result["stderr"].as_str().unwrap().chars().count() <= 512,
            "stderr must stay bounded in JSON and text output"
        );
        assert_eq!(
            result["min_daemon_minor"],
            crate::daemon_reconcile::RECONCILE_MIN_DAEMON_MINOR
        );
    }

    #[test]
    fn successor_reconcile_preserves_non_json_diagnostics() {
        let result = successor_reconcile_result(
            1,
            true,
            Some(0),
            b"not json",
            b"current successor could not open the intent scope",
            Path::new("current-telex"),
        );
        assert_eq!(result["attempted"], true);
        assert!(result["error"]
            .as_str()
            .unwrap()
            .contains("output was not JSON"));
        assert_eq!(
            result["stderr"],
            "current successor could not open the intent scope"
        );
    }

    fn test_ctx() -> Ctx {
        Ctx {
            cfg: crate::config::Config {
                backend_selector: None,
                db_override: None,
                default_address: None,
                liveness_window_secs: 30,
            },
            fmt: crate::output::Format::Json,
            address: None,
        }
    }

    /// A command that outlives any bound this test gives it, with no grandchildren to orphan.
    fn long_sleep_command() -> tokio::process::Command {
        #[cfg(windows)]
        {
            let mut command = tokio::process::Command::new("powershell");
            command.args(["-NoProfile", "-Command", "Start-Sleep -Seconds 120"]);
            command
        }
        #[cfg(unix)]
        {
            let mut command = tokio::process::Command::new("sh");
            command.args(["-c", "sleep 120"]);
            command
        }
    }

    #[cfg(unix)]
    fn process_is_running(pid: u32) -> bool {
        // The child has already been reaped, so ESRCH here means "gone", not "not ours".
        unsafe { libc::kill(pid as libc::pid_t, 0) == 0 }
    }

    #[cfg(windows)]
    fn process_is_running(pid: u32) -> bool {
        // Reuse the daemon's own liveness probe, which is deliberately conservative: an ambiguous
        // failure reports *alive*, so this assertion cannot pass by accident.
        crate::session_watch::process_alive(pid)
    }

    /// `telex daemon reconcile` exits non-zero for exactly the case it explains best: a pass that
    /// did not run. Keying on the exit status first discarded that structured answer, so an
    /// operator saw "the successor rejected `daemon reconcile`" for a successor that had in fact
    /// answered in full.
    #[test]
    fn successor_reconcile_preserves_a_structured_report_from_a_nonzero_exit() {
        let stdout = br#"{"reconciled":false,"store_key":"scope-abc","error":"no reconcile pass completed before the timeout"}"#;
        let result = successor_reconcile_result(
            3,
            false,
            Some(1),
            stdout,
            b"",
            Path::new("versions/v2/telex"),
        );
        assert_eq!(result["attempted"], true);
        assert_eq!(result["reconciled"], false);
        assert_eq!(result["exit_code"], 1);
        assert_eq!(result["recoverable_at_drain"], 3);
        assert_eq!(
            result["error"], "no reconcile pass completed before the timeout",
            "the successor's own diagnosis must survive its non-zero exit: {result}"
        );
        assert_eq!(result["store_key"], "scope-abc");
        assert_eq!(result["successor_binary"], "versions/v2/telex");
        // Additive compatibility: every non-zero exit still carries the old hint fields.
        assert_eq!(
            result["min_daemon_minor"],
            crate::daemon_reconcile::RECONCILE_MIN_DAEMON_MINOR
        );
    }

    #[test]
    fn successor_reconcile_bounds_a_structured_error_from_a_nonzero_exit() {
        let stdout = serde_json::to_vec(&json!({
            "reconciled": false,
            "store_key": "s".repeat(4096),
            "error": "x".repeat(9000),
        }))
        .expect("encode oversized successor report");
        let result =
            successor_reconcile_result(1, false, Some(1), &stdout, b"", Path::new("next-telex"));
        assert_eq!(result["reconciled"], false);
        assert!(
            result["error"].as_str().unwrap().chars().count() <= SUCCESSOR_DIAGNOSTIC_CHARS,
            "a child-provided error must stay bounded: {result}"
        );
        assert!(
            result["store_key"].as_str().unwrap().chars().count() <= SUCCESSOR_DIAGNOSTIC_CHARS,
            "a child-provided store key must stay bounded: {result}"
        );
    }

    /// Whichever way the successor step ends, the result must say which binary it is about —
    /// otherwise a report from `upgrade` and one from `rollback` are indistinguishable.
    #[tokio::test]
    async fn successor_reconcile_names_the_binary_on_every_branch() {
        let ctx = test_ctx();
        let missing = std::env::current_dir()
            .expect("cwd")
            .join("no-such-successor-telex");
        let skipped = vec![
            (
                "no station-intent report",
                verify_successor_reconcile(&ctx, &json!({}), &missing).await,
            ),
            (
                "nothing recoverable",
                verify_successor_reconcile(
                    &ctx,
                    &json!({"station_intents": {"recoverable": 0}}),
                    &missing,
                )
                .await,
            ),
            (
                "missing successor binary",
                verify_successor_reconcile(
                    &ctx,
                    &json!({"station_intents": {"recoverable": 2}}),
                    &missing,
                )
                .await,
            ),
        ];
        for (label, result) in &skipped {
            assert_eq!(result["attempted"], false, "{label}: {result}");
            assert_eq!(
                result["successor_binary"],
                missing.to_string_lossy().as_ref(),
                "{label} must still name the successor: {result}"
            );
        }

        let spawn_failure = run_successor_reconcile(
            tokio::process::Command::new(&missing),
            &missing,
            2,
            Duration::from_secs(5),
        )
        .await;
        assert_eq!(spawn_failure["attempted"], true);
        assert_eq!(
            spawn_failure["successor_binary"],
            missing.to_string_lossy().as_ref(),
            "a spawn failure must name the successor: {spawn_failure}"
        );

        let reconciled = successor_reconcile_result(
            1,
            true,
            Some(0),
            br#"{"reconciled":true,"report":{"restored":1,"pass_seq":4}}"#,
            b"",
            Path::new("done-telex"),
        );
        let structured_failure = successor_reconcile_result(
            1,
            false,
            Some(1),
            br#"{"reconciled":false,"error":"the daemon did not answer"}"#,
            b"",
            Path::new("failed-telex"),
        );
        let malformed_zero =
            successor_reconcile_result(1, true, Some(0), b"not json", b"", Path::new("odd-telex"));
        let malformed_nonzero = successor_reconcile_result(
            1,
            false,
            Some(2),
            b"error: unrecognized subcommand",
            b"usage: telex daemon",
            Path::new("old-telex"),
        );
        for (label, result, expected) in [
            ("reconciled", &reconciled, "done-telex"),
            ("structured failure", &structured_failure, "failed-telex"),
            ("malformed on exit 0", &malformed_zero, "odd-telex"),
            ("malformed on exit 2", &malformed_nonzero, "old-telex"),
        ] {
            assert_eq!(result["attempted"], true, "{label}: {result}");
            assert_eq!(
                result["successor_binary"], expected,
                "{label} must name the successor: {result}"
            );
        }
        assert_eq!(reconciled["restored"], 1);
        assert_eq!(reconciled["pass_seq"], 4);
        assert_eq!(reconciled["reconciled"], true);
    }

    /// A successor CLI child that never exits must not outlive the `upgrade` that started it: the
    /// old code dropped the direct child on timeout, which neither signals nor collects it.
    #[tokio::test]
    async fn successor_reconcile_kills_and_reaps_a_child_that_overruns_the_bound() {
        let started = Instant::now();
        let result = run_successor_reconcile(
            long_sleep_command(),
            Path::new("hung-telex"),
            4,
            Duration::from_millis(750),
        )
        .await;
        assert!(
            started.elapsed() < Duration::from_secs(30),
            "the successor step must stay bounded, took {:?}",
            started.elapsed()
        );
        assert_eq!(result["attempted"], true, "{result}");
        assert_eq!(result["timed_out"], true, "{result}");
        assert_eq!(result["terminated"], true, "{result}");
        assert_eq!(result["reaped"], true, "{result}");
        assert_eq!(result["recoverable_at_drain"], 4, "{result}");
        assert_eq!(result["successor_binary"], "hung-telex", "{result}");
        assert!(
            result["error"]
                .as_str()
                .unwrap()
                .contains("did not report a reconcile pass within 750ms"),
            "the message must name the bound it actually applied: {result}"
        );
        let pid = result["successor_pid"]
            .as_u64()
            .expect("the timeout branch must report the pid it killed") as u32;
        assert!(
            !process_is_running(pid),
            "the timed-out successor (pid {pid}) must be terminated, not abandoned"
        );
    }

    /// The capture is bounded, but the reader must keep draining past the cap — a reader that
    /// stops reading is exactly how a child blocks forever on a full pipe.
    #[tokio::test]
    async fn successor_capture_is_bounded_but_keeps_draining() {
        let payload = vec![b'x'; 200_000];
        let captured = drain_capped(&payload[..], 1024).await;
        assert_eq!(
            captured.len(),
            1024,
            "capture must stop growing at the cap while the drain continues"
        );
        let bounded = bounded_diagnostic(&captured);
        assert!(bounded.chars().count() <= SUCCESSOR_DIAGNOSTIC_CHARS);
    }
}
