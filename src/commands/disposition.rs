use anyhow::{anyhow, Result};

use crate::cli::{Ctx, DispArgs};
use crate::config;
use crate::model::Disposition;
use crate::output::emit;

/// Apply a disposition (`state` is one of the canonical disposition strings) to a message.
pub async fn run(ctx: &Ctx, state: &str, args: DispArgs) -> Result<i32> {
    // Validate the state up front so a bad call fails clearly.
    let _ = Disposition::parse(state)?;

    let backend = ctx.backend().await?;
    let msg = backend
        .get_message(args.id)
        .await?
        .ok_or_else(|| anyhow!("message {} not found", args.id))?;

    let recipient = args
        .recipient
        .clone()
        .unwrap_or_else(|| msg.to_addr.clone());
    let by = config::principal();
    let row = backend
        .insert_disposition(args.id, &recipient, state, args.note.as_deref(), Some(&by))
        .await?;

    emit(ctx.fmt, &row, || {
        println!(
            "{} #{} by {} ({}){}",
            row.state,
            row.message_id,
            row.by_principal.as_deref().unwrap_or("?"),
            row.recipient,
            row.note
                .as_ref()
                .map(|n| format!(": {n}"))
                .unwrap_or_default()
        );
    });
    Ok(0)
}
