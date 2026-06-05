//! Resident holder: the answerback drum.
//!
//! Holds a long-lived Postgres connection, writes a TTL heartbeat for one
//! address, learns of new messages (poll-with-cursor by default, plus optional
//! LISTEN/NOTIFY push with `--push`), buffers them, and serves ephemeral waiters
//! over a local TCP socket (stand-in for a named pipe). It never takes an agent
//! turn, so it survives across waiter exits and agent turns — the property the
//! two-process split exists to prove.

use anyhow::Result;
use clap::Parser;
use futures_util::{stream, StreamExt};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use telex_spike::{
    connect, make_tls, now_ms, pg_config, Frame, NotifyPayload, Request, NOTIFY_CHANNEL,
};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{Mutex, Notify};
use tokio_postgres::{AsyncMessage, Client};

#[derive(Parser)]
struct Args {
    #[arg(long)]
    address: String,
    #[arg(long, default_value_t = 47655)]
    port: u16,
    #[arg(long, default_value_t = 5)]
    heartbeat_secs: u64,
    #[arg(long, default_value_t = 1)]
    poll_secs: u64,
    #[arg(long, default_value_t = 3)]
    keepalive_secs: u64,
    #[arg(long, default_value = "spike-holder")]
    occupant: String,
    /// Enable LISTEN/NOTIFY push delivery in addition to the poll backstop.
    #[arg(long)]
    push: bool,
    /// After N seconds, stop heartbeating and stop answering waiters — to
    /// exercise the waiter's hang detection.
    #[arg(long)]
    simulate_hang_after_secs: Option<u64>,
}

#[derive(Clone)]
struct MsgRow {
    id: i64,
    address: String,
    body: String,
    attention: String,
    sent_at_ms: i64,
    buffered_at_ms: i64,
}

struct State {
    queue: Mutex<VecDeque<MsgRow>>,
    notify: Notify,
    cursor: Mutex<i64>,
    last_heartbeat_ms: AtomicI64,
    hung: AtomicBool,
    keepalive: Duration,
}

fn hostname() -> String {
    std::env::var("COMPUTERNAME").unwrap_or_else(|_| "unknown".into())
}
fn whoami() -> String {
    std::env::var("USERNAME").unwrap_or_else(|_| "unknown".into())
}

/// Pull rows past the cursor and buffer them. Serialized by holding the cursor
/// lock across the query, so the poll and push paths cannot double-deliver.
async fn drain_new(client: &Client, address: &str, st: &State, source: &str) {
    let mut cur = st.cursor.lock().await;
    let rows = match client
        .query(
            "SELECT id, address, body, attention, COALESCE(sent_at_ms,0) AS sent_at_ms \
             FROM messages WHERE address=$1 AND id>$2 ORDER BY id",
            &[&address, &*cur],
        )
        .await
    {
        Ok(r) => r,
        Err(e) => {
            eprintln!("[holder] drain ({source}) error: {e}");
            return;
        }
    };
    if rows.is_empty() {
        return;
    }
    let recv = now_ms();
    let mut q = st.queue.lock().await;
    for row in rows {
        let m = MsgRow {
            id: row.get("id"),
            address: row.get("address"),
            body: row.get("body"),
            attention: row.get("attention"),
            sent_at_ms: row.get("sent_at_ms"),
            buffered_at_ms: recv,
        };
        let lag = if m.sent_at_ms > 0 {
            format!("{} ms", recv - m.sent_at_ms)
        } else {
            "n/a".into()
        };
        eprintln!("[holder] buffered id={} via {source} lag={lag}", m.id);
        *cur = m.id;
        q.push_back(m);
    }
    drop(q);
    drop(cur);
    st.notify.notify_waiters();
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let state = Arc::new(State {
        queue: Mutex::new(VecDeque::new()),
        notify: Notify::new(),
        cursor: Mutex::new(0),
        last_heartbeat_ms: AtomicI64::new(now_ms()),
        hung: AtomicBool::new(false),
        keepalive: Duration::from_secs(args.keepalive_secs),
    });

    let setup = connect().await?;
    setup
        .execute(
            "INSERT INTO addresses(address, description) VALUES ($1,$2) \
             ON CONFLICT (address) DO NOTHING",
            &[&args.address, &"spike address"],
        )
        .await?;
    setup
        .execute(
            "INSERT INTO leases(address, occupant, host, principal, heartbeat_at) \
             VALUES ($1,$2,$3,$4, now()) \
             ON CONFLICT (address) DO UPDATE SET occupant=excluded.occupant, \
                 host=excluded.host, principal=excluded.principal, heartbeat_at=now()",
            &[&args.address, &args.occupant, &hostname(), &whoami()],
        )
        .await?;
    // Start the cursor at the current max id so we only deliver new messages.
    {
        let max: i64 = setup
            .query_one(
                "SELECT COALESCE(MAX(id),0) m FROM messages WHERE address=$1",
                &[&args.address],
            )
            .await?
            .get("m");
        *state.cursor.lock().await = max;
    }

    eprintln!(
        "[holder] pid={} address={} port={} heartbeat={}s poll={}s push={}",
        std::process::id(),
        args.address,
        args.port,
        args.heartbeat_secs,
        args.poll_secs,
        args.push
    );

    if let Some(after) = args.simulate_hang_after_secs {
        let st = state.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(after)).await;
            eprintln!("[holder] *** SIMULATING HANG NOW (no more heartbeats or keepalives) ***");
            st.hung.store(true, Ordering::SeqCst);
        });
    }

    // Heartbeat task (TTL liveness) on its own connection.
    {
        let st = state.clone();
        let hb = connect().await?;
        let address = args.address.clone();
        let interval = args.heartbeat_secs.max(1);
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(interval));
            loop {
                tick.tick().await;
                if st.hung.load(Ordering::SeqCst) {
                    continue;
                }
                match hb
                    .execute("UPDATE leases SET heartbeat_at = now() WHERE address=$1", &[&address])
                    .await
                {
                    Ok(_) => st.last_heartbeat_ms.store(now_ms(), Ordering::SeqCst),
                    Err(e) => eprintln!("[holder] heartbeat error: {e}"),
                }
            }
        });
    }

    // Poll task (cursor-based delivery backstop) on its own connection.
    {
        let st = state.clone();
        let poll = connect().await?;
        let address = args.address.clone();
        let interval = args.poll_secs.max(1);
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(interval));
            loop {
                tick.tick().await;
                if st.hung.load(Ordering::SeqCst) {
                    continue;
                }
                drain_new(&poll, &address, &st, "poll").await;
            }
        });
    }

    // Optional push: a LISTEN connection signals, a query connection drains.
    if args.push {
        let notify_new = Arc::new(Notify::new());

        // LISTEN connection, driven via poll_message.
        {
            let (listen_client, listen_conn) = pg_config()?.connect(make_tls()?).await?;
            let notify_new = notify_new.clone();
            let address = args.address.clone();
            tokio::spawn(async move {
                let mut conn = listen_conn;
                let messages = stream::poll_fn(move |cx| conn.poll_message(cx));
                tokio::pin!(messages);
                while let Some(msg) = messages.next().await {
                    match msg {
                        Ok(AsyncMessage::Notification(note)) => {
                            if let Ok(p) = serde_json::from_str::<NotifyPayload>(note.payload()) {
                                if p.address == address {
                                    notify_new.notify_one();
                                }
                            }
                        }
                        Ok(_) => {}
                        Err(e) => {
                            eprintln!("[holder] listen conn error: {e}");
                            break;
                        }
                    }
                }
            });
            listen_client
                .batch_execute(&format!("LISTEN {NOTIFY_CHANNEL}"))
                .await?;
            Box::leak(Box::new(listen_client));
            eprintln!("[holder] push enabled (LISTEN {NOTIFY_CHANNEL})");
        }

        // Drain-on-signal task.
        {
            let st = state.clone();
            let push_conn = connect().await?;
            let address = args.address.clone();
            tokio::spawn(async move {
                loop {
                    notify_new.notified().await;
                    if st.hung.load(Ordering::SeqCst) {
                        continue;
                    }
                    drain_new(&push_conn, &address, &st, "push").await;
                }
            });
        }
    }

    let listener = TcpListener::bind(("127.0.0.1", args.port)).await?;
    eprintln!("[holder] listening on 127.0.0.1:{}", args.port);
    loop {
        let (stream, _) = listener.accept().await?;
        let st = state.clone();
        tokio::spawn(async move {
            if let Err(e) = handle_conn(stream, st).await {
                eprintln!("[holder] conn error: {e}");
            }
        });
    }
}

async fn handle_conn(stream: TcpStream, st: Arc<State>) -> Result<()> {
    let (read_half, mut write_half) = stream.into_split();
    let mut reader = BufReader::new(read_half);
    let mut line = String::new();
    if reader.read_line(&mut line).await? == 0 {
        return Ok(());
    }
    let req: Request = serde_json::from_str(line.trim())?;

    match req {
        Request::Ping => {
            let age = now_ms() - st.last_heartbeat_ms.load(Ordering::SeqCst);
            send(&mut write_half, &Frame::Pong { heartbeat_age_ms: age }).await
        }
        Request::Wait { timeout_ms, .. } => {
            let deadline = Instant::now() + Duration::from_millis(timeout_ms);
            let mut ka = tokio::time::interval(st.keepalive);
            ka.tick().await; // consume immediate tick
            loop {
                if let Some(msg) = {
                    let mut q = st.queue.lock().await;
                    q.pop_front()
                } {
                    return send(
                        &mut write_half,
                        &Frame::Message {
                            id: msg.id,
                            address: msg.address,
                            body: msg.body,
                            attention: msg.attention,
                            sent_at_ms: msg.sent_at_ms,
                            buffered_at_ms: msg.buffered_at_ms,
                        },
                    )
                    .await;
                }
                if st.hung.load(Ordering::SeqCst) {
                    // Wedge: never respond again, so the waiter's hang timer fires.
                    std::future::pending::<()>().await;
                }
                let remaining = deadline.saturating_duration_since(Instant::now());
                tokio::select! {
                    _ = st.notify.notified() => continue,
                    _ = ka.tick() => {
                        let age = now_ms() - st.last_heartbeat_ms.load(Ordering::SeqCst);
                        send(&mut write_half, &Frame::Keepalive { heartbeat_age_ms: age }).await?;
                    }
                    _ = tokio::time::sleep(remaining) => {
                        return send(&mut write_half, &Frame::Timeout).await;
                    }
                }
            }
        }
    }
}

async fn send<W: AsyncWriteExt + Unpin>(w: &mut W, frame: &Frame) -> Result<()> {
    let mut s = serde_json::to_string(frame)?;
    s.push('\n');
    w.write_all(s.as_bytes()).await?;
    w.flush().await?;
    Ok(())
}
