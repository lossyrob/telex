//! Insert a message for an address via the chosen backend, stamp `sent_at_ms`,
//! and fire a best-effort push notify (no-op on SQLite).
use anyhow::Result;
use clap::Parser;
use telex_spike::{make_backend, now_ms};

#[derive(Parser)]
struct Args {
    #[arg(long)]
    address: String,
    #[arg(long)]
    body: String,
    #[arg(long, default_value = "background")]
    attention: String,
    #[arg(long, default_value = "postgres")]
    backend: String,
    #[arg(long, default_value = "telex-spike.db")]
    db: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let backend = make_backend(&args.backend, &args.db).await?;
    let sent_at_ms = now_ms();
    let id = backend
        .insert_message(&args.address, &args.body, &args.attention, sent_at_ms)
        .await?;
    backend.notify_new(&args.address, id, sent_at_ms).await?;
    println!("inserted message id={id} to {} via {}", args.address, backend.kind());
    Ok(())
}
