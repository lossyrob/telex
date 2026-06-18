use anyhow::Result;
use serde_json::json;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

use crate::cli::Ctx;
use crate::ipc::{self, Request};
use crate::output::emit;

/// Ask a running holder to shut down via IPC. Returns true if it acknowledged.
async fn ipc_shutdown(address: &str) -> bool {
    let stream = match ipc::connect(address).await {
        Ok(s) => s,
        Err(_) => return false,
    };
    let (read_half, mut write_half) = tokio::io::split(stream);
    let mut line = match serde_json::to_string(&Request::Shutdown) {
        Ok(s) => s,
        Err(_) => return false,
    };
    line.push('\n');
    if write_half.write_all(line.as_bytes()).await.is_err() {
        return false;
    }
    let _ = write_half.flush().await;

    // Read one frame; any successful read means the holder accepted the request.
    let mut reader = BufReader::new(read_half);
    let mut buf = String::new();
    matches!(
        tokio::time::timeout(std::time::Duration::from_secs(3), reader.read_line(&mut buf)).await,
        Ok(Ok(n)) if n > 0
    )
}

pub async fn run(ctx: &Ctx) -> Result<i32> {
    let address = ctx.cfg.require_address(&ctx.address)?;

    let via_ipc = ipc_shutdown(&address).await;

    // Also release the lease directly, in case the holder is already gone.
    let backend = ctx.backend().await?;
    let occupant = backend
        .get_lease(&address)
        .await?
        .and_then(|l| l.occupant)
        .unwrap_or_default();
    let released = backend.release_lease(&address, &occupant).await?;

    // Drop the station from the session ownership registry. The holder unregisters on its own clean
    // shutdown, but detach also covers the case where the holder is already gone. Best-effort.
    if let Err(e) = crate::session_registry::unregister_station(&address) {
        eprintln!("telex: session registry unregister failed (ignoring): {e}");
    }

    let out = json!({
        "address": address,
        "ipc_shutdown": via_ipc,
        "lease_released": released,
    });
    emit(ctx.fmt, &out, || {
        println!("detached {address} (holder-signaled={via_ipc}, lease-released={released})");
    });
    Ok(0)
}
