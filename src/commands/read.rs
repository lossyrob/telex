use anyhow::{anyhow, Result};
use serde_json::json;

use crate::cli::{Ctx, ReadArgs};
use crate::model::{cc_recipients, delivery_role, requires_disposition_for_recipient};
use crate::output::emit;

pub async fn run(ctx: &Ctx, args: ReadArgs) -> Result<i32> {
    let backend = ctx.backend().await?;
    let msg = backend
        .get_message(args.id)
        .await?
        .ok_or_else(|| anyhow!("message {} not found", args.id))?;
    let dispositions = backend.dispositions_for(args.id).await?;

    let mut out = json!({
        "message": msg,
        "dispositions": dispositions,
    });
    if let Some(address) = ctx.address.as_deref() {
        let cc = cc_recipients(msg.cc.as_deref());
        let role = delivery_role(address, &msg.to_addr, msg.cc.as_deref());
        out["delivery"] = json!({
            "delivered_to": address,
            "primary_to": msg.to_addr.clone(),
            "cc": cc,
            "delivery_role": role,
            "requires_disposition_for_current_recipient": requires_disposition_for_recipient(
                msg.requires_disposition,
                address,
                &msg.to_addr,
            ),
        });
    }

    if args.thread || args.full {
        let thread = backend.thread_messages(msg.thread_id).await?;
        if args.full {
            // Full history: every message plus its dispositions.
            let mut full = Vec::new();
            for m in &thread {
                let d = backend.dispositions_for(m.id).await?;
                full.push(json!({ "message": m, "dispositions": d }));
            }
            out["thread"] = json!(full);
        } else {
            // Compact context: id/from/subject/attention per message.
            let compact: Vec<_> = thread
                .iter()
                .map(|m| {
                    json!({
                        "id": m.id,
                        "parent_id": m.parent_id,
                        "from": m.from_addr,
                        "subject": m.subject,
                        "attention": m.attention,
                    })
                })
                .collect();
            out["thread"] = json!(compact);
        }
    }

    emit(ctx.fmt, &out, || {
        println!(
            "#{} [{}] {} -> {}",
            msg.id,
            msg.attention,
            msg.from_addr.as_deref().unwrap_or("?"),
            msg.to_addr
        );
        if let Some(s) = &msg.subject {
            println!("subject: {s}");
        }
        println!("{}", msg.body);
        if !dispositions.is_empty() {
            println!("--- dispositions ---");
            for d in &dispositions {
                println!(
                    "  {} by {} ({})",
                    d.state,
                    d.by_principal.as_deref().unwrap_or("?"),
                    d.recipient
                );
            }
        }
    });
    Ok(0)
}
