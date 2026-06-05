use anyhow::Result;
use serde_json::json;

use crate::cli::{Ctx, ExportArgs};
use crate::output::print_jsonl;

/// Emit messages with their disposition history as JSON lines (one object per message).
pub async fn run(ctx: &Ctx, args: ExportArgs) -> Result<i32> {
    let backend = ctx.backend().await?;
    let address = args.address.clone().or_else(|| ctx.address.clone());
    let msgs = backend
        .export(address.as_deref(), args.thread, args.since)
        .await?;

    for m in &msgs {
        let dispositions = backend.dispositions_for(m.id).await?;
        print_jsonl(&json!({ "message": m, "dispositions": dispositions }));
    }
    Ok(0)
}
