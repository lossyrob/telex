//! Latency benchmark: watch the SAME inserted messages via BOTH delivery paths
//! concurrently (poll-with-cursor vs LISTEN/NOTIFY push) and report per-message
//! and summary lag, measured from the sender's `sent_at_ms` (same-machine wall
//! clock) to each path's receive time. This isolates backend delivery lag from
//! agent-wake latency, which sits above the telex layer.

use anyhow::Result;
use clap::Parser;
use futures_util::{stream, StreamExt};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use telex_spike::{connect, make_tls, now_ms, pg_config, NotifyPayload, NOTIFY_CHANNEL, SCHEMA};
use tokio::sync::Mutex;
use tokio_postgres::AsyncMessage;

#[derive(Parser)]
struct Args {
    #[arg(long, default_value = "workstream:bench/node:lat")]
    address: String,
    #[arg(long, default_value_t = 10)]
    count: usize,
    #[arg(long, default_value_t = 1500)]
    interval_ms: u64,
    /// Poll-path interval, matching the holder's default.
    #[arg(long, default_value_t = 1000)]
    poll_ms: u64,
}

#[derive(Default)]
struct Data {
    sent: HashMap<i64, i64>,      // id -> sent_at_ms
    poll_recv: HashMap<i64, i64>, // id -> recv wall ms (poll path)
    push_recv: HashMap<i64, i64>, // id -> recv wall ms (push path)
    order: Vec<i64>,              // ids in send order
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let data = Arc::new(Mutex::new(Data::default()));

    // Ensure schema + address exist.
    let setup = connect().await?;
    setup.batch_execute(SCHEMA).await?;
    setup
        .execute(
            "INSERT INTO addresses(address, description) VALUES ($1,$2) \
             ON CONFLICT (address) DO NOTHING",
            &[&args.address, &"bench address"],
        )
        .await?;

    // ---- PUSH path: a LISTEN connection driven by poll_message ----
    {
        let (listen_client, listen_conn) = pg_config()?.connect(make_tls()?).await?;
        let data = data.clone();
        let address = args.address.clone();
        // Drive the connection + forward notifications.
        tokio::spawn(async move {
            let mut conn = listen_conn;
            let messages = stream::poll_fn(move |cx| conn.poll_message(cx));
            tokio::pin!(messages);
            while let Some(msg) = messages.next().await {
                match msg {
                    Ok(AsyncMessage::Notification(note)) => {
                        let recv = now_ms();
                        if let Ok(p) = serde_json::from_str::<NotifyPayload>(note.payload()) {
                            if p.address == address {
                                let mut d = data.lock().await;
                                d.push_recv.entry(p.id).or_insert(recv);
                            }
                        }
                    }
                    Ok(_) => {}
                    Err(e) => {
                        eprintln!("[bench] listen conn error: {e}");
                        break;
                    }
                }
            }
        });
        listen_client
            .batch_execute(&format!("LISTEN {NOTIFY_CHANNEL}"))
            .await?;
        // keep the client alive for the duration
        Box::leak(Box::new(listen_client));
    }

    // ---- POLL path: cursor-based, on its own connection ----
    {
        let poll = connect().await?;
        let data = data.clone();
        let address = args.address.clone();
        let poll_ms = args.poll_ms;
        let mut cursor: i64 = poll
            .query_one(
                "SELECT COALESCE(MAX(id),0) m FROM messages WHERE address=$1",
                &[&address],
            )
            .await?
            .get("m");
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_millis(poll_ms));
            loop {
                tick.tick().await;
                let rows = match poll
                    .query(
                        "SELECT id FROM messages WHERE address=$1 AND id>$2 ORDER BY id",
                        &[&address, &cursor],
                    )
                    .await
                {
                    Ok(r) => r,
                    Err(e) => {
                        eprintln!("[bench] poll error: {e}");
                        continue;
                    }
                };
                let recv = now_ms();
                let mut d = data.lock().await;
                for row in rows {
                    let id: i64 = row.get("id");
                    cursor = id;
                    d.poll_recv.entry(id).or_insert(recv);
                }
            }
        });
    }

    // Give LISTEN a moment to be fully established.
    tokio::time::sleep(Duration::from_millis(500)).await;

    // ---- SENDER: insert + NOTIFY, spaced out ----
    let sender = connect().await?;
    eprintln!(
        "[bench] sending {} messages, {} ms apart, poll interval {} ms",
        args.count, args.interval_ms, args.poll_ms
    );
    for i in 0..args.count {
        let sent = now_ms();
        let body = format!("bench message {i}");
        let row = sender
            .query_one(
                "INSERT INTO messages(address, body, attention, sent_at_ms) \
                 VALUES ($1,$2,'background',$3) RETURNING id",
                &[&args.address, &body, &sent],
            )
            .await?;
        let id: i64 = row.get("id");
        let payload =
            serde_json::json!({"address": args.address, "id": id, "sent_at_ms": sent}).to_string();
        sender
            .execute("SELECT pg_notify($1,$2)", &[&NOTIFY_CHANNEL, &payload])
            .await?;
        {
            let mut d = data.lock().await;
            d.sent.insert(id, sent);
            d.order.push(id);
        }
        tokio::time::sleep(Duration::from_millis(args.interval_ms)).await;
    }

    // Let the poll path catch the last message (one extra interval + slack).
    tokio::time::sleep(Duration::from_millis(args.poll_ms + 2000)).await;

    report(&*data.lock().await);
    Ok(())
}

fn report(d: &Data) {
    println!("\n=== per-message lag (ms from sent_at to receive) ===");
    println!("{:>6}  {:>10}  {:>10}  {:>12}", "id", "push_ms", "poll_ms", "poll-push");
    let mut push_lags = Vec::new();
    let mut poll_lags = Vec::new();
    for &id in &d.order {
        let sent = d.sent[&id];
        let push = d.push_recv.get(&id).map(|r| r - sent);
        let poll = d.poll_recv.get(&id).map(|r| r - sent);
        if let Some(p) = push {
            push_lags.push(p);
        }
        if let Some(p) = poll {
            poll_lags.push(p);
        }
        let diff = match (push, poll) {
            (Some(a), Some(b)) => format!("{}", b - a),
            _ => "-".into(),
        };
        println!(
            "{:>6}  {:>10}  {:>10}  {:>12}",
            id,
            push.map(|v| v.to_string()).unwrap_or_else(|| "MISS".into()),
            poll.map(|v| v.to_string()).unwrap_or_else(|| "MISS".into()),
            diff
        );
    }
    println!("\n=== summary ===");
    print_stats("push (LISTEN/NOTIFY)", &mut push_lags, d.order.len());
    print_stats("poll (cursor)", &mut poll_lags, d.order.len());
}

fn print_stats(label: &str, lags: &mut [i64], total: usize) {
    if lags.is_empty() {
        println!("{label:>22}: no samples");
        return;
    }
    lags.sort_unstable();
    let n = lags.len();
    let min = lags[0];
    let max = lags[n - 1];
    let median = lags[n / 2];
    let mean = lags.iter().sum::<i64>() as f64 / n as f64;
    println!(
        "{label:>22}: n={n}/{total}  min={min}  median={median}  mean={mean:.1}  max={max}  (ms)"
    );
}
