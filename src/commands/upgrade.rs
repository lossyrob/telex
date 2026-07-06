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
    let layout = install::layout_from_optional_root(args.root)?;
    let tag = args
        .version
        .unwrap_or_else(|| format!("v{}", env!("CARGO_PKG_VERSION")));
    let source = resolve_source_binary(&args.from)?;
    let source_metadata = source_metadata(&source, &layout.root)?;
    let installed = install::install_binary(
        &layout,
        &tag,
        &source,
        &format!("local:{}", args.from.display()),
        false,
        Some(source_metadata),
    )?;
    if !args.no_switch {
        let manifest = install::read_manifest(&layout, &tag)?;
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
        Some(install::switch_to(&layout, &tag)?)
    };
    let out = json!({
        "upgrade": true,
        "installed": installed,
        "drain": drain,
        "switch": switched,
    });
    emit(ctx.fmt, &out, || {
        println!("installed {}", tag);
        for warning in &installed.warnings {
            println!("warning {warning}");
        }
        if let Some(switched) = switched {
            println!("current {}", switched.switched_to);
            println!("binary {}", switched.current_binary);
        } else {
            println!("current unchanged (--no-switch)");
        }
    });
    Ok(0)
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
        let mut child = Command::new(source)
            .arg("--json")
            .arg("version")
            .arg("--root")
            .arg(root)
            .env(install::LAUNCHER_GUARD_ENV, "1")
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
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
        Ok(Err(e)) => return Err(e.into()),
        Err(_) => bail!("daemon drain timed out after {timeout_ms}ms"),
    };
    match response {
        Response::Ack { .. } => Ok(json!({"drained": true, "status": "draining"})),
        Response::Error { code, message, .. } if code == ERROR_NOT_RUNNING => {
            Ok(json!({"drained": false, "status": "not_running", "message": message}))
        }
        Response::Error { code, message, .. } if code == ERROR_UNAUTHORIZED => {
            bail!("daemon drain unauthorized: {message}")
        }
        Response::Error { code, message, .. } => bail!("daemon drain failed: {code}: {message}"),
        other => bail!("unexpected daemon drain response: {other:?}"),
    }
}
