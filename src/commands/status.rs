use anyhow::Result;

use crate::cli::Ctx;
use crate::output::emit;

pub async fn run(ctx: &Ctx) -> Result<i32> {
    let (name, profile) = ctx.resolved()?;
    let backend = ctx.backend().await?;
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
        info["address"] = serde_json::json!(addr);
        info["occupancy"] = serde_json::to_value(&occ)?;
        info["lease"] = serde_json::to_value(&lease)?;
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
    });
    Ok(0)
}
