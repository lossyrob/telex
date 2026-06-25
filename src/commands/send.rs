use anyhow::{anyhow, Result};

use crate::cli::{Ctx, SendArgs};
use crate::daemon_ipc::{NeedsAttachReason, Request, Response, SentReceipt, ERROR_NEEDS_ATTACH};
use crate::identity::{default_occupant, resolve_session_id};
use crate::model::Attention;
use crate::output::emit;

pub async fn run(ctx: &Ctx, args: SendArgs) -> Result<i32> {
    let body = crate::commands::resolve_body(args.body.clone(), args.body_file.clone())?;
    let _ = Attention::parse(&args.attention)?;
    let store_key = ctx.store_key()?;
    let session_id = resolve_session_id(args.session.as_deref())?;
    let from_addr = args.from.clone().or_else(|| ctx.address.clone());

    let mut retried_after_attach = false;
    loop {
        let response = send_once(
            &store_key,
            &session_id,
            from_addr.clone(),
            &args,
            body.clone(),
        )
        .await?;
        match response {
            Response::Sent { receipt } => {
                emit_receipt(ctx, &receipt);
                return Ok(0);
            }
            Response::Error {
                code,
                message,
                needs_attach_reason,
            } if code == ERROR_NEEDS_ATTACH => {
                if needs_attach_reason == Some(NeedsAttachReason::DeliberatelyDetached) {
                    return Err(anyhow!("{code}: {message}"));
                }
                if retried_after_attach {
                    return Err(anyhow!("{code}: {message}"));
                }
                let attach_addr = from_addr.clone().ok_or_else(|| {
                    anyhow!(
                        "NeedsAttach but no address is available to re-attach; pass --address or --from"
                    )
                })?;
                register_for_retry(&store_key, &session_id, &attach_addr).await?;
                retried_after_attach = true;
            }
            Response::Error { code, message, .. } => return Err(anyhow!("{code}: {message}")),
            other => return Err(anyhow!("unexpected daemon send response: {other:?}")),
        }
    }
}

async fn send_once(
    store_key: &str,
    session_id: &str,
    from_addr: Option<String>,
    args: &SendArgs,
    body: String,
) -> Result<Response> {
    let mut client = crate::daemon::connect_or_spawn(store_key).await?;
    Ok(client
        .request(&Request::Send {
            store_key: store_key.to_string(),
            session_id: session_id.to_string(),
            from_addr,
            to_addr: args.to.clone(),
            cc: args.cc.clone(),
            kind: args.kind.clone(),
            attention: args.attention.clone(),
            requires_disposition: args.requires_disposition,
            subject: args.subject.clone(),
            body,
            metadata: args.metadata.clone(),
        })
        .await?)
}

async fn register_for_retry(store_key: &str, session_id: &str, address: &str) -> Result<()> {
    let mut client = crate::daemon::connect_or_spawn(store_key).await?;
    let response = client
        .request(&Request::Register {
            store_key: store_key.to_string(),
            address: address.to_string(),
            session_id: session_id.to_string(),
            occupant: default_occupant(),
            description: None,
            scope: None,
            tags: None,
            watch_pids: Vec::new(),
            recovery: true,
        })
        .await?;
    match response {
        Response::Registered { .. } => Ok(()),
        Response::Error { code, message, .. } => Err(anyhow!("{code}: {message}")),
        other => Err(anyhow!("unexpected daemon register response: {other:?}")),
    }
}

fn emit_receipt(ctx: &Ctx, receipt: &SentReceipt) {
    emit(ctx.fmt, receipt, || {
        println!(
            "{}: id={} thread={} to={}",
            receipt.receipt, receipt.id, receipt.thread_id, receipt.to
        );
    });
}
