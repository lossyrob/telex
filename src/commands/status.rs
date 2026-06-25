use anyhow::Result;

use crate::cli::Ctx;
use crate::daemon_ipc::{DaemonStatus, Request, Response};
use crate::output::emit;

pub async fn run(ctx: &Ctx) -> Result<i32> {
    let (name, profile) = ctx.resolved()?;
    let backend = ctx.backend().await?;
    let store_key = ctx.store_key()?;
    let caps = backend.capabilities();
    let mut info = serde_json::json!({
        "backend": name,
        "kind": backend.kind(),
        "target": profile.target(),
        "default_address": ctx.cfg.default_address,
        "liveness_window_secs": ctx.cfg.liveness_window_secs,
        "capabilities": {
            "durable": caps.durable,
            "push": caps.push,
            "lease": caps.lease,
        },
    });

    if let Some(addr) = ctx.address.clone() {
        let occ = backend
            .occupancy(&addr, ctx.cfg.liveness_window_secs)
            .await?;
        let lease = backend.get_lease(&addr).await?;
        let daemon_status = daemon_detail(ctx).await?;
        let daemon_members = daemon_status
            .as_ref()
            .map(|status| {
                status
                    .members
                    .iter()
                    .filter(|member| {
                        member.store_key == store_key && member.address == addr && !member.idle
                    })
                    .cloned()
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let live_waiters = daemon_status
            .as_ref()
            .map(|status| {
                status
                    .live_waiters
                    .iter()
                    .filter(|waiter| waiter.store_key == store_key && waiter.address == addr)
                    .cloned()
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        info["address"] = serde_json::json!(addr);
        info["occupancy"] = serde_json::to_value(&occ)?;
        if !daemon_members.is_empty() {
            info["occupancy"]["occupied"] = serde_json::json!(true);
            info["occupancy"]["occupant"] = serde_json::json!(daemon_members[0].occupant);
        }
        info["lease"] = serde_json::to_value(&lease)?;
        info["daemon_members"] = serde_json::to_value(&daemon_members)?;
        info["live_waiters"] = serde_json::to_value(&live_waiters)?;
    }

    emit(ctx.fmt, &info, || {
        println!("backend  {} ({})", name, backend.kind());
        println!("target   {}", profile.target());
        println!(
            "address  {}",
            ctx.address.clone().unwrap_or_else(|| "(none)".into())
        );
        if let Some(occ) = info.get("occupancy") {
            println!(
                "occupied {}  (age {:.1}s)",
                occ["occupied"], occ["age_secs"]
            );
        }
        if let Some(members) = info.get("daemon_members").and_then(|v| v.as_array()) {
            println!("daemon_members {}", members.len());
        }
        if let Some(waiters) = info.get("live_waiters").and_then(|v| v.as_array()) {
            println!("live_waiters {}", waiters.len());
        }
    });
    Ok(0)
}

async fn daemon_detail(ctx: &Ctx) -> Result<Option<DaemonStatus>> {
    let paths = crate::daemon::DaemonPaths::current()?;
    let cap = match crate::daemon::read_cap_file(&paths.cap_path) {
        Ok(cap) => cap,
        Err(_) => return Ok(None),
    };
    let store_key = ctx.store_key()?;
    match crate::daemon::connect_existing(&store_key).await {
        Ok(mut client) => {
            let response = client
                .request(&Request::Status {
                    store_key: Some(store_key),
                    detail: true,
                    proof: Some(cap.admin_cap),
                })
                .await?;
            match response {
                Response::StatusReport { status } => Ok(Some(status)),
                Response::Error { code, message, .. } => Err(anyhow::anyhow!("{code}: {message}")),
                other => Err(anyhow::anyhow!(
                    "unexpected daemon status response: {other:?}"
                )),
            }
        }
        Err(crate::daemon::DaemonError::NotRunning(_)) => Ok(None),
        Err(e) => Err(e.into()),
    }
}
