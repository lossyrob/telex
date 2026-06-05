use anyhow::Result;

use crate::cli::{Ctx, InboxArgs};
use crate::output::emit;

pub async fn run(ctx: &Ctx, args: InboxArgs) -> Result<i32> {
    let address = ctx.cfg.require_address(&ctx.address)?;
    let backend = ctx.backend().await?;
    let items = backend.inbox(&address, args.all, args.limit).await?;

    let out = serde_json::json!({
        "address": address,
        "count": items.len(),
        "items": items,
    });
    emit(ctx.fmt, &out, || {
        if items.is_empty() {
            println!("(empty)");
            return;
        }
        for it in &items {
            let m = &it.message;
            let flag = if it.actionable { "*" } else { " " };
            let disp = it.latest_disposition.as_deref().unwrap_or("-");
            println!(
                "{flag} #{:<5} [{}] {} :: {}  (disp={})",
                m.id,
                m.attention,
                m.from_addr.as_deref().unwrap_or("?"),
                m.subject.as_deref().unwrap_or(m.body.as_str()),
                disp
            );
        }
    });
    Ok(0)
}
