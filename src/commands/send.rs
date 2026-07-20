use anyhow::{anyhow, Result};

use crate::cli::{Ctx, SendArgs};
use crate::daemon_ipc::{NeedsAttachReason, Request, Response, SentReceipt, ERROR_NEEDS_ATTACH};
use crate::identity::{default_occupant, resolve_session_id};
use crate::model::Attention;
use crate::output::emit;

pub async fn run(ctx: &Ctx, args: SendArgs) -> Result<i32> {
    let body =
        crate::commands::resolve_body(args.body.clone(), args.body_file.clone(), args.body_stdin)?;
    let _ = Attention::parse(&args.attention)?;
    let store_key = ctx.store_key()?;
    let session_id = resolve_session_id(args.session.as_deref())?;
    let from_addr = args.from.clone().or_else(|| ctx.address.clone());

    // This narrow branch is consumed only by the experimental Watcher subprocess.  It must be
    // selected by an explicit session rather than by an ambient TELEX_SESSION_ID, otherwise a
    // normal `telex send` could silently lose its established recovery behavior.
    if watcher_private_send_active(
        std::env::var("TELEX_WATCHER_INTERNAL_SEND_ONCE_V1")
            .ok()
            .as_deref(),
        args.session.as_deref(),
    ) {
        let response = send_once(&store_key, &session_id, from_addr, &args, body).await?;
        println!("{}", watcher_private_response_json(&response)?);
        return Ok(0);
    }

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

fn watcher_private_send_active(token: Option<&str>, explicit_session: Option<&str>) -> bool {
    matches!((token, explicit_session), (Some(token), Some(session)) if token == session)
}

fn watcher_private_response_json(response: &Response) -> Result<String> {
    Ok(serde_json::to_string(response)?)
}

async fn send_once(
    store_key: &str,
    session_id: &str,
    from_addr: Option<String>,
    args: &SendArgs,
    body: String,
) -> Result<Response> {
    let mut client = crate::daemon::connect_existing(store_key).await?;
    Ok(client
        .request(&Request::Send {
            store_key: store_key.to_string(),
            session_id: session_id.to_string(),
            from_addr,
            to_addr: args.to.clone(),
            cc: normalize_cc(&args.cc),
            kind: args.kind.clone(),
            attention: args.attention.clone(),
            requires_disposition: args.requires_disposition,
            subject: args.subject.clone(),
            body,
            metadata: args.metadata.clone(),
        })
        .await?)
}

fn normalize_cc(values: &[String]) -> Option<String> {
    let mut seen = std::collections::BTreeSet::new();
    let mut out = Vec::new();
    for value in values {
        for part in value.split(',') {
            let trimmed = part.trim();
            if trimmed.is_empty() {
                continue;
            }
            if seen.insert(trimmed.to_string()) {
                out.push(trimmed.to_string());
            }
        }
    }
    if out.is_empty() {
        None
    } else {
        Some(out.join(","))
    }
}

async fn register_for_retry(store_key: &str, session_id: &str, address: &str) -> Result<()> {
    let mut client = crate::daemon::connect_existing(store_key).await?;
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
            on_deliver: None,
            replace_on_deliver: false,
            on_deliver_wake_on_cc: false,
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

#[cfg(test)]
mod tests {
    use super::{watcher_private_response_json, watcher_private_send_active};
    use crate::daemon_ipc::{NeedsAttachReason, Response, ERROR_NEEDS_ATTACH};

    #[test]
    fn watcher_private_mode_requires_an_exact_explicit_session_match() {
        assert!(watcher_private_send_active(
            Some("runtime-1"),
            Some("runtime-1")
        ));
        assert!(!watcher_private_send_active(
            Some("runtime-1"),
            Some("runtime-2")
        ));
        assert!(!watcher_private_send_active(Some("runtime-1"), None));
        assert!(!watcher_private_send_active(None, Some("runtime-1")));
    }

    #[test]
    fn watcher_private_result_preserves_typed_needs_attach_reason() {
        let response = Response::Error {
            code: ERROR_NEEDS_ATTACH.to_string(),
            message: "membership was lost".to_string(),
            needs_attach_reason: Some(NeedsAttachReason::RestartLost),
        };

        let json = watcher_private_response_json(&response).unwrap();
        assert!(json.contains(r#""type":"error""#));
        assert!(json.contains(r#""needs_attach_reason":"restart_lost""#));
    }
}
