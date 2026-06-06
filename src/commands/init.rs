use anyhow::Result;

use crate::cli::Ctx;
use crate::config;
use crate::output::emit;
use crate::profiles;

pub async fn run(ctx: &Ctx) -> Result<i32> {
    let home = config::ensure_home()?;
    let (name, profile) = ctx.resolved()?;

    // Materialize an explicit `default` sqlite backend if nothing is configured yet, so
    // the config file exists and the default is visible (zero-config still works without).
    let mut cfg = profiles::load()?;
    if cfg.backends.is_empty() && ctx.cfg.backend_selector.is_none() {
        cfg.backends.insert("default".into(), profile.clone());
        cfg.default = Some("default".into());
        profiles::save(&cfg)?;
    }

    // Building the backend initializes its schema.
    let backend = ctx.backend().await?;
    let info = serde_json::json!({
        "initialized": true,
        "telex_home": home.to_string_lossy(),
        "config": profiles::config_path()?.to_string_lossy(),
        "backend": name,
        "kind": backend.kind(),
        "target": profile.target(),
    });
    emit(ctx.fmt, &info, || {
        println!(
            "initialized telex (backend '{}' = {}) -> {}",
            name,
            backend.kind(),
            profile.target()
        );
    });
    Ok(0)
}
