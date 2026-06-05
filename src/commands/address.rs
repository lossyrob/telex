use anyhow::{anyhow, Result};
use serde_json::json;

use crate::cli::{AddressCmd, AddressListArgs, Ctx, ResolveArgs};
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
        entries.push(json!({
            "address": a.address,
            "description": a.description,
            "scope": a.scope,
            "tags": a.tags,
            "status": a.status,
            "occupied": occ.occupied,
            "occupant": occ.occupant,
            "age_secs": occ.age_secs,
        }));
    }

    let out = json!({ "count": entries.len(), "addresses": entries });
    emit(ctx.fmt, &out, || {
        println!("{:<40} {:<11} DESCRIPTION", "ADDRESS", "OCCUPANCY");
        for e in &entries {
            let occ = if e["occupied"].as_bool().unwrap_or(false) {
                "occupied"
            } else {
                "unoccupied"
            };
            println!(
                "{:<40} {:<11} {}",
                e["address"].as_str().unwrap_or(""),
                occ,
                e["description"].as_str().unwrap_or("")
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
    let out = json!({
        "address": addr,
        "lease": lease,
        "occupancy": occ,
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
    });
    Ok(0)
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
