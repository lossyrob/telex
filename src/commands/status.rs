use anyhow::Result;
use std::collections::BTreeMap;

use crate::cli::Ctx;
use crate::daemon_ipc::{DaemonStatus, Request, Response, ERROR_UNAUTHORIZED};
use crate::identity::optional_session_id;
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
        let also_active_on = daemon_status
            .as_ref()
            .map(|status| alternate_backend_activity(status, &store_key, &addr))
            .unwrap_or_default();
        let current_session_id = optional_session_id(None);
        let foreign_members: Vec<_> = daemon_members
            .iter()
            .filter(|member| {
                current_session_id
                    .as_ref()
                    .map_or(true, |session_id| member.session_id != *session_id)
            })
            .cloned()
            .collect();
        let deaf_warn = daemon_members.iter().any(|member| member.deaf_warn);
        info["address"] = serde_json::json!(addr);
        info["occupancy"] = serde_json::to_value(&occ)?;
        if !daemon_members.is_empty() {
            info["occupancy"]["occupied"] = serde_json::json!(true);
            info["occupancy"]["occupant"] = serde_json::json!(daemon_members[0].occupant);
            info["station_health"] = serde_json::json!(daemon_members[0].station_health);
            info["health_detail"] = serde_json::json!(daemon_members[0].health_detail);
            info["pending_unconsumed_count"] =
                serde_json::json!(daemon_members[0].pending_unconsumed_count);
            info["live_waiters_count"] = serde_json::json!(daemon_members[0].live_waiters_count);
            info["unattended_since_ms"] = serde_json::json!(daemon_members[0].unattended_since_ms);
            info["unattended_for_ms"] = serde_json::json!(daemon_members[0].unattended_for_ms);
            info["deaf_since_ms"] = serde_json::json!(daemon_members[0].deaf_since_ms);
            info["deaf_for_ms"] = serde_json::json!(daemon_members[0].deaf_for_ms);
            info["deaf_warn"] = serde_json::json!(deaf_warn);
            info["push_registered"] = serde_json::json!(daemon_members[0].push_registered);
            info["last_waiter_outcome"] = serde_json::json!(daemon_members[0].last_waiter_outcome);
            info["last_waiter_exit_code"] =
                serde_json::json!(daemon_members[0].last_waiter_exit_code);
            info["last_waiter_detail"] =
                serde_json::json!(daemon_members[0].last_waiter_detail.clone());
            info["last_waiter_pid"] = serde_json::json!(daemon_members[0].last_waiter_pid);
        }
        info["lease"] = serde_json::to_value(&lease)?;
        info["daemon_members"] = serde_json::to_value(&daemon_members)?;
        info["foreign_members"] = serde_json::to_value(&foreign_members)?;
        info["live_waiters"] = serde_json::to_value(&live_waiters)?;
        info["also_active_on"] = serde_json::to_value(&also_active_on)?;
        if daemon_members.is_empty() && !also_active_on.is_empty() {
            info["backend_warning"] = serde_json::json!(
                "address has live station activity on another backend/store; current backend may be wrong"
            );
        }
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
        if let Some(health) = info.get("station_health").and_then(|v| v.as_str()) {
            let pending = info
                .get("pending_unconsumed_count")
                .map(|v| v.to_string())
                .unwrap_or_else(|| "null".to_string());
            println!(
                "station_health {} pending={}{}{}",
                health,
                pending,
                if info
                    .get("push_registered")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false)
                {
                    " push"
                } else {
                    ""
                },
                if info
                    .get("deaf_warn")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false)
                {
                    " DEAF"
                } else {
                    ""
                }
            );
        }
        if let Some(foreign) = info.get("foreign_members").and_then(|v| v.as_array()) {
            if !foreign.is_empty() {
                println!("foreign_sessions {}", foreign.len());
            }
        }
        if let Some(waiters) = info.get("live_waiters").and_then(|v| v.as_array()) {
            println!("live_waiters {}", waiters.len());
        }
        if let Some(warning) = info.get("backend_warning").and_then(|v| v.as_str()) {
            println!("warning  {warning}");
        }
    });
    Ok(0)
}

fn alternate_backend_activity(
    status: &DaemonStatus,
    selected_store_key: &str,
    address: &str,
) -> Vec<serde_json::Value> {
    let names = store_key_backend_names();
    status
        .members
        .iter()
        .filter(|member| {
            member.address == address && !member.idle && member.store_key != selected_store_key
        })
        .map(|member| {
            serde_json::json!({
                "store_key": member.store_key,
                "backend": names.get(&member.store_key).cloned(),
                "session_id": member.session_id,
                "station_health": member.station_health,
                "pending_unconsumed_count": member.pending_unconsumed_count,
                "live_waiters_count": member.live_waiters_count,
            })
        })
        .collect()
}

fn store_key_backend_names() -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    if let Ok(cfg) = crate::profiles::load() {
        for (name, profile) in cfg.backends {
            out.insert(crate::profiles::store_key(&profile, None), name);
        }
    }
    let implicit = crate::profiles::implicit_sqlite(None);
    out.entry(crate::profiles::store_key(&implicit, None))
        .or_insert_with(|| "default".to_string());
    out
}

async fn daemon_detail(ctx: &Ctx) -> Result<Option<DaemonStatus>> {
    let paths = crate::daemon::DaemonPaths::current()?;
    let store_key = ctx.store_key()?;
    for attempt in 0..2 {
        let cap = match crate::daemon::read_cap_file(&paths.cap_path) {
            Ok(cap) => cap,
            Err(_) => return Ok(None),
        };
        match crate::daemon::connect_existing(&store_key).await {
            Ok(mut client) => {
                let response = client
                    .request(&Request::Status {
                        store_key: Some(store_key.clone()),
                        detail: true,
                        proof: Some(cap.admin_cap),
                    })
                    .await?;
                match response {
                    Response::StatusReport { status } => return Ok(Some(status)),
                    Response::Error { code, .. } if code == ERROR_UNAUTHORIZED && attempt == 0 => {
                        continue;
                    }
                    Response::Error { code, message, .. } => {
                        return Err(anyhow::anyhow!("{code}: {message}"))
                    }
                    other => {
                        return Err(anyhow::anyhow!(
                            "unexpected daemon status response: {other:?}"
                        ))
                    }
                }
            }
            Err(crate::daemon::DaemonError::NotRunning(_)) => return Ok(None),
            Err(crate::daemon::DaemonError::Unauthorized(_)) if attempt == 0 => continue,
            Err(e) => return Err(e.into()),
        }
    }
    Ok(None)
}
