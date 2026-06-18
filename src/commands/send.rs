use anyhow::Result;

use crate::cli::{Ctx, SendArgs};
use crate::identity::{resolve_from, FromPlan};
use crate::model::{now_ms, Attention, NewMessage, STATUS_RETIRED};
use crate::output::emit;

pub async fn run(ctx: &Ctx, args: SendArgs) -> Result<i32> {
    // Resolve the body before any backend side effects so an invalid --body/--body-file
    // combination fails without auto-creating the destination address.
    let body = crate::commands::resolve_body(args.body.clone(), args.body_file.clone())?;
    let attention = Attention::parse(&args.attention)?;

    // Resolve `from` (and apply the un-repliable / ambiguity guardrails) before any backend side
    // effect too, so a refused send neither opens the store nor auto-creates the destination.
    let backend_key = ctx.resolved()?.1.target();
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
                "to": args.to,
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

    // Reject sends to a retired address; auto-create unknown addresses as a queue target.
    match backend.get_address(&args.to).await? {
        Some(addr) if addr.status == STATUS_RETIRED => {
            let receipt = serde_json::json!({
                "receipt": "rejected-retired",
                "to": args.to,
            });
            emit(ctx.fmt, &receipt, || {
                println!("rejected: address {} is retired", args.to);
            });
            return Ok(3);
        }
        Some(_) => {}
        None => backend.ensure_address(&args.to, None, None, None).await?,
    }

    let new = NewMessage {
        parent_id: None,
        from_addr: from.clone(),
        to_addr: args.to.clone(),
        cc: args.cc.clone(),
        kind: args.kind.clone(),
        attention,
        requires_disposition: args.requires_disposition,
        subject: args.subject.clone(),
        body,
        metadata: args.metadata.clone(),
        sent_at_ms: now_ms(),
    };
    let row = backend.insert_message(&new).await?;
    backend
        .notify_new(&args.to, row.id, row.sent_at_ms)
        .await
        .ok();

    let occ = backend
        .occupancy(&args.to, ctx.cfg.liveness_window_secs)
        .await?;
    let delivery = if occ.occupied {
        "delivered"
    } else {
        "queued-unoccupied"
    };

    let receipt = serde_json::json!({
        "receipt": delivery,
        "id": row.id,
        "thread_id": row.thread_id,
        "to": args.to,
        "from": from,
        "attention": row.attention,
        "requires_disposition": row.requires_disposition,
        "occupied": occ.occupied,
    });
    emit(ctx.fmt, &receipt, || {
        println!(
            "{delivery}: id={} thread={} to={}",
            row.id, row.thread_id, args.to
        );
    });
    Ok(0)
}
