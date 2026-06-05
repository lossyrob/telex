//! Report whether an address is occupied, by TTL heartbeat freshness.
use anyhow::Result;
use clap::Parser;
use telex_spike::connect;

#[derive(Parser)]
struct Args {
    #[arg(long)]
    address: String,
    #[arg(long, default_value_t = 15)]
    window_secs: i64,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let client = connect().await?;
    let row = client
        .query_opt(
            "SELECT occupant, \
                    EXTRACT(EPOCH FROM (now()-heartbeat_at))::float8 AS age, \
                    (heartbeat_at > now() - make_interval(secs => $2::double precision)) AS occupied \
             FROM leases WHERE address=$1",
            &[&args.address, &(args.window_secs as f64)],
        )
        .await?;
    match row {
        None => println!(
            "{}",
            serde_json::json!({"address": args.address, "occupied": false, "reason": "no lease"})
        ),
        Some(r) => {
            let occupied: bool = r.get("occupied");
            let age: f64 = r.get("age");
            let occupant: Option<String> = r.get("occupant");
            println!(
                "{}",
                serde_json::json!({
                    "address": args.address, "occupied": occupied,
                    "age_secs": age, "occupant": occupant
                })
            );
        }
    }
    Ok(())
}
