use anyhow::{anyhow, Result};
use serde_json::json;

use crate::cli::{Ctx, StationCmd, StationStopArgs};
use crate::daemon_ipc::{Request, Response};
use crate::identity::resolve_session_id;
use crate::output::emit;

pub async fn run(ctx: &Ctx, cmd: StationCmd) -> Result<i32> {
    match cmd {
        StationCmd::Stop(args) => stop(ctx, args).await,
    }
}

async fn stop(ctx: &Ctx, args: StationStopArgs) -> Result<i32> {
    let address = ctx.cfg.require_address(&ctx.address)?;
    let store_key = ctx.store_key()?;
    let session_id = resolve_session_id(args.session.as_deref())?;

    let mut client = crate::daemon::connect_or_spawn(&store_key).await?;
    let response = client
        .request(&Request::StationStop {
            store_key: store_key.clone(),
            session_id: session_id.clone(),
            address: address.clone(),
            wait_grace_ms: args.wait_grace_ms,
        })
        .await?;

    match response {
        Response::StationStopped {
            detached,
            waiters_before,
            waiters_after,
            live_waiters,
            message,
            lease_epoch,
            ..
        } => {
            let out = json!({
                "address": address,
                "store_key": store_key,
                "session_id": session_id,
                "detached": detached,
                "message": message,
                "lease_epoch": lease_epoch,
                "waiters_before": waiters_before,
                "waiters_after": waiters_after,
                "live_waiters": live_waiters,
            });
            emit(ctx.fmt, &out, || {
                if waiters_after == 0 {
                    println!(
                        "station stopped {address} session={session_id} waiters={waiters_before}->0"
                    );
                } else {
                    println!(
                        "station stopped {address} session={session_id} waiters={waiters_before}->{waiters_after}"
                    );
                }
            });
            Ok(0)
        }
        Response::Error { code, message, .. } => Err(anyhow!("{code}: {message}")),
        other => Err(anyhow!(
            "unexpected daemon station-stop response: {other:?}"
        )),
    }
}
