use anyhow::{anyhow, Result};

use crate::cli::{Ctx, DaemonCmd, DaemonResetArgs, DaemonSessionEndArgs};
use crate::daemon_ipc::{Request, Response};
use crate::identity::resolve_session_id;
use crate::output::emit;

pub async fn run(ctx: &Ctx, cmd: DaemonCmd) -> Result<i32> {
    match cmd {
        DaemonCmd::Serve => {
            crate::daemon::serve().await?;
            Ok(0)
        }
        DaemonCmd::Status => status(ctx).await,
        DaemonCmd::Version => version(ctx),
        DaemonCmd::Reset(args) => reset(ctx, args).await,
        DaemonCmd::SessionEnd(args) => session_end(ctx, args).await,
        DaemonCmd::Stop(args) => {
            if !args.drain {
                return Err(anyhow!("only `telex daemon stop --drain` is supported"));
            }
            stop_drain(ctx).await
        }
    }
}

fn version(ctx: &Ctx) -> Result<i32> {
    let info = crate::daemon::daemon_version_metadata();
    emit(ctx.fmt, &info, || {
        println!("daemon_version {}", info.daemon_version);
        println!(
            "protocol {}.{}",
            info.protocol_version.major, info.protocol_version.minor
        );
        println!("auth_policy {}", info.auth_policy_version);
    });
    Ok(0)
}

async fn reset(ctx: &Ctx, args: DaemonResetArgs) -> Result<i32> {
    let address_arg = args.address.or_else(|| ctx.address.clone());
    let address = ctx.cfg.require_address(&address_arg)?;
    let paths = crate::daemon::DaemonPaths::current()?;
    let cap = crate::daemon::read_cap_file(&paths.cap_path)?;
    let store_key = ctx.store_key()?;
    let mut client = crate::daemon::connect_existing(&store_key).await?;
    let response = client
        .request(&Request::Reset {
            store_key: store_key.clone(),
            address: address.clone(),
            proof: Some(cap.admin_cap),
        })
        .await?;
    match response {
        Response::Ack { .. } => {
            emit(
                ctx.fmt,
                &serde_json::json!({"reset": true, "address": address, "store_key": store_key}),
                || {
                    println!("daemon reset {address}");
                },
            );
            Ok(0)
        }
        Response::Error { code, message, .. } => Err(anyhow!("{code}: {message}")),
        other => Err(anyhow!("unexpected daemon reset response: {other:?}")),
    }
}

async fn session_end(ctx: &Ctx, args: DaemonSessionEndArgs) -> Result<i32> {
    let paths = crate::daemon::DaemonPaths::current()?;
    let cap = crate::daemon::read_cap_file(&paths.cap_path)?;
    let store_key = ctx.store_key()?;
    let session_id = resolve_session_id(args.session.as_deref())?;
    let mut client = crate::daemon::connect_existing(&store_key).await?;
    let response = client
        .request(&Request::SessionEnd {
            store_key: store_key.clone(),
            session_id: session_id.clone(),
            proof: Some(cap.admin_cap),
        })
        .await?;
    match response {
        Response::Ack { .. } => {
            emit(
                ctx.fmt,
                &serde_json::json!({"session_end": true, "session_id": session_id, "store_key": store_key}),
                || {
                    println!("daemon session-end {session_id}");
                },
            );
            Ok(0)
        }
        Response::Error { code, message, .. } => Err(anyhow!("{code}: {message}")),
        other => Err(anyhow!("unexpected daemon session-end response: {other:?}")),
    }
}

async fn status(ctx: &Ctx) -> Result<i32> {
    let paths = crate::daemon::DaemonPaths::current()?;
    let store_key = ctx.store_key()?;
    match crate::daemon::connect_existing(&store_key).await {
        Ok(mut client) => {
            let response = client
                .request(&Request::Status {
                    store_key: Some(store_key),
                    detail: false,
                    proof: None,
                })
                .await?;
            match response {
                Response::StatusReport { status } => {
                    emit(ctx.fmt, &status, || {
                        println!("daemon  running");
                        println!("version {}", status.daemon_version);
                        println!("instance {}", status.instance_id);
                        println!("singleton {}", status.singleton_key);
                    });
                    Ok(0)
                }
                Response::Error { code, message, .. } => Err(anyhow!("{code}: {message}")),
                other => Err(anyhow!("unexpected daemon status response: {other:?}")),
            }
        }
        Err(crate::daemon::DaemonError::NotRunning(_)) => {
            let info = crate::daemon::local_status_metadata(&paths);
            emit(ctx.fmt, &info, || {
                println!("daemon  not running");
                println!("endpoint {}", paths.endpoint.display());
                println!("cap      {}", paths.cap_path.display());
            });
            Ok(0)
        }
        Err(e) => Err(e.into()),
    }
}

async fn stop_drain(ctx: &Ctx) -> Result<i32> {
    let paths = crate::daemon::DaemonPaths::current()?;
    let cap = crate::daemon::read_cap_file(&paths.cap_path)?;
    let store_key = ctx.store_key()?;
    let mut client = crate::daemon::connect_existing(&store_key).await?;
    let response = client
        .request(&Request::Drain {
            proof: Some(cap.admin_cap),
        })
        .await?;
    match response {
        Response::Ack { .. } => {
            emit(ctx.fmt, &serde_json::json!({"draining": true}), || {
                println!("daemon drain requested");
            });
            Ok(0)
        }
        Response::Error { code, message, .. } => Err(anyhow!("{code}: {message}")),
        other => Err(anyhow!("unexpected daemon drain response: {other:?}")),
    }
}
