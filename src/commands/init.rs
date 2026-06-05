use anyhow::Result;

use crate::cli::Ctx;
use crate::config;
use crate::output::emit;

pub async fn run(ctx: &Ctx) -> Result<i32> {
    let home = config::ensure_home()?;
    // Building the backend initializes its schema.
    let backend = ctx.backend().await?;
    let info = serde_json::json!({
        "initialized": true,
        "telex_home": home.to_string_lossy(),
        "backend": backend.kind(),
        "db": ctx.cfg.db_path_str(),
    });
    emit(ctx.fmt, &info, || {
        println!(
            "initialized telex (backend={}) home={} db={}",
            backend.kind(),
            home.to_string_lossy(),
            ctx.cfg.db_path_str()
        );
    });
    Ok(0)
}
