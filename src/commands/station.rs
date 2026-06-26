use anyhow::{anyhow, Result};
use serde_json::json;

use crate::cli::{Ctx, StationCmd, StationStatusArgs, StationStopArgs};
use crate::daemon_ipc::{Request, Response};
use crate::identity::resolve_session_id;
use crate::output::emit;

pub async fn run(ctx: &Ctx, cmd: StationCmd) -> Result<i32> {
    match cmd {
        StationCmd::Status(args) => status(ctx, args).await,
        StationCmd::Stop(args) => stop(ctx, args).await,
    }
}

async fn status(ctx: &Ctx, args: StationStatusArgs) -> Result<i32> {
    let paths = crate::daemon::DaemonPaths::current()?;
    let cap = crate::daemon::read_cap_file(&paths.cap_path)?;
    let store_key = ctx.store_key()?;
    let session_id = resolve_session_id(args.session.as_deref())?;
    let mut client = crate::daemon::connect_existing(&store_key).await?;
    let response = client
        .request(&Request::Status {
            store_key: Some(store_key.clone()),
            detail: true,
            proof: Some(cap.admin_cap),
        })
        .await?;
    match response {
        Response::StatusReport { status } => {
            let stations: Vec<_> = status
                .members
                .into_iter()
                .filter(|member| member.store_key == store_key && member.session_id == session_id)
                .map(|member| {
                    json!({
                        "store_key": member.store_key,
                        "backend": member.backend,
                        "session_id": member.session_id,
                        "address": member.address,
                        "station_health": member.station_health,
                        "health_detail": member.health_detail,
                        "waiters": member.waiters,
                        "live_waiters_count": member.live_waiters_count,
                        "pending_unconsumed_count": member.pending_unconsumed_count,
                        "last_waiter_exit_at_ms": member.last_waiter_exit_at_ms,
                        "last_waiter_outcome": member.last_waiter_outcome,
                        "last_delivered_message_id": member.last_delivered_message_id,
                        "idle": member.idle,
                        "live_waiters": member.live_waiters,
                    })
                })
                .collect();
            let out = json!({
                "session_id": session_id,
                "store_key": store_key,
                "count": stations.len(),
                "stations": stations.clone(),
            });
            emit(ctx.fmt, &out, || {
                if stations.is_empty() {
                    println!("(no stations for session {session_id})");
                } else {
                    for station in &stations {
                        println!(
                            "{} waiters={} pending={} health={}",
                            station["address"].as_str().unwrap_or("?"),
                            station["live_waiters_count"],
                            station["pending_unconsumed_count"],
                            station["station_health"].as_str().unwrap_or("?")
                        );
                    }
                }
            });
            Ok(0)
        }
        Response::Error { code, message, .. } => Err(anyhow!("{code}: {message}")),
        other => Err(anyhow!("unexpected daemon station-status response: {other:?}")),
    }
}

async fn stop(ctx: &Ctx, args: StationStopArgs) -> Result<i32> {
    let address = ctx.cfg.require_address(&ctx.address)?;
    let store_key = ctx.store_key()?;
    let session_id = resolve_session_id(args.session.as_deref())?;

    let mut client = crate::daemon::connect_existing(&store_key).await?;
    let response = client
        .request(&Request::StationStop {
            store_key: store_key.clone(),
            session_id: session_id.clone(),
            address: address.clone(),
            wait_grace_ms: args.wait_grace_ms,
        })
        .await?;

    match response {
        Response::StationStopped {
            detached,
            waiters_before,
            waiters_after,
            live_waiters,
            message,
            lease_epoch,
            ..
        } => {
            let out = json!({
                "address": address,
                "store_key": store_key,
                "session_id": session_id,
                "detached": detached,
                "message": message,
                "lease_epoch": lease_epoch,
                "waiters_before": waiters_before,
                "waiters_after": waiters_after,
                "live_waiters": live_waiters,
            });
            emit(ctx.fmt, &out, || {
                if waiters_after == 0 {
                    println!(
                        "station stopped {address} session={session_id} waiters={waiters_before}->0"
                    );
                } else {
                    println!(
                        "station stopped {address} session={session_id} waiters={waiters_before}->{waiters_after}"
                    );
                }
            });
            Ok(0)
        }
        Response::Error { code, message, .. } => Err(anyhow!("{code}: {message}")),
        other => Err(anyhow!(
            "unexpected daemon station-stop response: {other:?}"
        )),
    }
}
