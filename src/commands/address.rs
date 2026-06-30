use anyhow::{anyhow, Result};
use serde_json::json;

use crate::cli::{AddressCmd, AddressListArgs, Ctx, ResolveArgs};
use crate::daemon_ipc::{DaemonStatus, Request, Response, ERROR_UNAUTHORIZED};
use crate::identity::optional_session_id;
use crate::model::{AddressRow, STATUS_RETIRED};
use crate::output::emit;

pub async fn run(ctx: &Ctx, cmd: AddressCmd) -> Result<i32> {
    match cmd {
        AddressCmd::List(a) => list(ctx, a).await,
        AddressCmd::Show => show(ctx).await,
        AddressCmd::Retire => retire(ctx).await,
    }
}

fn matches(a: &AddressRow, m: &Option<String>, tag: &Option<String>) -> bool {
    let m_ok = match m {
        None => true,
        Some(s) => {
            let s = s.to_ascii_lowercase();
            a.address.to_ascii_lowercase().contains(&s)
                || a.description
                    .as_deref()
                    .map(|d| d.to_ascii_lowercase().contains(&s))
                    .unwrap_or(false)
        }
    };
    let tag_ok = match tag {
        None => true,
        Some(t) => a
            .tags
            .as_deref()
            .map(|tags| tags.to_ascii_lowercase().contains(&t.to_ascii_lowercase()))
            .unwrap_or(false),
    };
    m_ok && tag_ok
}

async fn list(ctx: &Ctx, args: AddressListArgs) -> Result<i32> {
    let backend = ctx.backend().await?;
    let store_key = ctx.store_key()?;
    let current_session_id = optional_session_id(None);
    let daemon_status = daemon_detail(ctx).await?;
    let rows = backend
        .list_addresses(args.scope.as_deref(), args.all)
        .await?;

    let mut entries = Vec::new();
    for a in rows
        .into_iter()
        .filter(|a| matches(a, &args.r#match, &args.tag))
    {
        let occ = backend
            .occupancy(&a.address, ctx.cfg.liveness_window_secs)
            .await?;
        let daemon_members: Vec<_> = daemon_status
            .as_ref()
            .map(|status| {
                status
                    .members
                    .iter()
                    .filter(|member| {
                        member.store_key == store_key && member.address == a.address && !member.idle
                    })
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        let daemon_member = daemon_members.first();
        let foreign_members: Vec<_> = daemon_members
            .iter()
            .filter(|member| {
                current_session_id
                    .as_ref()
                    .map_or(true, |session_id| member.session_id != *session_id)
            })
            .cloned()
            .collect();
        let deaf_warn = daemon_members.iter().any(|m| m.deaf_warn);
        entries.push(json!({
            "address": a.address,
            "description": a.description,
            "scope": a.scope,
            "tags": a.tags,
            "status": a.status,
            "occupied": daemon_member.is_some() || occ.occupied,
            "occupant": daemon_member.map(|m| m.occupant.clone()).or(occ.occupant),
            "age_secs": occ.age_secs,
            "station_health": daemon_member.map(|m| serde_json::json!(m.station_health)),
            "health_detail": daemon_member.and_then(|m| m.health_detail.clone()),
            "pending_unconsumed_count": daemon_member.map(|m| m.pending_unconsumed_count),
            "live_waiters_count": daemon_member.map(|m| m.live_waiters_count),
            "unattended_since_ms": daemon_member.and_then(|m| m.unattended_since_ms),
            "unattended_for_ms": daemon_member.and_then(|m| m.unattended_for_ms),
            "deaf_since_ms": daemon_member.and_then(|m| m.deaf_since_ms),
            "deaf_for_ms": daemon_member.and_then(|m| m.deaf_for_ms),
            "deaf_warn": deaf_warn,
            "foreign_members": foreign_members,
            "daemon_members": daemon_members,
        }));
    }

    let out = json!({ "count": entries.len(), "addresses": entries });
    emit(ctx.fmt, &out, || {
        println!(
            "{:<40} {:<11} {:<24} DESCRIPTION",
            "ADDRESS", "OCCUPANCY", "HEALTH"
        );
        for e in &entries {
            let occ = if e["occupied"].as_bool().unwrap_or(false) {
                "occupied"
            } else {
                "unoccupied"
            };
            let health = e["station_health"].as_str().unwrap_or("-");
            let suffix = if e["deaf_warn"].as_bool().unwrap_or(false) {
                " DEAF"
            } else if e["foreign_members"]
                .as_array()
                .map(|a| !a.is_empty())
                .unwrap_or(false)
            {
                " foreign"
            } else {
                ""
            };
            println!(
                "{:<40} {:<11} {:<24} {}{}",
                e["address"].as_str().unwrap_or(""),
                occ,
                health,
                e["description"].as_str().unwrap_or(""),
                suffix
            );
        }
    });
    Ok(0)
}

async fn show(ctx: &Ctx) -> Result<i32> {
    let address = ctx.cfg.require_address(&ctx.address)?;
    let backend = ctx.backend().await?;
    let addr = backend
        .get_address(&address)
        .await?
        .ok_or_else(|| anyhow!("address {address} not found"))?;
    let lease = backend.get_lease(&address).await?;
    let occ = backend
        .occupancy(&address, ctx.cfg.liveness_window_secs)
        .await?;
    let store_key = ctx.store_key()?;
    let current_session_id = optional_session_id(None);
    let daemon_status = daemon_detail(ctx).await?;
    let daemon_members: Vec<_> = daemon_status
        .as_ref()
        .map(|status| {
            status
                .members
                .iter()
                .filter(|member| {
                    member.store_key == store_key && member.address == address && !member.idle
                })
                .cloned()
                .collect()
        })
        .unwrap_or_default();
    let foreign_members: Vec<_> = daemon_members
        .iter()
        .filter(|member| {
            current_session_id
                .as_ref()
                .map_or(true, |session_id| member.session_id != *session_id)
        })
        .cloned()
        .collect();
    let deaf_warn = daemon_members.iter().any(|m| m.deaf_warn);
    let out = json!({
        "address": addr,
        "lease": lease,
        "occupancy": {
            "occupied": !daemon_members.is_empty() || occ.occupied,
            "occupant": daemon_members.first().map(|m| m.occupant.clone()).or(occ.occupant),
            "age_secs": occ.age_secs,
        },
        "station_health": daemon_members.first().map(|m| m.station_health),
        "health_detail": daemon_members.first().and_then(|m| m.health_detail.clone()),
        "pending_unconsumed_count": daemon_members.first().map(|m| m.pending_unconsumed_count),
        "live_waiters_count": daemon_members.first().map(|m| m.live_waiters_count),
        "unattended_since_ms": daemon_members.first().and_then(|m| m.unattended_since_ms),
        "unattended_for_ms": daemon_members.first().and_then(|m| m.unattended_for_ms),
        "deaf_since_ms": daemon_members.first().and_then(|m| m.deaf_since_ms),
        "deaf_for_ms": daemon_members.first().and_then(|m| m.deaf_for_ms),
        "deaf_warn": deaf_warn,
        "last_waiter_outcome": daemon_members.first().and_then(|m| m.last_waiter_outcome),
        "last_waiter_exit_code": daemon_members.first().and_then(|m| m.last_waiter_exit_code),
        "last_waiter_detail": daemon_members.first().and_then(|m| m.last_waiter_detail.clone()),
        "last_waiter_pid": daemon_members.first().and_then(|m| m.last_waiter_pid),
        "foreign_members": foreign_members,
        "daemon_members": daemon_members,
    });
    emit(ctx.fmt, &out, || {
        println!("address      {}", addr.address);
        println!("status       {}", addr.status);
        println!(
            "description  {}",
            addr.description.as_deref().unwrap_or("(none)")
        );
        println!("occupied     {} (age {:.1}s)", occ.occupied, occ.age_secs);
        if let Some(l) = &lease {
            println!("occupant     {}", l.occupant.as_deref().unwrap_or("?"));
        }
        if let Some(health) = out.get("station_health").and_then(|v| v.as_str()) {
            println!(
                "health       {} pending={}{}",
                health,
                out.get("pending_unconsumed_count")
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "null".to_string()),
                if out
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
        if out
            .get("foreign_members")
            .and_then(|v| v.as_array())
            .map(|a| !a.is_empty())
            .unwrap_or(false)
        {
            println!(
                "foreign      {} session(s)",
                out["foreign_members"]
                    .as_array()
                    .map(|a| a.len())
                    .unwrap_or(0)
            );
        }
    });
    Ok(0)
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
                        return Err(anyhow!("{code}: {message}"))
                    }
                    other => return Err(anyhow!("unexpected daemon status response: {other:?}")),
                }
            }
            Err(crate::daemon::DaemonError::NotRunning(_)) => return Ok(None),
            Err(crate::daemon::DaemonError::Unauthorized(_)) if attempt == 0 => continue,
            Err(e) => return Err(e.into()),
        }
    }
    Ok(None)
}

async fn retire(ctx: &Ctx) -> Result<i32> {
    let address = ctx.cfg.require_address(&ctx.address)?;
    let backend = ctx.backend().await?;
    let changed = backend.set_address_status(&address, STATUS_RETIRED).await?;
    if !changed {
        return Err(anyhow!("address {address} not found"));
    }
    let out = json!({ "address": address, "status": STATUS_RETIRED });
    emit(ctx.fmt, &out, || println!("retired {address}"));
    Ok(0)
}

pub async fn resolve(ctx: &Ctx, args: ResolveArgs) -> Result<i32> {
    if args.r#match.is_none() && args.tag.is_none() {
        return Err(anyhow!("resolve requires --match or --tag"));
    }
    let backend = ctx.backend().await?;
    let rows = backend.list_addresses(args.scope.as_deref(), false).await?;
    let matched: Vec<_> = rows
        .into_iter()
        .filter(|a| a.status != STATUS_RETIRED)
        .filter(|a| matches(a, &args.r#match, &args.tag))
        .collect();

    let mut entries = Vec::new();
    for a in &matched {
        let occ = backend
            .occupancy(&a.address, ctx.cfg.liveness_window_secs)
            .await?;
        entries.push(json!({
            "address": a.address,
            "description": a.description,
            "occupied": occ.occupied,
        }));
    }

    let out = json!({ "count": entries.len(), "matches": entries });
    emit(ctx.fmt, &out, || {
        for e in &entries {
            println!(
                "{}  {}",
                e["address"].as_str().unwrap_or(""),
                e["description"].as_str().unwrap_or("")
            );
        }
    });
    Ok(0)
}
