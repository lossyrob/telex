//! Resident holder: the answerback drum. Backend-generic — runs over Postgres or
//! SQLite via the `Backend` trait. Holds the lease (TTL heartbeat), learns of
//! messages (poll-with-cursor; optional `--push` LISTEN/NOTIFY on Postgres),
//! buffers them, and serves ephemeral waiters over a local TCP socket.

use anyhow::Result;
use clap::Parser;
use futures_util::{stream, StreamExt};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use telex_spike::{make_backend, now_ms, pg_config, Backend, Frame, NotifyPayload, Request, NOTIFY_CHANNEL};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{Mutex, Notify};
use tokio_postgres::AsyncMessage;

#[derive(Parser)]
struct Args {
    #[arg(long)]
    address: String,
    #[arg(long, default_value_t = 47655)]
    port: u16,
    #[arg(long, default_value = "postgres")]
    backend: String,
    #[arg(long, default_value = "telex-spike.db")]
    db: String,
    #[arg(long, default_value_t = 5)]
    heartbeat_secs: u64,
    #[arg(long, default_value_t = 1)]
    poll_secs: u64,
    #[arg(long, default_value_t = 3)]
    keepalive_secs: u64,
    #[arg(long, default_value = "spike-holder")]
    occupant: String,
    /// Enable LISTEN/NOTIFY push (Postgres only) in addition to the poll backstop.
    #[arg(long)]
    push: bool,
    #[arg(long)]
    simulate_hang_after_secs: Option<u64>,
}

#[derive(Clone)]
struct Buffered {
    id: i64,
    address: String,
    body: String,
    attention: String,
    sent_at_ms: i64,
    buffered_at_ms: i64,
}

struct State {
    queue: Mutex<VecDeque<Buffered>>,
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

async fn drain_new(backend: &Arc<dyn Backend>, address: &str, st: &State, source: &str) {
    let mut cur = st.cursor.lock().await;
    let rows = match backend.fetch_after(address, *cur).await {
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
    for m in rows {
        let lag = if m.sent_at_ms > 0 {
            format!("{} ms", recv - m.sent_at_ms)
        } else {
            "n/a".into()
        };
        eprintln!("[holder] buffered id={} via {source} lag={lag}", m.id);
        *cur = m.id;
        q.push_back(Buffered {
            id: m.id,
            address: m.address,
            body: m.body,
            attention: m.attention,
            sent_at_ms: m.sent_at_ms,
            buffered_at_ms: recv,
        });
    }
    drop(q);
    drop(cur);
    st.notify.notify_waiters();
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let backend = make_backend(&args.backend, &args.db).await?;
    let state = Arc::new(State {
        queue: Mutex::new(VecDeque::new()),
        notify: Notify::new(),
        cursor: Mutex::new(0),
        last_heartbeat_ms: AtomicI64::new(now_ms()),
        hung: AtomicBool::new(false),
        keepalive: Duration::from_secs(args.keepalive_secs),
    });

    backend.ensure_address(&args.address, "spike address").await?;
    backend
        .claim_lease(&args.address, &args.occupant, &hostname(), &whoami())
        .await?;
    *state.cursor.lock().await = backend.max_id(&args.address).await?;

    eprintln!(
        "[holder] pid={} backend={} address={} port={} heartbeat={}s poll={}s push={}",
        std::process::id(),
        backend.kind(),
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
            eprintln!("[holder] *** SIMULATING HANG NOW ***");
            st.hung.store(true, Ordering::SeqCst);
        });
    }

    // Heartbeat task.
    {
        let st = state.clone();
        let backend = backend.clone();
        let address = args.address.clone();
        let interval = args.heartbeat_secs.max(1);
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(interval));
            loop {
                tick.tick().await;
                if st.hung.load(Ordering::SeqCst) {
                    continue;
                }
                match backend.heartbeat(&address).await {
                    Ok(_) => st.last_heartbeat_ms.store(now_ms(), Ordering::SeqCst),
                    Err(e) => eprintln!("[holder] heartbeat error: {e}"),
                }
            }
        });
    }

    // Poll task (delivery backstop for both backends).
    {
        let st = state.clone();
        let backend = backend.clone();
        let address = args.address.clone();
        let interval = args.poll_secs.max(1);
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(interval));
            loop {
                tick.tick().await;
                if st.hung.load(Ordering::SeqCst) {
                    continue;
                }
                drain_new(&backend, &address, &st, "poll").await;
            }
        });
    }

    // Optional Postgres push.
    if args.push && backend.kind() == "postgres" {
        let notify_new = Arc::new(Notify::new());
        {
            let (listen_client, listen_conn) = pg_config()?.connect(telex_spike::make_tls()?).await?;
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
        {
            let st = state.clone();
            let backend = backend.clone();
            let address = args.address.clone();
            tokio::spawn(async move {
                loop {
                    notify_new.notified().await;
                    if st.hung.load(Ordering::SeqCst) {
                        continue;
                    }
                    drain_new(&backend, &address, &st, "push").await;
                }
            });
        }
    } else if args.push {
        eprintln!("[holder] --push ignored: backend {} has no native push", backend.kind());
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
            ka.tick().await;
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
