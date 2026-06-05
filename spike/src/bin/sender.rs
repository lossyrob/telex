//! Insert a message for an address (simulates an incoming Telex message).
//! Stamps `sent_at_ms` (client wall clock) and fires a NOTIFY so push-mode
//! listeners can wake immediately.
use anyhow::Result;
use clap::Parser;
use serde_json::json;
use telex_spike::{connect, now_ms, NOTIFY_CHANNEL};

#[derive(Parser)]
struct Args {
    #[arg(long)]
    address: String,
    #[arg(long)]
    body: String,
    #[arg(long, default_value = "background")]
    attention: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let client = connect().await?;
    let sent_at_ms = now_ms();
    let row = client
        .query_one(
            "INSERT INTO messages(address, body, attention, sent_at_ms) \
             VALUES ($1,$2,$3,$4) RETURNING id",
            &[&args.address, &args.body, &args.attention, &sent_at_ms],
        )
        .await?;
    let id: i64 = row.get("id");
    let payload = json!({"address": args.address, "id": id, "sent_at_ms": sent_at_ms}).to_string();
    client
        .execute("SELECT pg_notify($1, $2)", &[&NOTIFY_CHANNEL, &payload])
        .await?;
    println!("inserted message id={id} to {} (notified)", args.address);
    Ok(())
}

