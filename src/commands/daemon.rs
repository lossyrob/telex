use anyhow::{anyhow, Result};

use crate::cli::{
    Ctx, DaemonCmd, DaemonReconcileArgs, DaemonRecoverIntentsArgs, DaemonResetArgs,
    DaemonSessionEndArgs,
};
use crate::daemon::reconcile as telex_reconcile;
use crate::daemon_ipc::{Request, Response, ERROR_NOT_RUNNING, ERROR_UNAUTHORIZED};
use crate::identity::resolve_session_id;
use crate::output::emit;

pub async fn run(ctx: &Ctx, cmd: DaemonCmd) -> Result<i32> {
    match cmd {
        DaemonCmd::Serve => {
            crate::daemon::serve().await?;
            Ok(0)
        }
        DaemonCmd::Status => status(ctx).await,
        DaemonCmd::Version => version(ctx),
        DaemonCmd::Reset(args) => reset(ctx, args).await,
        DaemonCmd::SessionEnd(args) => session_end(ctx, args).await,
        DaemonCmd::Reconcile(args) => reconcile(ctx, args).await,
        DaemonCmd::RecoverIntents(args) => recover_intents(ctx, args).await,
        DaemonCmd::Stop(args) => {
            if !args.drain {
                return Err(anyhow!("only `telex daemon stop --drain` is supported"));
            }
            stop_drain(ctx).await
        }
    }
}

/// Run the explicitly offline recovery path. This intentionally never starts a daemon: a bounded
/// daemon pass can only publish lower bounds, while this command establishes the supported-local
/// floor and requires the normal writer to be stopped before it can make exact claims.
async fn recover_intents(ctx: &Ctx, args: DaemonRecoverIntentsArgs) -> Result<i32> {
    let store_key = ctx.store_key()?;
    match crate::daemon::connect_existing(&store_key).await {
        Ok(_) => {
            return Err(anyhow!(
                "refusing offline station-intent recovery while the daemon is running; \
                 stop it with `telex daemon stop --drain` and stop other intent writers first"
            ))
        }
        Err(crate::daemon::DaemonError::NotRunning(_)) => {}
        Err(error) => {
            return Err(anyhow!(
                "cannot establish that the daemon is stopped; refusing offline station-intent recovery: {error}"
            ))
        }
    }

    let paths = crate::daemon::DaemonPaths::current()?;
    let store = crate::station_intent::IntentStore::open_existing(
        &paths.run_dir,
        &paths.singleton_hash,
    )?
    .ok_or_else(|| {
        anyhow!(
            "station-intent scope is absent; refusing offline recovery because supported local storage cannot be established"
        )
    })?;

    // This is the proof behind every exact number below. It validates the local-storage floor and
    // completes the enumeration before any optional reclamation begins.
    let before = store.scan_complete_local(0)?;
    if before.discovery_truncated {
        return Err(anyhow!(
            "offline station-intent enumeration was incomplete; refusing exact counts or reclamation"
        ));
    }

    let reclaimed = if args.gc {
        let host = crate::platform_fs::host_id().map_err(|error| {
            anyhow!("cannot establish local host identity for recovery: {error}")
        })?;
        let boot = crate::platform_fs::boot_id().map_err(|error| {
            anyhow!("cannot establish local boot identity for recovery: {error}")
        })?;
        let report = store.gc_complete_local(crate::model::now_ms(), Some(&host), Some(&boot))?;
        if !report.complete {
            return Err(anyhow!(
                "offline station-intent GC was incomplete; refusing a reclamation claim"
            ));
        }
        report.removed.len()
    } else {
        0
    };

    // GC is allowed to reclaim only after the complete scan above. Re-enumerate afterward so the
    // remaining count is exact too, rather than subtracting a best-effort deletion count.
    let after = if args.gc {
        let page = store.scan_complete_local(0)?;
        if page.discovery_truncated {
            return Err(anyhow!(
                "post-GC station-intent enumeration was incomplete; refusing an exact remaining count"
            ));
        }
        Some(page.observed_count)
    } else {
        None
    };
    let payload = serde_json::json!({
        "offline": true,
        "complete_enumeration": true,
        "observed_count": before.observed_count,
        "over_cap": before.over_cap,
        "gc_requested": args.gc,
        "reclaimed": reclaimed,
        "remaining_count": after,
    });
    emit(ctx.fmt, &payload, || {
        println!(
            "offline station-intent scan complete: {} entries{}",
            before.observed_count,
            if before.over_cap {
                " (at or over write cap)"
            } else {
                ""
            }
        );
        if args.gc {
            println!(
                "offline station-intent GC complete: reclaimed {reclaimed}, remaining {}",
                after.expect("GC path sets remaining count")
            );
        }
    });
    Ok(0)
}

fn version(ctx: &Ctx) -> Result<i32> {
    let info = crate::daemon::daemon_version_metadata();
    emit(ctx.fmt, &info, || {
        println!("daemon_version {}", info.daemon_version);
        println!(
            "protocol {}.{}",
            info.protocol_version.major, info.protocol_version.minor
        );
        println!("auth_policy {}", info.auth_policy_version);
    });
    Ok(0)
}

async fn reset(ctx: &Ctx, args: DaemonResetArgs) -> Result<i32> {
    let address_arg = args.address.or_else(|| ctx.address.clone());
    let address = ctx.cfg.require_address(&address_arg)?;
    let paths = crate::daemon::DaemonPaths::current()?;
    let cap = crate::daemon::read_cap_file(&paths.cap_path)?;
    let store_key = ctx.store_key()?;
    let mut client = crate::daemon::connect_existing(&store_key).await?;
    let response = client
        .request(&Request::Reset {
            store_key: store_key.clone(),
            address: address.clone(),
            proof: Some(cap.admin_cap),
        })
        .await?;
    match response {
        Response::Ack { .. } => {
            emit(
                ctx.fmt,
                &serde_json::json!({"reset": true, "address": address, "store_key": store_key}),
                || {
                    println!("daemon reset {address}");
                },
            );
            Ok(0)
        }
        Response::Error { code, message, .. } => Err(anyhow!("{code}: {message}")),
        other => Err(anyhow!("unexpected daemon reset response: {other:?}")),
    }
}

async fn session_end(ctx: &Ctx, args: DaemonSessionEndArgs) -> Result<i32> {
    let paths = crate::daemon::DaemonPaths::current()?;
    let cap = crate::daemon::read_cap_file(&paths.cap_path)?;
    let store_key = ctx.store_key()?;
    let session_id = resolve_session_id(args.session.as_deref())?;
    let mut client = crate::daemon::connect_existing(&store_key).await?;
    let response = client
        .request(&Request::SessionEnd {
            store_key: store_key.clone(),
            session_id: session_id.clone(),
            proof: Some(cap.admin_cap),
        })
        .await?;
    match response {
        Response::Ack { .. } => {
            emit(
                ctx.fmt,
                &serde_json::json!({"session_end": true, "session_id": session_id, "store_key": store_key}),
                || {
                    println!("daemon session-end {session_id}");
                },
            );
            Ok(0)
        }
        Response::Error { code, message, .. } => Err(anyhow!("{code}: {message}")),
        other => Err(anyhow!("unexpected daemon session-end response: {other:?}")),
    }
}

async fn status(ctx: &Ctx) -> Result<i32> {
    let paths = crate::daemon::DaemonPaths::current()?;
    let store_key = ctx.store_key()?;
    for attempt in 0..2 {
        match crate::daemon::connect_existing(&store_key).await {
            Ok(mut client) => {
                let cap = crate::daemon::read_cap_file(&paths.cap_path)?;
                let response = client
                    .request(&Request::Status {
                        store_key: Some(store_key.clone()),
                        detail: true,
                        proof: Some(cap.admin_cap),
                    })
                    .await?;
                match response {
                    Response::StatusReport { status } => {
                        emit(ctx.fmt, &status, || {
                            println!("daemon  running");
                            println!("version {}", status.daemon_version);
                            println!("instance {}", status.instance_id);
                            println!("singleton {}", status.singleton_key);
                        });
                        return Ok(0);
                    }
                    Response::Error { code, .. } if code == ERROR_UNAUTHORIZED && attempt == 0 => {
                        continue;
                    }
                    Response::Error { code, message, .. } => {
                        return Err(anyhow!("{code}: {message}"))
                    }
                    other => return Err(anyhow!("unexpected daemon status response: {other:?}")),
                }
            }
            Err(crate::daemon::DaemonError::NotRunning(_)) => {
                let info = crate::daemon::local_status_metadata(&paths);
                emit(ctx.fmt, &info, || {
                    println!("daemon  not running");
                    println!("endpoint {}", paths.endpoint.display());
                    println!("cap      {}", paths.cap_path.display());
                });
                return Ok(0);
            }
            Err(crate::daemon::DaemonError::Unauthorized(_)) if attempt == 0 => continue,
            Err(e) => return Err(e.into()),
        }
    }
    Err(anyhow!("daemon status failed after retry"))
}

/// Run one reconciliation pass on this store's daemon, spawning it if needed.
///
/// Retries while the daemon reports `draining` and while a pass reports that it did **not run**
/// (drain suppression, single-flight contention). Both were previously indistinguishable from
/// "ran and restored nothing", so `telex upgrade` could print a successful verification for a
/// recovery that never started.
///
/// `--timeout-ms` is a ceiling on the whole retry loop, and every individual attempt is bounded
/// too. Without the per-attempt bound the loop's own deadline was only consulted *between*
/// attempts, so a daemon that accepted the connection and never answered held the command open
/// forever — the one failure the timeout was there to cover.
async fn reconcile(ctx: &Ctx, args: DaemonReconcileArgs) -> Result<i32> {
    use std::time::{Duration, Instant};
    let store_key = ctx.store_key()?;
    let deadline = Instant::now() + Duration::from_millis(args.timeout_ms.max(1));
    let mut last_error: Option<String> = None;
    let mut result: Option<crate::daemon_ipc::ReconcileReport> = None;
    while Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let attempt = tokio::time::timeout(
            remaining.min(telex_reconcile::RECONCILE_REQUEST_DEADLINE),
            reconcile_attempt(&store_key, args.scope.clone()),
        )
        .await;
        match attempt {
            Ok(Ok(report)) if report.ran => {
                result = Some(report);
                break;
            }
            Ok(Ok(report)) => {
                last_error = Some(format!(
                    "pass {} did not run ({})",
                    report.pass_seq,
                    report.skipped_reason.as_deref().unwrap_or("unknown")
                ));
            }
            Ok(Err(e)) => last_error = Some(e),
            Err(_) => {
                last_error = Some(format!(
                    "the daemon did not answer a reconcile request within {} ms",
                    telex_reconcile::RECONCILE_REQUEST_DEADLINE.as_millis()
                ))
            }
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        tokio::time::sleep(remaining.min(Duration::from_millis(250))).await;
    }
    let payload = match &result {
        Some(report) => serde_json::json!({
            "reconciled": true,
            "store_key": store_key,
            "report": report,
        }),
        None => serde_json::json!({
            "reconciled": false,
            "store_key": store_key,
            "error": last_error
                .clone()
                .unwrap_or_else(|| "no reconcile pass completed before the timeout".to_string()),
        }),
    };
    emit(ctx.fmt, &payload, || match &result {
        Some(report) => println!(
            "reconciled pass {} restored {} refreshed {} deferred {} failed {}",
            report.pass_seq,
            report.restored,
            report.refreshed_no_op,
            report.deferred_lease,
            report.failed
        ),
        None => println!(
            "reconcile incomplete: {}",
            last_error.as_deref().unwrap_or("timed out")
        ),
    });
    Ok(if result.is_some() { 0 } else { 1 })
}

async fn reconcile_attempt(
    store_key: &str,
    scope: Option<String>,
) -> std::result::Result<crate::daemon_ipc::ReconcileReport, String> {
    let mut client = crate::daemon::connect_or_spawn(store_key)
        .await
        .map_err(|e| e.to_string())?;
    let paths = crate::daemon::DaemonPaths::current().map_err(|e| e.to_string())?;
    let cap = crate::daemon::read_cap_file(&paths.cap_path).map_err(|e| e.to_string())?;
    let response = client
        .request(&Request::ReconcileIntents {
            proof: Some(cap.admin_cap),
            scope,
        })
        .await
        .map_err(|e| e.to_string())?;
    match response {
        Response::Reconciled { report } => Ok(report),
        Response::Error { code, message, .. } if code == ERROR_NOT_RUNNING => {
            Err(format!("{code}: {message}"))
        }
        Response::Error { code, message, .. } => Err(format!("{code}: {message}")),
        other => Err(format!("unexpected daemon reconcile response: {other:?}")),
    }
}

async fn stop_drain(ctx: &Ctx) -> Result<i32> {
    let paths = crate::daemon::DaemonPaths::current()?;
    let cap = crate::daemon::read_cap_file(&paths.cap_path)?;
    let store_key = ctx.store_key()?;
    let mut client = crate::daemon::connect_existing(&store_key).await?;
    let response = client
        .request(&Request::Drain {
            proof: Some(cap.admin_cap),
        })
        .await?;
    match response {
        Response::Ack { drain_intents, .. } => {
            // The pre-drain station-intent signal (issue #106). Computed by the daemon from
            // in-memory state before it released any lease, so it describes what a successor will
            // find rather than what is true after the fact. An older daemon omits it entirely,
            // which is rendered as "unavailable" rather than silently as zero.
            let report = drain_intents.clone();
            let payload = serde_json::json!({
                "draining": true,
                "station_intents": report,
            });
            emit(ctx.fmt, &payload, || {
                println!("daemon drain requested");
                match &report {
                    Some(report) => {
                        println!(
                            "station intents  recoverable {} pending {} degraded {} incompatible {} unknown {}",
                            report.recoverable,
                            report.pending,
                            report.degraded,
                            report.incompatible,
                            report.unknown
                        );
                        if report.pending > 0 {
                            println!(
                                "station intents  {} pending intent(s) are not finalized and will NOT be restored automatically; \
                                 they finalize at the next Copilot turn boundary",
                                report.pending
                            );
                        }
                        if report.over_cap {
                            println!(
                                "station intents  WARNING: {} entries exceed the per-scope write cap; \
                                 the daemon reclaims eligible entries automatically on its GC cadence and at startup; \
                                 to make an entry eligible now, run `telex --address <station> copilot detach`",
                                report.observed_count
                            );
                        }
                        if report.degraded > 0 || report.incompatible > 0 {
                            println!(
                                "station intents  {} intent(s) will NOT be restored automatically; run `telex --address <station> copilot resume` after the successor starts",
                                report.degraded + report.incompatible
                            );
                        }
                        println!("station intents  index_as_of_ms {}", report.index_as_of_ms);
                    }
                    None => println!(
                        "station intents  unavailable (the running daemon predates station-intent reporting)"
                    ),
                }
            });
            Ok(0)
        }
        Response::Error { code, message, .. } => Err(anyhow!("{code}: {message}")),
        other => Err(anyhow!("unexpected daemon drain response: {other:?}")),
    }
}
