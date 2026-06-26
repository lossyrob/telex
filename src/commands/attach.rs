use anyhow::{anyhow, Result};

use crate::cli::{AttachArgs, Ctx};
use crate::daemon_ipc::{Request, Response, WatchPidSpec};
use crate::identity::{default_occupant, resolve_session_id};
use crate::output::emit;
use crate::session_watch::{resolve_session_pid, SessionBinding, UnboundReason};

/// `telex attach` is a one-shot daemon Register. Deprecated holder-loop flags
/// remain accepted by clap for compatibility, but they no longer start a
/// resident process or write holder-registry records.
pub async fn run(ctx: &Ctx, args: AttachArgs) -> Result<i32> {
    let address = ctx.cfg.require_address(&ctx.address)?;
    let store_key = ctx.store_key()?;
    let session_id = resolve_session_id(args.session.as_deref())?;
    let occupant = args.occupant.clone().unwrap_or_else(default_occupant);

    if args.push {
        eprintln!("telex: warning: --push is ignored by one-shot daemon attach");
    }

    let watch_pids = resolve_watch_pids(&args);
    let register = Request::Register {
        store_key: store_key.clone(),
        address: address.clone(),
        session_id: session_id.clone(),
        occupant: occupant.clone(),
        description: args.description.clone(),
        scope: args.scope.clone(),
        tags: args.tags.clone(),
        watch_pids,
        recovery: false,
    };
    let response = crate::daemon::request_connect_or_spawn(&store_key, &register).await?;

    match response {
        Response::Registered {
            lease_epoch,
            owner_instance_id,
        } => {
            let out = serde_json::json!({
                "address": address,
                "store_key": store_key,
                "session_id": session_id,
                "occupant": occupant,
                "lease_epoch": lease_epoch,
                "owner_instance_id": owner_instance_id,
            });
            emit(ctx.fmt, &out, || {
                println!("attached {address} session={session_id} epoch={lease_epoch}");
            });
            Ok(0)
        }
        Response::Error { code, message, .. } => Err(anyhow!("{code}: {message}")),
        other => Err(anyhow!("unexpected daemon register response: {other:?}")),
    }
}

fn resolve_watch_pids(args: &AttachArgs) -> Vec<WatchPidSpec> {
    let mut watch_pids = args.watch_pid.clone();
    let env_pid = std::env::var("TELEX_SESSION_PID").ok();
    match resolve_session_pid(args.no_session_bind, args.session_pid, env_pid.as_deref()) {
        SessionBinding::Bound(pid) => watch_pids.push(WatchPidSpec::anchor(pid)),
        SessionBinding::Unbound(UnboundReason::MalformedEnv(raw)) => {
            eprintln!("telex: warning: ignoring malformed TELEX_SESSION_PID={raw:?}");
        }
        SessionBinding::Unbound(_) => {}
    }
    watch_pids
}
