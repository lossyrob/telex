//! Report whether an address is occupied, by TTL heartbeat freshness, via the
//! chosen backend.
use anyhow::Result;
use clap::Parser;
use telex_spike::make_backend;

#[derive(Parser)]
struct Args {
    #[arg(long)]
    address: String,
    #[arg(long, default_value_t = 15)]
    window_secs: i64,
    #[arg(long, default_value = "postgres")]
    backend: String,
    #[arg(long, default_value = "telex-spike.db")]
    db: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let backend = make_backend(&args.backend, &args.db).await?;
    let occ = backend.occupancy(&args.address, args.window_secs).await?;
    println!(
        "{}",
        serde_json::json!({
            "address": args.address,
            "backend": backend.kind(),
            "occupied": occ.occupied,
            "age_secs": occ.age_secs,
            "occupant": occ.occupant
        })
    );
    Ok(())
}
