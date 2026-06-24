use anyhow::{anyhow, Result};

use crate::cli::{Ctx, DispArgs};
use crate::config;
use crate::daemon_ipc::{Request, Response};
use crate::identity::resolve_session_id;
use crate::model::Disposition;
use crate::output::emit;

pub async fn ack(ctx: &Ctx, args: DispArgs) -> Result<i32> {
    let address = args
        .recipient
        .clone()
        .or_else(|| ctx.address.clone())
        .ok_or_else(|| anyhow!("ack requires --recipient or global --address"))?;
    let store_key = ctx.store_key()?;
    let session_id = resolve_session_id(args.session.as_deref())?;

    let response = ack_once(&store_key, &session_id, &address, args.id).await?;
    match response {
        Response::Ack {
            delivery_outcome,
            address,
            message_id,
            lease_epoch,
            ..
        } => {
            let out = serde_json::json!({
                "state": "acknowledged",
                "message_id": message_id.unwrap_or(args.id),
                "recipient": address,
                "delivery_outcome": delivery_outcome,
                "lease_epoch": lease_epoch,
            });
            emit(ctx.fmt, &out, || {
                println!(
                    "acknowledged #{} for {} ({:?})",
                    args.id,
                    out.get("recipient").and_then(|v| v.as_str()).unwrap_or("?"),
                    delivery_outcome
                );
            });
            Ok(0)
        }
        Response::Error { code, message } => Err(anyhow!("{code}: {message}")),
        other => Err(anyhow!("unexpected daemon ack response: {other:?}")),
    }
}

async fn ack_once(
    store_key: &str,
    session_id: &str,
    address: &str,
    message_id: i64,
) -> Result<Response> {
    let mut client = crate::daemon::connect_or_spawn(store_key).await?;
    Ok(client
        .request(&Request::Ack {
            store_key: store_key.to_string(),
            session_id: session_id.to_string(),
            address: address.to_string(),
            message_id,
        })
        .await?)
}

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
