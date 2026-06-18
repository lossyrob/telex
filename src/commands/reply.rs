use anyhow::{anyhow, Result};

use crate::cli::{Ctx, ReplyArgs};
use crate::identity::{resolve_from, FromPlan};
use crate::model::{now_ms, Attention, NewMessage};
use crate::output::emit;

pub async fn run(ctx: &Ctx, args: ReplyArgs) -> Result<i32> {
    // Resolve the body up front so an invalid --body/--body-file combination fails before any
    // backend lookups or address creation.
    let body = crate::commands::resolve_body(args.body.clone(), args.body_file.clone())?;
    let attention = Attention::parse(&args.attention)?;

    // Resolve `from` (and apply the un-repliable / ambiguity guardrails) before any backend side
    // effect, so a refused reply neither opens the store nor creates the reply destination.
    let backend_key = ctx.store_key()?;
    let from = match resolve_from(
        args.from.as_deref(),
        ctx.address.as_deref(),
        &backend_key,
        args.requires_disposition,
        attention,
    )
    .await
    {
        FromPlan::Refuse { receipt, message } => {
            let out = serde_json::json!({
                "receipt": receipt,
                "reply_to": args.to_message,
                "reason": message,
            });
            emit(ctx.fmt, &out, || {
                println!("refused: {message}");
            });
            return Ok(4);
        }
        FromPlan::Proceed { from, warning } => {
            if let Some(w) = warning {
                eprintln!("telex: warning: {w}");
            }
            from
        }
    };

    let backend = ctx.backend().await?;
    let parent = backend
        .get_message(args.to_message)
        .await?
        .ok_or_else(|| anyhow!("message {} not found", args.to_message))?;

    // Reply goes back to the parent's sender.
    let to = parent
        .from_addr
        .clone()
        .ok_or_else(|| anyhow!("message {} has no from address to reply to", parent.id))?;

    let subject = args
        .subject
        .clone()
        .or_else(|| parent.subject.as_ref().map(|s| format!("Re: {s}")));

    backend.ensure_address(&to, None, None, None).await?;

    let new = NewMessage {
        parent_id: Some(parent.id),
        from_addr: from.clone(),
        to_addr: to.clone(),
        cc: None,
        kind: args.kind.clone(),
        attention,
        requires_disposition: args.requires_disposition,
        subject,
        body,
        metadata: None,
        sent_at_ms: now_ms(),
    };
    let row = backend.insert_message(&new).await?;
    backend.notify_new(&to, row.id, row.sent_at_ms).await.ok();

    let occ = backend.occupancy(&to, ctx.cfg.liveness_window_secs).await?;
    let delivery = if occ.occupied {
        "delivered"
    } else {
        "queued-unoccupied"
    };
    let receipt = serde_json::json!({
        "receipt": delivery,
        "id": row.id,
        "thread_id": row.thread_id,
        "parent_id": row.parent_id,
        "to": to,
        "from": from,
        "occupied": occ.occupied,
    });
    emit(ctx.fmt, &receipt, || {
        println!(
            "{delivery}: id={} thread={} reply-to={} to={}",
            row.id, row.thread_id, parent.id, to
        );
    });
    Ok(0)
}
