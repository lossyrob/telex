use anyhow::{anyhow, Result};
use serde_json::json;

use crate::cli::{Ctx, DetachArgs};
use crate::daemon_ipc::{Request, Response};
use crate::identity::resolve_session_id;
use crate::output::emit;

pub async fn run(ctx: &Ctx, args: DetachArgs) -> Result<i32> {
    let address = ctx.cfg.require_address(&ctx.address)?;
    let store_key = ctx.store_key()?;
    let session_id = resolve_session_id(args.session.as_deref())?;

    let mut client = crate::daemon::connect_or_spawn(&store_key).await?;
    let response = client
        .request(&Request::Detach {
            store_key: store_key.clone(),
            session_id: session_id.clone(),
            address: address.clone(),
        })
        .await?;

    match response {
        Response::Ack { message, .. } => {
            let out = json!({
                "address": address,
                "store_key": store_key,
                "session_id": session_id,
                "detached": true,
                "message": message,
            });
            emit(ctx.fmt, &out, || {
                println!("detached {address} session={session_id}");
            });
            Ok(0)
        }
        Response::Error { code, message } => Err(anyhow!("{code}: {message}")),
        other => Err(anyhow!("unexpected daemon detach response: {other:?}")),
    }
}
